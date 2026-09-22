//! `svn --xml` 输出的反序列化。
//!
//! 只用于**非热路径**：`log -v`、`info`。状态查询走 `porcelain.rs`（文本，快一个量级）。
//!
//! ⚠️ quick-xml 的 serde 有几个坑，改动这里时留意：
//! - 属性要用 `@name` 前缀，且需要 `serde` feature。
//! - 重复子元素用 `Vec<T>` + `#[serde(default)]`。
//! - 缺失的可选字段用 `Option<T>`，不要 `#[serde(default)]` + 非 Option 混用。

use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::domain::{Error, LogEntry, LogPath, NodeKind, RepoInfo, Result, StatusEntry, StatusKind};

// ---------------------------------------------------------------- status

#[derive(Debug, Deserialize)]
struct StatusXml {
    #[serde(default)]
    target: Vec<StatusTargetXml>,
}

#[derive(Debug, Deserialize)]
struct StatusTargetXml {
    #[serde(default)]
    entry: Vec<StatusEntryXml>,
}

#[derive(Debug, Deserialize)]
struct StatusEntryXml {
    #[serde(rename = "@path")]
    path: String,
    #[serde(rename = "wc-status")]
    wc: Option<WcStatusXml>,
}

#[derive(Debug, Deserialize)]
struct WcStatusXml {
    #[serde(rename = "@item")]
    item: StatusKind,
    #[serde(rename = "@props", default)]
    props: Option<StatusKind>,
    #[serde(rename = "@tree-conflicted", default)]
    tree_conflicted: Option<String>,
    #[serde(rename = "@copied", default)]
    copied: Option<String>,
    #[serde(rename = "@switched", default)]
    switched: Option<String>,
}

/// 解析 `svn status --xml`。
///
/// 路径是 `@path` 拼接：target 下的 entry 的 path 需要拼上 target 的 path。
/// 这里简化处理为直接用 entry 的 path（svn 对单目标查询时它就是相对路径）。
pub fn parse_status_xml(xml: &str) -> Result<Vec<StatusEntry>> {
    let doc: StatusXml = quick_xml::de::from_str(xml).map_err(|e| Error::Parse(e.to_string()))?;

    let mut out = Vec::new();
    for t in doc.target {
        for e in t.entry {
            let wc = match e.wc {
                Some(w) => w,
                None => continue,
            };
            // 归一：XML 的 item="normal" 与文本路径第 1 列的空格都表示"无改动"，
            // 必须收敛到同一个 StatusKind::None。否则同一份状态走两条解析路径会
            // 得到不同枚举值，任何 `entry.text == StatusKind::Normal` 的判断都会漏判。
            let text = match wc.item {
                StatusKind::Normal => StatusKind::None,
                other => other,
            };
            let mut entry = StatusEntry {
                path: Utf8PathBuf::from(&e.path),
                text,
                props: wc.props.unwrap_or(StatusKind::None),
                locked: false,
                copied: wc.copied.as_deref() == Some("true"),
                switched: wc.switched.as_deref() == Some("true"),
                lock_token: None,
                tree_conflict: wc.tree_conflicted.as_deref() == Some("true"),
                kind: NodeKind::Unknown,
            };
            // XML 里 External 有时出现在 item=unversioned 且 switched 为 true 的组合，
            // 这里只做保守归一：不擅自改 item，交给上层决定。
            if entry.switched && entry.text == StatusKind::Unversioned {
                entry.kind = NodeKind::Dir;
            }
            out.push(entry);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------- log

#[derive(Debug, Deserialize)]
struct LogXml {
    #[serde(default, rename = "logentry")]
    entries: Vec<LogEntryXml>,
}

#[derive(Debug, Deserialize)]
struct LogEntryXml {
    #[serde(rename = "@revision")]
    revision: u64,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    paths: Option<LogPathsXml>,
}

#[derive(Debug, Deserialize)]
struct LogPathsXml {
    #[serde(default, rename = "path")]
    items: Vec<LogPathXml>,
}

#[derive(Debug, Deserialize)]
struct LogPathXml {
    #[serde(rename = "@action")]
    action: String,
    #[serde(rename = "@kind", default)]
    kind: Option<String>,
    #[serde(rename = "@copyfrom-path", default)]
    copy_from: Option<String>,
    #[serde(rename = "$text")]
    text: Option<String>,
}

/// 解析 `svn log --xml [-v]`。不带 `-v` 时 `paths` 为空。
pub fn parse_log_xml(xml: &str) -> Result<Vec<LogEntry>> {
    let doc: LogXml = quick_xml::de::from_str(xml).map_err(|e| Error::Parse(e.to_string()))?;

    Ok(doc
        .entries
        .into_iter()
        .map(|e| {
            let paths = e
                .paths
                .map(|p| {
                    p.items
                        .into_iter()
                        .map(|i| LogPath {
                            action: i.action.chars().next().unwrap_or('M'),
                            path: Utf8PathBuf::from(i.text.unwrap_or_default()),
                            copy_from: i.copy_from.map(Utf8PathBuf::from),
                            kind: match i.kind.as_deref() {
                                Some("dir") => NodeKind::Dir,
                                Some("file") => NodeKind::File,
                                Some("symlink") => NodeKind::Symlink,
                                _ => NodeKind::Unknown,
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();

            LogEntry {
                revision: e.revision,
                author: e.author.unwrap_or_default(),
                date: e.date.unwrap_or_default(),
                msg: e.msg.unwrap_or_default(),
                paths,
            }
        })
        .collect())
}

// ------------------------------------------------------------------ info

#[derive(Debug, Deserialize)]
struct InfoXml {
    entry: Option<InfoEntryXml>,
}

#[derive(Debug, Deserialize)]
struct InfoEntryXml {
    #[serde(rename = "@revision")]
    revision: u64,
    #[serde(default)]
    url: Option<String>,
    #[serde(rename = "relative-url", default)]
    relative_url: Option<String>,
    #[serde(default)]
    repository: Option<InfoRepoXml>,
    #[serde(rename = "wc-info", default)]
    wc_info: Option<InfoWcXml>,
}

#[derive(Debug, Deserialize)]
struct InfoRepoXml {
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InfoWcXml {
    #[serde(rename = "wcroot-abspath", default)]
    wcroot_abspath: Option<String>,
}

/// 解析 `svn info --xml`。
pub fn parse_info_xml(xml: &str, fallback_root: &str) -> Result<RepoInfo> {
    let doc: InfoXml = quick_xml::de::from_str(xml).map_err(|e| Error::Parse(e.to_string()))?;
    let e = doc.entry.ok_or_else(|| Error::Parse("info: 没有 entry 元素".into()))?;

    let root = e
        .wc_info
        .and_then(|w| w.wcroot_abspath)
        .unwrap_or_else(|| fallback_root.to_string());

    Ok(RepoInfo {
        root: Utf8PathBuf::from(root),
        url: e.url.unwrap_or_default(),
        relative_url: e.relative_url,
        revision: e.revision,
        repository_root: e.repository.as_ref().and_then(|r| r.root.clone()),
        uuid: e.repository.and_then(|r| r.uuid),
    })
}
// ---------------------------------------------------------------- list

/// `svn list --xml` 的一个条目。
///
/// 远端条目**没有本地状态**（不是工作副本里的文件），
/// 所以只有 name / kind / size / 最后修改的 revision-info。
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// 文件大小（字节）。目录没有。
    pub size: Option<u64>,
    /// 最后改动的 revision。
    pub revision: Option<u64>,
    pub author: Option<String>,
    /// 已压成 `YYYY-MM-DD`。原始值是 ISO8601（`2024-05-01T03:04:05.123456Z`），
    /// 全长 27 字符，按字节截 10 正好是日期部分（ASCII 边界安全）。
    pub date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListXml {
    #[serde(default, rename = "list")]
    lists: Vec<ListEl>,
}

#[derive(Debug, Deserialize)]
struct ListEl {
    #[serde(default, rename = "entry")]
    entries: Vec<ListEntryXml>,
}

#[derive(Debug, Deserialize)]
struct ListEntryXml {
    #[serde(rename = "@kind", default)]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    commit: Option<ListCommitXml>,
}

#[derive(Debug, Deserialize)]
struct ListCommitXml {
    #[serde(rename = "@revision")]
    revision: Option<u64>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

/// 解析 `svn list --xml`。
///
/// 目录名带尾斜杠是 `svn list` 的文本输出习惯，XML 里没有 ——
/// 统一去掉，面板自己按 `is_dir` 渲染，避免 URL 拼接时多一个斜杠。
pub fn parse_list_xml(xml: &str) -> Result<Vec<DirEntry>> {
    let doc: ListXml = quick_xml::de::from_str(xml).map_err(|e| Error::Parse(e.to_string()))?;

    let mut out = Vec::new();
    for l in doc.lists {
        for e in l.entries {
            let name = e.name.unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let c = e.commit;
            out.push(DirEntry {
                is_dir: e.kind.as_deref() == Some("dir"),
                name,
                size: e.size,
                revision: c.as_ref().and_then(|c| c.revision),
                author: c.as_ref().and_then(|c| c.author.clone()),
                date: c.as_ref().and_then(|c| c.date.clone()).map(|d| {
                    if d.len() >= 10 { d[..10].to_string() } else { d }
                }),
            });
        }
    }
    Ok(out)
}
