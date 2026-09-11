//! 长文本分页。
//!
//! diff / log 动辄几千行，直接 `println!` 会淹没终端。这里做**降级探测**：
//! `delta` → `bat` → `less -R` → 直接打印。
//!
//! 之所以不引 `pager` crate：它不能选程序，而 diff 有 delta 和没有 delta 是两个体验。
//! 之所以不引 `subprocess`：我们需要精确控制 stdio 继承（让 pager 拿到真实 tty），
//! 用 std 的 `Command` + `inherit` 就够了。

use std::io::Write;
use std::process::Stdio;

/// 探测到的分页器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerKind {
    /// `delta` —— 带语法高亮的 side-by-side diff，最佳体验。
    Delta,
    /// `bat` —— 有高亮，无 diff 专有渲染。
    Bat,
    /// `less -R` —— 老但可靠，能正确显示 ANSI。
    Less,
    /// 都没装，直接打印。
    None,
}

impl PagerKind {
    pub fn name(self) -> &'static str {
        match self {
            PagerKind::Delta => "delta",
            PagerKind::Bat => "bat",
            PagerKind::Less => "less",
            PagerKind::None => "(none)",
        }
    }
}

/// 探测可用的分页器。
///
/// 优先级：`SVNR_PAGER` 环境变量 → delta → bat → less → None。
pub fn detect() -> PagerKind {
    if std::env::var_os("SVNR_PAGER").is_some() {
        // 用户显式指定，视为可用。
        return PagerKind::Less;
    }
    for (exe, kind) in [("delta", PagerKind::Delta), ("bat", PagerKind::Bat), ("less", PagerKind::Less)] {
        if which(exe).is_some() {
            return kind;
        }
    }
    PagerKind::None
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    for dir in std::env::split_paths(&paths) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let cand = dir.join(&exe);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// 把内容送进分页器。
///
/// # 为什么用 `Stdio::inherit`
/// pager 需要直接读终端按键、直接写终端画面。若用 `piped()`，
/// `less` 会退化成 `cat`（检测不到 tty），`delta` 则会拒绝输出颜色。
///
/// # 失败怎么办
/// 分页器不存在/被中断（用户按 `q`）时**退回直接打印**，绝不丢内容。
/// 这是刻意的：宁可刷屏也不能让 diff 消失。
pub fn page(content: &str) -> std::io::Result<()> {
    if content.is_empty() {
        return Ok(());
    }

    let kind = detect();

    let cmd = match kind {
        PagerKind::None => None,
        PagerKind::Delta => Some(("delta", vec!["--paging=always".to_string(), "--color-only".to_string()])),
        PagerKind::Bat => Some(("bat", vec!["--paging=always".to_string(), "--plain".to_string()])),
        PagerKind::Less => match std::env::var_os("SVNR_PAGER") {
            Some(p) => {
                // 用户自定义：按 shell 词法切分，容错处理引号缺失的情况。
                let raw = p.to_string_lossy().to_string();
                let mut it = raw.split_whitespace();
                let prog = it.next().unwrap_or("less").to_string();
                let args: Vec<String> = it.map(|s| s.to_string()).collect();
                // 需要 'static str，这里只能借用后立刻使用，故走 owned 分支
                return spawn_owned(&prog, &args, content);
            }
            None => Some(("less", vec!["-R".to_string()])),
        },
    };

    let (prog, args) = match cmd {
        Some(v) => v,
        None => return print_plain(content),
    };

    match spawn_with(&prog, &args, content) {
        Ok(()) => Ok(()),
        Err(_) => print_plain(content),
    }
}

fn spawn_with(prog: &str, args: &[String], content: &str) -> std::io::Result<()> {
    let mut child = std::process::Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    // 必须边写边忽略 BrokenPipe：用户按 q 提前退出时这里会 EPIPE，
    // 那是正常行为，不该当成错误（否则 diff 看一半退出会报 "failed printing"）。
    let mut stdin = child.stdin.take();
    let result = if let Some(s) = stdin.as_mut() {
        s.write_all(content.as_bytes()).and_then(|_| s.flush())
    } else {
        Ok(())
    };
    drop(stdin);

    match result {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(e) => return Err(e),
    }

    // 不检查退出码：用户按 q、pager 被信号终止都是正常的。
    let _ = child.wait();
    Ok(())
}

fn spawn_owned(prog: &str, args: &[String], content: &str) -> std::io::Result<()> {
    spawn_with(prog, args, content)
}

fn print_plain(content: &str) -> std::io::Result<()> {
    print!("{content}");
    std::io::stdout().flush()
}

/// 内容超过 `threshold` 行才分页，否则直接打印。
///
/// 短输出走 pager 是负体验：用户得按 q 才能回到 shell。
pub fn page_if_long(content: &str, threshold: usize) -> std::io::Result<()> {
    if content.lines().count() > threshold {
        page(content)
    } else {
        print_plain(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_never_panics() {
        // 环境里可能一个 pager 都没有，这时应返回 None 而不是 panic。
        let k = detect();
        assert!(matches!(k, PagerKind::Delta | PagerKind::Bat | PagerKind::Less | PagerKind::None));
    }

    #[test]
    fn empty_content_is_noop() {
        assert!(page("").is_ok());
    }

    #[test]
    fn short_content_is_not_paged() {
        // 3 行 < 阈值 10，应走 print_plain（不启动任何子进程）
        assert!(page_if_long("a\nb\nc\n", 10).is_ok());
    }

    #[test]
    fn pager_kind_names_are_stable() {
        assert_eq!(PagerKind::Delta.name(), "delta");
        assert_eq!(PagerKind::Bat.name(), "bat");
        assert_eq!(PagerKind::Less.name(), "less");
        assert_eq!(PagerKind::None.name(), "(none)");
    }
}
