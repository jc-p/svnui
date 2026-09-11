//! 危险级定级表。
//!
//! 定级依据只有一个问题：**误触一次，损失能不能挽回？**

/// 操作标识。用于错误与确认文案，不参与 svn 参数拼装。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Status,
    Diff,
    Log,
    Info,
    Add,
    Remove,
    Revert,
    Commit,
    Update,
    Resolve,
    Cleanup,
    PurgeMine,
}

impl std::fmt::Display for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Op::Status => "status",
            Op::Diff => "diff",
            Op::Log => "log",
            Op::Info => "info",
            Op::Add => "add",
            Op::Remove => "remove",
            Op::Revert => "revert",
            Op::Commit => "commit",
            Op::Update => "update",
            Op::Resolve => "resolve",
            Op::Cleanup => "cleanup",
            Op::PurgeMine => "purge-mine",
        };
        f.write_str(s)
    }
}

/// 危险等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Danger {
    /// 只读。
    Safe,
    /// 可撤销（如 add 可 revert 回来）。
    Low,
    /// 会改变状态但通常可恢复（commit 后可再提交修复）。
    Medium,
    /// **不可撤销**：丢本地改动或删未纳入版本的文件。
    High,
}

impl Danger {
    pub fn label(self) -> &'static str {
        match self {
            Danger::Safe => "只读",
            Danger::Low => "低危",
            Danger::Medium => "中危",
            Danger::High => "高危·不可撤销",
        }
    }

    pub fn needs_confirm(self) -> bool {
        matches!(self, Danger::Medium | Danger::High)
    }
}

/// 静态定级。`cleanup` 例外：它本身不危险，但带 `--remove-*` 就是核弹。
pub fn danger_of(op: Op) -> Danger {
    match op {
        Op::Status | Op::Diff | Op::Log | Op::Info => Danger::Safe,
        Op::Add => Danger::Low,
        Op::Remove | Op::Commit | Op::Update | Op::Resolve | Op::PurgeMine => Danger::Medium,
        Op::Revert => Danger::High,
        // 基础 cleanup 只是解锁，安全；带 remove 参数由调用方升级为 High。
        Op::Cleanup => Danger::Low,
    }
}

/// `cleanup` 的专用定级：只要碰了 `--remove-*` 就是高危。
///
/// `--remove-unversioned` 会**永久删除**所有未纳入版本的文件 ——
/// 包括你刚写了一半还没 add 的新文件。这是整个工具里最危险的一个开关。
pub fn cleanup_danger(remove_unversioned: bool, remove_ignored: bool) -> Danger {
    if remove_unversioned || remove_ignored {
        Danger::High
    } else {
        Danger::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_ops_are_safe() {
        for op in [Op::Status, Op::Diff, Op::Log, Op::Info] {
            assert_eq!(danger_of(op), Danger::Safe);
        }
    }

    #[test]
    fn revert_is_high() {
        assert_eq!(danger_of(Op::Revert), Danger::High);
        assert!(danger_of(Op::Revert).needs_confirm());
    }

    #[test]
    fn cleanup_escalates_with_remove_flags() {
        assert_eq!(cleanup_danger(false, false), Danger::Low);
        assert_eq!(cleanup_danger(true, false), Danger::High);
        assert_eq!(cleanup_danger(false, true), Danger::High);
        assert_eq!(cleanup_danger(true, true), Danger::High);
    }

    #[test]
    fn danger_is_ordered() {
        assert!(Danger::High > Danger::Medium);
        assert!(Danger::Medium > Danger::Low);
        assert!(!Danger::Safe.needs_confirm());
    }

    #[test]
    fn op_display_matches_subcommand() {
        // 显示名必须与 CLI 子命令一致，否则错误文案里会给出不存在的命令。
        assert_eq!(Op::Revert.to_string(), "revert");
        assert_eq!(Op::PurgeMine.to_string(), "purge-mine");
    }
}
