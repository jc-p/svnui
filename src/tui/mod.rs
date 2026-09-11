//! TUI 入口。
//!
//! 只有 `run()` 是公开的 —— 它负责接管终端、跑事件循环、并在**任何情况下**
//! 恢复终端状态（包括 panic）。

pub mod app;
pub mod panels;
pub mod theme;

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
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
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;
        Ok(Self { term })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
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
        guard.term.draw(|f| {
            let area = f.area();
            app.render(f, area);
        })?;

        // 轮询式事件读取。250ms 超时让界面不至于卡死在等待输入
        // （后续接 daemon 增量刷新时，这个超时正好用来检查状态变化）。
        if event::poll(Duration::from_millis(250))? {
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
                Event::Resize(..) => {
                    // 什么都不做：下一帧 draw 会自动用新尺寸
                }
                _ => {}
            }
        }
    }

    Ok(())
}
