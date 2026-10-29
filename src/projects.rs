//! 工作副本清单 —— 这台机器上有哪些 svn 项目。
//!
//! # 为什么要有这个模块
//!
//! `svnui` 之前没有任何一处记录"这台机器上有哪些工作副本"：
//! 每次都是从一个目录 discover 出一个仓库，用完就忘。
//! 于是换项目只能"再开一个终端、cd 过去、再敲一次"。
//!
//! # 两个来源，以及为什么暂时没有第三个
//!
//! - **登记**：配置里的 `[[projects]]`，`svnui projects add` 写入。稳定、可起别名。
//! - **历史**：从状态缓存反推（见 [`crate::cache::known_roots`]）。
//!   打开过就自动记住，零配置。
//!
//! 常见设计里还有第三个来源：**递归扫描若干根目录**找 `.svn`。
//! 这里**暂时不做**，因为它同时有三个问题：必须限深并排除
//! `node_modules` / `target`（否则一次扫描几十秒）；结果每次不一样，
//! 列表会飘；而且它仍需用户先告诉我们在哪扫 —— 那跟显式登记没区别，
//! 只多了不确定性。等真有"扫一次补齐"的需求再加。
//!
//! # 写配置文件为什么用文本追加
//!
//! `add` / `remove` **刻意不反序列化再序列化**：那份配置文件每个字段
//! 都带注释，是给用户读的，序列化会把注释和字段顺序全丢掉。
//! 这里只按 `[[projects]]` 块做文本级别的增删。

use std::path::{Path, PathBuf};

use crate::config::Config;

/// 一个已知的工作副本。
#[derive(Debug, Clone)]
pub struct Project {
    /// 工作副本根（绝对路径）。
    pub path: PathBuf,
    /// 显示名：登记的别名，或目录名。
    pub name: String,
    /// 从哪儿知道的这个路径。
    pub source: Source,
    /// 现在能不能直接打开。
    ///
    /// 登记的项目即使打不开也留在列表里 —— 那是用户显式要的，
    /// 但面板得知道它现在的状态，不能让用户选中了才发现打不开。
    pub state: State,
}

/// 项目现在能不能用。
///
/// 刻意拆成三档而不是一个 `bool`：**"目录被删了"和"目录在但不是 svn
/// 工作副本"对用户是完全不同的两件事** —— 前者该从列表里清掉，
/// 后者多半只是还没检出，删掉反而丢了一条有用的登记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// `.svn` 在，可以直接打开。
    Ready,
    /// 目录在，但不是工作副本（还没检出，或 `.svn` 被删了）。
    NotWorkingCopy,
    /// 目录本身已经不在磁盘上了。
    Missing,
}

/// 项目的出处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 配置里登记的。
    Registered,
    /// 打开过，从状态缓存反推出来的。
    History,
}

/// 列出所有已知项目。按名字排序，同路径只出现一次。
///
/// **不跑任何 svn 命令**：信息全部来自配置和缓存文件（纯读盘），
/// 所以打开列表是瞬时的 —— 面板一旦要联网或跑 svn，切换项目就变慢了。
pub fn list(cfg: &Config) -> Vec<Project> {
    let mut out: Vec<Project> = Vec::new();

    for e in &cfg.projects {
        let path = expand(&e.path);
        if path.as_os_str().is_empty() {
            continue;
        }
        let name = if e.name.trim().is_empty() {
            dir_name(&path)
        } else {
            e.name.trim().to_string()
        };
        push(&mut out, path, name, Source::Registered);
    }

    for root in crate::cache::known_roots() {
        // 只收现在还能打开的：缓存里残留的是"以前打开过"，
        // 不是"现在还在用" —— 把已经删掉的仓库列出来只是噪音。
        if state_of(&root) != State::Ready {
            continue;
        }
        push(&mut out, root.clone(), dir_name(&root), Source::History);
    }

    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    out
}

/// 追加一个项目，返回给人看的一句话（含写入的文件路径）。
pub fn add(path: &Path, name: Option<&str>) -> Result<String, String> {
    let path = absolute(path).ok_or_else(|| format!("拿不到 {} 的绝对路径", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{} 不是目录", path.display()));
    }

    let file = crate::config::Config::path().ok_or("无法确定配置文件路径")?;
    // 文件不存在就先生成一份带注释的默认配置，别往空文件里只塞一段 projects。
    if !file.is_file() {
        crate::config::Config::ensure_file();
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();

    let shown = short_path(&path);
    if blocks_of(&text).iter().any(|b| b.path.as_deref() == Some(shown.as_str())) {
        return Err(format!("{} 已经在列表里了", shown));
    }

    let mut out = text;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n[[projects]]\n");
    let label = match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => dir_name(&path),
    };
    out.push_str(&format!("name = \"{}\"\npath = \"{}\"\n", escape(&label), escape(&shown)));

    std::fs::write(&file, out).map_err(|e| format!("写入 {} 失败：{e}", file.display()))?;

    // 登记一个还没检出的目录是合法的（先占位、后检出），
    // 但得说清楚 —— 否则回头在列表里看到它打不开会以为是坏了。
    let mut msg = format!("已登记 {label} → {shown}");
    if state_of(&path) != State::Ready {
        msg.push_str("\n注意：它还不是 svn 工作副本（.svn 不在），要先检出才能打开。");
    }
    msg.push_str(&format!("\n写入 {}", file.display()));
    Ok(msg)
}

/// 从配置里移除一个项目。
pub fn remove(path: &Path) -> Result<String, String> {
    let path = absolute(path).ok_or_else(|| format!("拿不到 {} 的绝对路径", path.display()))?;
    let file = crate::config::Config::path().ok_or("无法确定配置文件路径")?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| format!("读不到 {}（{}）", file.display(), e))?;

    let shown = short_path(&path);
    let blocks = blocks_of(&text);
    let hit = blocks.iter().find(|b| b.path.as_deref() == Some(shown.as_str()));
    let Some(hit) = hit else {
        return Err(format!("列表里没有 {}", shown));
    };

    let lines: Vec<&str> = text.lines().collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut dropped = false;
    for (i, line) in lines.iter().enumerate() {
        // 整块丢弃：这段里可能有用户写的注释，跟着一起走才不会留下孤儿注释。
        if i >= hit.start && i < hit.end {
            dropped = true;
            continue;
        }
        kept.push(line);
    }
    if !dropped {
        return Err(format!("列表里没有 {}", shown));
    }

    let mut out = kept.join("\n");
    out.push('\n');
    std::fs::write(&file, out).map_err(|e| format!("写入 {} 失败：{e}", file.display()))?;
    Ok(format!("已从列表移除 {}", shown))
}

/// 配置文件里的一个 `[[projects]]` 段。
struct Block {
    /// 行号区间 `[start, end)`，相对 `text.lines()`。
    start: usize,
    end: usize,
    path: Option<String>,
}

/// 切出所有 `[[projects]]` 段。
///
/// 段的边界：从 `[[projects]]` 那行起，到下一个以 `[` 开头的行（或文件尾）。
fn blocks_of(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;

    for (i, raw) in lines.iter().enumerate() {
        let t = raw.trim();
        if t == "[[projects]]" {
            // 两个段紧挨着时，前一段的终点就是这一行 —— 不先收掉就会被覆盖丢掉。
            if let Some(s) = start.take() {
                out.push(Block { start: s, end: i, path: path_in(&lines[s..i]) });
            }
            start = Some(i);
        } else if t.starts_with('[') {
            if let Some(s) = start.take() {
                out.push(Block { start: s, end: i, path: path_in(&lines[s..i]) });
            }
        }
    }
    if let Some(s) = start {
        out.push(Block { start: s, end: lines.len(), path: path_in(&lines[s..]) });
    }
    out
}

/// 取段里的 `path` 值。
///
/// 段里第一行是 `[[projects]]`，不匹配就继续找下一行 ——
/// 这里**不能用 `?`**：那会让它直接返回 `None`，于是每个段都"没有 path"。
fn path_in(block: &[&str]) -> Option<String> {
    for raw in block {
        let t = raw.trim();
        let rest = match t.strip_prefix("path") {
            Some(r) => r.trim_start(),
            None => continue,
        };
        let rest = match rest.strip_prefix('=') {
            Some(r) => r.trim_start(),
            None => continue,
        };
        let value = rest.trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn push(out: &mut Vec<Project>, path: PathBuf, name: String, source: Source) {
    if out.iter().any(|p| p.path == path) {
        return;
    }
    out.push(Project { state: state_of(&path), path, name, source });
}

/// 判定一个目录现在的状态。
fn state_of(path: &Path) -> State {
    if !path.exists() {
        State::Missing
    } else if path.join(".svn").exists() {
        State::Ready
    } else {
        State::NotWorkingCopy
    }
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// 展开 `~`。只对 `~` 或 `~/...` 生效，其他原样返回。
fn expand(p: &str) -> PathBuf {
    if p == "~" {
        return std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    }
    match p.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .unwrap_or_else(|| PathBuf::from(p)),
        None => PathBuf::from(p),
    }
}

/// 绝对路径。`canonicalize` 会解析符号链接和 `..`，失败时退回原样拼接。
fn absolute(p: &Path) -> Option<PathBuf> {
    p.canonicalize()
        .ok()
        .or_else(|| std::env::current_dir().ok().map(|c| c.join(p)))
}

/// 显示 / 存储形态：能缩成 `~` 就缩。
///
/// 换机器时绝对路径没意义，而面板宽度有限 —— `~` 也更好认。
pub fn short_path(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var_os("HOME") {
        Some(h) => {
            let home = PathBuf::from(h).display().to_string();
            match s.strip_prefix(&home) {
                Some(rest) if rest.starts_with('/') => format!("~{rest}"),
                Some("") => "~".to_string(),
                _ => s,
            }
        }
        None => s,
    }
}

/// TOML 基本字符串里只需要转义这几个。
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, name: &str) -> crate::config::ProjectEntry {
        crate::config::ProjectEntry { name: name.to_string(), path: path.to_string() }
    }

    /// 登记优先，且同路径只出现一次。
    #[test]
    fn same_path_appears_once() {
        let cfg = Config { projects: vec![entry("/a/x", "别名"), entry("/a/x", "")], ..Default::default() };
        let out = list(&cfg);
        assert_eq!(out.len(), 1, "同一路径不能列两遍");
        assert_eq!(out[0].name, "别名", "登记的名字要保住");
    }

    /// 没写 name 就用目录名 —— 不然列表里一片空白。
    #[test]
    fn missing_name_falls_back_to_dir_name() {
        let cfg = Config { projects: vec![entry("/tmp/proj-demo", "")], ..Default::default() };
        let out = list(&cfg);
        assert_eq!(out[0].name, "proj-demo");
    }

    /// `~` 要展开，否则配置里的路径永远是错的。
    #[test]
    fn tilde_expands_to_home() {
        let home = std::env::var("HOME").unwrap_or_default();
        assert_eq!(expand("~/work").display().to_string(), format!("{home}/work"));
        // 中间出现的 ~ 不是 home 缩写，不能动
        assert_eq!(expand("/a/~/b").display().to_string(), "/a/~/b");
    }

    /// 存进配置要缩回 `~`：换机器时绝对路径没意义。
    #[test]
    fn stored_path_uses_tilde_when_possible() {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        assert_eq!(short_path(&home.join("work/x")), "~/work/x");
        assert_eq!(short_path(Path::new("/opt/other")), "/opt/other");
    }

    #[test]
    fn block_scanning_finds_every_project_section() {
        let text = "# 注释\n[llm]\nenabled = false\n\n[[projects]]\nname = \"a\"\npath = \"/a\"\n\n[[projects]]\npath = \"/b\"\n\n[log]\npage_size = 100\n";
        let blocks = blocks_of(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].path.as_deref(), Some("/a"));
        assert_eq!(blocks[1].path.as_deref(), Some("/b"));
    }

    /// 最后一个段没有后续段落兜底，必须也能识别 —— 否则删不掉它。
    #[test]
    fn trailing_block_at_end_of_file_is_found() {
        let text = "[[projects]]\npath = \"/last\"\n";
        let blocks = blocks_of(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].path.as_deref(), Some("/last"));
    }

    /// 转义要挡住路径里的引号 —— 一个引号能让整份配置解析失败。
    #[test]
    fn escaping_keeps_toml_valid() {
        assert_eq!(escape("a\"b"), "a\\\"b");
        assert_eq!(escape("a\\b"), "a\\\\b");
    }
}
