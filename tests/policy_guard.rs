//! policy 与 pager 的行为契约测试。
//!
//! 这两块都是"没跑起来看不出错、错了代价很高"的类型，必须有测试守着。

use svnui::policy::{cleanup_danger, danger_of, judge, render_targets, Danger, Op, Verdict};

// ------------------------------------------------------------------ 护栏

#[test]
fn dry_run_never_executes() {
    // 干跑优先于一切：即使是高危，也不该要求确认
    assert_eq!(judge(Op::Revert, Danger::High, true, false, false), Verdict::DryRun);
    assert_eq!(judge(Op::Cleanup, Danger::High, true, false, false), Verdict::DryRun);
}

#[test]
fn revert_without_yes_in_script_is_refused() {
    // 这是最重要的一条：CI/脚本里误跑 svnui revert 必须被拦下
    let v = judge(Op::Revert, Danger::High, false, false, false);
    assert!(matches!(v, Verdict::Refused { .. }), "非 TTY + 无 --yes 应拒绝，实际 {v:?}");
}

#[test]
fn revert_with_yes_proceeds() {
    assert_eq!(judge(Op::Revert, Danger::High, false, true, false), Verdict::Proceed);
}

#[test]
fn safe_ops_never_blocked_even_in_script() {
    for op in [Op::Status, Op::Diff, Op::Log, Op::Info] {
        assert_eq!(judge(op, Danger::Safe, false, false, false), Verdict::Proceed);
    }
}

#[test]
fn cleanup_plain_is_low_but_remove_flags_are_high() {
    assert_eq!(cleanup_danger(false, false), Danger::Low);
    assert_eq!(cleanup_danger(true, false), Danger::High);
    assert_eq!(cleanup_danger(false, true), Danger::High);
}

#[test]
fn high_danger_requires_confirm_on_tty() {
    let v = judge(Op::Revert, Danger::High, false, false, true);
    match v {
        Verdict::NeedConfirm { reason } => assert!(reason.contains("不可撤销")),
        other => panic!("TTY 下应要求确认，实际 {other:?}"),
    }
}

#[test]
fn danger_ordering_is_total() {
    assert!(Danger::High > Danger::Medium);
    assert!(Danger::Medium > Danger::Low);
    assert!(Danger::Low > Danger::Safe);
}

#[test]
fn commit_is_medium_not_high() {
    // commit 可再提交修复，不算不可撤销
    assert_eq!(danger_of(Op::Commit), Danger::Medium);
    assert_eq!(danger_of(Op::Revert), Danger::High);
}

// ------------------------------------------------------------------ 清单

#[test]
fn render_targets_shows_overflow_count() {
    let v: Vec<String> = (0..100).map(|i| format!("src/f{i}.rs")).collect();
    let s = render_targets("将回滚：", &v, 50);
    assert!(s.contains("src/f0.rs"));
    assert!(s.contains("还有 50 项"));
    // 只显示 50 条 + 1 行溢出提示
    assert_eq!(s.lines().count(), 52);
}

#[test]
fn render_targets_handles_empty() {
    let s = render_targets("将回滚：", &[], 50);
    assert!(s.contains("(无)"));
}

// ------------------------------------------------------------------ pager

#[test]
fn pager_detect_returns_known_kind() {
    let k = svnui::output::pager::detect();
    assert!(matches!(
        k,
        svnui::output::PagerKind::Delta
            | svnui::output::PagerKind::Bat
            | svnui::output::PagerKind::Less
            | svnui::output::PagerKind::None
    ));
}

#[test]
fn pager_empty_is_noop() {
    assert!(svnui::output::pager::page("").is_ok());
}

#[test]
fn short_output_not_paged() {
    // 短内容走 pager 是负体验（看 3 行还要按 q）
    assert!(svnui::output::pager::page_if_long("a\nb\nc\n", 10).is_ok());
}
