//! 本人提交说明独立于仓库历史；读取失败和取消都不改当前提交草稿。

use crossterm::event::KeyEvent;

use super::{App, BgDone, Mode};
use crate::tui::panels::commit_history::{Action, CommitHistoryPanel};

impl App {
    pub(super) fn open_commit_history(&mut self) {
        // 面板重开本身也是新会话，即使尚未填写作者，也不能接收上次查询的结果。
        self.commit_history_gen = self.commit_history_gen.wrapping_add(1);
        self.commit_history_from_commit = self.mode == Mode::Commit;
        let author = self
            .commit_history_authors
            .get(&self.svn.root)
            .cloned()
            .unwrap_or_default();
        let auto_query = !author.is_empty();
        let mut panel = if self.commit_history_from_commit {
            CommitHistoryPanel::new(author)
        } else {
            CommitHistoryPanel::read_only(author)
        };
        let action = if auto_query {
            panel.query()
        } else {
            Action::None
        };
        self.commit_history = Some(panel);
        self.mode = Mode::CommitHistory;
        self.apply_commit_history_action(action);
    }

    pub(super) fn handle_commit_history_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        let action = self
            .commit_history
            .as_mut()
            .map(|panel| panel.handle_key(key))
            .unwrap_or(Action::Close);
        self.apply_commit_history_action(action);
        Ok(())
    }

    fn apply_commit_history_action(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Close => {
                self.commit_history_gen = self.commit_history_gen.wrapping_add(1);
                self.commit_history = None;
                self.mode = if self.commit_history_from_commit {
                    Mode::Commit
                } else {
                    Mode::Browse
                };
            }
            Action::Insert { revision, message } => {
                self.commit_history_gen = self.commit_history_gen.wrapping_add(1);
                if let Some(panel) = self.commit.as_mut() {
                    panel.insert_str(&message);
                    panel.clear_hint();
                    panel.set_notice(format!("已插入本人 r{revision} 的提交说明；可继续编辑"));
                }
                self.commit_history = None;
                self.mode = Mode::Commit;
            }
            Action::Load { author, before } => {
                self.commit_history_authors
                    .insert(self.svn.root.clone(), author.clone());
                self.commit_history_gen = self.commit_history_gen.wrapping_add(1);
                let generation = self.commit_history_gen;
                let svn = self.svn.clone();
                let tx = self.tx.clone();
                // 按修订号分批读取；本批没有本人记录时仍保留继续向前查找的游标。
                std::thread::spawn(move || {
                    let result = svn.log_by_author(&author, before, 100);
                    let _ = tx.send(BgDone::CommitHistory(generation, before.is_some(), result));
                });
            }
        }
    }
}
