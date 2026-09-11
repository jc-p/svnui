//! 缓存与实时查询的一致性契约。
//!
//! M7 的完成判据是「`--no-cache` 的结果必须与走缓存的结果一致」。
//! 真机上有 svn 时可以这样验：
//!
//! ```bash
//! a=$(svnui status --porcelain --no-cache)
//! b=$(svnui status --porcelain)
//! [ "$a" = "$b" ] && echo OK || diff <(echo "$a") <(echo "$b")
//! ```
//!
//! 这里用 mock 工作副本验证等价的性质：**指纹未变 → 命中且内容一致**。
//! 与 `cache.rs` 的单元测试互补（那边验证失效条件，这边验证命中路径的内容正确性）。

use std::path::PathBuf;
use std::time::Duration;

use svnui::cache::Cache;
use svnui::domain::{NodeKind, Snapshot, StatusEntry, StatusKind};

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("svnui-cig-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    let _ = std::fs::create_dir_all(&p);
    p
}

fn fake_wc(tag: &str) -> PathBuf {
    let root = tmp(tag);
    std::fs::create_dir_all(root.join(".svn")).unwrap();
    std::fs::write(root.join(".svn").join("wc.db"), "db").unwrap();
    std::fs::create_dir_all(root.join("src").join("deep")).unwrap();
    std::fs::write(root.join("src").join("a.txt"), "a").unwrap();
    std::fs::write(root.join("src").join("deep").join("b.txt"), "b").unwrap();
    root
}

fn entry(path: &str, text: StatusKind) -> StatusEntry {
    StatusEntry {
        // StatusEntry.path 是 Utf8PathBuf
        // 用 Snapshot::insert + get 验证时不需要构造，直接塞默认值也行，
        // 但为可读性还是构造一个 —— 通过 domain 暴露的 From<&str>。
        path: Utf8PathBuf::from(path),
        text,
        props: StatusKind::None,
        locked: false,
        copied: false,
        switched: false,
        lock_token: None,
        tree_conflict: false,
        kind: NodeKind::File,
    }
}

fn snapshot_of(root: &str, entries: Vec<StatusEntry>) -> Snapshot {
    let mut s = Snapshot::new(root);
    s.ready = true;
    for e in entries {
        s.insert(e);
    }
    s.build_bubbles();
    s
}

#[test]
fn hit_returns_identical_content() {
    let root = fake_wc("hit");
    let dir = tmp("hit-cache");
    let c = Cache::open_in(&root, &dir);

    let root_s = root.to_string_lossy().to_string();
    let original = snapshot_of(
        &root_s,
        vec![
            entry("src/a.txt", StatusKind::Modified),
            entry("src/deep/b.txt", StatusKind::Added),
            entry("src/old.txt", StatusKind::Deleted),
        ],
    );

    c.save(&original).unwrap();
    let loaded = c.load(Duration::from_secs(300)).expect("指纹未变，必须命中");

    // 条目集合一致
    assert_eq!(loaded.entries.len(), original.entries.len());
    for (p, e) in &original.entries {
        let got = loaded.entries.get(p).unwrap_or_else(|| panic!("缓存丢了 {p}"));
        assert_eq!(got.text, e.text);
        assert_eq!(got.porcelain(), e.porcelain());
    }

    // 变更集一致（yazi 靠这个算提交范围）
    let mut a: Vec<_> = original.changed().iter().map(|e| e.path.as_str()).collect();
    let mut b: Vec<_> = loaded.changed().iter().map(|e| e.path.as_str()).collect();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bubbles_are_rebuilt_not_lost() {
    let root = fake_wc("bub");
    let dir = tmp("bub-cache");
    let c = Cache::open_in(&root, &dir);

    let root_s = root.to_string_lossy().to_string();
    let original = snapshot_of(&root_s, vec![entry("src/deep/b.txt", StatusKind::Conflicted)]);
    let expected = original.mark_of("src").map(|m| m.sign);

    c.save(&original).unwrap();
    let loaded = c.load(Duration::from_secs(300)).unwrap();

    assert_eq!(loaded.mark_of("src").map(|m| m.sign), expected, "冒泡索引必须随缓存一起恢复");
    assert_eq!(loaded.conflicts().len(), 1);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repeated_hits_are_stable() {
    let root = fake_wc("rep");
    let dir = tmp("rep-cache");
    let c = Cache::open_in(&root, &dir);

    let root_s = root.to_string_lossy().to_string();
    c.save(&snapshot_of(&root_s, vec![entry("src/a.txt", StatusKind::Modified)])).unwrap();

    // 连续多次加载必须返回同一结果（不能因为 build_bubbles 改了 stamp 就失配）
    let first = c.load(Duration::from_secs(300)).unwrap();
    let second = c.load(Duration::from_secs(300)).unwrap();
    assert_eq!(first.entries.len(), second.entries.len());
    assert_eq!(
        first.get("src/a.txt").map(|e| e.text),
        second.get("src/a.txt").map(|e| e.text)
    );

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&dir);
}
