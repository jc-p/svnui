//! 通知摘要与完整详情。原文独立保留，换行、滚动和复制均不截断诊断信息。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::tui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    /// 兼容已有字符串通知；只检查标题，避免文件路径中的“失败”等字样误报。
    pub fn of(message: &str) -> Self {
        let heading = message
            .lines()
            .next()
            .unwrap_or("")
            .split(['：', ':'])
            .next()
            .unwrap_or("");
        if heading.contains("失败") || heading.starts_with("错误") || heading.starts_with("无法")
        {
            Self::Error
        } else if ["⚠", "请", "未", "没有", "不能"]
            .iter()
            .any(|s| heading.starts_with(s))
        {
            Self::Warning
        } else if heading.starts_with("已") {
            Self::Success
        } else {
            Self::Info
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Info => "提示",
            Self::Success => "成功",
            Self::Warning => "警告",
            Self::Error => "错误",
        }
    }

    pub fn style(self) -> Style {
        let color = match self {
            Self::Info => Color::Cyan,
            Self::Success => Color::Green,
            Self::Warning => Color::Yellow,
            Self::Error => Color::LightRed,
        };
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }
}

pub enum Action {
    None,
    Close,
    Copy,
}

pub struct NoticePanel {
    message: String,
    pub level: Level,
    rows: Vec<String>,
    wrap_width: Option<u16>,
    scroll: usize,
    viewport: usize,
    feedback: Option<(Level, String)>,
}

impl NoticePanel {
    pub fn new(message: String) -> Self {
        Self {
            level: Level::of(&message),
            message,
            rows: Vec::new(),
            wrap_width: None,
            scroll: 0,
            viewport: 0,
            feedback: None,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn feedback(&mut self, level: Level, message: String) {
        self.feedback = Some((level, message));
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_add(n).min(self.max_scroll());
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    fn max_scroll(&self) -> usize {
        self.rows.len().saturating_sub(self.viewport)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::F(4) => {
                return Action::Close
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => return Action::Copy,
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::PageDown => self.scroll_down(self.viewport.max(1)),
            KeyCode::PageUp => self.scroll_up(self.viewport.max(1)),
            KeyCode::Home | KeyCode::Char('g') => self.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.scroll = self.max_scroll(),
            _ => {}
        }
        Action::None
    }

    /// 摘要明确显示省略号，并为详情入口预留宽度；不修改独立保存的完整原文。
    pub fn summary(message: &str, width: u16) -> Line<'static> {
        let level = Level::of(message);
        let prefix = format!("[{}] ", level.label());
        let suffix = "  F4 详情";
        let reserved = Line::from(prefix.as_str()).width() + Line::from(suffix).width();
        if usize::from(width) <= reserved {
            return Line::from(Span::styled("F4 详情", level.style()));
        }
        let text = message.lines().next().unwrap_or("");
        let available = usize::from(width) - reserved;
        let truncated = Line::from(text).width() > available || message.contains('\n');
        let limit = available.saturating_sub(usize::from(truncated));
        let mut visible = String::new();
        let mut used = 0;
        for ch in text.chars() {
            let cells = Line::from(ch.to_string()).width();
            if used + cells > limit {
                break;
            }
            visible.push(ch);
            used += cells;
        }
        if truncated {
            visible.push('…');
        }
        Line::from(vec![
            Span::styled(prefix, level.style()),
            Span::styled(visible, level.style().remove_modifier(Modifier::BOLD)),
            Span::styled(suffix, level.style()),
        ])
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let pad_x = if area.width >= 40 { 2 } else { 0 };
        let pad_y = u16::from(area.height >= 8);
        let pop = Rect::new(
            area.x + pad_x,
            area.y + pad_y,
            area.width.saturating_sub(pad_x * 2),
            area.height.saturating_sub(pad_y * 2),
        );
        f.render_widget(Clear, pop);
        let block = theme::block()
            .borders(Borders::ALL)
            .title(format!(" {} · 完整提示 ", self.level.label()))
            .border_style(self.level.style());
        let inner = block.inner(pop);
        f.render_widget(block, pop);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let keys = if inner.width >= 80 {
            "↑↓ 滚动  PgUp/PgDn 翻页  Home/End 首尾  y 复制全文  Esc 关闭"
        } else {
            "↑↓ 滚动  PgUp/PgDn 翻页\nHome/End 首尾  y 复制全文\nEsc 关闭"
        };
        let mut footer = self
            .feedback
            .as_ref()
            .map(|(level, text)| (text.clone(), *level))
            .into_iter()
            .collect::<Vec<_>>();
        footer.extend(
            wrap_text(keys, inner.width)
                .into_iter()
                .map(|s| (s, Level::Info)),
        );
        let footer_height = footer.len().min(inner.height.saturating_sub(1) as usize) as u16;
        let body = Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(1),
            inner.height - footer_height,
        );
        // 换行结果按宽度缓存；只在尺寸改变时重新处理长诊断文本。
        if self.wrap_width != Some(body.width) {
            self.rows = wrap_text(&self.message, body.width);
            self.wrap_width = Some(body.width);
        }
        self.viewport = body.height as usize;
        self.scroll = self.scroll.min(self.max_scroll());
        let lines = self
            .rows
            .iter()
            .skip(self.scroll)
            .take(self.viewport)
            .map(|s| Line::from(s.as_str()))
            .collect::<Vec<_>>();
        f.render_widget(
            Paragraph::new(lines).style(Style::default().fg(Color::White)),
            body,
        );
        if self.rows.len() > self.viewport && inner.width > 1 {
            let mut state = ScrollbarState::new(self.rows.len())
                .position(self.scroll)
                .viewport_content_length(self.viewport);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                Rect::new(inner.x, inner.y, inner.width, body.height),
                &mut state,
            );
        }
        let footer_area = Rect::new(inner.x, body.bottom(), inner.width, footer_height);
        f.render_widget(
            Paragraph::new(
                footer
                    .into_iter()
                    .take(footer_height as usize)
                    .map(|(text, level)| Line::from(Span::styled(text, level.style())))
                    .collect::<Vec<_>>(),
            ),
            footer_area,
        );
    }
}

/// 按终端列宽硬换行，保留多行、空格和长路径；不使用 Paragraph 的 u16 滚动偏移。
fn wrap_text(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut row = String::new();
        let mut used = 0;
        for ch in line.trim_end_matches('\r').replace('\t', "    ").chars() {
            let cells = Line::from(ch.to_string()).width();
            if used + cells > width as usize && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            row.push(ch);
            used += cells;
        }
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn notice_wrap_preserves_long_paths_and_error_tail() {
        let message = format!(
            "提交失败：E155011\n\n/{}\nEND_OF_ERROR",
            "中文 路径/".repeat(30)
        );
        for width in [4, 12, 28, 78, 138] {
            let rows = wrap_text(&message, width);
            assert!(rows
                .iter()
                .all(|s| Line::from(s.as_str()).width() <= width as usize));
            assert_eq!(rows.concat(), message.replace('\n', ""));
            assert!(rows.iter().any(String::is_empty));
        }
    }

    #[test]
    fn notice_colors_and_summary_keep_details_entry() {
        for (message, level, color) in [
            ("提交失败：/a/path/失败.sql", Level::Error, Color::LightRed),
            ("已提交 3 项", Level::Success, Color::Green),
            ("⚠️ 实际提交了 0 项", Level::Warning, Color::Yellow),
            ("排序：状态", Level::Info, Color::Cyan),
        ] {
            assert_eq!(Level::of(message), level);
            for width in [20, 40, 100] {
                let line = NoticePanel::summary(message, width);
                assert!(line.width() <= width as usize);
                assert!(line.to_string().contains("F4 详情"));
                assert_eq!(line.spans[0].style.fg, Some(color));
            }
        }
        assert_eq!(Level::of("已复制：/失败.sql"), Level::Success);
    }

    #[test]
    fn notice_scroll_and_resize_reach_complete_error_tail() {
        let message = format!(
            "提交失败：E155011\n{}\nEND_OF_ERROR",
            "very/long/中文 path/".repeat(200)
        );
        let mut panel = NoticePanel::new(message.clone());
        for (width, height) in [(100, 24), (30, 10), (80, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| panel.render(f, f.area())).unwrap();
            panel.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
            let frame = terminal.draw(|f| panel.render(f, f.area())).unwrap();
            let text = frame
                .buffer
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains("END_OF_ERROR"));
            assert_eq!(panel.message(), message);
            assert!(frame
                .buffer
                .content
                .iter()
                .any(|cell| cell.fg == Color::LightRed));
            panel.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
            assert_eq!(panel.scroll, 0);
        }
    }
}
