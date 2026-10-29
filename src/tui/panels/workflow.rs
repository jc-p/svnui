//! 分支操作表单。准备和执行由 App 后台任务负责，面板只保存输入。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::svn::workflow::{WorkflowOperation, WorkflowRequest};
use crate::tui::theme::{self, Theme};

#[derive(Debug, PartialEq, Eq)]
pub enum InputAction {
    None,
    Close,
    Submit,
}

pub struct WorkflowPanel {
    operation: WorkflowOperation,
    values: [String; 3],
    focus: usize,
}

impl Default for WorkflowPanel {
    fn default() -> Self {
        Self {
            operation: WorkflowOperation::Branch,
            values: [String::new(), "BASE".into(), String::new()],
            focus: 0,
        }
    }
}

impl WorkflowPanel {
    fn field_count(&self) -> usize {
        if self.operation.is_copy() {
            3
        } else {
            2
        }
    }

    pub fn request(&self) -> WorkflowRequest {
        WorkflowRequest {
            operation: self.operation,
            url: self.values[0].trim().into(),
            revision: (!self.values[1].trim().is_empty()).then(|| self.values[1].trim().into()),
            message: self.values[2].clone(),
        }
    }

    pub fn insert(&mut self, text: &str) {
        // URL 和提交信息均为单行；粘贴的控制字符不能成为快捷键或破坏布局。
        self.values[self.focus].extend(text.chars().filter(|ch| !ch.is_control()));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputAction {
        match key.code {
            KeyCode::Esc => return InputAction::Close,
            KeyCode::F(number @ 1..=4) => {
                self.operation = match number {
                    1 => WorkflowOperation::Branch,
                    2 => WorkflowOperation::Tag,
                    3 => WorkflowOperation::Switch,
                    _ => WorkflowOperation::MergePreview,
                };
                self.values[1] = match self.operation {
                    WorkflowOperation::Branch | WorkflowOperation::Tag => "BASE".into(),
                    WorkflowOperation::Switch => "HEAD".into(),
                    WorkflowOperation::MergePreview => String::new(),
                };
                self.focus = 0;
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % self.field_count(),
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = (self.focus + self.field_count() - 1) % self.field_count();
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputAction::Submit;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.values[self.focus].clear()
            }
            KeyCode::Backspace => {
                self.values[self.focus].pop();
            }
            KeyCode::Enter if self.focus + 1 == self.field_count() => return InputAction::Submit,
            KeyCode::Enter => self.focus += 1,
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&ch.to_string());
            }
            _ => {}
        }
        InputAction::None
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);
        let block = theme::block()
            .borders(Borders::ALL)
            .title(format!(" 分支工作流 · {} ", self.operation.title()))
            .border_style(Theme::border_active());
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.width < 24 || inner.height < 14 {
            f.render_widget(
                Paragraph::new("请放大终端以编辑分支表单；Esc 返回").wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        let tabs = [
            (WorkflowOperation::Branch, "F1 分支  "),
            (WorkflowOperation::Tag, "F2 标签  "),
            (WorkflowOperation::Switch, "F3 切换  "),
            (WorkflowOperation::MergePreview, "F4 合并预演"),
        ];
        f.render_widget(
            Paragraph::new(Line::from(
                tabs.into_iter()
                    .map(|(op, label)| {
                        Span::styled(
                            label,
                            if op == self.operation {
                                Theme::props()
                            } else {
                                Theme::dim()
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            )),
            Rect { height: 1, ..inner },
        );
        let labels = [
            if self.operation.is_copy() {
                "新目标 URL（父目录须存在）"
            } else {
                "源 URL"
            },
            if self.operation == WorkflowOperation::MergePreview {
                "版本区间 N:M（留空自动合并）"
            } else {
                "版本（整数 / HEAD / BASE）"
            },
            "提交信息（立即写入仓库）",
        ];
        for (index, label) in labels.iter().enumerate().take(self.field_count()) {
            let field = Rect {
                x: inner.x,
                y: inner.y + 2 + index as u16 * 3,
                width: inner.width,
                height: 3,
            };
            let border = theme::block()
                .borders(Borders::ALL)
                .title(*label)
                .border_style(if self.focus == index {
                    Theme::border_active()
                } else {
                    Theme::border()
                });
            let input = border.inner(field);
            let width = Line::from(self.values[index].as_str()).width();
            let scroll = width
                .saturating_sub(input.width.saturating_sub(1) as usize)
                .min(u16::MAX as usize) as u16;
            f.render_widget(
                Paragraph::new(self.values[index].as_str())
                    .block(border)
                    .scroll((0, scroll)),
                field,
            );
            if self.focus == index && input.width > 0 {
                f.set_cursor_position((
                    input.x + (width.saturating_sub(scroll as usize) as u16).min(input.width - 1),
                    input.y,
                ));
            }
        }
        let hint = if self.operation.is_copy() {
            "完整 URL 或 ^/branches/name、^/tags/name；默认 BASE，不含本地改动。创建后不会自动切换。"
        } else if self.operation == WorkflowOperation::Switch {
            "切换整个工作副本；请先处理改动和未纳管项。保留 SVN 祖先检查，不切换 externals。"
        } else {
            "只预演，不应用改动。N:M 表示从 N 到 M 的差异；所有输入不会触发真实 merge。"
        };
        let top = inner.y + 2 + self.field_count() as u16 * 3;
        f.render_widget(
            Paragraph::new(vec![
                Line::from(hint),
                Line::from("Tab 切字段 · Ctrl+U 清空 · Ctrl+S 准备/预演 · Esc 返回"),
            ])
            .wrap(Wrap { trim: false }),
            Rect {
                x: inner.x,
                y: top,
                width: inner.width,
                height: inner.bottom().saturating_sub(top),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn workflow_form_keeps_paste_as_text_and_changes_operation_defaults() {
        let mut panel = WorkflowPanel::default();
        panel.insert("^/branches/中文q\n\u{1b}");
        assert_eq!(panel.request().url, "^/branches/中文q");
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE)),
            InputAction::None
        );
        assert_eq!(panel.request().operation, WorkflowOperation::Switch);
        assert_eq!(panel.request().revision.as_deref(), Some("HEAD"));
        panel.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE));
        assert_eq!(panel.request().operation, WorkflowOperation::MergePreview);
        assert!(panel.request().revision.is_none());
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            InputAction::Submit
        );
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InputAction::Close
        );
    }

    #[test]
    fn workflow_form_renders_long_unicode_inputs_and_small_terminals() {
        let mut panel = WorkflowPanel::default();
        panel.insert(&format!("^/branches/{}尾部", "很长的名字".repeat(50)));
        for (width, height) in [(100, 24), (45, 18), (20, 7), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| panel.render(frame, frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            if width == 100 {
                // 中文占两列，后一列是占位单元，不能直接拼接所有 symbol 后查连续字符串。
                assert!(
                    buffer
                        .content
                        .windows(3)
                        .any(|cells| { cells[0].symbol() == "尾" && cells[2].symbol() == "部" }),
                    "聚焦字段应展示输入末尾"
                );
            }
        }
    }
}
