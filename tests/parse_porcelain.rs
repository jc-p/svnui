//! 7 列文本解析（状态查询热路径）。
//!
//! ⚠️ 样本是手工构造的（环境无 svnadmin）。若某天 `svn` 改了输出格式，
//! 这些测试会立刻红 —— 那就是需要重新校准的信号。

mod common;

use svnui::domain::StatusKind;
use svnui::svn::porcelain::parse_status;

#[test]
fn parses_all_entries() {
    let v = parse_status(&common::fixture("status_plain.txt"));
    assert_eq!(v.len(), 14, "状态行数量应与样本一致");
}

#[test]
fn covers_every_status_kind_in_sample() {
    let v = parse_status(&common::fixture("status_plain.txt"));
    let find = |p: &str| {
        v.iter().find(|e| e.path == p).unwrap_or_else(|| panic!("缺少 {p}")).clone()
    };

    assert_eq!(find("src/main.rs").text, StatusKind::Modified);
    assert_eq!(find("src/new.rs").text, StatusKind::Added);
    assert_eq!(find("src/replaced.rs").text, StatusKind::Replaced);
    assert_eq!(find("src/old.rs").text, StatusKind::Deleted);
    assert_eq!(find("src/untracked.rs").text, StatusKind::Unversioned);
    assert_eq!(find("src/missing.rs").text, StatusKind::Missing);
    assert_eq!(find("src/obstructed").text, StatusKind::Obstructed);
    assert_eq!(find("target").text, StatusKind::Ignored);
    assert_eq!(find("vendor/ext").text, StatusKind::External);
    assert_eq!(find("src/conflicted.rs").text, StatusKind::Conflicted);
}

#[test]
fn prop_only_change_and_both_change() {
    let v = parse_status(&common::fixture("status_plain.txt"));
    let lib = v.iter().find(|e| e.path == "src/lib.rs").unwrap();
    assert_eq!(lib.text, StatusKind::None);
    assert_eq!(lib.props, StatusKind::Modified);
    assert_eq!(lib.porcelain(), "_M");

    let both = v.iter().find(|e| e.path == "src/both.rs").unwrap();
    assert_eq!(both.text, StatusKind::Modified);
    assert_eq!(both.props, StatusKind::Modified);
    assert_eq!(both.porcelain(), "MM");
}

#[test]
fn copied_flag_is_parsed() {
    let v = parse_status(&common::fixture("status_plain.txt"));
    let c = v.iter().find(|e| e.path == "src/copied.rs").unwrap();
    assert!(c.copied, "第 4 列的 + 应解析为 copied");
}

#[test]
fn utf8_and_spaces_in_path() {
    let v = parse_status(&common::fixture("status_plain.txt"));
    assert!(v.iter().any(|e| e.path == "src/中文 文件.rs"), "中文+空格路径必须原样保留");
}

#[test]
fn conflict_sample_filters_detail_and_summary() {
    let v = parse_status(&common::fixture("status_conflict.txt"));
    assert_eq!(v.len(), 3, "说明行与 Summary 块必须被过滤");

    let text = v.iter().find(|e| e.path == "src/text_conflict.rs").unwrap();
    assert_eq!(text.text, StatusKind::Conflicted);
    assert!(!text.tree_conflict);

    let tree = v.iter().find(|e| e.path == "src/tree_conflict.rs").unwrap();
    assert!(tree.tree_conflict, "第 7 列 C 应解析为树冲突");
    assert_eq!(tree.sign(), 'T');

    let prop = v.iter().find(|e| e.path == "src/prop_conflict.rs").unwrap();
    assert!(prop.tree_conflict);
    assert_eq!(prop.props, StatusKind::Modified);
}

#[test]
fn snapshot_bubbles_conflict_up() {
    use svnui::domain::Snapshot;
    let entries = parse_status(&common::fixture("status_conflict.txt"));
    let mut s = Snapshot::new("/repo");
    for e in entries {
        s.insert(e);
    }
    s.build_bubbles();

    // src 下有 text/tree/prop 三个冲突，冒泡后 src 应显示最严重的（C 或 T）
    let m = s.mark_of("src").expect("src 应有冒泡标记");
    assert_eq!(m.priority, 100);
    assert!(s.conflicts().len() == 3, "三项都算冲突");
}
