//! 无需 SVN 的本地文件预览；复用文件分流、预览面板和终端守卫。

use std::{path::Path, sync::Arc, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::Paragraph,
};

use super::{
    image_support,
    panels::{file_preview::FileContent, preview::PreviewPanel},
    TerminalGuard, TerminalState,
};
use crate::domain::Result;

pub(super) fn run(path: &Path, content: FileContent, state: Arc<TerminalState>) -> Result<()> {
    let cfg = crate::config::Config::load();
    if let Some(notice) = super::theme::initialize(&cfg.ui.theme) {
        eprintln!("{notice}");
    }
    let mut guard = TerminalGuard::enter(state)?;
    let title = path.display().to_string();
    let mut panel = match content {
        FileContent::Text(text) | FileContent::Document(text) => PreviewPanel::file(title, &text),
        FileContent::Image { pixels, bytes } => {
            PreviewPanel::image(title, pixels, bytes, &image_support::detect())
        }
        FileContent::Disabled(reason) => PreviewPanel::notice(title, reason),
    };
    loop {
        guard.term.draw(|frame| {
            frame.render_widget(ratatui::widgets::Block::default().style(super::theme::Theme::base()), frame.area());
            let [body, hint] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
            panel.render(frame, body);
            frame.render_widget(Paragraph::new(" 本地预览 · q / Esc / Ctrl+C 退出").style(super::theme::Theme::dim()), hint);
        })?;
        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if handle_key(&mut panel, key) {
                        break;
                    }
                }
                // 调整窗口大小后，下一帧按新尺寸绘制；粘贴内容不作为快捷键。
                _ => {}
            }
        }
    }
    Ok(())
}

/// 返回 true 表示退出；保持与主界面全屏预览一致的滚动键位。
fn handle_key(panel: &mut PreviewPanel, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => return true,
        KeyCode::Char('j') | KeyCode::Down => panel.scroll_down(1),
        KeyCode::Char('k') | KeyCode::Up => panel.scroll_up(1),
        KeyCode::Char('d') | KeyCode::PageDown => panel.scroll_down(10),
        KeyCode::Char('u') | KeyCode::PageUp => panel.scroll_up(10),
        KeyCode::Char('g') => panel.scroll_top(),
        KeyCode::Char('h') | KeyCode::Left => panel.scroll_left(2),
        KeyCode::Char('l') | KeyCode::Right => panel.scroll_right(2),
        KeyCode::Char('>') | KeyCode::Char('.') => panel.scroll_right(panel.page_step()),
        KeyCode::Char('<') | KeyCode::Char(',') => panel.scroll_left(panel.page_step()),
        KeyCode::Char('0') => panel.scroll_home(),
        _ => {}
    }
    false
}
