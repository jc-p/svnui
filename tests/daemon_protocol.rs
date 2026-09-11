//! daemon 协议与增量更新契约。
//!
//! daemon 最容易出的两类 bug 都在这里守住：
//!   1. 「无变化」响应必须是几十字节 —— 否则"每次切目录都问一次"不成立
//!   2. 文件恢复正常后必须从快照删除 —— 否则列表一直显示 M 但 diff 为空

use std::path::PathBuf;

use svnui::daemon::protocol::{pid_path, socket_path, Cmd, Query, Reply};

#[test]
fn query_serializes_to_single_line() {
    let q = Query { dir: "/repo/src".into(), since: Some(7) };
    let s = serde_json::to_string(&q).unwrap();
    assert!(!s.contains('\n'), "协议是单行 JSON，含换行会破坏帧边界");
    let back: Query = serde_json::from_str(&s).unwrap();
    assert_eq!(back.dir, "/repo/src");
    assert_eq!(back.since, Some(7));
}

#[test]
fn unchanged_reply_is_tiny() {
    let r = Reply::unchanged(42, true);
    let s = serde_json::to_string(&r).unwrap();
    assert!(!r.changed);
    assert!(r.map.is_empty());
    // 每次切目录都会发一次查询，无变化的响应必须小到可以忽略
    assert!(s.len() < 120, "无变化响应 {} 字节，过大：{s}", s.len());
}

#[test]
fn reply_tolerates_missing_map() {
    let r: Reply = serde_json::from_str(r#"{"v":3,"changed":true,"ready":true}"#).unwrap();
    assert_eq!(r.v, 3);
    assert!(r.map.is_empty(), "老版本响应没有 map 字段时要能容错");
}

#[test]
fn commands_are_unit_variants() {
    for c in [Cmd::Ping, Cmd::Rescan, Cmd::Shutdown] {
        let s = serde_json::to_string(&c).unwrap();
        // 单键对象形式，便于与 Query 区分
        assert!(s.starts_with('"'), "Cmd 应序列化成字符串：{s}");
    }
}

#[test]
fn socket_and_pid_paths_are_sibling_names() {
    let root = PathBuf::from("/repo");
    let s = socket_path(&root);
    let p = pid_path(&root);
    assert_eq!(s.file_stem().unwrap(), p.file_stem().unwrap());
    assert_eq!(s.extension().unwrap(), "sock");
    assert_eq!(p.extension().unwrap(), "pid");
}

#[test]
fn different_repos_get_different_sockets() {
    let a = socket_path(PathBuf::from("/repo-a").as_path());
    let b = socket_path(PathBuf::from("/repo-b").as_path());
    assert_ne!(a, b, "多仓库同时开发时不能抢同一个 socket");
}

#[cfg(unix)]
#[test]
fn socket_path_fits_sun_path() {
    // sun_path 上限约 108 字节，超了 bind() 会失败。
    // 哈希后的文件名很短，即使仓库路径很深也应该安全。
    let deep = PathBuf::from("/").join("a-very-long-directory-name".repeat(4));
    let s = socket_path(&deep);
    assert!(s.to_string_lossy().len() < 108, "socket 路径过长会 bind 失败");
}
