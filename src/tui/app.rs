//! 应用状态机。
//!
//! 一个 enum 表示"当前在哪个模式"，所有输入先落到这里再分发。
//! 刻意不做成嵌套的弹框栈 —— 三个模式足够，加栈只会让返回路径变复杂。

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{layout::Rect, Frame};

// use crate::svn::;
use crate::svn::{StatusOpts, Svn};
use crate::tui::panels::commit::{CommitPanel, Focus};
use crate::tui::panels::diff::DiffPanel;
use crate::tui::panels::status::StatusPanel;
use crate::tui::panels::Item;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// 主状态列表。
    Status,
    /// 提交面板（弹框）。
    Commit,
    /// diff（弹框）。
    Diff,
}

pub struct App {
    svn: Svn,
    items: Vec<Item>,
    mode: Mode,

    status: StatusPanel,
    commit: Option<CommitPanel>,
    diff: Option<DiffPanel>,

    /// 最近一条提示（错误或成功信息）。
    notice: Option<String>,
    should_quit: bool,
}

impl App {
    pub fn new(svn: Svn) -> crate::domain::Result<Self> {
        let mut app = Self {
            svn,
            items: Vec::new(),
            mode: Mode::Status,
            status: StatusPanel::new(),
            commit: None,
            diff: None,
            notice: None,
            should_quit: false,
        };
        app.reload()?;
        if !app.items.is_empty() {
            app.status.select(Some(0));
        }
        Ok(app)
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// 重新扫描。
    fn reload(&mut self) -> crate::domain::Result<()> {
        let snap = self.svn.snapshot(&StatusOpts::default())?;
        let root = std::path::PathBuf::from(snap.root.as_str());

        let mut items: Vec<Item> = snap
            .changed()
            .into_iter()
            .map(|e| Item::from_entry(e, &root))
            .collect();

        // 冲突优先排在最前 —— 它是唯一会阻塞提交的
        items.sort_by_key(|i| match i.sign {
            'C' | 'T' => 0,
            '!' | 'D' => 1,
            'R' => 2,
            'A' => 3,
            'M' => 4,
            '?' => 5,
            _ => 6,
        });

        self.items = items;
        Ok(())
    }

    fn selected_item(&self) -> Option<&Item> {
        self.status.selected().and_then(|i| self.items.get(i))
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_item()
            .map(|i| vec![i.abs.clone()])
            .unwrap_or_default()
    }

    // ── 输入分发 ──────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        match self.mode {
            Mode::Status => self.handle_status_key(key),
            Mode::Commit => self.handle_commit_key(key),
            Mode::Diff => self.handle_diff_key(key),
        }
    }

    fn handle_status_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => self.should_quit = true,
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => self.status.move_down(self.items.len()),
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => self.status.move_up(),

            (KeyCode::Char('c'), _) => self.open_commit(),

            (KeyCode::Enter, _) => self.open_diff()?,

            (KeyCode::Char('a'), _) => self.do_add()?,
            (KeyCode::Char('r'), _) => self.do_revert()?,
            (KeyCode::Char('u'), _) => self.do_update()?,
            (KeyCode::Char('h'), _) => self.do_doctor()?,
            (KeyCode::Char('R'), _) => {
                self.reload()?;
                self.notice = Some(format!("已刷新：{} 项变更", self.items.len()));
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_commit_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        let Some(panel) = self.commit.as_mut() else {
            self.mode = Mode::Status;
            return Ok(());
        };

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                self.commit = None;
                self.mode = Mode::Status;
                self.notice = Some("已取消提交".into());
                return Ok(());
            }
            (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
                panel.switch_focus();
                return Ok(());
            }
            (KeyCode::Enter, KeyModifiers::CONTROL) => return self.do_commit(),
            _ => {}
        }

        match panel.focus() {
            Focus::Files => match key.code {
                KeyCode::Char('j') | KeyCode::Down => panel.move_down(),
                KeyCode::Char('k') | KeyCode::Up => panel.move_up(),
                KeyCode::Char(' ') => panel.toggle_current(),
                KeyCode::Char('a') => panel.all(),
                KeyCode::Char('n') => panel.none(),
                _ => {}
            },
            Focus::Message => {
                // TextArea 自己处理输入（含 Emacs 快捷键、undo/redo）
                panel.textarea_mut().input(key);
            }
        }
        Ok(())
    }

    fn handle_diff_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        let Some(panel) = self.diff.as_mut() else {
            self.mode = Mode::Status;
            return Ok(());
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.diff = None;
                self.mode = Mode::Status;
            }
            KeyCode::Char('j') | KeyCode::Down => panel.scroll_down(1),
            KeyCode::Char('k') | KeyCode::Up => panel.scroll_up(1),
            KeyCode::Char('d') | KeyCode::PageDown => panel.scroll_down(10),
            KeyCode::Char('u') | KeyCode::PageUp => panel.scroll_up(10),
            KeyCode::Char('g') => panel.scroll_top(),
            _ => {}
        }
        Ok(())
    }

    // ── 动作 ──────────────────────────────────────────────

    fn open_commit(&mut self) {
        if self.items.is_empty() {
            self.notice = Some("没有可提交的变更".into());
            return;
        }
        self.commit = Some(CommitPanel::new(self.items.clone()));
        self.mode = Mode::Commit;
    }

    fn open_diff(&mut self) -> crate::domain::Result<()> {
        let Some(item) = self.selected_item().cloned() else {
            return Ok(());
        };

        // 未版本化文件 svn diff 会报 E155010（没有 BASE 可比较），
        // 直接给提示，别去惊动 svn。
        if item.sign == '?' {
            self.notice = Some(format!(
                "{} 未版本化，无 diff。按 a 先 svn add",
                item.display()
            ));
            return Ok(());
        }

        let out = self.svn.diff(&[item.abs.clone()], None, false)?;
        let raw = out.stdout.clone();
        self.diff = Some(DiffPanel::new(item.display(), &raw));
        self.mode = Mode::Diff;
        Ok(())
    }

    fn do_add(&mut self) -> crate::domain::Result<()> {
        // 只对未版本化的执行。混一个已版本化的会导致整批失败（E150002）。
        let targets: Vec<PathBuf> = self
            .items
            .iter()
            .filter(|i| i.sign == '?')
            .map(|i| i.abs.clone())
            .collect();

        if targets.is_empty() {
            self.notice = Some("没有未版本化的文件".into());
            return Ok(());
        }

        match self.svn.add(&targets) {
            Ok(_) => {
                let n = targets.len();
                self.reload()?;
                self.notice = Some(format!("已 add {} 项", n));
            }
            Err(e) => self.notice = Some(format!("add 失败：{}", e)),
        }
        Ok(())
    }

    fn do_revert(&mut self) -> crate::domain::Result<()> {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return Ok(());
        }
        // revert 是高危操作：先干跑一次，把受影响清单放进提示，
        // 让用户看到后果再决定。这里默认干跑，真正执行留给 y/n 确认。
        match self.svn.revert(&paths, true) {
            Ok(_) => {
                self.notice = Some(format!(
                    "revert 干跑：{} 项将被还原。正式执行请另开终端跑 svn revert",
                    paths.len()
                ));
            }
            Err(e) => self.notice = Some(format!("revert 失败：{}", e)),
        }
        Ok(())
    }

    fn do_update(&mut self) -> crate::domain::Result<()> {
        match self.svn.update(None, &[]) {
            Ok(out) => {
                let msg = out.stdout.clone();
                self.reload()?;
                self.notice = Some(msg.lines().last().unwrap_or("已更新").to_string());
            }
            Err(e) => self.notice = Some(format!("update 失败：{}", e)),
        }
        Ok(())
    }

    fn do_doctor(&mut self) -> crate::domain::Result<()> {
        match self.svn.conflicts() {
            Ok(c) if c.is_empty() => self.notice = Some("✓ 无冲突，工作副本健康".into()),
            Ok(c) => self.notice = Some(format!("⚠ {} 处冲突，需要处理", c.len())),
            Err(e) => self.notice = Some(format!("体检失败：{}", e)),
        }
        Ok(())
    }

    fn do_commit(&mut self) -> crate::domain::Result<()> {
        let Some(panel) = self.commit.as_ref() else {
            return Ok(());
        };

        let msg = panel.message();
        if msg.is_empty() {
            // 借可变借用把提示塞进去
            if let Some(p) = self.commit.as_mut() {
                p.set_hint("提交信息不能为空");
            }
            return Ok(());
        }

        let paths = panel.checked_paths();
        if paths.is_empty() {
            if let Some(p) = self.commit.as_mut() {
                p.set_hint("没有勾选任何文件");
            }
            return Ok(());
        }

        let n = paths.len();
        // 先把面板摘掉，避免借用冲突
        let result = self.svn.commit(&msg, &paths);
        self.commit = None;
        self.mode = Mode::Status;

        match result {
            Ok(out) => {
                let s = out.stdout.clone();
                self.reload()?;
                self.notice = Some(format!(
                    "已提交 {} 项 — {}",
                    n,
                    s.lines()
                        .find(|l| l.contains("Committed"))
                        .unwrap_or("完成")
                ));
            }
            Err(e) => self.notice = Some(format!("提交失败：{}", e)),
        }
        Ok(())
    }

    // ── 渲染 ──────────────────────────────────────────────

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        self.status.render(f, area, &self.items);

        if let Some(notice) = &self.notice {
            use ratatui::text::{Line, Span};
            use ratatui::widgets::{Clear, Paragraph};

            // 底部浮一条提示，不遮挡列表
            let h = 1u16;
            let y = area.bottom().saturating_sub(h + 1);
            let rect = Rect::new(area.x + 1, y, area.width.saturating_sub(2), h);
            f.render_widget(Clear, rect);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    notice.as_str(),
                    crate::tui::theme::Theme::title(),
                ))),
                rect,
            );
        }

        if let Some(panel) = self.commit.as_mut() {
            panel.render(f, centered_rect(80, 80, area));
        }
        if let Some(panel) = self.diff.as_mut() {
            panel.render(f, centered_rect(90, 90, area));
        }
    }
}

/// 居中的弹框区域。
///
/// `percent_x/y` 是占父区域的百分比。
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    use ratatui::layout::{Constraint, Direction, Layout};

    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r)[1];

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup)[1]
}
