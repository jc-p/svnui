use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use super::StatusEntry;

/// 一个"标记"：符号 + 严重度。
///
/// 单独抽出来是因为冒泡时要在两个候选之间取更严重的那个，
/// 只存 `char` 没法比较，只存优先级又丢了树冲突要显示 `T` 的信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub sign: char,
    pub priority: u8,
}

impl Mark {
    pub fn new(sign: char, priority: u8) -> Self {
        Self { sign, priority }
    }

    /// 取更严重的一个。相等时保留先来的（稳定）。
    pub fn worse(self, other: Mark) -> Mark {
        if other.priority > self.priority {
            other
        } else {
            self
        }
    }
}

/// 一次 `svn status` 扫描的**纯数据快照**。
///
/// ⚠️ 这里不含任何 IO。落盘/读盘在 `crate::cache`；扫描在 `crate::svn::client`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// 工作副本根。
    pub root: Utf8PathBuf,
    /// 版本戳：任何写操作后自增，供 yazi/daemon 做增量判断。
    pub stamp: u64,
    /// 全量扫描是否已完成。false 时 `entries` 只是部分结果（分层懒加载中间态）。
    pub ready: bool,
    /// 相对 root 的路径 → 状态。
    pub entries: HashMap<Utf8PathBuf, StatusEntry>,
    /// 预计算的目录冒泡结果：目录 → 子树里最严重的标记。
    bubbles: HashMap<Utf8PathBuf, Mark>,
}

/// 某个目录层的视图，直接喂给 yazi 渲染。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotLayer {
    /// 直接子条目（文件或目录自身的条目）→ 标记。
    pub files: HashMap<String, Mark>,
    /// 直接子目录（来自冒泡）→ 标记。
    pub dirs: HashMap<String, Mark>,
    pub ready: bool,
}

impl Snapshot {
    pub fn new(root: impl Into<Utf8PathBuf>) -> Self {
        Self { root: root.into(), stamp: 0, ready: false, entries: HashMap::new(), bubbles: HashMap::new() }
    }

    pub fn insert(&mut self, entry: StatusEntry) {
        self.entries.insert(entry.path.clone(), entry);
    }

    /// 移除一条。增量刷新时**必须**用它清掉已恢复正常的路径 ——
    /// 否则一个文件改完又 revert 掉，状态会永远留在表里。
    pub fn remove(&mut self, rel: impl AsRef<Utf8Path>) -> Option<StatusEntry> {
        self.entries.remove(rel.as_ref())
    }

    pub fn get(&self, rel: impl AsRef<Utf8Path>) -> Option<&StatusEntry> {
        self.entries.get(rel.as_ref())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 所有本地变更（提交/回滚的目标集合）。
    pub fn changed(&self) -> Vec<&StatusEntry> {
        let mut v: Vec<&StatusEntry> = self.entries.values().filter(|e| e.is_changed()).collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    }

    /// 所有冲突项。
    pub fn conflicts(&self) -> Vec<&StatusEntry> {
        let mut v: Vec<&StatusEntry> = self.entries.values().filter(|e| e.is_conflicted()).collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    }

    /// 扫完 entries 后调用：把子树状态向上冒泡到每一级祖先目录。
    ///
    /// 预计算一遍，之后 yazi 查询是 O(1)，不会在大仓库里每次渲染都遍历全表。
    pub fn build_bubbles(&mut self) {
        let mut acc: HashMap<Utf8PathBuf, Mark> = HashMap::new();
        for e in self.entries.values() {
            let mark = Mark::new(e.sign(), e.priority());
            if mark.priority == 0 {
                continue;
            }
            let mut cur = e.path.parent();
            while let Some(dir) = cur {
                if dir.as_str().is_empty() {
                    break;
                }
                acc.entry(dir.to_path_buf())
                    .and_modify(|m| *m = m.worse(mark))
                    .or_insert(mark);
                cur = dir.parent();
            }
        }
        self.bubbles = acc;
        self.stamp = self.stamp.wrapping_add(1);
    }

    /// 查某个相对路径的标记：先查自身条目，再查目录冒泡。
    pub fn mark_of(&self, rel: &str) -> Option<Mark> {
        let rel = normalize_dir(rel);
        if rel.is_empty() {
            return None;
        }
        if let Some(e) = self.entries.get(Utf8Path::new(rel)) {
            return Some(Mark::new(e.sign(), e.priority()));
        }
        self.bubbles.get(Utf8Path::new(rel)).copied()
    }

    /// 取某一层目录的视图。`dir` 传 `""` 或 `"."` 表示工作副本根。
    pub fn layer(&self, dir: &str) -> SnapshotLayer {
        let dir = normalize_dir(dir);
        let mut layer = SnapshotLayer { ready: self.ready, ..Default::default() };

        for (path, e) in &self.entries {
            if parent_of(path) == dir {
                let name = path.file_name().unwrap_or_default().to_string();
                layer.files.insert(name, Mark::new(e.sign(), e.priority()));
            }
        }

        for (d, m) in &self.bubbles {
            if parent_of(d) == dir {
                let name = d.file_name().unwrap_or_default().to_string();
                layer.dirs.insert(name, *m);
            }
        }

        layer
    }
}

/// 目录参数归一化：`""` / `"."` / `"./"` 都表示根。
pub fn normalize_dir(dir: &str) -> &str {
    let d = dir.trim_end_matches('/');
    if d.is_empty() || d == "." {
        ""
    } else {
        d
    }
}

/// 父目录字符串，根层的条目返回 `""`。
fn parent_of(path: &Utf8Path) -> &str {
    path.parent().map(|p| p.as_str()).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{NodeKind, StatusKind};

    fn entry(path: &str, text: StatusKind) -> StatusEntry {
        StatusEntry {
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

    fn snap() -> Snapshot {
        let mut s = Snapshot::new("/repo");
        s.insert(entry("src/deep/a.rs", StatusKind::Modified));
        s.insert(entry("src/deep/b.rs", StatusKind::Conflicted));
        s.insert(entry("src/top.rs", StatusKind::Added));
        s.insert(entry("README.md", StatusKind::Normal));
        s.build_bubbles();
        s
    }

    #[test]
    fn bubble_takes_worst_from_subtree() {
        let s = snap();
        // a.rs=M(40) b.rs=C(100) → src/deep 与 src 都应显示 C
        assert_eq!(s.mark_of("src/deep").map(|m| m.sign), Some('C'));
        assert_eq!(s.mark_of("src").map(|m| m.sign), Some('C'));
    }

    #[test]
    fn own_entry_wins_over_bubble() {
        let s = snap();
        // src/top.rs = A(50)，但 src 自身冒泡是 C(100)
        assert_eq!(s.mark_of("src/top.rs").map(|m| m.sign), Some('A'));
    }

    #[test]
    fn root_layer_sees_only_top_level() {
        let s = snap();
        let l = s.layer("");
        assert!(l.files.contains_key("README.md"));
        assert!(l.dirs.contains_key("src"));
        assert!(!l.files.contains_key("a.rs"));
    }

    #[test]
    fn changed_excludes_normal() {
        let s = snap();
        let c = s.changed();
        assert_eq!(c.len(), 3, "README.md 是 normal，不该进 changed");
        assert!(c.iter().all(|e| e.path != Utf8PathBuf::from("README.md")));
    }

    #[test]
    fn conflicts_only_conflicted() {
        let s = snap();
        let c = s.conflicts();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].path, Utf8PathBuf::from("src/deep/b.rs"));
    }

    #[test]
    fn stamp_increments_on_rebuild() {
        let mut s = snap();
        let before = s.stamp;
        s.build_bubbles();
        assert!(s.stamp > before);
    }
}
