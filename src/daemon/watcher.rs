//! 文件系统监听 + 增量刷新。
//!
//! 职责：把 `notify` 的事件流收敛成「一小批需要重新 status 的路径」。
//!
//! 三个关键点：
//! - **过滤 `.svn`**：svn 自己的每次操作都会写 `.svn`，不过滤会导致自己触发自己，
//!   形成无限循环（status 改 wc.db → 事件 → status → ...）。
//! - **debounce**：编辑器保存一次可能触发 create+write+remove 多个事件，
//!   300ms 窗口合并后再处理。
//! - **只 status 变更路径**：单文件 `svn status <path>` 是毫秒级，
//!   全量是秒到几十秒级。这是 daemon 存在的全部意义。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// `Watcher` 必须显式导入：`watch()` / `unwatch()` 是它的 trait 方法，
// 不在作用域内会报 "no method named `watch` found"。
use notify::Watcher;

/// 事件去抖窗口。
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// 待处理的路径集合，由 watcher 线程写入、daemon 主循环读取。
pub type Pending = Arc<Mutex<HashSet<PathBuf>>>;

/// 「svn 元数据已变化」标志。
///
/// # 为什么需要它
///
/// `watch()` 刻意**忽略 `.svn` 下的一切** —— 因为 svn 自己的每次操作都会写
/// `.svn`，不过滤会自己触发自己形成无限循环。
///
/// 但这也留下一个洞：用户在**终端里**跑 `svn add / svn ci / svn up` 时，
/// 改的是 `.svn/wc.db`，工作副本里的文件 mtime 一个都没变。
/// daemon 收不到任何事件 → 快照一直停留在旧状态 →
/// **yazi 里显示的还是提交前的状态**，而用户以为刷新过了。
///
/// 解法：单独盯 `.svn/wc.db` 这一个文件。它变了就置位，
/// daemon 主循环看到它就去跑一次全量 status（而不是局部）。
pub type MetaDirty = Arc<AtomicBool>;

/// 启动监听。
///
/// 返回 pending 集合；**必须**保存返回的 watcher，drop 掉监听就停了。
/// `notify::Error` → `std::io::Error`。
///
/// 不依赖 notify 是否实现了 `From<Error> for io::Error` ——
/// 显式转换在任何版本上都能编译。
fn notify_err(e: notify::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
}

pub fn watch(root: &Path) -> std::io::Result<(notify::RecommendedWatcher, Pending, MetaDirty)> {
    let pending: Pending = Arc::new(Mutex::new(HashSet::new()));
    let p = pending.clone();
    let meta = Arc::new(AtomicBool::new(false));
    let m = meta.clone();

    let mut watcher = notify::RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for path in &ev.paths {
                    if is_wc_db(path) {
                        // 元数据变了 → 需要做一次全量重扫
                        m.store(true, Ordering::SeqCst);
                        continue;
                    }
                    if is_svn_internal(path) {
                        continue;
                    }
                    match p.lock() {
                        Ok(mut s) => {
                            s.insert(path.clone());
                        }
                        Err(e) => {
                            e.into_inner().insert(path.clone());
                        }
                    };
                }
            }
        },
        notify::Config::default(),
    )
    .map_err(notify_err)?;

    watcher
        .watch(root, notify::RecursiveMode::Recursive)
        .map_err(notify_err)?;

    // 单独盯 wc.db：注意这里**不能**用 Recursive，
    // 且失败不算致命（SVN 1.6 布局没有 wc.db），只退化成"感知不到终端里的 svn 操作"。
    let wc_db = root.join(".svn").join("wc.db");
    if wc_db.exists() {
        let _ = watcher.watch(&wc_db, notify::RecursiveMode::NonRecursive);
    }

    Ok((watcher, pending, meta))
}

/// 是否是 `.svn/wc.db`（SVN 1.7+ 的集中元数据库）。
fn is_wc_db(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some("wc.db")
        && path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some(".svn")
}

/// 是否属于 svn 内部文件。`.svn` 下的一切都要忽略。
fn is_svn_internal(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".svn")
}

/// 等待事件攒够一个 debounce 窗口，然后取出待处理路径。
///
/// 超时返回空集 —— 调用方据此执行空闲检查（比如该不该自动退出）。
pub fn drain_after_debounce(pending: &Pending) -> HashSet<PathBuf> {
    std::thread::sleep(DEBOUNCE);
    let mut set = match pending.lock() {
        Ok(s) => s,
        Err(e) => e.into_inner(),
    };
    std::mem::take(&mut *set)
}

/// 事件通道的轻量封装。当前 watcher 直接写 pending，
/// 这个类型保留给未来需要把 watcher 挪到独立线程时用。
pub struct EventBus {
    #[allow(dead_code)]
    tx: Sender<PathBuf>,
    #[allow(dead_code)]
    rx: Receiver<PathBuf>,
}

#[allow(dead_code)]
impl EventBus {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }

    /// 非阻塞取一批事件。没有就返回空。
    pub fn take_batch(&self, _timeout: Duration) -> Vec<PathBuf> {
        match self.rx.recv_timeout(Duration::from_millis(1)) {
            Ok(p) => vec![p],
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svn_internal_paths_are_filtered() {
        assert!(is_svn_internal(Path::new("/repo/.svn/wc.db")));
        assert!(is_svn_internal(Path::new("/repo/.svn/pristine/xx")));
        assert!(is_svn_internal(Path::new("/repo/src/.svn/tmp")));
        // 正常路径不能被误伤
        assert!(!is_svn_internal(Path::new("/repo/src/main.rs")));
        // 名字里带 svn 但不是 .svn 目录的，不该过滤
        assert!(!is_svn_internal(Path::new("/repo/svn-utils.rs")));
    }

    #[test]
    fn drain_empties_pending() {
        let pending: Pending = Arc::new(Mutex::new(HashSet::new()));
        {
            let mut s = pending.lock().unwrap();
            s.insert(PathBuf::from("/repo/a.rs"));
            s.insert(PathBuf::from("/repo/b.rs"));
        }
        let got = drain_after_debounce(&pending);
        assert_eq!(got.len(), 2);
        // 取完必须清空，否则同一批会被反复处理
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn debounce_window_is_reasonable() {
        // 太短：编辑器一次保存产生多个事件会被重复处理
        // 太长：用户能感知到延迟
        assert!(DEBOUNCE.as_millis() >= 100 && DEBOUNCE.as_millis() <= 1000);
    }
}
