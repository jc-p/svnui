//! 状态缓存。
//!
//! # 为什么需要严格校验
//!
//! `svn status` 只输出**非 normal** 的条目。所以缓存里天然不包含"干净的文件"，
//! 于是"一个干净文件被改脏了"这件事**无法通过比对缓存条目发现** —— 它压根不在缓存里。
//!
//! 同理，新建的未版本化文件（`?`）也不在缓存里，而它的产生**不会**改动 `.svn/wc.db`
//! （创建文件只改父目录 mtime，且修改已有文件连父目录 mtime 都不改）。
//!
//! 结论：**光比对 `wc.db` 指纹是不够的，会静默丢状态。** 因此这里采用全树指纹比对：
//! 记录工作副本内每个文件/目录的 `(mtime, size)`，加载时重新走一遍树并逐条比对。
//!
//! 这样做的代价是命中时仍要遍历一次文件系统（约 `O(files)`），
//! 但它只做 `stat`，比 `svn status`（要做内容校验和）**快一到两个数量级**。
//! 真正的零成本方案是 M9 的 daemon —— 用 `notify` 监听文件事件，连这次遍历都省掉。
//!
//! # 正确性
//!
//! 只要全树指纹 + `wc.db` 指纹全部一致，工作副本的状态就与写入缓存时**完全一致**，
//! 与缓存年龄无关。所以 TTL 在这里只是安全阀，不是正确性保障。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::domain::{Error, Result, Snapshot, StatusEntry};

/// 缓存格式版本。结构变更时 +1 —— 旧缓存会因版本不符自动作废，
/// 而不是被解析成结构不同的数据。
pub const CACHE_VERSION: u32 = 1;

/// 默认 TTL（秒）。注意：严格校验已经保证了正确性，TTL 只是安全阀。
pub const DEFAULT_TTL_SECS: u64 = 300;

/// 跟踪节点上限。超过这个数就不写缓存（JSON 会太大，读写反而不划算）。
const MAX_TRACKED: usize = 200_000;

/// 文件指纹：`(mtime secs, mtime nanos, size)`。
///
/// 保留 nanos 是必须的：同一秒内的连续编辑，只看秒会漏判。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub secs: u64,
    pub nanos: u32,
    pub size: u64,
}

/// `Default` 是"从未采集过"的哨兵值，用于 `CachedDoc` 的反序列化兜底
/// （旧版本缓存文件里可能没有这个字段）。
///
/// 刻意让它**不等于任何真实指纹**：全 0 意味着"必然失效"，
/// 这样缺字段的旧缓存会直接走一次真 svn status，而不是给出错误结果。
impl Default for Fingerprint {
    fn default() -> Self {
        Self { secs: 0, nanos: 0, size: 0 }
    }
}

impl Fingerprint {
    /// 取路径指纹。不存在/无权限/不支持 mtime 时返回 `None`。
    pub fn of(path: &Path) -> Option<Self> {
        // 用 symlink_metadata：不跟随符号链接，避免链接目标变化被误判为内容变化。
        let md = std::fs::symlink_metadata(path).ok()?;
        let mt = md.modified().ok()?;
        let d = mt.duration_since(UNIX_EPOCH).ok()?;
        Some(Self { secs: d.as_secs(), nanos: d.subsec_nanos(), size: md.len() })
    }
}

/// 磁盘缓存内容。
///
/// `tree` 刻意用**扁平元组数组**而不是 `HashMap<String, Fingerprint>`：
/// 大仓库下 JSON 体积能省近一半（少了每个条目的字段名开销）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CachedDoc {
    version: u32,
    /// 工作副本根，防止不同仓库的缓存互相串用。
    root: String,
    /// 写入时刻（unix secs），仅用于 TTL 与 `svnui cache` 展示。
    written_at: u64,
    /// `.svn/wc.db` 指纹。任何 svn 写操作（add/rm/revert/commit/update）都会改它。
    wc_db: Fingerprint,
    tree: Vec<(String, u64, u32, u64)>,
    entries: Vec<StatusEntry>,
}

/// 缓存句柄。
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
    file: PathBuf,
    wc_db: PathBuf,
}

/// `svnui cache` 展示用的信息。
#[derive(Debug)]
pub struct Stats {
    pub path: PathBuf,
    pub exists: bool,
    pub bytes: u64,
    pub age_secs: Option<u64>,
    pub entries: Option<usize>,
    pub tree_nodes: Option<usize>,
    pub root: Option<String>,
}

impl Cache {
    /// 打开（或创建）某工作副本的缓存。
    ///
    /// 返回 `None` 的情况：
    /// - 拿不到缓存目录
    /// - `.svn/wc.db` 不存在 → 说明是 SVN 1.6 及更早的布局，
    ///   那时的 svn 操作不集中改一个数据库，我们的校验模型不成立，**不缓存**。
    pub fn open(root: &Path) -> Option<Self> {
        let dir = cache_dir()?;
        let wc_db = root.join(".svn").join("wc.db");
        if !wc_db.is_file() {
            return None;
        }
        let _ = std::fs::create_dir_all(&dir);
        Some(Self::open_in(root, &dir))
    }

    /// 指定缓存目录（测试用，避免环境变量在并行测试间互相干扰）。
    pub fn open_in(root: &Path, dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        Self {
            root: root.to_path_buf(),
            file: dir.join(format!("{}.json", hash_path(root))),
            wc_db: root.join(".svn").join("wc.db"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    /// 作废缓存。所有写操作之后都必须调用 —— 写操作会改 `wc.db`，
    /// 缓存其实也会因此失效，但显式删掉更省事，也避免下次加载时白走一次树。
    pub fn invalidate(&self) {
        let _ = std::fs::remove_file(&self.file);
    }

    /// 保存。树太大时静默跳过（不缓存比缓存了拖慢启动要好）。
    pub fn save(&self, snap: &Snapshot) -> Result<()> {
        let tree = match walk_tree(&self.root) {
            Some(t) => t,
            None => return Ok(()),
        };
        let wc_db = match Fingerprint::of(&self.wc_db) {
            Some(f) => f,
            None => return Ok(()),
        };

        let doc = CachedDoc {
            version: CACHE_VERSION,
            root: self.root.to_string_lossy().to_string(),
            written_at: now_secs(),
            wc_db,
            tree: tree
                .into_iter()
                .map(|(p, f)| (p, f.secs, f.nanos, f.size))
                .collect(),
            entries: snap.entries.values().cloned().collect(),
        };

        let s = serde_json::to_string(&doc).map_err(|e| Error::Parse(format!("缓存序列化失败: {e}")))?;
        std::fs::write(&self.file, s)?;
        Ok(())
    }

    /// 加载。校验全部通过才返回快照，否则 `None`（调用方应回退到真跑 `svn status`）。
    ///
    /// 校验顺序刻意**从便宜到昂贵**：版本 → 根路径 → TTL → `wc.db` → 全树。
    /// 这样绝大多数"缓存已废"的情况在前几步就被拦下，不会白走树。
    pub fn load(&self, ttl: Duration) -> Option<Snapshot> {
        let raw = std::fs::read_to_string(&self.file).ok()?;
        let doc: CachedDoc = serde_json::from_str(&raw).ok()?;

        if doc.version != CACHE_VERSION {
            return None;
        }
        if doc.root != self.root.to_string_lossy() {
            return None;
        }
        if now_secs().saturating_sub(doc.written_at) > ttl.as_secs() {
            return None;
        }
        if Fingerprint::of(&self.wc_db)? != doc.wc_db {
            return None;
        }

        // 全树比对：这一步能同时兜住"本地编辑"和"新建/删除文件"。
        let tree = walk_tree(&self.root)?;
        if tree.len() != doc.tree.len() {
            return None;
        }
        for (p, secs, nanos, size) in &doc.tree {
            match tree.get(p.as_str()) {
                Some(f) if f.secs == *secs && f.nanos == *nanos && f.size == *size => {}
                _ => return None,
            }
        }

        let mut snap = Snapshot::new(doc.root.as_str());
        snap.ready = true;
        for e in doc.entries {
            snap.insert(e);
        }
        snap.build_bubbles();
        Some(snap)
    }

    /// 缓存统计，`svnui cache` 用。
    pub fn stats(&self) -> Stats {
        let md = std::fs::metadata(&self.file);
        let (exists, bytes) = match md {
            Ok(m) => (true, m.len()),
            Err(_) => (false, 0),
        };

        let mut st = Stats {
            path: self.file.clone(),
            exists,
            bytes,
            age_secs: None,
            entries: None,
            tree_nodes: None,
            root: None,
        };

        if let Ok(raw) = std::fs::read_to_string(&self.file) {
            if let Ok(doc) = serde_json::from_str::<CachedDoc>(&raw) {
                st.age_secs = Some(now_secs().saturating_sub(doc.written_at));
                st.entries = Some(doc.entries.len());
                st.tree_nodes = Some(doc.tree.len());
                st.root = Some(doc.root);
            }
        }
        st
    }
}

/// 便利函数：写操作后作废缓存。拿不到缓存就什么都不做。
pub fn invalidate_for(root: &Path) {
    if let Some(c) = Cache::open(root) {
        c.invalidate();
    }
}

// ---------------------------------------------------------------- 内部

/// 遍历工作副本，收集所有文件/目录的指纹。
///
/// 返回 `None` = 超限或读不了，调用方应视为"不可缓存"。
///
/// 两个关键细节：
/// - **跳过 `.svn`** —— 它是 svn 自己的状态，每次 svn 操作都变，
///   若纳入比对会让缓存永远失效。
/// - **用 `DirEntry::file_type()` 而非 `path.is_dir()`** —— 前者不跟随符号链接，
///   否则遇到指向父目录的软链会无限递归。
fn walk_tree(root: &Path) -> Option<HashMap<String, Fingerprint>> {
    let mut out: HashMap<String, Fingerprint> = HashMap::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for ent in rd.flatten() {
            let name = ent.file_name();
            if name == ".svn" {
                continue;
            }

            let p = ent.path();
            let rel = p.strip_prefix(root).unwrap_or(&p);
            let rel_s = rel.to_string_lossy().to_string();

            if let Some(fp) = Fingerprint::of(&p) {
                if out.insert(rel_s, fp).is_none() && out.len() > MAX_TRACKED {
                    return None;
                }
            }

            if let Ok(ft) = ent.file_type() {
                if ft.is_dir() {
                    stack.push(p);
                }
            }
        }
    }

    Some(out)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// FNV-1a 64，把工作副本根路径映射成缓存文件名。
///
/// 用哈希而不是转义路径：路径里的 `/`、`空格`、中文都可能撞上文件系统的限制。
///
/// 对 daemon 公开：socket / pid 文件名也用它，保证同一工作副本的
/// 缓存文件、socket、pid 三者前缀一致，便于排查。
pub fn hash_root(root: &Path) -> String {
    hash_path(root)
}

fn hash_path(root: &Path) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in root.to_string_lossy().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("SVNR_CACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    let base = if let Some(d) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(d)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library").join("Caches")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".cache")
    };
    Some(base.join("svnui"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{NodeKind, StatusKind};
    use camino::{Utf8Path, Utf8PathBuf};

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("svnui-cache-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        let _ = std::fs::create_dir_all(&p);
        p
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        let a = hash_path(Path::new("/repo"));
        let b = hash_path(Path::new("/repo"));
        let c = hash_path(Path::new("/repo2"));
        assert_eq!(a, b, "同一路径必须映射到同一缓存文件");
        assert_ne!(a, c);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn fingerprint_detects_edit() {
        let d = tmp("fp");
        let f = d.join("a.txt");
        std::fs::write(&f, "one").unwrap();
        let f1 = Fingerprint::of(&f).unwrap();

        // 同长度改写：只有 mtime 变
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, "two").unwrap();
        let f2 = Fingerprint::of(&f).unwrap();
        assert_ne!(f1, f2, "同长度改写也必须被检测到（靠 mtime）");

        // 同内容重写：mtime 仍然变
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, "two").unwrap();
        assert_ne!(f2, Fingerprint::of(&f).unwrap());

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fingerprint_of_missing_is_none() {
        assert!(Fingerprint::of(Path::new("/definitely/not/here")).is_none());
    }

    /// 构造一个"像工作副本"的目录：有 .svn/wc.db，有源码树。
    fn fake_wc(tag: &str) -> PathBuf {
        let root = tmp(tag);
        std::fs::create_dir_all(root.join(".svn")).unwrap();
        std::fs::write(root.join(".svn").join("wc.db"), "db").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("a.txt"), "a").unwrap();
        root
    }

    #[test]
    fn open_requires_wc_db() {
        let root = tmp("nowcdb");
        let dir = tmp("nowcdb-cache");
        // 没有 .svn/wc.db → 视为老版 SVN，不缓存
        let c = Cache::open_in(&root, &dir);
        assert!(!c.wc_db.is_file());
        // open() 会因缺少 wc.db 返回 None
        assert!(Cache::open(&root).is_none());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let root = fake_wc("rt");
        let dir = tmp("rt-cache");
        let c = Cache::open_in(&root, &dir);

        let mut snap = Snapshot::new(root.to_string_lossy().to_string());
        snap.insert(StatusEntry {
            path: Utf8PathBuf::from("src/a.txt"),
            text: StatusKind::Modified,
            props: StatusKind::None,
            locked: false,
            copied: false,
            switched: false,
            lock_token: None,
            tree_conflict: false,
            kind: NodeKind::File,
        });
        snap.build_bubbles();

        c.save(&snap).unwrap();
        assert!(c.path().is_file(), "缓存文件应已写出");

        let loaded = c.load(Duration::from_secs(300)).expect("指纹未变，应命中");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(
            loaded.get(Utf8Path::new("src/a.txt")).unwrap().text,
            StatusKind::Modified
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editing_a_file_invalidates_cache() {
        let root = fake_wc("edit");
        let dir = tmp("edit-cache");
        let c = Cache::open_in(&root, &dir);

        let snap = Snapshot::new(root.to_string_lossy().to_string());
        c.save(&snap).unwrap();
        assert!(c.load(Duration::from_secs(300)).is_some(), "刚写完应命中");

        // 改一个源文件 —— wc.db 没动，但文件内容变了
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("src").join("a.txt"), "changed").unwrap();

        assert!(
            c.load(Duration::from_secs(300)).is_none(),
            "本地编辑必须让缓存失效 —— 这是本模块最重要的不变量"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_unversioned_file_invalidates_cache() {
        let root = fake_wc("newfile");
        let dir = tmp("newfile-cache");
        let c = Cache::open_in(&root, &dir);

        c.save(&Snapshot::new(root.to_string_lossy().to_string())).unwrap();
        assert!(c.load(Duration::from_secs(300)).is_some());

        // 新建文件：wc.db 不会变（这正是当初设计时的陷阱）
        std::fs::write(root.join("src").join("b.txt"), "b").unwrap();

        assert!(c.load(Duration::from_secs(300)).is_none(), "新建文件必须让缓存失效");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wc_db_change_invalidates_cache() {
        let root = fake_wc("wcdb");
        let dir = tmp("wcdb-cache");
        let c = Cache::open_in(&root, &dir);

        c.save(&Snapshot::new(root.to_string_lossy().to_string())).unwrap();
        assert!(c.load(Duration::from_secs(300)).is_some());

        // 模拟 svn 写操作：改 wc.db 而不动源码
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join(".svn").join("wc.db"), "db-changed").unwrap();

        assert!(c.load(Duration::from_secs(300)).is_none());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ttl_expiry_works() {
        let root = fake_wc("ttl");
        let dir = tmp("ttl-cache");
        let c = Cache::open_in(&root, &dir);

        c.save(&Snapshot::new(root.to_string_lossy().to_string())).unwrap();
        assert!(c.load(Duration::from_secs(300)).is_some());
        assert!(c.load(Duration::from_secs(0)).is_none(), "ttl=0 应总是失效");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalidate_removes_file() {
        let root = fake_wc("inv");
        let dir = tmp("inv-cache");
        let c = Cache::open_in(&root, &dir);

        c.save(&Snapshot::new(root.to_string_lossy().to_string())).unwrap();
        assert!(c.path().is_file());
        c.invalidate();
        assert!(!c.path().is_file());
        assert!(c.load(Duration::from_secs(300)).is_none());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn walk_tree_skips_dot_svn() {
        let root = fake_wc("walk");
        let t = walk_tree(&root).expect("应能遍历");
        assert!(!t.keys().any(|k| k.contains(".svn")), ".svn 必须被排除，否则缓存永远失效");
        assert!(t.contains_key("src/a.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn symlink_does_not_cause_infinite_recursion() {
        let root = fake_wc("link");
        let target = root.join("src");
        let link = root.join("loop");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).ok();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target, &link).ok();

        if link.exists() {
            // 能建出软链就验证不会栈溢出/死循环
            let t = walk_tree(&root);
            assert!(t.is_some(), "符号链接环必须被 file_type 挡住");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stats_reports_contents() {
        let root = fake_wc("stats");
        let dir = tmp("stats-cache");
        let c = Cache::open_in(&root, &dir);

        let st = c.stats();
        assert!(!st.exists);

        c.save(&Snapshot::new(root.to_string_lossy().to_string())).unwrap();
        let st = c.stats();
        assert!(st.exists);
        assert!(st.bytes > 0);
        assert_eq!(st.entries, Some(0));
        assert!(st.tree_nodes.unwrap() > 0);
        assert!(st.root.is_some());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_roots_do_not_share_cache() {
        let root_a = fake_wc("ra");
        let root_b = fake_wc("rb");
        let dir = tmp("share-cache");

        let ca = Cache::open_in(&root_a, &dir);
        let cb = Cache::open_in(&root_b, &dir);
        assert_ne!(ca.path(), cb.path(), "不同仓库必须落到不同缓存文件");

        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
