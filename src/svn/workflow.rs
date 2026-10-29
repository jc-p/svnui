//! 分支工作流：先固定 URL 和版本，再确认写入；合并仅允许原生 dry-run。

use serde::Serialize;

use crate::domain::{Error, RepoInfo, Result, StatusKind};
use crate::policy::Op;

use super::{parser, RawOutput, StatusOpts, Svn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowOperation {
    Branch,
    Tag,
    Switch,
    MergePreview,
}

impl WorkflowOperation {
    pub fn title(self) -> &'static str {
        match self {
            Self::Branch => "创建分支",
            Self::Tag => "创建标签",
            Self::Switch => "切换分支",
            Self::MergePreview => "合并预演",
        }
    }

    pub fn policy_op(self) -> Op {
        match self {
            Self::Branch => Op::Branch,
            Self::Tag => Op::Tag,
            Self::Switch => Op::Switch,
            Self::MergePreview => Op::MergePreview,
        }
    }

    pub fn is_copy(self) -> bool {
        matches!(self, Self::Branch | Self::Tag)
    }
}

/// URL 必须是同仓库的完整 URL 或 `^/` 路径；revision 在准备阶段解析。
#[derive(Debug, Clone)]
pub struct WorkflowRequest {
    pub operation: WorkflowOperation,
    pub url: String,
    pub revision: Option<String>,
    pub message: String,
}

/// 字段不开放写入，避免调用方在确认后替换目标或版本。
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowPlan {
    operation: WorkflowOperation,
    source: String,
    target: String,
    revision: u64,
    revisions: Option<String>,
    message: String,
    working_url: String,
    working_root: std::path::PathBuf,
}

impl WorkflowPlan {
    pub fn operation(&self) -> WorkflowOperation {
        self.operation
    }

    pub fn describe(&self) -> Vec<String> {
        let mut lines = vec![
            format!("源：{}@{}", self.source, self.revision),
            format!("目标：{}", self.target),
        ];
        if let Some(range) = &self.revisions {
            lines.push(format!("版本区间：{range}（应用两个端点之间的差异）"));
        }
        if !self.operation.is_copy() {
            lines.push(format!("当前工作副本 URL：{}", self.working_url));
        }
        lines.push(match self.operation {
            WorkflowOperation::Branch | WorkflowOperation::Tag => {
                format!(
                    "立即提交仓库副本；不包含本地未提交改动。提交信息：{}",
                    self.message
                )
            }
            WorkflowOperation::Switch => {
                "切换整个工作副本；要求无本地改动，不处理 externals。".into()
            }
            WorkflowOperation::MergePreview => {
                "仅运行 svn merge --dry-run；不会应用改动或提交。".into()
            }
        });
        lines
    }

    /// 写操作在确认之后再次检查本地状态；失败也使缓存失效，因为 switch 可能部分完成。
    pub fn execute(&self, svn: &Svn) -> Result<RawOutput> {
        if svn.root != self.working_root || svn.info()?.url != self.working_url {
            return Err(Error::Parse("工作副本已改变，请重新准备分支操作".into()));
        }
        if self.operation.is_copy() {
            svn.require_new_destination(&self.target)?;
        } else {
            svn.require_clean_workflow()?;
        }

        let source = format!("{}@{}", self.source, self.revision);
        let mut args = match self.operation {
            WorkflowOperation::Branch | WorkflowOperation::Tag => vec![
                "copy".into(),
                "-r".into(),
                self.revision.to_string(),
                "-m".into(),
                self.message.clone(),
                "--".into(),
                source,
                // 空 peg 后缀防止目标名称中的 @ 被误当作版本分隔符。
                format!("{}@", self.target),
            ],
            WorkflowOperation::Switch => vec![
                "switch".into(),
                "--accept".into(),
                "postpone".into(),
                "--ignore-externals".into(),
                "-r".into(),
                self.revision.to_string(),
                "--".into(),
                source,
                ".".into(),
            ],
            WorkflowOperation::MergePreview => vec![
                "merge".into(),
                "--dry-run".into(),
                "--accept".into(),
                "postpone".into(),
            ],
        };
        if self.operation == WorkflowOperation::MergePreview {
            if let Some(range) = &self.revisions {
                args.extend(["-r".into(), range.clone()]);
            }
            args.extend([
                "--".into(),
                format!("{}@{}", self.source, self.revision),
                ".".into(),
            ]);
        }
        let result = svn.op_no_targets(&args);
        if self.operation == WorkflowOperation::Switch {
            crate::cache::invalidate_for(&svn.root);
        }
        result
    }
}

impl Svn {
    /// 所有准备步骤只读。BASE 是复制的默认值，避免把用户尚未更新的远端版本打成标签。
    pub fn prepare_workflow(&self, request: &WorkflowRequest) -> Result<WorkflowPlan> {
        let info = self.info()?;
        let root = info
            .repository_root
            .as_deref()
            .filter(|root| !root.is_empty())
            .ok_or_else(|| Error::Parse("svn info 未返回仓库根 URL".into()))?;
        let url = repository_url(root, &request.url)?;
        let (source, target) = if request.operation.is_copy() {
            if request.message.trim().is_empty() || request.message.contains('\0') {
                return Err(Error::Parse("创建分支/标签必须提供非空提交信息".into()));
            }
            self.require_new_destination(&url)?;
            (info.url.clone(), url)
        } else {
            self.require_clean_workflow()?;
            (url, self.root.to_string_lossy().into_owned())
        };
        let input = request
            .revision
            .as_deref()
            .unwrap_or(if request.operation.is_copy() {
                "BASE"
            } else {
                "HEAD"
            });
        let (revision, revisions) = if request.operation == WorkflowOperation::MergePreview {
            match request.revision.as_deref() {
                Some(range) => {
                    let (start, end) = revision_range(range)?;
                    (start.max(end).to_string(), Some(format!("{start}:{end}")))
                }
                None => ("HEAD".into(), None),
            }
        } else {
            (revision_number(input, info.revision)?, None)
        };
        let remote = self.workflow_info(&source, &revision)?;
        if remote.uuid != info.uuid || remote.repository_root != info.repository_root {
            return Err(Error::Parse("分支操作只允许同一个 SVN 仓库".into()));
        }
        Ok(WorkflowPlan {
            operation: request.operation,
            source: remote.url,
            target,
            revision: remote.revision,
            revisions,
            message: request.message.clone(),
            working_url: info.url,
            working_root: self.root.clone(),
        })
    }

    fn workflow_info(&self, url: &str, revision: &str) -> Result<RepoInfo> {
        let out = self.op_no_targets(&[
            "info".into(),
            "--xml".into(),
            "-r".into(),
            revision.into(),
            "--".into(),
            format!("{url}@{revision}"),
        ])?;
        parser::parse_info_xml(&out.stdout, &self.root.to_string_lossy())
    }

    fn require_new_destination(&self, url: &str) -> Result<()> {
        let (parent, name) = url
            .rsplit_once('/')
            .ok_or_else(|| Error::Parse("目标必须包含父目录和名称".into()))?;
        let name = decode_segment(name)?;
        // svn copy 对已存在的目录会创建子项，必须显式拒绝这种隐式改写目标的行为。
        let entries = self.list(&format!("{parent}@"))?;
        if entries.iter().any(|entry| entry.name == name) {
            return Err(Error::Parse(format!("目标已存在，已拒绝复制：{url}")));
        }
        Ok(())
    }

    fn require_clean_workflow(&self) -> Result<()> {
        let entries = self.status(&StatusOpts::default())?;
        let blocked = entries
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.text,
                    StatusKind::None
                        | StatusKind::Normal
                        | StatusKind::External
                        | StatusKind::Ignored
                ) || !matches!(entry.props, StatusKind::None | StatusKind::Normal)
                    || entry.tree_conflict
                    || entry.locked
                    || entry.switched
                    || entry.copied
            })
            .map(|entry| entry.path.to_string())
            .collect::<Vec<_>>();
        if !blocked.is_empty() {
            return Err(Error::Parse(crate::policy::render_targets(
                "工作副本含改动、未纳管项或切换子树；请先处理，再进行 switch/合并预演：",
                &blocked,
                20,
            )));
        }
        Ok(())
    }
}

fn revision_number(value: &str, base: u64) -> Result<String> {
    match value {
        "HEAD" => Ok(value.into()),
        "BASE" => Ok(base.to_string()),
        _ if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => value
            .parse::<u64>()
            .map(|revision| revision.to_string())
            .map_err(|_| Error::Parse("版本号超出范围".into())),
        _ => Err(Error::Parse("版本必须为非负整数、HEAD 或 BASE".into())),
    }
}

fn revision_range(value: &str) -> Result<(u64, u64)> {
    let (start, end) = value
        .split_once(':')
        .ok_or_else(|| Error::Parse("合并版本区间必须为 N:M，例如 12:18".into()))?;
    let parse = |part: &str| -> Result<u64> {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::Parse("合并区间端点必须为非负整数".into()));
        }
        part.parse()
            .map_err(|_| Error::Parse("合并区间端点超出范围".into()))
    };
    Ok((parse(start)?, parse(end)?))
}

fn repository_url(root: &str, input: &str) -> Result<String> {
    let root = root.trim_end_matches('/');
    let url = if let Some(relative) = input.strip_prefix("^/") {
        format!("{root}/{relative}")
    } else {
        input.to_string()
    };
    let url = url.trim_end_matches('/');
    let prefix = format!("{root}/");
    let path = url.strip_prefix(&prefix).ok_or_else(|| {
        Error::Parse("URL 必须是同仓库的完整 URL 或 ^/ 路径，且不能是仓库根".into())
    })?;
    if path.contains(['?', '#']) || url.chars().any(char::is_control) {
        return Err(Error::Parse("URL 不能包含控制字符、查询串或片段".into()));
    }
    for segment in path.split('/') {
        let decoded = decode_segment(segment)?;
        if decoded.is_empty()
            || matches!(decoded.as_str(), "." | "..")
            || decoded.contains(['/', '\\'])
            || decoded.chars().any(char::is_control)
        {
            return Err(Error::Parse("URL 含空路径、越级路径或非法分隔符".into()));
        }
    }
    Ok(url.into())
}

/// 用于路径边界检查及 svn list 的名称比较；不把百分号编码当作未校验的绕过入口。
fn decode_segment(value: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next().and_then(|byte| (byte as char).to_digit(16));
            let low = input.next().and_then(|byte| (byte as char).to_digit(16));
            match (high, low) {
                (Some(high), Some(low)) => bytes.push((high * 16 + low) as u8),
                _ => return Err(Error::Parse("URL 含无效百分号编码".into())),
            }
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| Error::Parse("URL 路径必须为 UTF-8".into()))
}
