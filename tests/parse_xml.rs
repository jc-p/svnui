//! XML 解析（log -v / info 等非热路径）。
//!
//! ⚠️ quick-xml 的 serde 行为对版本敏感。**这组测试是 `Cargo.toml` 里
//! quick-xml 版本能不能升级的判据** —— 升级后必须仍然全绿。
//! 尤其注意 `<path action="M" kind="file">/trunk/x.rs</path>` 的文本节点（`$text`）。

mod common;

use svnui::svn::parser::{parse_info_xml, parse_log_xml};

#[test]
fn log_xml_parses_revisions_and_paths() {
    let v = parse_log_xml(&common::fixture("log.xml")).expect("log.xml 应解析成功");
    assert_eq!(v.len(), 2);

    let first = &v[0];
    assert_eq!(first.revision, 1234);
    assert_eq!(first.author, "alice");
    assert_eq!(first.path_count(), 3);

    // 文本节点必须取到，否则说明 $text 在该 quick-xml 版本下不工作。
    assert!(first.paths.iter().any(|p| p.path == "/trunk/src/main.rs"), "路径文本不应为空");

    let copied = first.paths.iter().find(|p| p.path == "/trunk/src/new.rs").unwrap();
    assert_eq!(copied.action, 'A');
    // copy_from 是 Option<Utf8PathBuf>。这里转成 &str 比较，
    // 避免在 integration test 里直接依赖 camino（test target 的可见依赖不如 src 内确定）。
    assert_eq!(copied.copy_from.as_ref().map(|p| p.as_str()), Some("/trunk/src/old.rs"));
    assert_eq!(copied.kind, svnui::domain::NodeKind::File);
}

#[test]
fn log_without_paths_gives_empty_vec() {
    let v = parse_log_xml(&common::fixture("log.xml")).unwrap();
    // 第二条 logentry 没有 <paths>
    assert_eq!(v[1].revision, 1233);
    assert!(v[1].paths.is_empty());
}

#[test]
fn log_oneline_keeps_utf8_and_drops_body() {
    let v = parse_log_xml(&common::fixture("log.xml")).unwrap();
    let s = v[0].oneline();
    assert!(s.contains("r1234"));
    assert!(s.contains("中文路径"));
    assert!(!v[1].oneline().contains("正文第二行"));
}

#[test]
fn info_xml_reads_root_url_and_branch() {
    let i = parse_info_xml(&common::fixture("info.xml"), "/fallback").expect("info.xml 应解析成功");
    assert_eq!(i.revision, 1234);
    assert_eq!(i.root, "/repo");
    assert_eq!(i.branch_name().as_deref(), Some("feature-x"));
    assert_eq!(i.repository_root.as_deref(), Some("svn://example.com/repo"));
    assert!(i.uuid.is_some());
}

#[test]
fn info_without_wcroot_falls_back() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<info><entry kind="dir" path="/x" revision="7">
<url>svn://x/repo/trunk</url>
<repository><root>svn://x/repo</root></repository>
</entry></info>"#;
    let i = parse_info_xml(xml, "/fallback").unwrap();
    assert_eq!(i.root, "/fallback");
    assert_eq!(i.branch_name().as_deref(), Some("trunk"));
}
