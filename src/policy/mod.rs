//! 危险操作护栏。
//!
//! SVN 的写操作里有一批**不可逆**的：`revert` 丢改动、`cleanup --remove-unversioned`
//! 删未纳入版本的文件。这类操作在 yazi 里被误触一次的代价太高，必须在这一层拦住。
//!
//! 设计原则：
//! - **默认不执行**，除非显式 `--yes`
//! - **非 TTY 一律拒绝** —— 脚本/CI 里没法交互确认，静默执行更危险
//! - **`--dry-run` 优先** —— 干跑永远不需要确认

pub mod danger;

pub use danger::{cleanup_danger, danger_of, Danger, Op};

/// 护栏判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 放行。
    Proceed,
    /// 干跑：只打印计划，不执行。
    DryRun,
    /// 需要用户在终端确认。
    NeedConfirm { reason: String },
    /// 一律拒绝：非 TTY 环境下无法确认高危操作。
    Refused { reason: String },
}

/// 核心判定。**纯函数** —— 不读终端、不打印，便于测试与上层自由渲染。
pub fn judge(op: Op, danger: Danger, dry_run: bool, yes: bool, tty: bool) -> Verdict {
    if dry_run {
        return Verdict::DryRun;
    }

    match danger {
        Danger::Safe | Danger::Low => Verdict::Proceed,
        Danger::Medium => {
            if yes {
                Verdict::Proceed
            } else if tty {
                Verdict::NeedConfirm { reason: format!("{op} 会改变工作副本") }
            } else {
                // 中危在非 TTY 下要求显式 --yes，这是"脚本必须知情"的底线。
                Verdict::Refused { reason: format!("{op} 需要显式 --yes（非交互环境）") }
            }
        }
        Danger::High => {
            if yes {
                Verdict::Proceed
            } else if tty {
                Verdict::NeedConfirm { reason: format!("{op} 不可撤销，将丢失本地改动") }
            } else {
                Verdict::Refused { reason: format!("{op} 不可撤销且非交互环境，已拒绝执行") }
            }
        }
    }
}

/// 渲染受影响清单。高危操作执行前必须让用户看见要动哪些文件。
pub fn render_targets(title: &str, targets: &[String], limit: usize) -> String {
    let mut out = String::new();
    out.push_str(title);
    out.push('\n');

    if targets.is_empty() {
        out.push_str("  (无)\n");
        return out;
    }

    for t in targets.iter().take(limit) {
        out.push_str(&format!("  {t}\n"));
    }
    if targets.len() > limit {
        out.push_str(&format!("  ... 还有 {} 项\n", targets.len() - limit));
    }
    out
}

/// 简单的终端 Y/n 确认。
///
/// 刻意不用任何 crate：只需要读一行，且必须**只在实际是 tty 时**才调用
/// （上层已用 `judge` 保证这一点）。
pub fn confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();

    let mut s = String::new();
    match std::io::stdin().read_line(&mut s) {
        Ok(_) => matches!(s.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_always_wins() {
        assert_eq!(judge(Op::Revert, Danger::High, true, true, true), Verdict::DryRun);
        assert_eq!(judge(Op::Revert, Danger::High, true, false, false), Verdict::DryRun);
    }

    #[test]
    fn safe_ops_never_block() {
        assert_eq!(judge(Op::Status, Danger::Safe, false, false, false), Verdict::Proceed);
        assert_eq!(judge(Op::Add, Danger::Low, false, false, false), Verdict::Proceed);
    }

    #[test]
    fn high_danger_needs_yes_or_tty() {
        assert_eq!(judge(Op::Revert, Danger::High, false, true, false), Verdict::Proceed);
        assert!(matches!(judge(Op::Revert, Danger::High, false, false, true), Verdict::NeedConfirm { .. }));
        assert!(matches!(judge(Op::Revert, Danger::High, false, false, false), Verdict::Refused { .. }));
    }

    #[test]
    fn medium_danger_refused_without_tty() {
        assert!(matches!(judge(Op::Commit, Danger::Medium, false, false, false), Verdict::Refused { .. }));
        assert_eq!(judge(Op::Commit, Danger::Medium, false, true, false), Verdict::Proceed);
    }

    #[test]
    fn render_targets_truncates() {
        let v: Vec<String> = (0..20).map(|i| format!("f{i}")).collect();
        let s = render_targets("将回滚：", &v, 5);
        assert!(s.contains("将回滚："));
        assert!(s.contains("f0"));
        assert!(s.contains("还有 15 项"));
    }

    #[test]
    fn render_empty_targets() {
        let s = render_targets("将回滚：", &[], 5);
        assert!(s.contains("(无)"));
    }
}
