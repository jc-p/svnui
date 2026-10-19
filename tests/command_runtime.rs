//! 命令生命周期回归只创建临时目录和本测试程序的子进程，不接触用户 SVN 工作副本。
//! command_helper 是专用子进程入口，当前测试无配置时直接返回。
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use svnui::domain::Error;
use svnui::svn::command::{run, run_preview, run_streaming, run_with_stdin, CancelToken, RunOpts};

const CONFIG: &str = "command-helper-mode";
const ARGS: &[&str] = &["--exact", "command_helper", "--nocapture"];

fn opts(timeout: Duration) -> RunOpts {
    RunOpts {
        timeout,
        allow_fail: false,
        global: vec![],
    }
}

fn fixture(mode: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(CONFIG), mode).unwrap();
    dir
}

fn exe() -> PathBuf {
    std::env::current_exe().unwrap()
}

fn pid(dir: &Path) -> u32 {
    std::fs::read_to_string(dir.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn signal(pid: u32, signal: i32) -> i32 {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // SAFETY: PID 仅来自本测试创建的子进程标记；信号 0 只查询，9 只做测试清理。
    unsafe { kill(i32::try_from(pid).unwrap(), signal) }
}

fn assert_reaped(dir: &Path) {
    assert_eq!(signal(pid(dir), 0), -1, "direct child must be reaped");
    // ESRCH 在 Darwin/Linux 均为 3，std 对它的 ErrorKind 映射不是 NotFound 契约。
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3));
}

fn process_group() -> i32 {
    unsafe extern "C" {
        fn getpgrp() -> i32;
    }
    // SAFETY: POSIX getpgrp 没有参数和内存输入，只返回当前进程组。
    unsafe { getpgrp() }
}

fn assert_bounded(start: Instant) {
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "command outlived its deadline: {:?}",
        start.elapsed()
    );
}

#[test]
fn command_helper() {
    let Ok(mode) = std::fs::read_to_string(CONFIG) else {
        return;
    };
    std::fs::write("pid", std::process::id().to_string()).unwrap();
    match mode.as_str() {
        "process-group" => println!("group:{}", process_group()),
        "dual-output" => {
            let err = std::thread::spawn(|| {
                std::io::stderr()
                    .write_all(&vec![b'e'; 256 * 1024])
                    .unwrap();
            });
            std::io::stdout()
                .write_all(&vec![b'o'; 256 * 1024])
                .unwrap();
            err.join().unwrap();
        }
        "no-stdin" | "leaf" => std::thread::sleep(Duration::from_secs(15)),
        "broken-pipe" => {
            std::io::stderr()
                .write_all(b"deliberate child refusal")
                .unwrap();
            std::process::exit(7);
        }
        "stdin-echo-length" => {
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            println!("input-bytes:{}", input.len());
        }
        "huge-stdout" => {
            let mut out = std::io::stdout().lock();
            for _ in 0..1040 {
                if out.write_all(&vec![b'x'; 64 * 1024]).is_err() {
                    break;
                }
            }
        }
        "preview-truncate" => {
            let mut out = std::io::stdout().lock();
            for _ in 0..50 {
                out.write_all(&vec![b'x'; 64 * 1024]).unwrap();
            }
            std::io::stderr()
                .write_all(&vec![b'e'; 300 * 1024])
                .unwrap();
        }
        "long-lines" => {
            let mut out = std::io::stdout().lock();
            out.write_all(&vec![b'x'; 4 * 1024 * 1024]).unwrap();
            out.write_all(b"\nfollowing-line\n").unwrap();
        }
        "inherited-pipes" | "escaped-pipes" => {
            use std::os::unix::process::CommandExt;
            let leaf_dir = std::env::current_dir().unwrap().join("leaf");
            std::fs::create_dir(&leaf_dir).unwrap();
            std::fs::write(leaf_dir.join(CONFIG), "leaf").unwrap();
            let mut cmd = Command::new(exe());
            cmd.args(ARGS)
                .current_dir(&leaf_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            if mode == "escaped-pipes" {
                cmd.process_group(0);
            }
            let child = cmd.spawn().unwrap();
            std::fs::write("leaf-pid", child.id().to_string()).unwrap();
            let start = Instant::now();
            while !leaf_dir.join("pid").exists() {
                assert!(start.elapsed() < Duration::from_secs(2));
                std::thread::sleep(Duration::from_millis(5));
            }
            // 直接进程退出，但后代继承 stdout/stderr；执行器仍须遵守原始期限。
        }
        other => panic!("unknown helper mode: {other}"),
    }
    std::process::exit(0);
}

#[test]
fn drains_both_pipes_larger_than_os_capacity() {
    let dir = fixture("dual-output");
    let output = run(&exe(), ARGS, dir.path(), &opts(Duration::from_secs(5))).unwrap();
    assert!(output.stdout.ends_with(&"o".repeat(256 * 1024)));
    assert_eq!(output.stderr.len(), 256 * 1024);
    assert_reaped(dir.path());
}

#[test]
fn blocked_stdin_is_inside_execution_deadline_and_input_is_private() {
    let dir = fixture("no-stdin");
    let secret = "test-private-stdin-value".repeat(128 * 1024);
    let args = ARGS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let start = Instant::now();
    let error = run_with_stdin(
        &exe(),
        &args,
        dir.path(),
        &secret,
        &opts(Duration::from_millis(250)),
    )
    .unwrap_err();
    assert!(matches!(error, Error::Timeout(_)));
    assert!(!error.to_string().contains("test-private-stdin-value"));
    assert_bounded(start);
    assert_reaped(dir.path());
}

#[test]
fn stdin_writer_closes_eof_and_preserves_complete_input() {
    let dir = fixture("stdin-echo-length");
    let args = ARGS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let input = "中文正文".repeat(200_000);
    let output = run_with_stdin(
        &exe(),
        &args,
        dir.path(),
        &input,
        &opts(Duration::from_secs(5)),
    )
    .unwrap();
    assert!(output
        .stdout
        .contains(&format!("input-bytes:{}", input.len())));
    assert_reaped(dir.path());
}

#[test]
fn early_stdin_close_preserves_child_failure() {
    let dir = fixture("broken-pipe");
    let args = ARGS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let error = run_with_stdin(
        &exe(),
        &args,
        dir.path(),
        &"s".repeat(1024 * 1024),
        &opts(Duration::from_secs(5)),
    )
    .unwrap_err();
    match error {
        Error::SvnFailed { code, stderr } => {
            assert_eq!(code, 7);
            assert!(stderr.contains("deliberate child refusal"));
        }
        other => panic!("child error lost: {other}"),
    }
    assert_reaped(dir.path());
}

#[test]
fn complete_output_overflow_is_error_instead_of_partial_success() {
    let dir = fixture("huge-stdout");
    let error = run(&exe(), ARGS, dir.path(), &opts(Duration::from_secs(10))).unwrap_err();
    assert!(matches!(
        error,
        Error::OutputLimit {
            stream: "stdout",
            limit: 67_108_864
        }
    ));
    assert_reaped(dir.path());
}

#[test]
fn preview_truncation_is_explicit_and_stderr_is_bounded() {
    let dir = fixture("preview-truncate");
    let output = run_preview(
        &exe(),
        ARGS,
        dir.path(),
        &opts(Duration::from_secs(5)),
        &CancelToken::default(),
    )
    .unwrap();
    assert!(output.stdout_truncated && output.stderr_truncated);
    assert_eq!(output.output.stdout.len(), 2 * 1024 * 1024);
    assert!(output.output.stderr.len() < 257 * 1024);
    assert!(output.output.stderr.ends_with("[stderr truncated]"));
    assert_reaped(dir.path());
}

#[test]
fn streaming_limits_unterminated_line_and_continues_with_following_lines() {
    let dir = fixture("long-lines");
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_callback = seen.clone();
    let output = run_streaming(
        &exe(),
        ARGS,
        dir.path(),
        &opts(Duration::from_secs(5)),
        move |line| {
            seen_callback.lock().unwrap().push(line.into());
        },
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen.iter().all(|line| line.len() <= 64 * 1024));
    assert!(seen.iter().any(|line| line == "following-line"));
    assert!(output.stdout.len() <= 2 * 1024 * 1024);
    assert_reaped(dir.path());
}

#[test]
fn streaming_callback_panic_does_not_change_command_success() {
    let dir = fixture("long-lines");
    let output = run_streaming(
        &exe(),
        ARGS,
        dir.path(),
        &opts(Duration::from_secs(5)),
        |_| {
            panic!("simulated progress consumer failure");
        },
    )
    .unwrap();
    assert!(output.is_success());
    assert_reaped(dir.path());
}

#[test]
fn cancellation_reaps_running_preview_child() {
    let dir = fixture("no-stdin");
    let cancel = CancelToken::default();
    let worker_cancel = cancel.clone();
    let worker_dir = dir.path().to_path_buf();
    let worker = std::thread::spawn(move || {
        run_preview(
            &exe(),
            ARGS,
            &worker_dir,
            &opts(Duration::from_secs(10)),
            &worker_cancel,
        )
    });
    let start = Instant::now();
    while !dir.path().join("pid").exists() {
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(5));
    }
    cancel.cancel();
    assert!(matches!(worker.join().unwrap(), Err(Error::Cancelled)));
    assert_bounded(start);
    assert_reaped(dir.path());
}

#[test]
fn pre_cancelled_preview_never_spawns_child() {
    let dir = fixture("no-stdin");
    let cancel = CancelToken::default();
    cancel.cancel();
    assert!(matches!(
        run_preview(
            &exe(),
            ARGS,
            dir.path(),
            &opts(Duration::from_secs(5)),
            &cancel
        ),
        Err(Error::Cancelled)
    ));
    assert!(!dir.path().join("pid").exists());
}

struct LeafCleanup(u32);
impl Drop for LeafCleanup {
    fn drop(&mut self) {
        signal(self.0, 9);
    }
}

fn descendant_case(mode: &str, preview: bool) -> (tempfile::TempDir, LeafCleanup, Error) {
    let dir = fixture(mode);
    let start = Instant::now();
    let run_opts = opts(Duration::from_millis(600));
    let error = if preview {
        run_preview(&exe(), ARGS, dir.path(), &run_opts, &CancelToken::default()).unwrap_err()
    } else {
        run(&exe(), ARGS, dir.path(), &run_opts).unwrap_err()
    };
    let leaf_pid = std::fs::read_to_string(dir.path().join("leaf-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let cleanup = LeafCleanup(leaf_pid);
    assert_bounded(start);
    assert_reaped(dir.path());
    (dir, cleanup, error)
}

#[test]
fn inherited_descendant_pipes_do_not_extend_deadline() {
    let (_dir, leaf, error) = descendant_case("inherited-pipes", false);
    assert!(matches!(error, Error::Timeout(_)));
    // 普通命令保留原前台组，只回收直接子进程；后代由本测试显式清理。
    assert_eq!(signal(leaf.0, 0), 0);
}

#[test]
fn isolated_preview_kills_descendants_that_inherit_its_pipes() {
    let (_dir, leaf, error) = descendant_case("inherited-pipes", true);
    assert!(matches!(error, Error::Timeout(_)));
    // 被 init 回收前可能暂留僵尸，不能把 kill(pid, 0) 成功等同于仍在执行。
    let start = Instant::now();
    loop {
        let state = Command::new("ps")
            .args(["-p", &leaf.0.to_string(), "-o", "stat="])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&state.stdout);
        if state.trim().is_empty() || state.trim().starts_with('Z') {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "descendant remained live: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn detached_descendant_is_an_explicit_cleanup_boundary_not_a_pipe_deadlock() {
    let (_dir, leaf, error) = descendant_case("escaped-pipes", true);
    assert!(matches!(error, Error::Timeout(_)));
    // 脱离进程组的后代不在执行器清理范围；本测试持有 PID 并显式负责清理。
    assert_eq!(signal(leaf.0, 0), 0);
}

#[test]
fn only_cancelable_previews_change_process_group() {
    let normal = fixture("process-group");
    let output = run(&exe(), ARGS, normal.path(), &opts(Duration::from_secs(5))).unwrap();
    assert!(output
        .stdout
        .contains(&format!("group:{}", process_group())));
    let preview = fixture("process-group");
    let output = run_preview(
        &exe(),
        ARGS,
        preview.path(),
        &opts(Duration::from_secs(5)),
        &CancelToken::default(),
    )
    .unwrap();
    assert!(output
        .output
        .stdout
        .contains(&format!("group:{}", pid(preview.path()))));
}
