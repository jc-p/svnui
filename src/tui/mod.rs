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
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;
        Ok(Self { term })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.term.backend_mut(), DisableBracketedPaste);
        let _ = execute!(self.term.backend_mut(), LeaveAlternateScreen);
        let _ = self.term.show_cursor();
    }
}

/// 启动 TUI。
///
/// `svn` 由调用方构造（已 discover 过工作副本）。
pub fn run(svn: Svn) -> Result<()> {
    let mut guard = TerminalGuard::enter()?;

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
            let _ = execute!(guard.term.backend_mut(), LeaveAlternateScreen);
            let _ = guard.term.show_cursor();

            let status = Command::new(&editor).arg(&path).status();

            // 无论成功失败都要收回终端，否则界面就没了
            let _ = execute!(guard.term.backend_mut(), EnterAlternateScreen);
            let _ = enable_raw_mode();
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

        guard.term.draw(|f| {
            let area = f.area();
            app.render(f, area);
        })?;

        // 收后台任务结果（diff 预览 / 扫描 / add / update / commit）。
        // 放在 draw 之前，这样结果能在这一帧就显示出来。
        app.tick();

        // 轮询式事件读取。120ms 超时 —— 后台任务完成时靠这个 tick 刷新画面，
        // 太长会让"处理中…"到结果的切换显得迟钝。
        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key) => {
                    // 只处理按下，忽略 release / repeat
                    // （crossterm 在某些终端会重复投递）
                    if key.kind == KeyEventKind::Press {
                        app.handle_key(key)?;
                        if app.should_quit() {
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
                Event::Resize(..) => {
                    // 什么都不做：下一帧 draw 会自动用新尺寸
                }
                _ => {}
            }
        }
    }

    Ok(())
}
