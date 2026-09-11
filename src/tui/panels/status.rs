//! 主状态列表。
//!
//! TUI 的默认视图：一个可滚动的变更清单，右侧是选中项的 diff 预览。

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::Item;
use crate::tui::theme;

#[derive(Default)]
pub struct StatusPanel {
    state: ListState,
}

impl StatusPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn select(&mut self, idx: Option<usize>) {
        self.state.select(idx);
    }

    pub fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub fn move_up(&mut self) {
        let i = match self.state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn move_down(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => (i + 1).min(len - 1),
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn render(&mut self, f: &mut Frame, area: ratatui::layout::Rect, items: &[Item]) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 顶部信息
                Constraint::Min(3),    // 列表
                Constraint::Length(1), // 底部帮助
            ])
            .split(area);

        // ── 顶部：分支 / 版本 / 统计
        let mut counts: Vec<(char, usize)> = Vec::new();
        for sign in ['C', 'T', '!', 'D', 'R', 'A', 'M', '?'] {
            let n = items.iter().filter(|i| i.sign == sign).count();
            if n > 0 {
                counts.push((sign, n));
            }
        }

        let mut spans = vec![Span::styled(" SVN ", theme::Theme::status_bar())];
        if counts.is_empty() {
            spans.push(Span::styled("✓ 工作副本干净", theme::Theme::clean()));
        } else {
            for (sign, n) in counts {
                spans.push(Span::styled(
                    format!(" {}{} ", sign, n),
                    theme::for_sign(sign),
                ));
            }
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
            chunks[0],
        );

        // ── 列表
        let list_items: Vec<ListItem> = items
            .iter()
            .map(|item| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", item.sign), theme::for_sign(item.sign)),
                    Span::styled(item.display(), theme::for_sign(item.sign)),
                    Span::styled(
                        format!("  {}", item.xy.trim_end()),
                        theme::Theme::dim(),
                    ),
                ]))
            })
            .collect();

        let list = List::new(list_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 变更 ")
                    .border_style(theme::Theme::border_active()),
            )
            .highlight_style(theme::Theme::selected());

        f.render_stateful_widget(list, chunks[1], &mut self.state);

        // ── 底部帮助
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " j/k 移动 | Enter 看 diff | c 提交 | a add | r revert | u update | h 体检 | R 刷新 | q 退出",
                theme::Theme::dim(),
            ))),
            chunks[2],
        );
    }
}
