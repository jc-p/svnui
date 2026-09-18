//! TUI 入口。
//!
//! 只有 `run()` 是公开的 —— 它负责接管终端、跑事件循环、并在**任何情况下**
//! 恢复终端状态（包括 panic）。

pub mod app;
pub mod panels;
pub mod theme;

use std::io::{self, Stdout};
use std::process::Command;
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
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste,
};
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

/// 终端守卫。
///
/// ⚠️ 为什么要有这个类型：TUI 一旦进了 raw mode + alternate screen，
///    如果中途 panic 或提前 return，用户的终端就废了（输入不回显、看不到历史）。
///
///    之前用 pager 时踩过类似的坑。所以这里用 Drop 兜底 ——
///    **任何**退出路径都会恢复终端，不需要每个分支手写 cleanup。
struct TerminalGuard {
    term: Terminal<Backend>,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        execute!(stdout, EnableBracketedPaste)?;
        // 鼠标捕获默认不开：开了之后终端不再处理鼠标（无法选中复制），
        // 且鼠标移动事件会淹没键盘事件。需要时按 M 键再开。
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;
        Ok(Self { term })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.term.backend_mut(), DisableBracketedPaste);
        // 必须关：不退出的编辑器（vim）会读到一串鼠标转义序列。
        let _ = execute!(self.term.backend_mut(), DisableMouseCapture);
        let _ = execute!(self.term.backend_mut(), LeaveAlternateScreen);
        let _ = self.term.show_cursor();
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
    let mut guard = TerminalGuard::enter()?;

    // 开诊断时先清空日志，免得跟上一次的混在一起判读不了。
    if std::env::var_os("SVNUI_DEBUG_KEY").is_some() {
        let _ = std::fs::write("/tmp/svnui-key.log", "");
    }

    // panic 兜底：默认 panic hook 会在 alternate screen 里打印，
    // 用户看不到任何东西。这里先恢复终端再让默认 hook 输出。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    let mut app = App::new(svn)?;

    loop {
        // 外部编辑器：必须先让出终端，否则 vim 在 raw mode + alternate screen 里
        // 会行为异常（方向键/退格/Ctrl-C 全乱）。让出 → 编辑 → 收回。
        if let Some((editor, path)) = app.take_editor_request() {
            let _ = disable_raw_mode();
            let _ = execute!(guard.term.backend_mut(), DisableMouseCapture);
            let _ = execute!(guard.term.backend_mut(), LeaveAlternateScreen);
            let _ = guard.term.show_cursor();

            let status = Command::new(&editor).arg(&path).status();

            // 无论成功失败都要收回终端，否则界面就没了
            let _ = execute!(guard.term.backend_mut(), EnterAlternateScreen);
            let _ = enable_raw_mode();
            // 按用户当前的开关恢复，别无脑开回来
            if app.mouse_on() {
                let _ = execute!(guard.term.backend_mut(), EnableMouseCapture);
            }
            let _ = guard.term.hide_cursor();
            let _ = guard.term.clear();

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
            if on {
                let _ = execute!(guard.term.backend_mut(), EnableMouseCapture);
            } else {
                let _ = execute!(guard.term.backend_mut(), DisableMouseCapture);
            }
        }

        guard.term.draw(|f| {
            let area = f.area();
            app.render(f, area);
        })?;

        // 收后台任务结果（diff 预览 / 扫描 / add / update / commit）。
        // 放在 draw 之前，这样结果能在这一帧就显示出来。
        app.tick();

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
