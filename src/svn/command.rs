use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{Error, Result};

/// 状态、XML 和 CLI diff 必须完整；超过限额报错，禁止消费半份结果。
const MAX_COMPLETE_BYTES: usize = 64 * 1024 * 1024;
/// 预览和进度回显只保留有界正文，截断状态单独返回。
const MAX_PREVIEW_BYTES: usize = 2 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 256 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const IO_CHUNK_BYTES: usize = 16 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// 一次 svn 调用的原始结果。不在此结构中添加元数据，保留已有字面量兼容性。
#[derive(Debug, Clone, Default)]
pub struct RawOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

impl RawOutput {
    pub fn is_success(&self) -> bool {
        self.code == Some(0)
    }
}

/// 执行参数。现有调用形式保持不变；预览策略通过专用入口选择。
#[derive(Debug, Clone)]
pub struct RunOpts {
    pub timeout: Duration,
    pub allow_fail: bool,
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

/// 只读预览请求的合作式取消标记；不能据此重试写操作。
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// 截断元数据与正文分开，避免截断提示混入 XML、补丁或其他机器输入。
#[derive(Debug, Clone)]
pub struct PreviewOutput {
    pub output: RawOutput,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// 完整输出入口。Unix 上执行期限覆盖 stdin、输出排空及子进程等待。
///
/// 参数始终以数组传递；本模块不会通过 shell 拼接执行参数。
/// 非 Unix 的兼容执行器保留原有功能，但不保证中断后代持有的阻塞管道。
pub fn run(exe: &Path, args: &[&str], cwd: &Path, opts: &RunOpts) -> Result<RawOutput> {
    execute(exe, args, cwd, opts, None, false, None, None).map(|r| r.output)
}

/// 只读预览入口：允许正文截断，支持取消；普通 CLI diff 仍要求完整输出。
pub fn run_preview(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    opts: &RunOpts,
    cancel: &CancelToken,
) -> Result<PreviewOutput> {
    execute(exe, args, cwd, opts, None, true, Some(cancel), None)
}

/// 进度回调接收有界行；超长行保留前 64 KiB，并继续排空剩余字节。
/// 回显正文最多 2 MiB。回调须快速返回，不得操作终端或阻塞等待。
/// Unix 上回调由调用执行器的线程调用，不能依赖原先的内部读线程身份。
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
    execute(
        exe,
        args,
        cwd,
        opts,
        None,
        true,
        None,
        Some(Box::new(on_line)),
    )
    .map(|r| r.output)
}

/// stdin 同时用于提交信息和密码；绝不把其内容加入错误或诊断。
pub fn run_with_stdin(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    input: &str,
    opts: &RunOpts,
) -> Result<RawOutput> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    execute(exe, &args, cwd, opts, Some(input), false, None, None).map(|r| r.output)
}

/// 独立后台文本任务：Unix 上以独立进程组运行，超时清理继承管道的后代。
/// stdin 不进入参数或错误信息；普通 SVN 前台入口的 Ctrl+C 语义不变。
pub fn run_isolated_with_stdin(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    input: &str,
    opts: &RunOpts,
) -> Result<RawOutput> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let cancel = CancelToken::default();
    execute(exe, &args, cwd, opts, Some(input), false, Some(&cancel), None)
        .map(|r| r.output)
}

type LineCallback = Box<dyn FnMut(&str) + Send>;

/// 在字节到达时限额，避免先分配整行/全文再截断。
struct Capture {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}

impl Capture {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let retained = bytes.len().min(self.limit.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&bytes[..retained]);
        self.truncated |= retained < bytes.len();
    }

    fn text(&self) -> String {
        // 截断发生在 UTF-8 码点中间时丢弃不完整尾部，而非生成误导的半个字符。
        let bytes = if self.truncated {
            match std::str::from_utf8(&self.bytes) {
                Err(e) if e.error_len().is_none() => &self.bytes[..e.valid_up_to()],
                _ => &self.bytes,
            }
        } else {
            &self.bytes
        };
        String::from_utf8_lossy(bytes).into_owned()
    }
}

struct Lines {
    callback: Option<LineCallback>,
    line: Vec<u8>,
    truncated: bool,
}

impl Lines {
    fn new(callback: LineCallback) -> Self {
        Self {
            callback: Some(callback),
            line: Vec::new(),
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            let remaining = MAX_LINE_BYTES.saturating_sub(self.line.len());
            self.line
                .extend_from_slice(&part[..part.len().min(remaining)]);
            self.truncated |= part.len() > remaining;
            if part.last() == Some(&b'\n') {
                self.emit();
            }
        }
    }

    fn emit(&mut self) {
        if self.line.is_empty() && !self.truncated {
            return;
        }
        let capture = Capture {
            bytes: std::mem::take(&mut self.line),
            limit: MAX_LINE_BYTES,
            truncated: self.truncated,
        };
        let mut text = capture.text();
        // 无效 UTF-8 的替换字符可能膨胀成三个字节，回调仍遵守字节上限。
        if text.len() > MAX_LINE_BYTES {
            let mut end = MAX_LINE_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        if let Some(callback) = self.callback.as_mut() {
            // 保留旧契约：进度回调 panic 只停止进度，不能把已执行的写操作判成失败。
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                callback(text.trim_end_matches(['\r', '\n']));
            }))
            .is_err()
            {
                self.callback = None;
            }
        }
        self.truncated = false;
    }
}

fn finish(
    stdout: Capture,
    stderr: Capture,
    status: ExitStatus,
    cwd: &Path,
    opts: &RunOpts,
) -> Result<PreviewOutput> {
    let mut diagnostic = stderr.text();
    if stderr.truncated {
        diagnostic.push_str("\n[stderr truncated]");
    }
    let out = RawOutput {
        stdout: stdout.text(),
        stderr: diagnostic,
        code: status.code(),
    };
    if !out.is_success() && !opts.allow_fail {
        if Error::is_lock_error(&out.stderr) {
            return Err(Error::Locked(cwd.to_path_buf()));
        }
        return Err(crate::domain::with_explanation(
            out.code.unwrap_or(-1),
            out.stderr,
        ));
    }
    Ok(PreviewOutput {
        output: out,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
    })
}

fn command(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    opts: &RunOpts,
    has_input: bool,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(&opts.global)
        .args(args)
        .current_dir(cwd)
        .env("LC_ALL", "C.UTF-8")
        .stdin(if has_input {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// guard 在配置管道前建立；所有 ?、取消、超时和回调 panic 均回收直接子进程。
struct ChildGuard {
    child: Child,
    finished: bool,
    isolated_group: bool,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.finished {
            #[cfg(unix)]
            if self.isolated_group {
                unix_pipe::kill_group(self.child.id());
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn execute(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    opts: &RunOpts,
    input: Option<&str>,
    truncate_stdout: bool,
    cancel: Option<&CancelToken>,
    callback: Option<LineCallback>,
) -> Result<PreviewOutput> {
    use std::os::unix::process::CommandExt;

    check_deadline(Instant::now(), opts.timeout, cancel)?;
    let start = Instant::now();
    let mut cmd = command(exe, args, cwd, opts, input.is_some());
    // 普通 CLI 保留前台组，Ctrl+C 仍同时送达 svn；仅只读预览隔离以便取消后代。
    let isolated_group = cancel.is_some();
    if isolated_group {
        cmd.process_group(0);
    }
    let child = cmd.spawn().map_err(map_spawn_err)?;
    let mut guard = ChildGuard {
        child,
        finished: false,
        isolated_group,
    };
    let mut out_h = guard.child.stdout.take();
    let mut err_h = guard.child.stderr.take();
    let mut in_h = guard.child.stdin.take();
    if let Some(h) = out_h.as_ref() {
        unix_pipe::nonblocking(h)?;
    }
    if let Some(h) = err_h.as_ref() {
        unix_pipe::nonblocking(h)?;
    }
    if let Some(h) = in_h.as_ref() {
        unix_pipe::nonblocking(h)?;
    }

    let mut stdout = Capture::new(if truncate_stdout {
        MAX_PREVIEW_BYTES
    } else {
        MAX_COMPLETE_BYTES
    });
    let mut stderr = Capture::new(MAX_DIAGNOSTIC_BYTES);
    let mut lines = callback.map(Lines::new);
    let mut status = None;
    let input = input.unwrap_or("").as_bytes();
    let mut written = 0;
    loop {
        check_deadline(start, opts.timeout, cancel)?;
        // 每轮每个流只做一次有界 IO，持续输出也不能饿死超时、取消或 stdin。
        let mut progressed = read_chunk(&mut out_h, &mut stdout, lines.as_mut())?;
        progressed |= read_chunk(&mut err_h, &mut stderr, None)?;
        if stdout.truncated && !truncate_stdout {
            return Err(Error::OutputLimit {
                stream: "stdout",
                limit: MAX_COMPLETE_BYTES,
            });
        }
        if let Some(h) = in_h.as_mut() {
            if written == input.len() {
                in_h = None;
            } else {
                let end = input.len().min(written.saturating_add(IO_CHUNK_BYTES));
                match h.write(&input[written..end]) {
                    Ok(0) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "svn stdin write returned zero",
                        )
                        .into())
                    }
                    Ok(n) => {
                        written += n;
                        progressed = true;
                    }
                    Err(e) if e.kind() == io::ErrorKind::BrokenPipe => in_h = None,
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if status.is_none() {
            status = guard.child.try_wait()?;
        }
        if let Some(status) = status {
            if out_h.is_none() && err_h.is_none() {
                // 子进程已退出，即使未消费完输入也无需继续向不存在的进程写。
                drop(in_h);
                if let Some(lines) = lines.as_mut() {
                    lines.emit();
                }
                guard.finished = true;
                return finish(stdout, stderr, status, cwd, opts);
            }
        }
        // 有输出时继续排空，空闲时退让。大输出仍每轮检查期限。
        if !progressed {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

#[cfg(unix)]
fn read_chunk<R: Read>(
    handle: &mut Option<R>,
    capture: &mut Capture,
    lines: Option<&mut Lines>,
) -> Result<bool> {
    let Some(h) = handle.as_mut() else {
        return Ok(false);
    };
    let mut buf = [0_u8; IO_CHUNK_BYTES];
    match h.read(&mut buf) {
        Ok(0) => *handle = None,
        Ok(n) => {
            capture.push(&buf[..n]);
            if let Some(lines) = lines {
                lines.push(&buf[..n]);
            }
            return Ok(true);
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) => {}
        Err(e) => return Err(e.into()),
    }
    Ok(false)
}

fn check_deadline(start: Instant, timeout: Duration, cancel: Option<&CancelToken>) -> Result<()> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(Error::Cancelled);
    }
    if start.elapsed() >= timeout {
        return Err(Error::Timeout(timeout.as_secs()));
    }
    Ok(())
}

#[cfg(unix)]
mod unix_pipe {
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::raw::c_int;

    // fcntl 的命令值跨 Darwin/Linux 相同，O_NONBLOCK 必须分别定义。
    const F_GETFL: c_int = 3;
    const F_SETFL: c_int = 4;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    const O_NONBLOCK: c_int = 0x0004;
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    const O_NONBLOCK: c_int = 0x0800;

    unsafe extern "C" {
        fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
        fn kill(pid: c_int, signal: c_int) -> c_int;
    }

    pub(super) fn nonblocking(handle: &impl AsRawFd) -> io::Result<()> {
        let fd = handle.as_raw_fd();
        // SAFETY: fd 来自当前拥有的活跃管道，GETFL 不修改内存，SETFL 保留原标记。
        let flags = unsafe { fcntl(fd, F_GETFL) };
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 第三参数是 SETFL 所需的整型标志，不传指针。
        if unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn kill_group(id: u32) {
        if let Ok(pid) = c_int::try_from(id) {
            // SAFETY: 此 PID 是 execute 刚创建的独立进程组 ID；负值仅作用该组。
            // 已退出/不存在的组允许返回 ESRCH。脱离进程组的后代不在清理承诺内。
            unsafe {
                kill(-pid, 9);
            }
        }
    }
}

/// 非 Unix 兼容路径仍并发排空输出、有界保存；平台原生阻塞 IO 的中断不在本轮承诺内。
/// 超时返回时可能留下等待继承句柄的读线程，需后续 Windows 专项处理。
#[cfg(not(unix))]
#[allow(clippy::too_many_arguments)]
fn execute(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    opts: &RunOpts,
    input: Option<&str>,
    truncate_stdout: bool,
    cancel: Option<&CancelToken>,
    mut callback: Option<LineCallback>,
) -> Result<PreviewOutput> {
    use std::sync::mpsc;
    check_deadline(Instant::now(), opts.timeout, cancel)?;
    let start = Instant::now();
    let child = command(exe, args, cwd, opts, input.is_some())
        .spawn()
        .map_err(map_spawn_err)?;
    let mut guard = ChildGuard {
        child,
        finished: false,
        isolated_group: false,
    };
    let (tx, rx) = mpsc::sync_channel(3);
    enum Event {
        Output(bool, io::Result<Capture>),
        Input(io::Result<()>),
    }
    for (is_stdout, handle) in [
        (
            true,
            guard
                .child
                .stdout
                .take()
                .map(|h| Box::new(h) as Box<dyn Read + Send>),
        ),
        (
            false,
            guard
                .child
                .stderr
                .take()
                .map(|h| Box::new(h) as Box<dyn Read + Send>),
        ),
    ] {
        let tx = tx.clone();
        let mut lines = if is_stdout {
            callback.take().map(Lines::new)
        } else {
            None
        };
        let limit = if !is_stdout {
            MAX_DIAGNOSTIC_BYTES
        } else if truncate_stdout {
            MAX_PREVIEW_BYTES
        } else {
            MAX_COMPLETE_BYTES
        };
        // 每条管道固定一个线程，不按输出块创建线程。
        std::thread::spawn(move || {
            let result = (|| -> io::Result<Capture> {
                let mut capture = Capture::new(limit);
                if let Some(mut handle) = handle {
                    let mut buf = [0_u8; IO_CHUNK_BYTES];
                    loop {
                        match handle.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                capture.push(&buf[..n]);
                                if let Some(lines) = lines.as_mut() {
                                    lines.push(&buf[..n]);
                                }
                                if is_stdout && capture.truncated && !truncate_stdout {
                                    break;
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
                if let Some(lines) = lines.as_mut() {
                    lines.emit();
                }
                Ok(capture)
            })();
            let _ = tx.send(Event::Output(is_stdout, result));
        });
    }
    let mut input_done = input.is_none();
    if let Some(input) = input {
        let input = input.as_bytes().to_vec();
        let mut handle = guard.child.stdin.take();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let result = if let Some(handle) = handle.as_mut() {
                handle.write_all(&input)
            } else {
                Ok(())
            };
            drop(handle);
            let _ = tx.send(Event::Input(result));
        });
    }
    drop(tx);
    let mut stdout = None;
    let mut stderr = None;
    let mut status = None;
    loop {
        check_deadline(start, opts.timeout, cancel)?;
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Output(true, capture) => {
                    let capture = capture?;
                    if capture.truncated && !truncate_stdout {
                        return Err(Error::OutputLimit {
                            stream: "stdout",
                            limit: MAX_COMPLETE_BYTES,
                        });
                    }
                    stdout = Some(capture);
                }
                Event::Output(false, capture) => stderr = Some(capture?),
                Event::Input(Err(e)) if e.kind() != io::ErrorKind::BrokenPipe => {
                    return Err(e.into())
                }
                Event::Input(_) => input_done = true,
            }
        }
        if status.is_none() {
            status = guard.child.try_wait()?;
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() && input_done {
            let stdout = stdout.take().expect("checked stdout");
            guard.finished = true;
            return finish(
                stdout,
                stderr.take().expect("checked stderr"),
                status.expect("checked status"),
                cwd,
                opts,
            );
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn map_spawn_err(e: io::Error) -> Error {
    if e.kind() == io::ErrorKind::NotFound {
        Error::SvnNotFound
    } else {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_exact_limit_and_utf8_boundary() {
        let mut capture = Capture::new(4);
        capture.push(b"abcd");
        assert!(!capture.truncated);
        capture.push(b"e");
        assert!(capture.truncated);
        assert_eq!(capture.text(), "abcd");
        let mut capture = Capture::new(4);
        capture.push("中文".as_bytes());
        assert_eq!(capture.text(), "中");
    }

    #[test]
    fn line_buffer_is_bounded_before_newline_and_continues_after_it() {
        let observed = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let output = observed.clone();
        let mut lines = Lines::new(Box::new(move |s| output.lock().unwrap().push(s.into())));
        for _ in 0..10 {
            lines.push(&vec![b'x'; MAX_LINE_BYTES]);
        }
        assert_eq!(lines.line.len(), MAX_LINE_BYTES);
        assert!(observed.lock().unwrap().is_empty());
        lines.push(b"\nnext\n");
        let output = observed.lock().unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].len(), MAX_LINE_BYTES);
        assert_eq!(output[1], "next");
    }

    #[cfg(unix)]
    #[test]
    fn read_errors_are_not_partial_success() {
        struct BadReader;
        impl Read for BadReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "test read failed",
                ))
            }
        }
        let error = read_chunk(&mut Some(BadReader), &mut Capture::new(8), None).unwrap_err();
        assert!(matches!(error, Error::Io(e) if e.kind() == io::ErrorKind::InvalidData));
    }

    #[test]
    fn invalid_utf8_line_also_obeys_callback_byte_limit() {
        let observed = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let output = observed.clone();
        let mut lines = Lines::new(Box::new(move |s| output.lock().unwrap().push(s.into())));
        lines.push(&vec![0xff; MAX_LINE_BYTES]);
        lines.push(b"\n");
        let output = observed.lock().unwrap();
        assert_eq!(output.len(), 1);
        assert!(output[0].len() <= MAX_LINE_BYTES);
    }
}
