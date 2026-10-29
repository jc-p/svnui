//! 分支工作流的异步编排：准备只读，确认后执行；过期准备结果不打开确认框。

use super::*;
use crate::svn::workflow::{WorkflowOperation, WorkflowPlan};
use crate::tui::panels::workflow::{InputAction, WorkflowPanel};

impl App {
    pub(super) fn open_workflow(&mut self) {
        self.workflow.get_or_insert_with(WorkflowPanel::default);
        self.mode = Mode::Workflow;
    }

    pub(super) fn handle_workflow_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        if self.busy.is_some() {
            // 准备过程可以退出；执行中的写操作须等待结果，不能误以为 Esc 已取消 SVN。
            if key.code == KeyCode::Esc && self.workflow_preparing {
                self.workflow = None;
                self.mode = Mode::Browse;
            }
            return Ok(());
        }
        let action = self.workflow.as_mut().map(|panel| panel.handle_key(key));
        match action {
            Some(InputAction::Close) => {
                self.workflow = None;
                self.mode = Mode::Browse;
            }
            Some(InputAction::Submit) => {
                let request = self.workflow.as_ref().expect("面板存在").request();
                let svn = self.svn.clone();
                let tx = self.tx.clone();
                self.busy = Some("准备分支操作（只读）".into());
                self.workflow_preparing = true;
                std::thread::spawn(move || {
                    let _ = tx.send(BgDone::WorkflowPrepared(svn.prepare_workflow(&request)));
                });
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn finish_workflow_prepare(&mut self, result: crate::domain::Result<WorkflowPlan>) {
        self.busy = None;
        self.workflow_preparing = false;
        if self.mode != Mode::Workflow {
            return;
        }
        match result {
            Ok(plan) if plan.operation() == WorkflowOperation::MergePreview => {
                self.execute_workflow(plan)
            }
            Ok(plan) => {
                let message = if plan.operation().is_copy() {
                    "此操作会立即在 SVN 仓库创建副本并产生新版本。Y 执行；Enter/N/Esc 取消。"
                } else {
                    "此操作会把整个工作副本切换到所示 URL 和版本。Y 执行；Enter/N/Esc 取消。"
                };
                // 危险级由 policy 定，不在调用点挑颜色：
                // branch/tag/switch 都是中危，改一处即全生效。
                let op = match plan.operation() {
                    WorkflowOperation::Branch => Op::Branch,
                    WorkflowOperation::Tag => Op::Tag,
                    WorkflowOperation::Switch => Op::Switch,
                    WorkflowOperation::MergePreview => Op::MergePreview,
                };
                self.confirm = Some(ConfirmPanel::new(
                    plan.operation().title(),
                    message,
                    plan.describe(),
                    op,
                    ConfirmAction::Workflow(plan),
                ));
                self.mode = Mode::Confirm;
            }
            Err(error) => self.notify_error("分支操作准备失败（输入已保留）", &error),
        }
    }

    pub(super) fn execute_workflow(&mut self, plan: WorkflowPlan) {
        let operation = plan.operation();
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        self.busy = Some(operation.title().into());
        self.mode = Mode::Workflow;
        std::thread::spawn(move || {
            let result = plan.execute(&svn).map(|output| {
                format!(
                    "{}\n{}\n{}",
                    operation.title(),
                    plan.describe().join("\n"),
                    output.stdout
                )
            });
            let _ = tx.send(BgDone::WorkflowFinished(operation, result));
        });
    }

    pub(super) fn finish_workflow(
        &mut self,
        operation: WorkflowOperation,
        result: crate::domain::Result<String>,
    ) {
        self.busy = None;
        match result {
            Ok(text) if operation == WorkflowOperation::MergePreview => {
                let mut lines = text
                    .lines()
                    .take(2000)
                    .map(|line| Line::from(line.to_owned()))
                    .collect::<Vec<_>>();
                if text.lines().nth(2000).is_some() {
                    lines.push(Line::from(
                        "……预演输出超过 2000 行；请用 CLI 查看完整结果。",
                    ));
                }
                self.diff = Some(PreviewPanel::with_lines(
                    PreviewKind::File,
                    "合并预演 · 未应用改动",
                    lines,
                    None,
                ));
                self.mode = Mode::Diff;
            }
            Ok(text) => {
                self.workflow = None;
                self.mode = Mode::Browse;
                self.notice = Some(text);
                self.spawn_reload();
            }
            Err(error) => {
                self.notify_error(
                    "分支操作失败（输入已保留；写操作请核对实际结果后重试）",
                    &error,
                );
                // switch 失败时也可能已经改了一部分路径，必须刷新实际状态。
                if operation == WorkflowOperation::Switch {
                    self.spawn_reload();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn native(cwd: &Path, executable: &str, args: &[&str]) -> String {
        let output = Command::new(executable)
            .args(args)
            .current_dir(cwd)
            .env("LC_ALL", "C.UTF-8")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn fixture() -> (tempfile::TempDir, App, String) {
        let temp = tempfile::tempdir().unwrap();
        native(temp.path(), "svnadmin", &["create", "repo"]);
        let url = format!("file://{}/repo", temp.path().display());
        native(
            temp.path(),
            "svn",
            &[
                "mkdir",
                &format!("{url}/trunk"),
                &format!("{url}/branches"),
                &format!("{url}/tags"),
                "-m",
                "layout",
            ],
        );
        native(
            temp.path(),
            "svn",
            &["checkout", &format!("{url}/trunk"), "wc"],
        );
        let svn = Svn::discover(&temp.path().join("wc"), Duration::from_secs(5), false).unwrap();
        let app = App::new(svn).unwrap();
        (temp, app, url)
    }

    fn key(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
            .unwrap();
    }

    fn submit(app: &mut App) {
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap();
        wait(app);
    }

    fn wait(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.is_busy() {
            assert!(Instant::now() < deadline, "后台工作流超时");
            app.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
    fn workflow_tui_requires_y_and_preserves_form_after_cancellation() {
        let (temp, mut app, url) = fixture();
        key(&mut app, KeyCode::Char('B'));
        assert_eq!(app.mode, Mode::Workflow);
        key(&mut app, KeyCode::F(2));
        key(&mut app, KeyCode::F(1));
        assert!(!app.show_help, "表单内 F1 是分支操作，不应被全局帮助截获");
        app.handle_paste("^/branches/topic".into());
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        app.handle_paste("create branch".into());
        submit(&mut app);
        assert_eq!(app.mode, Mode::Confirm);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(70, 35)).unwrap();
        terminal
            .draw(|frame| app.render(frame, frame.area()))
            .unwrap();
        let visible = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .flat_map(|cell| cell.symbol().chars())
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        assert!(
            visible.contains("topic"),
            "确认框应换行展示长 URL 末尾的分支名"
        );
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Workflow);
        assert_eq!(
            app.workflow.as_ref().unwrap().request().message,
            "create branch"
        );
        assert!(native(temp.path(), "svn", &["list", &format!("{url}/branches")]).is_empty());
        submit(&mut app);
        assert_eq!(app.mode, Mode::Confirm);
        key(&mut app, KeyCode::Char('Y'));
        wait(&mut app);
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(
            native(temp.path(), "svn", &["list", &format!("{url}/branches")]),
            "topic/\n"
        );
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
    fn workflow_tui_f4_opens_readonly_preview_and_discards_stale_preparation() {
        let (temp, mut app, url) = fixture();
        key(&mut app, KeyCode::Char('B'));
        key(&mut app, KeyCode::F(4));
        assert!(!app.show_notice, "表单内 F4 不能被通知面板截获");
        assert_eq!(
            app.workflow.as_ref().unwrap().request().operation,
            WorkflowOperation::MergePreview
        );
        app.handle_paste("^/trunk".into());
        submit(&mut app);
        assert_eq!(app.mode, Mode::Diff);
        assert!(app.confirm.is_none());
        assert!(native(temp.path(), "svn", &["status", "wc"]).is_empty());

        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('B'));
        key(&mut app, KeyCode::F(1));
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_paste("^/branches/unused".into());
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        app.handle_paste("unused".into());
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap();
        key(&mut app, KeyCode::Esc);
        wait(&mut app);
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.confirm.is_none(), "已退出的准备结果不能重新弹出写确认");
        assert!(native(temp.path(), "svn", &["list", &format!("{url}/branches")]).is_empty());
    }
}
