use std::io::Read;
use std::path::Path;
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::domain::{Error, Result};

/// 一次 svn 调用的原始结果。**不做任何语义判断** —— 那属于 `client.rs`。
#[derive(Debug, Clone, Default)]
pub struct RawOutput {
    pub stdout: String,
    pub stderr: String,
    /// `None` 表示被信号杀死（我们超时 kill 时可能发生）。
    pub code: Option<i32>,
}

impl RawOutput {
    pub fn is_success(&self) -> bool {
        self.code == Some(0)
    }
}

/// 执行参数。
#[derive(Debug, Clone)]
pub struct RunOpts {
    /// 超时后一定 kill 子进程，不留孤儿。
    pub timeout: Duration,
    /// 非零退出是否也算成功（`svn status` 部分失败时仍会输出可用结果）。
    pub allow_fail: bool,
    /// 每次调用都带的全局参数，例如 `--non-interactive`。
    pub global: Vec<String>,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            allow_fail: false,
            global: vec!["--non-interactive".to_string()],
        }
    }
}

/// 跑一条 svn 命令。
///
/// 关键设计（每一条都是踩过的坑）：
/// - **参数数组传递，永不 `sh -c`** —— 路径含 `;` `$` 空格都安全。
/// - **`--non-interactive` + `stdin=null`** —— 否则 svn 会挂在等用户输入的提示上。
/// - **`LC_ALL=C.UTF-8`** —— 错误文案不被本地化，解析/匹配才稳定。
/// - **stdout/stderr 并发读** —— 先 wait 再读会因管道写满（约 64KB）而死锁，diff 轻易就超。
/// - **超时后 kill + wait** —— 不留僵尸进程。
pub fn run(exe: &Path, args: &[&str], cwd: &Path, opts: &RunOpts) -> Result<RawOutput> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(&opts.global)
        .args(args)
        .current_dir(cwd)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(map_spawn_err)?;

    // 必须把 handle 移出 child，才能在等待的同时并发读。
    let mut out_h = child.stdout.take();
    let mut err_h = child.stderr.take();

    let t_out = std::thread::spawn(move || read_all(&mut out_h));
    let t_err = std::thread::spawn(move || read_all(&mut err_h));

    let status = wait_timeout(&mut child, opts.timeout)?;

    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();

    let out = RawOutput { stdout, stderr, code: status.code() };

    if !out.is_success() && !opts.allow_fail {
        if Error::is_lock_error(&out.stderr) {
            return Err(Error::Locked(cwd.to_path_buf()));
        }
        // 用 with_explanation 而不是直接 SvnFailed：
        // 让"E170013 连不上服务器"这类常见错误能显示成人话 + 下一步建议。
        // 识别不出来的仍走 SvnFailed（保留原始 stderr）。
        return Err(crate::domain::with_explanation(
            out.code.unwrap_or(-1),
            out.stderr,
        ));
    }

    Ok(out)
}

fn read_all<R: Read>(h: &mut Option<R>) -> String {
    let mut buf = Vec::new();
    if let Some(h) = h.as_mut() {
        let _ = h.read_to_end(&mut buf);
    }
    // 非 UTF-8（二进制 diff）用替换字符兜底，不能让整条命令因此失败。
    String::from_utf8_lossy(&buf).into_owned()
}

/// 轮询式超时等待。std 没有 `wait_timeout`，只能 sleep 轮询。
fn wait_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Ok(st),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Timeout(timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }
}

/// 带 stdin 的执行（`svn commit -F -` 专用）。
///
/// 与 `run` 的区别只有一点：stdin 是 piped 并在 spawn 后写入，而不是 null。
///
/// # 为什么必须并发读 stdout/stderr
/// 我们一边写 stdin 一边等子进程，若不同时排空 stdout/stderr，
/// 子进程写满管道（约 64KB）就会阻塞，我们写 stdin 也会阻塞 —— 经典死锁。
/// 提交大改动时 commit 的输出很容易超过 64KB，所以这不是理论问题。
pub fn run_with_stdin(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    input: &str,
    opts: &RunOpts,
) -> Result<RawOutput> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(&opts.global)
        .args(args)
        .current_dir(cwd)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(map_spawn_err)?;

    let mut out_h = child.stdout.take();
    let mut err_h = child.stderr.take();

    // 先起读线程，再写 stdin —— 顺序不能反。
    let t_out = std::thread::spawn(move || read_all(&mut out_h));
    let t_err = std::thread::spawn(move || read_all(&mut err_h));

    let write_result = (|| -> std::io::Result<()> {
        use std::io::Write;
        if let Some(s) = child.stdin.as_mut() {
            s.write_all(input.as_bytes())?;
            s.flush()?;
        }
        Ok(())
    })();
    // 必须 drop stdin，否则 svn 会一直等 EOF。
    drop(child.stdin.take());

    // BrokenPipe：svn 在读完前退出（例如提交信息无效）。不当成致命错误。
    if let Err(e) = write_result {
        if e.kind() != std::io::ErrorKind::BrokenPipe {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io(e));
        }
    }

    let status = wait_timeout(&mut child, opts.timeout)?;
    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();

    let out = RawOutput { stdout, stderr, code: status.code() };
    if !out.is_success() && !opts.allow_fail {
        if Error::is_lock_error(&out.stderr) {
            return Err(Error::Locked(cwd.to_path_buf()));
        }
        // 用 with_explanation 而不是直接 SvnFailed：
        // 让"E170013 连不上服务器"这类常见错误能显示成人话 + 下一步建议。
        // 识别不出来的仍走 SvnFailed（保留原始 stderr）。
        return Err(crate::domain::with_explanation(
            out.code.unwrap_or(-1),
            out.stderr,
        ));
    }
    Ok(out)
}

/// 把「找不到可执行文件」翻译成领域错误，而不是裸露的 `io::Error`。
fn map_spawn_err(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::SvnNotFound
    } else {
        Error::Io(e)
    }
}
