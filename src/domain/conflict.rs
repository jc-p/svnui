use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// SVN 有三类冲突，处理方式完全不同，必须分清。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictKind {
    /// 文本冲突：`file` 内容合并失败，伴生 `file.mine` / `file.rOLD` / `file.rNEW`。
    Text,
    /// 属性冲突：`svn:mergeinfo` 之类。
    Property,
    /// 树冲突：本地删除/移动 vs 远端修改。最容易让人懵，也最需要人工判断。
    Tree,
}

impl ConflictKind {
    pub fn sign(self) -> char {
        match self {
            ConflictKind::Text => 'C',
            ConflictKind::Property => 'P',
            ConflictKind::Tree => 'T',
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            ConflictKind::Text => "mine-full / theirs-full / 手动编辑三件套后 resolve",
            ConflictKind::Property => "通常 resolve --accept working 即可",
            ConflictKind::Tree => "需人工判断去留，慎用 theirs-full（可能丢本地改动）",
        }
    }
}

/// 一条冲突，附带冲突产生的"三件套"文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictEntry {
    pub path: Utf8PathBuf,
    pub kind: ConflictKind,
    /// `.mine` / `.r{OLD}` / `.r{NEW}`。解决后应由 svn 自动清理；没清掉就是残留。
    pub trio: Vec<Utf8PathBuf>,
}

impl ConflictEntry {
    /// 是否残留：`resolve` 已把状态清干净，但三件套还在磁盘上。
    ///
    /// `svnui doctor` 用它提示"可以安全 purge-mine"。
    pub fn is_residual(&self, still_conflicted: bool) -> bool {
        !still_conflicted && !self.trio.is_empty()
    }

    /// 生成可能存在的三件套文件名（不查磁盘，纯推算）。
    ///
    /// 由调用方决定要不要 stat —— domain 层零 IO。
    pub fn candidate_trio(path: &Utf8PathBuf) -> Vec<String> {
        let name = match path.file_name() {
            Some(n) => n,
            None => return Vec::new(),
        };
        vec![format!("{name}.mine")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_only_when_resolved_but_files_left() {
        let c = ConflictEntry {
            path: Utf8PathBuf::from("src/a.rs"),
            kind: ConflictKind::Text,
            trio: vec![Utf8PathBuf::from("src/a.rs.mine")],
        };
        assert!(!c.is_residual(true), "仍冲突 → 不是残留");
        assert!(c.is_residual(false), "已解决但文件还在 → 残留");
    }

    #[test]
    fn trio_candidate_uses_file_name() {
        let p = Utf8PathBuf::from("deep/nested/a.rs");
        assert_eq!(ConflictEntry::candidate_trio(&p), vec!["a.rs.mine"]);
    }

    #[test]
    fn tree_conflict_hint_warns_about_data_loss() {
        assert!(ConflictKind::Tree.hint().contains("慎用"));
    }
}
