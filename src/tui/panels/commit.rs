//! 提交面板。
//!
//! 这是整个 TUI 存在的理由。
//!
//! yazi 插件做不到的事（没有 Popup、`ya.confirm` 只有两个按钮、拿不到按键事件），
//! 这里全都是 closed loop：
//!
//! ```text
//! ┌──────────── SVN Commit ────────────┐
//! │ 待提交文件（Space 切换，a 全选，n 清空）│
//! │ [x] M  a.txt                        │
//! │ [x] A  c.txt                        │
//! │ [ ] M  docs/old.txt                 │
//! │                                     │
//! │ 提交信息（Ctrl+Enter 提交，Tab 切焦点）│
//! │ ┌─────────────────────────────────┐ │
//! │ │ 修复状态同步                      │ │
//! │ └─────────────────────────────────┘ │
//! └─────────────────────────────────────┘
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use tui_textarea::TextArea;

use super::Item;
use crate::tui::theme;

/// 焦点在哪个区域。Tab 切换。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Files,
    Message,
}

pub struct CommitPanel {
    items: Vec<Item>,
    /// 每项是否勾选。长度和 `items` 一致。
    checked: Vec<bool>,
    /// 列表光标。
    state: ListState,
    focus: Focus,
    /// 多行提交信息编辑器。
    textarea: TextArea<'static>,
    /// 底部提示（错误信息等）。
    hint: Option<String>,
}

impl CommitPanel {
    /// 新建。默认全选**可提交**的条目。
    ///
    /// 未版本化的 `?` 默认不勾 —— 进候选项列表让用户看见，
    /// 但不勾，避免误提交不该进库的东西。
    pub fn new(items: Vec<Item>) -> Self {
        let checked: Vec<bool> = items.iter().map(|i| i.committable()).collect();
        let mut state = ListState::default();
        if !items.is_empty() {
            state.select(Some(0));
        }

        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 提交信息 "),
        );
        textarea.set_placeholder_text("必填。Ctrl+Enter 提交，Tab 切回文件列表");

        Self {
            items,
            checked,
            state,
            focus: Focus::Message,
            textarea,
            hint: None,
        }
    }

    pub fn set_hint(&mut self, hint: impl Into<String>) {
        self.hint = Some(hint.into());
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.items.len() as isize - 1);
        self.state.select(Some(next as usize));
    }

    /// 切换当前项的勾选。
    fn toggle(&mut self) {
        if let Some(i) = self.state.selected() {
            if let Some(c) = self.checked.get_mut(i) {
                *c = !*c;
            }
        }
    }

    fn select_all(&mut self, on: bool) {
        for (i, c) in self.checked.iter_mut().enumerate() {
            // 全选只勾可提交的；清空则无条件清
            *c = on && self.items[i].committable();
        }
    }

    pub fn switch_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Files => Focus::Message,
            Focus::Message => Focus::Files,
        };
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// 当前勾选的绝对路径。
    pub fn checked_paths(&self) -> Vec<std::path::PathBuf> {
        self.items
            .iter()
            .zip(self.checked.iter())
            .filter(|(_, c)| **c)
            .map(|(i, _)| i.abs.clone())
            .collect()
    }

    /// 提交信息（去首尾空白后为空则视为未填）。
    pub fn message(&self) -> String {
        self.textarea.lines().join("\n").trim().to_string()
    }

    // ── 输入 ──────────────────────────────────────────────

    pub fn move_up(&mut self) {
        self.move_cursor(-1);
    }
    pub fn move_down(&mut self) {
        self.move_cursor(1);
    }
    pub fn toggle_current(&mut self) {
        self.toggle();
    }
    pub fn all(&mut self) {
        self.select_all(true);
    }
    pub fn none(&mut self) {
        self.select_all(false);
    }

    pub fn textarea_mut(&mut self) -> &mut TextArea<'static> {
        &mut self.textarea
    }

    // ── 渲染 ──────────────────────────────────────────────

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        // 弹框：先 Clear 擦掉底下，再画自己的边框
        f.render_widget(Clear, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),      // 文件列表
                Constraint::Length(7),   // 提交信息
                Constraint::Length(1),   // 提示行
            ])
            .split(area);

        let n_checked = self.checked.iter().filter(|c| **c).count();

        let list_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " 待提交文件（已选 {}/{}）",
                n_checked,
                self.items.len()
            ))
            .border_style(if self.focus == Focus::Files {
                theme::Theme::border_active()
            } else {
                theme::Theme::border()
            });

        let items: Vec<ListItem> = self
            .items
            .iter()
            .zip(self.checked.iter())
            .map(|(item, checked)| {
                let box_style = if *checked {
                    theme::Theme::checked()
                } else {
                    theme::Theme::unchecked()
                };
                let mark = if *checked { "[x]" } else { "[ ]" };

                // 不可提交的（? / I / X）整体压暗，提示"这个不会进提交"
                let dim = if item.committable() {
                    Style::default()
                } else {
                    theme::Theme::dim()
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", mark), box_style),
                    Span::styled(format!("{} ", item.sign), theme::for_sign(item.sign).patch(dim)),
                    Span::styled(item.display(), theme::for_sign(item.sign).patch(dim)),
                    Span::styled(
                        if item.committable() { "" } else { "  (未版本化，先 svn add)" },
                        theme::Theme::dim(),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(list_block)
            .highlight_style(theme::Theme::selected());

        f.render_stateful_widget(list, chunks[0], &mut self.state);

        // 提交信息：TextArea 自带 block
        f.render_widget(&self.textarea, chunks[1]);

        let hint = match &self.hint {
            Some(h) => Line::from(Span::styled(h.as_str(), theme::Theme::conflicted())),
            None => Line::from(Span::styled(
                " Space 切换 | a 全选 | n 清空 | Tab 切焦点 | Ctrl+Enter 提交 | Esc 取消",
                theme::Theme::dim(),
            )),
        };
        f.render_widget(Paragraph::new(hint), chunks[2]);
    }
}
