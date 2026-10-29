//! TUI 入口。
//!
//! 只有 `run()` 是公开的 —— 它负责接管终端、跑事件循环、并在**任何情况下**
//! 恢复终端状态（包括 panic）。

pub mod app;
mod image_support;
pub mod panels;
mod preview_worker;
mod refresh;
mod standalone_preview;
pub mod theme;

use std::io::{self, Stdout};
use std::process::Command;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
// 括号粘贴：必须开。
//
// 不开的话，粘贴内容会被终端以 `\x1b[200~...\x1b[201~` 包着发过来，
// crossterm 不认识这个序列，会把开头的 `\x1b` **单独解析成一个 Esc 键**。
// 后果：在提交框里 Cmd+V 粘贴 → 收到 Esc → 弹框被当成"取消"关掉，
// 刚写的东西全丢。开了之后整段粘贴作为一个 Paste 事件投递。
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
// 鼠标滚轮。开着它，滚轮才能滚预览（触控板横滑也能横向滚）。
//
// ⚠️ 代价：终端不再处理鼠标，文本选中复制会失效。
//    所以给了 M 键随时关掉，关掉后终端恢复正常选择行为。
//    （iTerm2 / Apple Terminal 里按住 Option 拖拽可以临时绕过，不用切。）
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::domain::Result;
use crate::svn::Svn;
use app::App;

type Backend = CrosstermBackend<Stdout>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalMode {
    Raw = 1,
    Alternate = 2,
    Paste = 4,
    Mouse = 8,
    HiddenCursor = 16,
}

#[derive(Default)]
struct TerminalState {
    active: AtomicU8,
}

impl TerminalState {
    fn restore_after_panic_with(
        &self,
        session_thread: std::thread::ThreadId,
        apply: &mut impl FnMut(TerminalMode, bool) -> io::Result<()>,
    ) -> io::Result<()> {
        // 后台任务可以捕获 panic 并继续工作，其 hook 不得让前台终端退出 raw mode。
        if std::thread::current().id() == session_thread {
            self.restore_with(apply)
        } else {
            Ok(())
        }
    }

    fn set_with(
        &self,
        mode: TerminalMode,
        enabled: bool,
        apply: &mut impl FnMut(TerminalMode, bool) -> io::Result<()>,
    ) -> io::Result<()> {
        let flag = mode as u8;
        // 控制序列可能在报错前已部分写出，启用失败也须尝试恢复。
        if enabled {
            self.active.fetch_or(flag, Ordering::SeqCst);
        }
        apply(mode, enabled)?;
        if !enabled {
            self.active.fetch_and(!flag, Ordering::SeqCst);
        }
        Ok(())
    }

    fn enter_with(
        &self,
        mouse: bool,
        apply: &mut impl FnMut(TerminalMode, bool) -> io::Result<()>,
    ) -> io::Result<()> {
        for mode in [
            TerminalMode::Raw,
            TerminalMode::Alternate,
            TerminalMode::Paste,
        ] {
            self.set_with(mode, true, apply)?;
        }
        if mouse {
            self.set_with(TerminalMode::Mouse, true, apply)?;
        }
        self.set_with(TerminalMode::HiddenCursor, true, apply)
    }

    fn restore_with(
        &self,
        apply: &mut impl FnMut(TerminalMode, bool) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut first_error = None;
        // 一项失败仍恢复其他模式，失败项保留供 Drop 再试。
        for mode in [
            TerminalMode::HiddenCursor,
            TerminalMode::Mouse,
            TerminalMode::Paste,
            TerminalMode::Alternate,
            TerminalMode::Raw,
        ] {
            if self.active.load(Ordering::SeqCst) & mode as u8 != 0 {
                if let Err(error) = self.set_with(mode, false, apply) {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn apply_terminal_mode(mode: TerminalMode, enabled: bool) -> io::Result<()> {
    let mut stdout = io::stdout();
    match (mode, enabled) {
        (TerminalMode::Raw, true) => enable_raw_mode(),
        (TerminalMode::Raw, false) => disable_raw_mode(),
        (TerminalMode::Alternate, true) => execute!(stdout, EnterAlternateScreen),
        (TerminalMode::Alternate, false) => execute!(stdout, LeaveAlternateScreen),
        (TerminalMode::Paste, true) => execute!(stdout, EnableBracketedPaste),
        (TerminalMode::Paste, false) => execute!(stdout, DisableBracketedPaste),
        (TerminalMode::Mouse, true) => execute!(stdout, EnableMouseCapture),
        (TerminalMode::Mouse, false) => execute!(stdout, DisableMouseCapture),
        (TerminalMode::HiddenCursor, true) => execute!(stdout, crossterm::cursor::Hide),
        (TerminalMode::HiddenCursor, false) => execute!(stdout, crossterm::cursor::Show),
    }
}

/// 首次改变终端前建立保障；Terminal::new 失败时也能清理。
struct TerminalModes(Arc<TerminalState>);

impl Drop for TerminalModes {
    fn drop(&mut self) {
        let _ = self.0.restore_with(&mut apply_terminal_mode);
    }
}

struct TerminalGuard {
    term: Terminal<Backend>,
    modes: TerminalModes,
}

impl TerminalGuard {
    fn enter(state: Arc<TerminalState>) -> io::Result<Self> {
        let modes = TerminalModes(state);
        // 鼠标默认关闭以便选择复制，M 键开启后单独记录。
        modes.0.enter_with(false, &mut apply_terminal_mode)?;
        let term = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Self { term, modes })
    }

    fn suspend(&mut self) -> io::Result<()> {
        self.modes.0.restore_with(&mut apply_terminal_mode)
    }

    fn resume(&mut self, mouse: bool) -> io::Result<()> {
        self.modes.0.enter_with(mouse, &mut apply_terminal_mode)?;
        self.term.clear()
    }

    fn set_mouse(&mut self, enabled: bool) -> io::Result<()> {
        self.modes
            .0
            .set_with(TerminalMode::Mouse, enabled, &mut apply_terminal_mode)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

/// 事件黑匣子（默认关闭）。
///
/// 排查"敲键没反应"时用：
///
/// ```sh
/// SVNUI_DEBUG_KEY=1 svnui tui    # 进去敲几下、动动鼠标、按 Esc 退出
/// cat /tmp/svnui-key.log
/// ```
///
/// 判读：
/// - 有 `Key` 行 → 事件到了程序，问题在分发（贴给我）
/// - 一堆 `Mouse`、几乎没有 `Key` → 鼠标事件把键盘挤死了（按 M 关掉再试）
/// - `Key` 的 kind 不是 `Press` → 就是上一版 `kind == Press` 把它挡掉的
/// - 啥都没有 → 终端没把事件投递进来（换 iTerm2 / 系统 Terminal 复测）
///
/// 为什么写文件而不是 eprintln：TUI 在 alternate screen 里，
/// 打出去的字会被界面覆盖，退出后也看不见。
fn evlog(ev: &Event) {
    // 鼠标事件可能上千条，别把日志写爆。
    const CAP: u32 = 500;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    if std::env::var_os("SVNUI_DEBUG_KEY").is_none() {
        return;
    }
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let line = match ev {
        Event::Key(k) => format!(
            "Key   code={:?} mod={:?} kind={:?}",
            k.code, k.modifiers, k.kind
        ),
        Event::Mouse(m) => format!("Mouse kind={:?} x={} y={}", m.kind, m.column, m.row),
        Event::Paste(t) => format!("Paste {} 字节", t.len()),
        Event::Resize(w, h) => format!("Resize {}x{}", w, h),
        _ => return,
    };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/svnui-key.log")
    {
        if n < CAP {
            let _ = writeln!(f, "{line}");
        } else if n == CAP {
            let _ = writeln!(f, "… 已达 {CAP} 条上限，停止记录（前面的足够判读了）");
        }
    }
}

/// 启动 TUI。
///
/// `svn` 由调用方构造（已 discover 过工作副本）。
pub fn run(svn: Svn) -> Result<()> {
    with_terminal_restore(|state| run_session(svn, None, state))
}

/// 从给定远端目录起步，不探测本地工作副本、不触发检出。
pub fn browse_repository(svn: Svn, url: &str) -> Result<()> {
    with_terminal_restore(|state| run_session(svn, Some(url), state))
}

/// 独立预览不构造 Svn/App，但共享完整的终端恢复保障。
pub fn preview(path: &std::path::Path) -> Result<()> {
    // 先读文件，路径/权限错误直接报告，不必进入 alternate screen。
    let content = panels::file_preview::load(path)?;
    with_terminal_restore(|state| standalone_preview::run(path, content, state))
}

fn with_terminal_restore(session: impl FnOnce(Arc<TerminalState>) -> Result<()>) -> Result<()> {
    let state = Arc::new(TerminalState::default());
    let session_thread = std::thread::current().id();
    // hook 先于终端初始化生效，panic 输出前恢复全部登记模式。
    let previous_hook = std::panic::take_hook();
    let panic_hook: Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync> =
        Arc::from(previous_hook);
    let hook_state = Arc::clone(&state);
    let hook = Arc::clone(&panic_hook);
    std::panic::set_hook(Box::new(move |info| {
        let _ = hook_state.restore_after_panic_with(session_thread, &mut apply_terminal_mode);
        hook(info);
    }));
    // set_hook 不能在展开期间执行；先捕获清理，再还原 hook、恢复展开。
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session(state)));
    std::panic::set_hook(Box::new(move |info| panic_hook(info)));
    match outcome {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn run_session(svn: Svn, repo_url: Option<&str>, state: Arc<TerminalState>) -> Result<()> {
    let mut guard = TerminalGuard::enter(state)?;

    // 开诊断时先清空日志，免得跟上一次的混在一起判读不了。
    if std::env::var_os("SVNUI_DEBUG_KEY").is_some() {
        let _ = std::fs::write("/tmp/svnui-key.log", "");
    }

    let picker = image_support::detect();
    let mut app = match repo_url {
        Some(url) => App::with_start_url(svn, picker, Some(url))?,
        None => App::with_image_picker(svn, picker)?,
    };

    loop {
        // 外部编辑器：必须先让出终端，否则 vim 在 raw mode + alternate screen 里
        // 会行为异常（方向键/退格/Ctrl-C 全乱）。让出 → 编辑 → 收回。
        if let Some((editor, path)) = app.take_editor_request() {
            guard.suspend()?;
            let status = Command::new(&editor).arg(&path).status();

            // 启动失败也恢复全部模式，恢复失败则退出并由守卫清理。
            guard.resume(app.mouse_on())?;

            match status {
                Ok(st) if st.success() => {
                    // 文件可能被改了，重新取一次 diff / 内容
                    app.refresh_preview_after_edit();
                }
                Ok(_) => {
                    app.set_notice(format!("{} 以非零状态退出", editor));
                }
                Err(e) => {
                    app.set_notice(format!("无法启动 {}：{}", editor, e));
                }
            }
        }

        // M 键改过鼠标开关就在这里生效（App 够不到 backend，只记了个标记）。
        if let Some(on) = app.take_mouse_toggle() {
            guard.set_mouse(on)?;
        }

        // 先收后台结果，再绘制，避免已完成的预览额外等待一次 poll。
        app.tick();
        guard.term.draw(|f| {
            let area = f.area();
            app.render(f, area);
        })?;

        // 轮询式事件读取。120ms 超时 —— 后台任务完成时靠这个 tick 刷新画面，
        // 太长会让"处理中…"到结果的切换显得迟钝。
        //
        // 有后台任务时压到 60ms：等待动画靠 poll 超时推进一次重绘，
        // 120ms 只有约 8fps，spinner 转起来一顿一顿的。
        let poll_ms = if app.is_busy() { 60 } else { 120 };
        if event::poll(Duration::from_millis(poll_ms))? {
            //
            // ⚠️ 为什么这里要排空队列而不是只读一个事件：
            //    开鼠标捕获时，鼠标移动会持续产生 MouseEvent。每帧只消费一个，
            //    键盘事件就被挤到队列后面 —— 表现是"敲键没反应、输入不进去"。
            //    budget 是防异常的：真来事件洪水时不至于把 UI 卡死。
            //
            let mut quit = false;
            let mut budget: u16 = 256;
            while budget > 0 {
                budget -= 1;
                let ev = event::read()?;
                evlog(&ev);
                match ev {
                    Event::Key(key) => {
                        // 只挡 Release，放行 Press 和 Repeat。
                        //
                        // ⚠️ 之前写成 `kind == Press`，把 Repeat 也挡在外面了。
                        //    问题是"是不是 Press"完全取决于终端的能力协商：
                        //    支持 kitty / ModifyOtherKeys 的终端可能不发 Press，
                        //    于是**所有按键都被静默丢弃** —— 界面能画、键盘全死，
                        //    跟"输入框是坏的"一模一样。
                        //    Repeat 放行是安全的：它本来就是"按住不放"的重复输入，
                        //    字符框里正常就该重复。
                        if key.kind != KeyEventKind::Release {
                            app.handle_key(key)?;
                            if app.should_quit() {
                                quit = true;
                                break;
                            }
                        }
                    }
                    // 粘贴：整段插入。开了 bracketed paste 才有这个事件。
                    // 不做逐字符模拟 —— 那样每个字符都会被当成快捷键判定一遍，
                    // 粘贴一段含 q / Esc 的文本会把界面点掉。
                    Event::Paste(text) => {
                        app.handle_paste(text);
                    }
                    Event::Mouse(m) => {
                        app.handle_mouse(m);
                    }
                    Event::Resize(..) => {
                        // 什么都不做：下一帧 draw 会自动用新尺寸
                    }
                    _ => {}
                }
                // 队列空了就走；还有就继续排。
                if !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }
            if quit {
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod terminal_tests {
    use super::{TerminalMode::*, *};

    const ENTER: [TerminalMode; 5] = [Raw, Alternate, Paste, Mouse, HiddenCursor];

    #[test]
    fn every_initialization_failure_cleans_attempted_modes_in_reverse_order() {
        for failed in 0..ENTER.len() {
            let state = TerminalState::default();
            let mut calls = Vec::new();
            let result = state.enter_with(true, &mut |mode, enabled| {
                calls.push((mode, enabled));
                if mode == ENTER[failed] {
                    Err(io::Error::other("injected initialization failure"))
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            state
                .restore_with(&mut |mode, enabled| {
                    calls.push((mode, enabled));
                    Ok(())
                })
                .unwrap();
            let expected: Vec<_> = ENTER[..=failed]
                .iter()
                .map(|mode| (*mode, true))
                .chain(ENTER[..=failed].iter().rev().map(|mode| (*mode, false)))
                .collect();
            assert_eq!(calls, expected, "failed stage {failed}");
            assert_eq!(state.active.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn cleanup_continues_after_failure_and_retries_only_failed_modes() {
        let state = TerminalState::default();
        state.enter_with(true, &mut |_, _| Ok(())).unwrap();
        let mut calls = Vec::new();
        assert!(state
            .restore_with(&mut |mode, enabled| {
                calls.push((mode, enabled));
                if mode == Paste {
                    Err(io::Error::other("injected cleanup failure"))
                } else {
                    Ok(())
                }
            })
            .is_err());
        assert_eq!(
            calls,
            vec![
                (HiddenCursor, false),
                (Mouse, false),
                (Paste, false),
                (Alternate, false),
                (Raw, false)
            ]
        );
        calls.clear();
        state
            .restore_with(&mut |mode, enabled| {
                calls.push((mode, enabled));
                Ok(())
            })
            .unwrap();
        assert_eq!(calls, vec![(Paste, false)]);
        state
            .restore_with(&mut |_, _| panic!("clean state must be a no-op"))
            .unwrap();
    }

    #[test]
    fn editor_round_trip_restores_paste_and_current_mouse_preference() {
        for mouse in [false, true] {
            let state = TerminalState::default();
            state.enter_with(mouse, &mut |_, _| Ok(())).unwrap();
            let mut calls = Vec::new();
            state
                .restore_with(&mut |mode, enabled| {
                    calls.push((mode, enabled));
                    Ok(())
                })
                .unwrap();
            assert!(calls.contains(&(Paste, false)));
            assert_eq!(calls.contains(&(Mouse, false)), mouse);
            assert_eq!(state.active.load(Ordering::SeqCst), 0);
            calls.clear();
            state
                .enter_with(mouse, &mut |mode, enabled| {
                    calls.push((mode, enabled));
                    Ok(())
                })
                .unwrap();
            assert!(calls.contains(&(Paste, true)));
            assert_eq!(calls.contains(&(Mouse, true)), mouse);
        }
    }

    #[test]
    fn disabled_mouse_is_not_reenabled_or_disabled_during_cleanup() {
        let state = TerminalState::default();
        state.enter_with(false, &mut |_, _| Ok(())).unwrap();
        state.set_with(Mouse, true, &mut |_, _| Ok(())).unwrap();
        state.set_with(Mouse, false, &mut |_, _| Ok(())).unwrap();
        state
            .restore_with(&mut |mode, _| {
                assert_ne!(mode, Mouse);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn interrupted_initialization_keeps_recovery_state_available() {
        let state = TerminalState::default();
        let result = std::panic::catch_unwind(|| {
            let _ = state.enter_with(false, &mut |mode, _| {
                if mode == Paste {
                    panic!("injected initialization panic");
                }
                Ok(())
            });
        });
        assert!(result.is_err());
        let mut calls = Vec::new();
        state
            .restore_with(&mut |mode, enabled| {
                calls.push((mode, enabled));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            calls,
            vec![(Paste, false), (Alternate, false), (Raw, false)]
        );
    }

    #[test]
    fn background_panic_recovery_does_not_restore_session_modes() {
        let state = Arc::new(TerminalState::default());
        state.enter_with(true, &mut |_, _| Ok(())).unwrap();
        let before = state.active.load(Ordering::SeqCst);
        let session_thread = std::thread::current().id();
        let background_state = Arc::clone(&state);
        // 只调用 hook 的恢复分支，不安装全局 hook，避免污染并行测试。
        std::thread::spawn(move || {
            background_state
                .restore_after_panic_with(session_thread, &mut |_, _| {
                    panic!("background panic must not change terminal modes")
                })
                .unwrap();
        })
        .join()
        .unwrap();
        assert_eq!(state.active.load(Ordering::SeqCst), before);
        let mut restored = Vec::new();
        state
            .restore_after_panic_with(session_thread, &mut |mode, enabled| {
                restored.push((mode, enabled));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            restored,
            vec![
                (HiddenCursor, false),
                (Mouse, false),
                (Paste, false),
                (Alternate, false),
                (Raw, false)
            ]
        );
        assert_eq!(state.active.load(Ordering::SeqCst), 0);
    }
}
