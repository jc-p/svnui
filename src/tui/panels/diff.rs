//! diff 面板。
//!
//! 长 diff 可滚动 —— 这是 yazi 插件里最别扭的地方
//! （`ya.confirm` 不保证可滚动，超长 diff 只能丢进 pager 让出终端）。

use ratatui::{
    layout::{Alignment, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::theme;

pub struct DiffPanel {
    /// 标题（通常是文件名）。
    title: String,
    /// 已着色的行。
    lines: Vec<Line<'static>>,
    /// 垂直滚动偏移。
    scroll: u16,
}

impl DiffPanel {
    /// 从 svn 原始 diff 输出构造。
    ///
    /// 按行首字符着色：`+` 绿、`-` 红、`@` 青（hunk 头）、其余默认。
    pub fn new(title: impl Into<String>, raw: &str) -> Self {
        let lines: Vec<Line<'static>> = raw
            .lines()
            .map(|l| {
                let style = if l.starts_with("+++") || l.starts_with("---") {
                    theme::Theme::dim()
                } else if l.starts_with('+') {
                    theme::Theme::added()
                } else if l.starts_with('-') {
                    theme::Theme::deleted()
                } else if l.starts_with("@@") {
                    theme::Theme::props()
                } else if l.starts_with("===") || l.starts_with("Index:") {
                    theme::Theme::props()
                } else {
                    theme::Theme::dim()
                };
                Line::from(Span::styled(l.to_string(), style))
            })
            .collect();

        Self {
            title: title.into(),
            lines,
            scroll: 0,
        }
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n);
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_top(&mut self) {
        self.scroll = 0;
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        use ratatui::widgets::Scrollbar;

        f.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", self.title))
            .title_bottom(if self.lines.is_empty() {
                Line::from(" （无差异）").alignment(Alignment::Center)
            } else {
                Line::from(format!(" {} 行 ", self.lines.len())).alignment(Alignment::Right)
            })
            .border_style(theme::Theme::border_active());

        // 内部高度：减掉上下边框
        let inner_h = area.height.saturating_sub(2) as usize;
        let max_scroll = self.lines.len().saturating_sub(inner_h) as u16;
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let text: Text<'static> = Text::from(self.lines.clone());
        let para = Paragraph::new(text)
            .block(block)
            .scroll((self.scroll, 0))
            .wrap(Wrap { trim: false });

        f.render_widget(para, area);

        // 右侧滚动条只在内容超出时出现
        if max_scroll > 0 {
            let mut sb_state = ratatui::widgets::ScrollbarState::new(self.lines.len())
                .position(self.scroll as usize);
            f.render_stateful_widget(
                Scrollbar::new(ratatui::widgets::ScrollbarOrientation::VerticalRight),
                area,
                &mut sb_state,
            );
        }
    }
}
