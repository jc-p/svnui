//! 常驻守护进程：把 `svn status` 的开销从"每次切目录"摊到"只在文件变化时"。
//!
//! M7 的磁盘缓存已经省掉了必跑的 `svn status`，但**每次加载仍要遍历整棵树**做指纹比对。
//! 在几万到几十万文件的仓库上，那次遍历本身就是几十到几百毫秒 ——
//! daemon 用 `notify` 监听文件事件，只在事件路径上跑局部 status，连遍历都省掉。

pub mod protocol;
pub mod server;
pub mod watcher;

pub use protocol::{Cmd, Query, Reply};
pub use server::{serve, Shared, IDLE_TIMEOUT};
pub use watcher::{drain_after_debounce, watch, Pending, DEBOUNCE};

use std::path::Path;
use std::time::Duration;

use crate::domain::Result;

/// 启动 daemon（后台）。
///
/// 用 `svnui daemon run` 重新拉起自己：Rust std 没有 fork/setsid，
/// 但 spawn 出的子进程在父进程退出后**会继续运行**（被 init/launchd 收养），
/// 这已经满足需求。stdin/stdout/stderr 全部 null，避免持有父进程的终端。
///
/// 若已在运行则直接返回（幂等）。
pub fn start(root: &Path, timeout: Duration) -> Result<bool> {
    if is_running(root) {
        return Ok(false);
    }

    let exe = std::env::current_exe().map_err(crate::domain::Error::Io)?;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .arg("run")
        .arg("--root")
        .arg(root)
        .arg("--timeout")
        .arg(timeout.as_secs().to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    cmd.spawn().map_err(crate::domain::Error::Io)?;

    // 等一小会儿让它把 socket 建起来，这样 start 之后立刻 q 能连上
    for _ in 0..50 {
        if is_running(root) {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(true)
}

/// daemon 是否在运行。**不能只看 socket 文件是否存在** ——
/// 上次异常退出会留下残骸。必须真的连一次。
pub fn is_running(root: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let sock = protocol::socket_path(root);
        if !sock.exists() {
            return false;
        }
        UnixStream::connect(&sock).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        false
    }
}

/// 停止 daemon：发 `Cmd::Shutdown` 让它优雅退出（会落盘缓存 + 清理 socket）。
pub fn stop(root: &Path) -> Result<bool> {
    if !is_running(root) {
        return Ok(false);
    }
    send_cmd(root, Cmd::Shutdown)?;

    // 等它退出
    for _ in 0..50 {
        if !is_running(root) {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(true)
}

/// 强制重新全量扫描。
pub fn rescan(root: &Path) -> Result<bool> {
    if !is_running(root) {
        return Ok(false);
    }
    send_cmd(root, Cmd::Rescan)?;
    Ok(true)
}

fn send_cmd(root: &Path, cmd: Cmd) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;
        use std::time::Duration as D;

        let mut stream = UnixStream::connect(protocol::socket_path(root))
            .map_err(crate::domain::Error::Io)?;
        stream.set_read_timeout(Some(D::from_secs(2))).map_err(crate::domain::Error::Io)?;
        stream.set_write_timeout(Some(D::from_secs(2))).map_err(crate::domain::Error::Io)?;

        let mut payload =
            serde_json::to_string(&cmd).map_err(|e| crate::domain::Error::Parse(e.to_string()))?;
        payload.push('\n');
        stream.write_all(payload.as_bytes()).map_err(crate::domain::Error::Io)?;
        stream.flush().map_err(crate::domain::Error::Io)?;

        let mut line = String::new();
        let _ = BufReader::new(stream).read_line(&mut line);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (root, cmd);
        Err(crate::domain::Error::Parse("daemon 仅支持 unix".into()))
    }
}
