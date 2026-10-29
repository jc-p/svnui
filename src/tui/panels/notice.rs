//! 通知摘要与完整详情。原文独立保留，换行、滚动和复制均不截断诊断信息。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Borders, Clear, Paragraph, ScrollbarOrientation, ScrollbarState},
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
        // 与确认面板的危险等级共用同一套色阶（见 theme 的严重度色阶）：
        // 否则同一个红在确认框里读作"危险"，在提示条里读作"普通失败"。
        let base = match self {
            Self::Info => theme::Theme::info(),
            Self::Success => theme::Theme::success(),
            Self::Warning => theme::Theme::caution(),
            Self::Error => theme::Theme::danger(),
        };
        base.add_modifier(Modifier::BOLD)
    }
}

/// 底栏上那条通知。
///
/// 之前只有 `Option<String>`：等级靠 `Level::of()` 从文案里猜
/// （路径含"失败"就会误判成错误），而且没有生命周期 ——
/// "已勾选 3 项"会一直挂在那儿，直到被下一条覆盖。
///
/// 现在等级在**产生通知的地方**显式给出，并带上时间戳供 TTL 清理。
#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub level: Level,
    at: std::time::Instant,
}

impl Notice {
    /// 等级从文案推断。仅用于历史调用点，新代码请用下面几个显式构造器。
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            level: Level::of(&text),
            text,
            at: std::time::Instant::now(),
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self::with_level(text, Level::Info)
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self::with_level(text, Level::Success)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self::with_level(text, Level::Warning)
    }

    /// 错误**不过期**：它描述的是已经发生的事，用户可能还没看见。
    pub fn error(text: impl Into<String>) -> Self {
        Self::with_level(text, Level::Error)
    }

    fn with_level(text: impl Into<String>, level: Level) -> Self {
        Self {
            text: text.into(),
            level,
            at: std::time::Instant::now(),
        }
    }

    /// 提示类通知活 8 秒，警告 15 秒，错误常驻。
    ///
    /// "已勾选 3 项"这种状态回执，八秒足够；超过之后还挂着就成了噪音，
    /// 反而把真正的错误挤出视野。
    pub fn expired(&self) -> bool {
        self.expired_at(std::time::Instant::now())
    }

    /// 可注入 `now` 的判定，便于测试（总不能真等 8 秒）。
    pub fn expired_at(&self, now: std::time::Instant) -> bool {
        let ttl = match self.level {
            Level::Info | Level::Success => 8,
            Level::Warning => 15,
            Level::Error => return false,
        };
        now.saturating_duration_since(self.at) >= std::time::Duration::from_secs(ttl)
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
    /// 等级由文案推断。仅保留给历史调用点 —— 新代码请用 `with_level`
    /// 在产生通知的地方把等级说清楚，别让文案去猜。
    pub fn new(message: String) -> Self {
        let level = Level::of(&message);
        Self::with_level(message, level)
    }

    pub fn with_level(message: String, level: Level) -> Self {
        Self {
            level,
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
    pub fn summary(message: &str, level: Level, width: u16) -> Line<'static> {
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
            Paragraph::new(lines).style(theme::Theme::body()),
            body,
        );
        if self.rows.len() > self.viewport && inner.width > 1 {
            let mut state = ScrollbarState::new(self.rows.len())
                .position(self.scroll)
                .viewport_content_length(self.viewport);
            f.render_stateful_widget(
                theme::scrollbar(ScrollbarOrientation::VerticalRight),
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
    use ratatui::style::Color;
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
                let line = NoticePanel::summary(message, level, width);
                assert!(line.width() <= width as usize);
                assert!(line.to_string().contains("F4 详情"));
                assert_eq!(line.spans[0].style.fg, Some(color));
            }
        }
        assert_eq!(Level::of("已复制：/失败.sql"), Level::Success);
    }

    #[test]
    fn transient_notices_expire_but_errors_persist() {
        use std::time::{Duration, Instant};
        let base = Instant::now();
        let later = |secs: u64| base + Duration::from_secs(secs);

        // 提示与成功：8 秒内还在，之后清掉。
        for notice in [Notice::info("已勾选 3 项"), Notice::success("已提交")] {
            assert!(!notice.expired_at(later(7)));
            assert!(notice.expired_at(later(9)));
        }
        // 警告宽限到 15 秒。
        let warning = Notice::warning("有 2 项未纳管");
        assert!(!warning.expired_at(later(14)));
        assert!(warning.expired_at(later(16)));
        // 错误永不过期：它描述已经发生的事，用户可能还没看见。
        let error = Notice::error("提交失败：连接被拒绝");
        assert!(!error.expired_at(later(600)));
    }

    #[test]
    fn explicit_levels_beat_guessing_from_text() {
        // 路径里含"失败"二字：文本推断会误判成错误，显式等级不会。
        let path = Notice::info("已选中 docs/失败案例.txt");
        assert_eq!(path.level, Level::Info);
        assert_eq!(Notice::error("提交失败").level, Level::Error);
        assert_eq!(Notice::warning("没有可提交的项").level, Level::Warning);
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
