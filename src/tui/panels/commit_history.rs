//! 从 SVN 读取本人曾经使用的提交说明；回填只是编辑草稿，不执行提交。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{domain::LogEntry, svn::client::AuthorLogPage, tui::theme};

pub enum Action {
    None,
    Close,
    Load { author: String, before: Option<u64> },
    Insert { revision: u64, message: String },
}

pub struct CommitHistoryPanel {
    allow_insert: bool,
    author: String,
    applied_author: String,
    editing: bool,
    entries: Vec<LogEntry>,
    selection: ListState,
    next_before: Option<u64>,
    scanned: usize,
    loading: bool,
    error: Option<String>,
    scroll: usize,
    viewport: usize,
    rows: usize,
}

impl CommitHistoryPanel {
    pub fn new(author: String) -> Self {
        Self {
            allow_insert: true,
            author,
            applied_author: String::new(),
            editing: true,
            entries: Vec::new(),
            selection: ListState::default(),
            next_before: None,
            scanned: 0,
            loading: false,
            error: None,
            scroll: 0,
            viewport: 0,
            rows: 0,
        }
    }

    pub fn editing_author(&self) -> bool {
        self.editing
    }

    pub fn read_only(author: String) -> Self {
        let mut panel = Self::new(author);
        panel.allow_insert = false;
        panel
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn insert_author(&mut self, text: &str) {
        if self.editing {
            self.author
                .extend(text.chars().filter(|ch| !ch.is_control()));
        }
    }

    pub fn query(&mut self) -> Action {
        let author = self.author.trim().to_string();
        if author.is_empty() {
            self.error = Some("请填写 SVN 登录用户名；按作者精确匹配，不使用电脑用户名猜测".into());
            return Action::None;
        }
        self.author = author.clone();
        self.applied_author = author.clone();
        self.editing = false;
        self.loading = true;
        self.error = None;
        self.entries.clear();
        self.selection.select(None);
        self.next_before = None;
        self.scanned = 0;
        self.scroll = 0;
        self.rows = 0;
        Action::Load {
            author,
            before: None,
        }
    }

    pub fn loaded(&mut self, page: AuthorLogPage, append: bool) {
        self.loading = false;
        self.error = None;
        self.next_before = page.next_before;
        self.scanned += page.scanned;
        if !append {
            self.entries.clear();
        }
        for entry in page.entries {
            // UI 再守一次作者边界，失败或迟到的结果不能混入其他人的说明。
            if entry.author == self.applied_author
                && !self
                    .entries
                    .iter()
                    .any(|old| old.revision == entry.revision)
            {
                self.entries.push(entry);
            }
        }
        if self.selection.selected().is_none() && !self.entries.is_empty() {
            self.selection.select(Some(0));
        }
    }

    pub fn failed(&mut self, message: String) {
        self.loading = false;
        self.error = Some(message);
    }

    fn selected(&self) -> Option<&LogEntry> {
        self.selection
            .selected()
            .and_then(|index| self.entries.get(index))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Esc {
            return Action::Close;
        }
        if self.editing {
            match key.code {
                KeyCode::Enter => return self.query(),
                KeyCode::Backspace => {
                    self.author.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.author.clear()
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.author.push(ch)
                }
                _ => {}
            }
            return Action::None;
        }
        let page = self.viewport.saturating_sub(1).max(1);
        let max = self.rows.saturating_sub(self.viewport);
        match key.code {
            KeyCode::Char('q') => return Action::Close,
            KeyCode::F(2) => self.editing = true,
            KeyCode::Char('r' | 'R') if !self.loading => return self.query(),
            KeyCode::Char('n' | 'N') if !self.loading => {
                if let Some(before) = self.next_before {
                    self.loading = true;
                    return Action::Load {
                        author: self.applied_author.clone(),
                        before: Some(before),
                    };
                }
            }
            KeyCode::Enter if self.allow_insert => {
                if let Some(entry) = self.selected().filter(|entry| !entry.msg.trim().is_empty()) {
                    return Action::Insert {
                        revision: entry.revision,
                        message: entry.msg.clone(),
                    };
                }
            }
            KeyCode::Down | KeyCode::Char('j') if !self.entries.is_empty() => {
                let next = self
                    .selection
                    .selected()
                    .unwrap_or(0)
                    .saturating_add(1)
                    .min(self.entries.len() - 1);
                self.selection.select(Some(next));
                self.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') if !self.entries.is_empty() => {
                let next = self.selection.selected().unwrap_or(0).saturating_sub(1);
                self.selection.select(Some(next));
                self.scroll = 0;
            }
            KeyCode::Char('J') => self.scroll = self.scroll.saturating_add(1).min(max),
            KeyCode::Char('K') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(page).min(max),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = max,
            _ => {}
        }
        Action::None
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        theme::clear(f, area);
        let block = theme::block()
            .borders(Borders::ALL)
            .title(" 本人提交说明 · 来自 SVN ")
            .border_style(theme::Theme::border_active());
        let inner = block.inner(area);
        f.render_widget(block, area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(inner);
        let author = theme::block()
            .borders(Borders::ALL)
            .title(if self.editing {
                " SVN 用户名 · Enter 查询 "
            } else {
                " SVN 用户名 · F2 修改 "
            })
            .border_style(if self.editing {
                theme::Theme::border_active()
            } else {
                theme::Theme::border()
            });
        let author_body = author.inner(chunks[0]);
        f.render_widget(author, chunks[0]);
        let value = if self.editing {
            format!("{}▏", self.author)
        } else {
            self.author.clone()
        };
        f.render_widget(
            Paragraph::new(theme::fit(&value, author_body.width as usize)),
            author_body,
        );
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[1]);
        let title = format!(" 已提交 · {} 条 ", self.entries.len());
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(vec![
                    Line::from(Span::styled(
                        format!(
                            "r{} · {}",
                            entry.revision,
                            entry.date.get(..10).unwrap_or(&entry.date)
                        ),
                        theme::Theme::dim(),
                    )),
                    Line::from(theme::fit(
                        entry.msg.lines().next().unwrap_or("（空说明）"),
                        cols[0].width.saturating_sub(4) as usize,
                    )),
                ])
            })
            .collect();
        f.render_stateful_widget(
            List::new(items)
                .block(theme::block().borders(Borders::ALL).title(title))
                .highlight_style(theme::Theme::selected())
                .highlight_symbol("› "),
            cols[0],
            &mut self.selection,
        );
        let detail = theme::block().borders(Borders::ALL).title(" 完整说明 ");
        let body = detail.inner(cols[1]);
        f.render_widget(detail, cols[1]);
        let message = self
            .selected()
            .map(|entry| entry.msg.clone())
            .unwrap_or_else(|| {
                if self.loading {
                    "正在从 SVN 读取本人已提交的说明…".into()
                } else if self.editing {
                    "填写 SVN 用户名后查询当前项目的本人提交说明。\n只展示作者精确匹配的记录。"
                        .into()
                } else if self.next_before.is_some() {
                    "本批次未匹配到本人记录，按 N 继续查找更早记录。".into()
                } else {
                    "没有匹配的本人提交说明。请检查 SVN 用户名，也可能没有当前路径的历史读取权限。"
                        .into()
                }
            });
        let rows = super::notice::wrap_text(&message, body.width);
        self.viewport = body.height as usize;
        self.rows = rows.len();
        self.scroll = self.scroll.min(self.rows.saturating_sub(self.viewport));
        let lines: Vec<Line> = rows
            .into_iter()
            .skip(self.scroll)
            .take(self.viewport)
            .map(Line::from)
            .collect();
        f.render_widget(Paragraph::new(lines), body);
        let back = if self.allow_insert {
            "提交框"
        } else {
            "主面板"
        };
        let status = self.error.clone().unwrap_or_else(|| {
            if self.loading {
                format!("读取中… · Esc 返回{back}，草稿保持不变")
            } else if self.editing {
                format!("填写 SVN 用户名 · Enter 查询 · Esc 返回{back}")
            } else {
                format!(
                    "检查 {} 条候选 · {} · {}",
                    self.scanned,
                    if self.next_before.is_some() {
                        "N 查找更早"
                    } else {
                        "已到末尾"
                    },
                    if self.allow_insert {
                        "Enter 插入光标位置"
                    } else {
                        "只读查看"
                    }
                )
            }
        });
        f.render_widget(
            Paragraph::new(vec![
                Line::from(theme::fit(&status, chunks[2].width as usize)),
                Line::from(theme::fit(
                    &format!("↑↓ 选择 · J/K 或 PgUp/PgDn 阅读 · F2 修改作者 · Esc 返回{back}"),
                    chunks[2].width as usize,
                )),
            ]),
            chunks[2],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_does_not_duplicate_an_inflight_request_and_retry_still_works() {
        let mut panel = CommitHistoryPanel::new("alice".into());
        panel.query();
        assert!(panel.is_loading());
        assert!(matches!(
            panel.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
            Action::None
        ));
        panel.failed("连接超时".into());
        assert!(matches!(
            panel.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
            Action::Load { before: None, .. }
        ));
    }

    fn entry(author: &str, message: &str, revision: u64) -> LogEntry {
        LogEntry {
            revision,
            author: author.into(),
            date: "2026-10-30".into(),
            msg: message.into(),
            paths: vec![],
        }
    }

    #[test]
    fn only_exact_author_is_shown_and_enter_returns_the_original_message() {
        let mut panel = CommitHistoryPanel::new("alice".into());
        assert!(matches!(panel.query(), Action::Load { before: None, .. }));
        panel.loaded(
            AuthorLogPage {
                entries: vec![
                    entry("bob", "他人提到 alice", 3),
                    entry("Alice", "另一位作者", 2),
                    entry("alice", "我的说明\n第二行", 1),
                ],
                next_before: None,
                scanned: 3,
            },
            false,
        );
        assert_eq!(panel.entries.len(), 1);
        match panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            Action::Insert { revision, message } => {
                assert_eq!(revision, 1);
                assert_eq!(message, "我的说明\n第二行");
            }
            _ => panic!("Enter 应只回填选中的说明"),
        }
    }

    #[test]
    fn empty_matching_page_can_continue_and_author_input_treats_q_as_text() {
        let mut panel = CommitHistoryPanel::new(String::new());
        assert!(matches!(panel.query(), Action::None));
        panel.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        panel.insert_author("?用户\n");
        assert_eq!(panel.author, "q?用户");
        panel.query();
        panel.loaded(
            AuthorLogPage {
                entries: vec![],
                next_before: Some(40),
                scanned: 100,
            },
            false,
        );
        assert!(matches!(
            panel.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT)),
            Action::Load {
                before: Some(40),
                ..
            }
        ));
        panel.failed("模拟连接失败".into());
        assert!(!panel.is_loading());
        assert!(matches!(
            panel.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Action::Close
        ));
    }

    #[test]
    fn long_personal_message_can_be_read_to_the_end_in_narrow_windows() {
        for (width, height) in [(30, 10), (40, 12), (80, 24), (140, 40)] {
            let mut panel = CommitHistoryPanel::new("alice".into());
            panel.query();
            let message = format!("{}\nSELF_END", "自己写过的长提交说明。".repeat(80));
            panel.loaded(
                AuthorLogPage {
                    entries: vec![entry("alice", &message, 1)],
                    next_before: None,
                    scanned: 1,
                },
                false,
            );
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| panel.render(frame, theme::centered(frame.area(), 92, 88, 30, 10)))
                .unwrap();
            assert!(panel.viewport > 0, "最小支持窗口也要保留可阅读的正文行");
            panel.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
            let frame = terminal
                .draw(|frame| panel.render(frame, theme::centered(frame.area(), 92, 88, 30, 10)))
                .unwrap();
            let text: String = frame
                .buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(
                text.contains("SELF_END"),
                "{width}×{height} 必须能看到本人说明末尾"
            );
        }
    }
}
