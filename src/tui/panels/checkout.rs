//! 检出新工作副本。
//!
//! 在没有工作副本的目录里启动 `svnui tui` 时会进到这里 ——
//! 否则用户得先退出、在命令行敲 checkout、再进来。

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::theme;

/// 字段顺序。
const FIELDS: &[(&str, bool)] = &[
    ("仓库 URL", false),
    ("本地路径", false),
    ("用户名", false),
    ("密码", true), // 掩码显示
];

pub struct CheckoutPanel {
    values: [String; 4],
    focus: usize,
    hint: Option<String>,
}

impl CheckoutPanel {
    pub fn new(default_path: String) -> Self {
        Self {
            values: [String::new(), default_path, String::new(), String::new()],
            focus: 0,
            hint: None,
        }
    }

    pub fn set_hint(&mut self, h: impl Into<String>) {
        self.hint = Some(h.into());
    }

    pub fn url(&self) -> &str {
        &self.values[0]
    }

    pub fn path(&self) -> &str {
        &self.values[1]
    }

    pub fn username(&self) -> Option<&str> {
        let s = self.values[2].trim();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub fn password(&self) -> Option<&str> {
        let s = &self.values[3];
        if s.is_empty() {
            None
        } else {
            Some(s.as_str())
        }
    }

    /// 切到下一个字段（密码后可回车）。
    pub fn next_field(&mut self) {
        self.focus = (self.focus + 1) % FIELDS.len();
    }

    pub fn prev_field(&mut self) {
        self.focus = (self.focus + FIELDS.len() - 1) % FIELDS.len();
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn push_char(&mut self, c: char) {
        self.values[self.focus].push(c);
    }

    /// 整段插入（粘贴）。
    ///
    /// 单行输入框吃不下换行：URL 里带 `\n` 会把框撑成两行、边框错位，
    /// 所以统一压成空格。制表符同理（Tab 是切字段的键，不能出现在内容里）。
    pub fn push_str(&mut self, s: &str) {
        let flat: String = s
            .chars()
            .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
            .collect();
        self.values[self.focus].push_str(&flat);
    }

    pub fn pop_char(&mut self) {
        self.values[self.focus].pop();
    }

    /// 清空当前字段（Ctrl+U）。粘错一长串 URL 时逐字符退格太慢。
    pub fn clear_field(&mut self) {
        self.values[self.focus].clear();
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 标题
                Constraint::Length(2), // URL
                Constraint::Length(2), // path
                Constraint::Length(2), // user
                Constraint::Length(2), // pass
                Constraint::Length(2), // 提示
            ])
            .split(area);

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " 检出一份工作副本 ",
                theme::Theme::title(),
            ))),
            rows[0],
        );

        for (i, (label, masked)) in FIELDS.iter().enumerate() {
            let active = i == self.focus;
            let shown = if *masked {
                "*".repeat(self.values[i].chars().count())
            } else {
                self.values[i].clone()
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", label))
                .border_style(if active {
                    theme::Theme::border_active()
                } else {
                    theme::Theme::border()
                });

            // ⚠️ 先算好再 format!：把 `shown` 直接塞进三元表达式会被 move，
            //    下面还要用它判断颜色，就借不到了。
            let placeholder = shown.is_empty() && !active;
            let content = if placeholder {
                format!("（{}{}）", label, if *masked { "，可留空" } else { "" })
            } else {
                shown
            };
            // 焦点框里补一个可见光标。
            // 原来只有边框变色，框里空空如也 —— 看不出"能在这儿打字"，
            // 尤其刚进来时四个框全空，很容易以为面板是死的。
            let content = if active && !placeholder {
                format!("{}▌", content)
            } else {
                content
            };

            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" {}", content),
                    if placeholder {
                        theme::Theme::dim()
                    } else {
                        theme::Theme::status_bar()
                    },
                )))
                .block(block),
                rows[i + 1],
            );
        }

        let hint = match &self.hint {
            Some(h) => Line::from(Span::styled(h.as_str(), theme::Theme::conflicted())),
            None => Line::from(Span::styled(
                " Tab 切换   Enter 开始检出（在最后一项按 Enter 直接开始）   Ctrl+U 清空   Esc 退出   可直接粘贴",
                theme::Theme::dim(),
            )),
        };
        f.render_widget(Paragraph::new(hint).alignment(Alignment::Left), rows[5]);
    }
}
