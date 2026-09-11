use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use super::NodeKind;

/// `svn log` 里单条改动路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPath {
    /// `A` / `D` / `M` / `R`（R = 替换）。
    pub action: char,
    pub path: Utf8PathBuf,
    /// 若这次改动是 copy 而来，记录来源路径。
    pub copy_from: Option<Utf8PathBuf>,
    pub kind: NodeKind,
}

impl LogPath {
    pub fn new(action: char, path: impl Into<Utf8PathBuf>) -> Self {
        Self {
            action,
            path: path.into(),
            copy_from: None,
            kind: NodeKind::Unknown,
        }
    }
}

/// 一次提交。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub revision: u64,
    pub author: String,
    /// ISO-8601 原文保留（`2026-09-09T10:11:12.345678Z`），不做时区转换。
    pub date: String,
    pub msg: String,
    /// 只有 `svn log -v` 才有内容。
    pub paths: Vec<LogPath>,
}

impl LogEntry {
    /// `r1234  alice  2026-09-09  fix: 修掉中文路径`
    ///
    /// 日期只取到天 —— yazi 弹框里省得横向空间。
    pub fn oneline(&self) -> String {
        let day = if self.date.len() >= 10 {
            &self.date[..10]
        } else {
            &self.date
        };
        let subject = self.msg.lines().next().unwrap_or("").trim();
        let subject = if subject.is_empty() { "(no message)" } else { subject };
        format!("r{:<7} {:<12} {}  {}", self.revision, self.author, day, subject)
    }

    /// 提交信息首行。
    pub fn subject(&self) -> &str {
        self.msg.lines().next().unwrap_or("").trim()
    }

    /// 影响的文件数（`-v` 才有意义）。
    pub fn path_count(&self) -> usize {
        self.paths.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> LogEntry {
        LogEntry {
            revision: 1234,
            author: "alice".into(),
            date: "2026-09-09T10:11:12.345678Z".into(),
            msg: "fix: 中文路径\r\n\r\n详细描述".into(),
            paths: vec![LogPath::new('M', "src/main.rs")],
        }
    }

    #[test]
    fn oneline_truncates_date_and_keeps_utf8() {
        let s = entry().oneline();
        assert!(s.starts_with("r1234"));
        assert!(s.contains("2026-09-09"));
        assert!(s.contains("fix: 中文路径"));
        // 不应把第二行正文带进来
        assert!(!s.contains("详细描述"));
    }

    #[test]
    fn subject_drops_body() {
        assert_eq!(entry().subject(), "fix: 中文路径");
    }

    #[test]
    fn empty_message_has_placeholder() {
        let mut e = entry();
        e.msg = String::new();
        assert!(e.oneline().contains("(no message)"));
    }
}
