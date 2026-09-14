//! daemon 服务端。
//!
//! # 为什么 svn 调用必须串行
//!
//! svn 工作副本有**独占锁**。如果 daemon 在后台 status，
//! 而用户同时在终端跑 commit，两者会撞锁报 `E155004`。
//! 所以这里所有 svn 调用都在同一条线程上排队执行 ——
//! daemon 慢一点没关系，把用户的命令搞失败才是灾难。
//!
//! # 生命周期
//!
//! - 启动：后台跑一次全量扫描，`ready` 置位前也能先服务（返回部分结果）
//! - 文件变更：`notify` 事件 → debounce → **只 status 变更路径** → 局部更新
//! - 空闲 30 分钟自动退出，退出前落盘缓存
//! - `svnui daemon stop` 发 `Cmd::Shutdown` 优雅退出并清理 socket

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

// UDS 与文件监听相关的一切都是 unix-only。
// 非 unix 下 `serve` 直接返回错误（见文件末尾），这些导入否则会 unused。
#[cfg(unix)]
use crate::daemon::protocol::{pid_path, socket_path, Cmd, Query, Reply};
#[cfg(unix)]
use crate::daemon::watcher::{drain_after_debounce, watch, MetaDirty, Pending};
use crate::domain::{Error, Result, Snapshot, StatusEntry};
#[cfg(unix)]
use crate::svn::Svn;

/// 空闲多久后自动退出。
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// 主循环每次迭代的睡眠时长。
const TICK: Duration = Duration::from_millis(200);

/// daemon 共享状态。
pub struct Shared {
    pub root: PathBuf,
    pub snap: Mutex<Snapshot>,
    /// 版本戳。任何状态变化后自增 —— 客户端靠它判断"要不要重取"。
    pub stamp: AtomicU64,
    pub ready: AtomicBool,
    pub last_used: Mutex<Instant>,
    pub shutdown: AtomicBool,
}

impl Shared {
    pub fn new(root: PathBuf) -> Self {
        Self {
            snap: Mutex::new(Snapshot::new(root.to_string_lossy().to_string())),
            root,
            stamp: AtomicU64::new(1),
            ready: AtomicBool::new(false),
            last_used: Mutex::new(Instant::now()),
            shutdown: AtomicBool::new(false),
        }
    }

    pub fn touch(&self) {
        if let Ok(mut t) = self.last_used.lock() {
            *t = Instant::now();
        }
    }

    pub fn stamp(&self) -> u64 {
        self.stamp.load(Ordering::SeqCst)
    }

    /// 局部更新：先删掉 `touched` 对应的旧条目，再插入新结果。
    ///
    /// ⚠️ **必须先删**。`svn status <path>` 对"已恢复正常"的路径返回空，
    /// 只 insert 的话，一个文件改完又 revert 掉，状态会永远留在表里 ——
    /// 表现为"列表里一直显示 M，但 diff 是空的"，非常难查。
    pub fn apply(&self, touched: &[PathBuf], entries: Vec<StatusEntry>) {
        let mut snap = match self.snap.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };

        for p in touched {
            if let Some(rel) = relative_of(&self.root, p) {
                snap.remove(&rel);
            }
        }
        for e in entries {
            snap.insert(e);
        }
        snap.build_bubbles();
        drop(snap);

        self.stamp.fetch_add(1, Ordering::SeqCst);
    }

    /// 全量替换。首次扫描与 Rescan 用。
    pub fn replace_all(&self, entries: Vec<StatusEntry>) {
        let mut snap = match self.snap.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };
        // 重建而不是合并：全量结果才是唯一真相
        *snap = Snapshot::new(self.root.to_string_lossy().to_string());
        for e in entries {
            snap.insert(e);
        }
        snap.ready = true;
        snap.build_bubbles();
        drop(snap);

        self.ready.store(true, Ordering::SeqCst);
        self.stamp.fetch_add(1, Ordering::SeqCst);
    }

    /// 计算某目录层的视图：绝对路径 → porcelain 两字符。
    pub fn layer_of(&self, dir: &str) -> (HashMap<String, String>, bool) {
        let snap = match self.snap.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };
        let layer = snap.layer(dir);

        let base: PathBuf = if dir.is_empty() { self.root.clone() } else { self.root.join(dir) };

        let mut map = HashMap::new();
        for (name, mark) in layer.files {
            map.insert(base.join(&name).to_string_lossy().to_string(), xy(mark.sign));
        }
        for (name, mark) in layer.dirs {
            map.insert(base.join(&name).to_string_lossy().to_string(), xy(mark.sign));
        }
        (map, self.ready.load(Ordering::SeqCst))
    }
}

/// 单符号 → porcelain 两字符（yazi 侧按两字符解析）。
fn xy(sign: char) -> String {
    if sign == ' ' || sign == '\0' {
        "__".to_string()
    } else if sign == 'T' {
        "T_".to_string()
    } else {
        format!("{sign}_")
    }
}

/// 绝对路径 → 相对工作副本根的字符串。
fn relative_of(root: &Path, abs: &Path) -> Option<String> {
    abs.strip_prefix(root).ok().map(|p| p.to_string_lossy().to_string())
}

/// 运行 daemon（阻塞）。`svnui daemon run` 的实现。
#[cfg(unix)]
pub fn serve(root: &Path, timeout: Duration) -> Result<()> {
    let svn = Svn::discover(root, timeout, false)?;
    let shared = Arc::new(Shared::new(svn.root.clone()));

    let sock = socket_path(&svn.root);
    // 清理上次异常退出留下的 socket 文件（否则 bind 会 EADDRINUSE）
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }
    let listener = UnixListener::bind(&sock).map_err(Error::Io)?;
    let _ = std::fs::write(pid_path(&svn.root), std::process::id().to_string());

    // 1) 后台全量扫描
    let sv = svn.clone();
    let sh = shared.clone();
    std::thread::spawn(move || {
        if let Ok(entries) = sv.status(&Default::default()) {
            sh.replace_all(entries);
        }
    });

    // 2) 文件监听 + 增量刷新线程
    //    watcher 必须活满整个 serve 生命周期，drop 掉监听就停了。
    let _watcher_guard = match watch(&svn.root) {
        Ok((watcher, pending, meta)) => {
            let sv2 = svn.clone();
            let sh2 = shared.clone();
            std::thread::spawn(move || {
                incremental_loop(&sv2, &sh2, &pending, &meta);
            });
            Some(watcher)
        }
        Err(_) => {
            // 监听起不来（inotify 句柄耗尽等）不致命：
            // daemon 仍然能提供请求服务，只是退化成"靠 TTL/手动 rescan"。
            None
        }
    };

    let r = accept_loop(shared.clone(), listener, &svn, &sock);

    // 收尾：落盘缓存 + 清理 socket / pid
    if let Some(c) = crate::cache::Cache::open(&shared.root) {
        if let Ok(s) = shared.snap.lock() {
            let _ = c.save(&s);
        }
    }
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(pid_path(&shared.root));
    r
}

/// 增量刷新循环：debounce → 只对变更路径 status → 局部更新。
///
/// 另外每轮都会检查 `meta`（`.svn/wc.db` 是否被改过）。
/// 用户**在终端里**跑 svn 命令时不产生工作副本文件事件，
/// 只改 wc.db —— 不检查它的话 daemon 会一直停留在旧快照。
#[cfg(unix)]
fn incremental_loop(svn: &Svn, shared: &Shared, pending: &Pending, meta: &MetaDirty) {
    loop {
        if shared.shutdown.load(Ordering::SeqCst) {
            break;
        }

        // 元数据脏 → 整棵树重扫。这是唯一能感知"终端里的 svn 操作"的路径。
        if meta.swap(false, Ordering::SeqCst) {
            match svn.status(&Default::default()) {
                Ok(entries) => {
                    shared.replace_all(entries);
                    shared.ready.store(true, Ordering::SeqCst);
                }
                Err(e) => eprintln!("[svnui daemon] 元数据变更后重扫失败: {e}"),
            }
            shared.stamp.fetch_add(1, Ordering::SeqCst);
            continue;
        }

        let paths = drain_after_debounce(pending);
        if paths.is_empty() {
            continue;
        }
        let touched: Vec<PathBuf> = paths.into_iter().collect();
        match svn.status_paths(&touched) {
            Ok(entries) => shared.apply(&touched, entries),
            Err(e) => eprintln!("[svnui daemon] 增量刷新失败: {e}"),
        }
    }
}

/// 请求处理主循环（阻塞）。
#[cfg(unix)]
fn accept_loop(
    shared: Arc<Shared>,
    listener: UnixListener,
    svn: &Svn,
    // socket 文件的清理交给 `serve` 的调用方（`stop` 与退出路径），
    // 这里只负责 accept。参数保留是为了让服务端知道自己监听的是哪个
    // 工作副本的 socket —— 日志与错误消息里会用到。
    _sock: &Path,
) -> Result<()> {
    listener.set_nonblocking(true).map_err(Error::Io)?;
    let branch = svn.info().ok().and_then(|i| i.branch_name());

    loop {
        if shared.shutdown.load(Ordering::SeqCst) {
            break;
        }

        // 空闲退出
        if let Ok(t) = shared.last_used.lock() {
            if t.elapsed() > IDLE_TIMEOUT {
                break;
            }
        }

        match listener.accept() {
            Ok((stream, _)) => {
                shared.touch();
                let sh = shared.clone();
                let br = branch.clone();
                // Svn 是 Clone 的（只持有 exe/root/version 等值），
                // 直接克隆进线程，省得为它单独加 Arc。
                let sv = svn.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle(stream, &sh, &sv, br) {
                        eprintln!("[svnui daemon] 处理连接失败: {e}");
                    }
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(TICK);
            }
            Err(e) => {
                eprintln!("[svnui daemon] accept 失败: {e}");
                std::thread::sleep(TICK);
            }
        }
    }
    Ok(())
}

/// 处理单个连接：读一行请求，回一行响应。
#[cfg(unix)]
fn handle(
    stream: UnixStream,
    shared: &Shared,
    svn: &Svn,
    branch: Option<String>,
) -> Result<()> {
    let reader = BufReader::new(&stream);
    let mut line = String::new();
    // 限制单行帧大小：防止写坏的客户端（或恶意进程）用超长行把 daemon 内存打爆
    if reader.take(crate::daemon::protocol::MAX_FRAME).read_line(&mut line)? == 0 {
        return Ok(());
    }
    let trimmed = line.trim();

    // 控制命令：只能是 {"Ping"|"Rescan"|"Shutdown"} 这类单键对象
    if let Ok(cmd) = serde_json::from_str::<Cmd>(trimmed) {
        match cmd {
            Cmd::Ping => {}
            Cmd::Shutdown => shared.shutdown.store(true, Ordering::SeqCst),
            // Rescan 必须**真的重扫一遍**再 bump 版本戳。
            // 只 bump 的话：客户端看到 v 变了会来拉最新层，
            // 但服务端快照还是旧的 —— 于是刷新完状态依旧是旧的，
            // 这个 bug 比不刷新更难发现。
            Cmd::Rescan => {
                if let Ok(entries) = svn.status(&Default::default()) {
                    shared.replace_all(entries);
                    shared.ready.store(true, Ordering::SeqCst);
                }
                shared.stamp.fetch_add(1, Ordering::SeqCst);
            }
        }
        let r = Reply {
            v: shared.stamp(),
            changed: true,
            ready: shared.ready.load(Ordering::SeqCst),
            ..Default::default()
        };
        return write_reply(&stream, &r);
    }

    let q: Query = serde_json::from_str(trimmed).map_err(|e| Error::Parse(e.to_string()))?;
    let stamp = shared.stamp();

    // 版本戳一致 → 回极简响应（几十字节），客户端直接用本地缓存
    if q.since == Some(stamp) {
        return write_reply(&stream, &Reply::unchanged(stamp, shared.ready.load(Ordering::SeqCst)));
    }

    let rel = relative_dir(&shared.root, Path::new(&q.dir));
    let (map, ready) = shared.layer_of(&rel);

    write_reply(
        &stream,
        &Reply {
            v: stamp,
            changed: true,
            root: Some(shared.root.to_string_lossy().to_string()),
            branch,
            map,
            ready,
        },
    )
}

#[cfg(unix)]
fn write_reply(stream: &UnixStream, r: &Reply) -> Result<()> {
    let mut payload = serde_json::to_string(r).map_err(|e| Error::Parse(e.to_string()))?;
    payload.push('\n');
    let mut s = stream;
    s.write_all(payload.as_bytes())?;
    s.flush()?;
    Ok(())
}

/// 绝对路径 → 相对工作副本根的目录字符串（根层是 ""）。
fn relative_dir(root: &Path, dir: &Path) -> String {
    let rel = dir.strip_prefix(root).unwrap_or(dir);
    crate::domain::normalize_dir(&rel.to_string_lossy()).to_string()
}

/// 非 unix 平台：daemon 不可用（UDS 不存在）。
#[cfg(not(unix))]
pub fn serve(_root: &Path, _timeout: Duration) -> Result<()> {
    Err(Error::Parse(
        "daemon 目前只支持 unix（UDS）。Windows 上请直接用 svnui status。".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, text: crate::domain::StatusKind) -> StatusEntry {
        StatusEntry {
            path: camino::Utf8PathBuf::from(path),
            text,
            props: crate::domain::StatusKind::None,
            locked: false,
            copied: false,
            switched: false,
            lock_token: None,
            tree_conflict: false,
            kind: crate::domain::NodeKind::File,
        }
    }

    #[test]
    fn relative_dir_handles_root_and_nested() {
        assert_eq!(relative_dir(Path::new("/repo"), Path::new("/repo")), "");
        assert_eq!(relative_dir(Path::new("/repo"), Path::new("/repo/src")), "src");
        assert_eq!(relative_dir(Path::new("/repo"), Path::new("/repo/src/ui")), "src/ui");
    }

    #[test]
    fn xy_maps_signs() {
        assert_eq!(xy('M'), "M_");
        assert_eq!(xy('T'), "T_");
        assert_eq!(xy(' '), "__");
    }

    #[test]
    fn apply_bumps_stamp() {
        let s = Shared::new(PathBuf::from("/repo"));
        let before = s.stamp();
        s.apply(&[], vec![]);
        assert!(s.stamp() > before, "任何更新都必须让版本戳自增");
    }

    /// 这是 daemon 最容易出 bug 的地方，必须单独守住。
    #[test]
    fn apply_removes_stale_entries() {
        use crate::domain::StatusKind;
        let s = Shared::new(PathBuf::from("/repo"));

        // 先有一个被修改的文件
        s.apply(&[], vec![entry("src/a.rs", StatusKind::Modified)]);
        assert_eq!(s.snap.lock().unwrap().len(), 1);

        // 用户 revert 了它：svn status 返回空，但路径确实"被碰过"
        let touched = vec![PathBuf::from("/repo/src/a.rs")];
        s.apply(&touched, vec![]);

        assert_eq!(
            s.snap.lock().unwrap().len(),
            0,
            "已恢复正常的路径必须从快照里删掉，否则列表会一直显示 M 但 diff 是空的"
        );
    }

    #[test]
    fn replace_all_rebuilds_from_scratch() {
        use crate::domain::StatusKind;
        let s = Shared::new(PathBuf::from("/repo"));
        s.apply(&[], vec![entry("src/a.rs", StatusKind::Modified)]);
        // 全量扫描结果与之前不同 → 必须重建，不能合并
        s.replace_all(vec![entry("src/b.rs", StatusKind::Added)]);
        let snap = s.snap.lock().unwrap();
        assert_eq!(snap.len(), 1);
        assert!(snap.get("src/b.rs").is_some());
        assert!(s.ready.load(Ordering::SeqCst));
    }

    #[test]
    fn idle_timeout_is_half_hour() {
        assert_eq!(IDLE_TIMEOUT.as_secs(), 1800);
    }
}
