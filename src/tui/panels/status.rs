//! 变更列表（左栏）。
//!
//! 只负责列表本身 —— 顶部信息栏和底部帮助由 `App` 画，
//! 这样布局调整集中在一个地方，不用每个面板各算一次。

use ratatui::{
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Scrollbar, ScrollbarOrientation, ScrollbarState},
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

    /// 移动光标，返回是否真的动了（没动就不用重新加载 diff）。
    pub fn move_up(&mut self) -> bool {
        let next = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            Some(_) => return false,
            None => 0,
        };
        self.state.select(Some(next));
        true
    }

    pub fn move_down(&mut self, len: usize) -> bool {
        if len == 0 {
            return false;
        }
        let next = match self.state.selected() {
            Some(i) if i + 1 < len => i + 1,
            Some(_) => return false,
            None => 0,
        };
        self.state.select(Some(next));
        true
    }

    /// 渲染到给定区域（整个区域都属于列表）。
    pub fn render(&mut self, f: &mut Frame, area: ratatui::layout::Rect, items: &[Item]) {
        let list_items: Vec<ListItem> = items
            .iter()
            .map(|item| {
                let style = theme::for_sign(item.sign);
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", item.sign), style),
                    Span::styled(item.display(), style),
                    Span::styled(format!(" {}", item.xy.trim_end()), theme::Theme::dim()),
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

        f.render_stateful_widget(list, area, &mut self.state);

        // 滚动条：变更条目超过一屏时，没有它看不出后面还有多少
        let visible_h = area.height.saturating_sub(2) as usize;
        if items.len() > visible_h {
            let mut sb =
                ScrollbarState::new(items.len()).position(self.state.selected().unwrap_or(0));
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area,
                &mut sb,
            );
        }
    }
}
