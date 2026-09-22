use std::io::{BufRead, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::domain::{Error, Result};

/// 流式读取时，拼回 `RawOutput.stdout` 的上限。
///
/// `run_streaming` 的进度靠**回调**逐行递出去，这份全文只是为了兼容
/// `RawOutput` 的契约。检出几万文件时 svn 的 stdout 轻松几十 MB，
/// 全留在内存里纯粹是浪费 —— 2MB 足够诊断用。
const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;

/// 单行上限。
///
/// `read_until(b'\n')` 只有遇到换行才停。输出里没有 `\n` 时（二进制内容、
/// 异常的长行），一次调用会把整个流读进 `line`。
const MAX_LINE_BYTES: usize = 64 * 1024;

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

/// 跑一条 svn 命令，并**边跑边把 stdout 逐行交给回调**。
///
/// 和 `run` 唯一的区别在 stdout：这里按行切、读一行回调一次。
/// 长操作（检出大仓库）全靠它把进度递回 UI —— 否则界面只能干等几十分钟。
///
/// ⚠️ 回调在**读线程**里跑，不能碰终端；它 panic 只会丢进度（join 失败），
///    不会把主流程带崩。
/// ⚠️ 最终结果仍以返回的 `RawOutput` 为准 —— 别拿回调次数当"完成数"，
///    最后一行读完就不再回调了。
/// ⚠️ 返回的 `RawOutput.stdout` 有长度上限（[`MAX_STREAM_BYTES`]），
///    超出部分**不会**出现在里面。回调是完整的，需要全量就自己攒。
pub fn run_streaming<F>(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    opts: &RunOpts,
    on_line: F,
) -> Result<RawOutput>
where
    F: FnMut(&str) + Send + 'static,
{
    let mut cmd = std::process::Command::new(exe);
    cmd.args(&opts.global)
        .args(args)
        .current_dir(cwd)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(map_spawn_err)?;

    let mut out_h = child.stdout.take();
    let mut err_h = child.stderr.take();

    // stdout 逐行回调；stderr 仍整块读 —— 错误信息要完整才有诊断价值。
    let t_out = std::thread::spawn(move || stream_lines(&mut out_h, on_line));
    let t_err = std::thread::spawn(move || read_all(&mut err_h));

    let status = wait_timeout(&mut child, opts.timeout)?;

    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();

    let out = RawOutput { stdout, stderr, code: status.code() };

    if !out.is_success() && !opts.allow_fail {
        if Error::is_lock_error(&out.stderr) {
            return Err(Error::Locked(cwd.to_path_buf()));
        }
        return Err(crate::domain::with_explanation(
            out.code.unwrap_or(-1),
            out.stderr,
        ));
    }

    Ok(out)
}

/// 按行读 stdout，每行回调一次，同时拼回全文（`RawOutput.stdout` 仍要能用）。
///
/// 用 `read_until(b'\n')` 而不是 `read_to_end`：后者要等子进程退出才返回，
/// 进度就全堵到最后一口气才出来，等于没有。
fn stream_lines<F>(h: &mut Option<ChildStdout>, mut on_line: F) -> String
where
    F: FnMut(&str),
{
    let Some(h) = h.as_mut() else {
        return String::new();
    };
    let mut r = std::io::BufReader::new(h);
    let mut all = String::new();
    let mut line: Vec<u8> = Vec::with_capacity(256);
    loop {
        line.clear();
        match r.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                // 单行长度也要封顶。`read_until(b'\n')` 遇到**没有换行**的输出
                // （比如 svn 吐了一整块二进制）会把整个流塞进 `line`，
                // 一行几十 MB 照样爆。超了就只保留头部，回调照常给。
                if line.len() > MAX_LINE_BYTES {
                    line.truncate(MAX_LINE_BYTES);
                }
                // 非 UTF-8 用替换字符兜底：进度行不重要，不能让它断掉整条流。
                let s = String::from_utf8_lossy(&line);
                let t = s.trim_end_matches(['\r', '\n']);
                on_line(t);
                // 全文只在额度内拼。检出几万文件时 stdout 能轻松几十 MB，
                // 而调用方真正要的进度早就通过 on_line 拿全了 ——
                // 这份 String 留着只是为了兼容 RawOutput 的契约，封顶不影响功能。
                if all.len() < MAX_STREAM_BYTES {
                    all.push_str(t);
                    all.push('\n');
                }
            }
            // 读失败（子进程被 kill 等）：已读到的照常返回，不把命令判成失败。
            Err(_) => break,
        }
    }
    all
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
