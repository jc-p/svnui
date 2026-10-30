//! 通知摘要与完整详情。原文独立保留，换行、滚动和复制均不截断诊断信息。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Borders, Paragraph, ScrollbarOrientation, ScrollbarState},
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

/// 右上角通知；普通回执不抢焦点，完整原文独立保留。
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
    content_width: u16,
}

impl NoticePanel {
    /// 等级由文案推断。仅保留给历史调用点 —— 新代码请用 `with_level`
    /// 在产生通知的地方把等级说清楚，别让文案去猜。
    pub fn new(message: String) -> Self {
        let level = Level::of(&message);
        Self::with_level(message, level)
    }

    pub fn with_level(message: String, level: Level) -> Self {
        // 消息只在创建时测量；长诊断不在每帧重复扫描。
        let content_width = message
            .lines()
            .map(|s| Line::from(s).width())
            .max()
            .unwrap_or(0)
            .min(92) as u16;
        Self {
            level,
            message,
            rows: Vec::new(),
            wrap_width: None,
            scroll: 0,
            viewport: 0,
            feedback: None,
            content_width,
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

    fn prepare_rows(&mut self, width: u16) {
        if self.wrap_width != Some(width) {
            self.rows = wrap_text(&self.message, width);
            self.wrap_width = Some(width);
        }
    }

    /// 非模态通知最多三行正文；省略只影响显示，F4 仍可读取、复制全文。
    pub fn render_toast(&mut self, f: &mut Frame, area: Rect, details_key: &str) {
        let width = (self.content_width + 4).clamp(32, 60);
        let bounds = corner_area(area, width, 5);
        if bounds.width < 6 || bounds.height < 3 {
            return;
        }
        self.prepare_rows(bounds.width.saturating_sub(4));
        let height = (self.rows.len().min(3) as u16 + 2).min(bounds.height);
        let pop = Rect::new(bounds.x, bounds.y, bounds.width, height);
        theme::clear(f, pop);
        let block = theme::block()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {} ", self.level.label()),
                self.level.style(),
            ))
            .title_bottom(
                Line::from(Span::styled(
                    format!(" {details_key} 详情 "),
                    theme::Theme::dim(),
                ))
                .alignment(Alignment::Right),
            )
            .border_style(self.level.style().remove_modifier(Modifier::BOLD));
        let body = padded_body(block.inner(pop));
        f.render_widget(block, pop);
        let mut lines = self
            .rows
            .iter()
            .take(body.height as usize)
            .cloned()
            .collect::<Vec<_>>();
        if self.rows.len() > lines.len() {
            if let Some(last) = lines.last_mut() {
                *last = theme::fit(&format!("{last}…"), body.width as usize);
            }
        }
        f.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .style(theme::Theme::body()),
            body,
        );
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        // 短消息详情也按内容收紧；长错误最多占窗口约三分之二，留出仓库上下文。
        let max_width = ((u32::from(area.width) * 2 / 3) as u16).clamp(40, 96);
        let width = (self.content_width + 4).clamp(40, max_width);
        let max_height = ((u32::from(area.height) * 2 / 3) as u16).max(5);
        let bounds = corner_area(area, width, max_height);
        if bounds.width < 6 || bounds.height < 3 {
            return;
        }
        self.prepare_rows(bounds.width.saturating_sub(4));
        let feedback_height = u16::from(self.feedback.is_some() && bounds.height >= 4);
        let height = self
            .rows
            .len()
            .saturating_add(2 + feedback_height as usize)
            .min(bounds.height as usize) as u16;
        let pop = Rect::new(bounds.x, bounds.y, bounds.width, height);
        let mut body = padded_body(Rect::new(
            pop.x + 1,
            pop.y + 1,
            pop.width.saturating_sub(2),
            pop.height.saturating_sub(2),
        ));
        body.height = body.height.saturating_sub(feedback_height);
        self.viewport = body.height as usize;
        self.scroll = self.scroll.min(self.max_scroll());
        let scrolling = self.rows.len() > self.viewport;
        let keys = if scrolling && pop.width >= 36 {
            " ↑↓ 滚动 · y 复制 · Esc 关闭 "
        } else {
            " y 复制 · Esc 关闭 "
        };
        let mut block = theme::block()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {} · 详情 ", self.level.label()),
                self.level.style(),
            ))
            .title_bottom(
                Line::from(Span::styled(keys, theme::Theme::dim())).alignment(Alignment::Right),
            )
            .border_style(self.level.style().remove_modifier(Modifier::BOLD));
        if scrolling && pop.width >= 48 {
            block = block.title_top(
                Line::from(Span::styled(
                    format!(" ↑↓ PgUp/PgDn · {}/{} ", self.scroll + 1, self.rows.len()),
                    theme::Theme::dim(),
                ))
                .alignment(Alignment::Right),
            );
        }
        theme::clear(f, pop);
        f.render_widget(block, pop);
        let lines = self
            .rows
            .iter()
            .skip(self.scroll)
            .take(self.viewport)
            .map(|s| Line::from(s.as_str()))
            .collect::<Vec<_>>();
        f.render_widget(Paragraph::new(lines).style(theme::Theme::body()), body);
        if scrolling {
            let mut state = ScrollbarState::new(self.rows.len())
                .position(self.scroll)
                .viewport_content_length(self.viewport);
            f.render_stateful_widget(
                theme::scrollbar(ScrollbarOrientation::VerticalRight),
                Rect::new(body.x, body.y, body.width + 1, body.height),
                &mut state,
            );
        }
        if let Some((level, text)) = &self.feedback {
            let footer = Rect::new(body.x, body.bottom(), body.width, feedback_height);
            f.render_widget(
                Paragraph::new(theme::fit(text, body.width as usize)).style(level.style()),
                footer,
            );
        }
    }
}

/// 避开顶栏与主体边框；极窄、极矮窗口按实际空间缩小，支持非零原点。
fn corner_area(area: Rect, width: u16, height: u16) -> Rect {
    let margin = u16::from(area.width >= 12);
    let top = if area.height >= 8 { 2 } else { 0 };
    let width = width.min(area.width.saturating_sub(margin * 2));
    Rect::new(
        area.right().saturating_sub(margin + width),
        area.y + top,
        width,
        height.min(
            area.height
                .saturating_sub(top + u16::from(area.height >= 8)),
        ),
    )
}

fn padded_body(inner: Rect) -> Rect {
    Rect::new(
        inner.x + u16::from(inner.width > 0),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    )
}

/// 按终端列宽硬换行，保留多行、空格和长路径；不使用 Paragraph 的 u16 滚动偏移。
pub(super) fn wrap_text(text: &str, width: u16) -> Vec<String> {
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
    fn notice_colors_and_toasts_keep_details_entry() {
        for (message, level, color) in [
            (
                "提交失败：/a/path/失败.sql",
                Level::Error,
                theme::Theme::danger().fg,
            ),
            ("已提交 3 项", Level::Success, theme::Theme::success().fg),
            (
                "⚠️ 实际提交了 0 项",
                Level::Warning,
                theme::Theme::caution().fg,
            ),
            ("排序：状态", Level::Info, theme::Theme::info().fg),
        ] {
            assert_eq!(Level::of(message), level);
            for width in [20, 40, 100] {
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                let mut panel = NoticePanel::with_level(message.into(), level);
                let frame = terminal
                    .draw(|f| panel.render_toast(f, f.area(), "F4"))
                    .unwrap();
                let text = frame
                    .buffer
                    .content
                    .iter()
                    .flat_map(|c| c.symbol().chars())
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>();
                assert!(text.contains("F4详情"));
                assert!(text.contains(level.label()));
                assert!(frame.buffer.content.iter().any(|c| Some(c.fg) == color));
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
                .any(|cell| Some(cell.fg) == theme::Theme::danger().fg));
            panel.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
            assert_eq!(panel.scroll, 0);
        }
    }

    #[test]
    fn short_notices_and_details_stay_compact_in_the_upper_right() {
        for (width, height) in [(30, 10), (80, 24), (180, 60)] {
            for details in [false, true] {
                let mut panel =
                    NoticePanel::with_level("已切换到 A8_V26.1.12.4_ZZJR".into(), Level::Success);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let area = Rect::new(2, 1, width - 4, height - 2);
                let frame = terminal
                    .draw(|f| {
                        f.render_widget(
                            Paragraph::new(
                                vec![".".repeat(width as usize); height as usize].join("\n"),
                            ),
                            f.area(),
                        );
                        if details {
                            panel.render(f, area);
                        } else {
                            panel.render_toast(f, area, "F4");
                        }
                    })
                    .unwrap();
                let changed = frame
                    .buffer
                    .content
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.symbol() != ".")
                    .map(|(i, _)| ((i % width as usize) as u16, (i / width as usize) as u16))
                    .collect::<Vec<_>>();
                let left = changed.iter().map(|(x, _)| *x).min().unwrap();
                let right = changed.iter().map(|(x, _)| *x).max().unwrap();
                let top = changed.iter().map(|(_, y)| *y).min().unwrap();
                let bottom = changed.iter().map(|(_, y)| *y).max().unwrap();
                assert_eq!(right, area.right() - 2);
                assert_eq!(top, area.y + 2);
                assert!(bottom - top < 5, "短消息不能占满高度");
                assert!(left >= area.x);
                assert!(right - left < 60, "宽窗口中的短消息仍须紧凑");
                let text = frame
                    .buffer
                    .content
                    .iter()
                    .flat_map(|c| c.symbol().chars())
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>();
                assert!(text.contains("成功"));
                assert!(text.contains(if details { "Esc关闭" } else { "F4详情" }));
                assert_eq!(panel.message(), "已切换到 A8_V26.1.12.4_ZZJR");
            }
        }
    }

    #[test]
    fn long_toast_is_capped_and_details_keep_copy_and_scroll() {
        let message = format!("已完成：{}\nTAIL", "中文路径/".repeat(100));
        let mut panel = NoticePanel::with_level(message.clone(), Level::Info);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let frame = terminal
            .draw(|f| panel.render_toast(f, f.area(), "F4"))
            .unwrap();
        let text = frame
            .buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains('…'));
        assert!(!text.contains("TAIL"));
        assert!(matches!(
            panel.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            Action::Copy
        ));
        assert_eq!(panel.message(), message);
        terminal.draw(|f| panel.render(f, f.area())).unwrap();
        panel.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let frame = terminal.draw(|f| panel.render(f, f.area())).unwrap();
        assert!(frame
            .buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains("TAIL"));
        panel.feedback(Level::Success, "已复制完整提示".into());
        let frame = terminal.draw(|f| panel.render(f, f.area())).unwrap();
        let text = frame
            .buffer
            .content
            .iter()
            .flat_map(|c| c.symbol().chars())
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(text.contains("已复制完整提示"));
        assert_eq!(panel.message(), message);
    }

    #[test]
    fn notice_render_survives_tiny_windows() {
        for width in [0, 1, 5, 12, 20] {
            for height in [0, 1, 2, 3, 7] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut panel = NoticePanel::new("错误：中文路径\nTAIL".into());
                terminal
                    .draw(|f| panel.render_toast(f, f.area(), "F4"))
                    .unwrap();
                terminal.draw(|f| panel.render(f, f.area())).unwrap();
            }
        }
    }
}
