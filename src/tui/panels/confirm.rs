//! 通用确认面板。
//!
//! ## 为什么需要它
//!
//! `svn revert` 是**不可逆**的：本地改动一旦还原就找不回来了。
//! `svn update` 在有远端改动时也可能制造一堆冲突。
//!
//! 这类操作必须先告诉用户"你要动的到底是哪些文件"，再让他决定。
//!
//! ## 为什么不用 `ya.confirm` 那种两按钮
//!
//! 那是 yazi 的限制（只有 `[Y]es`/`(N)o`）。在 ratatui 里我们完全
//! 自己画，可以：列出受影响文件、显示危险级别、用 `Enter`/`Esc` 操作。

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::tui::theme;

/// 危险级别。影响标题和边框配色 —— 让用户一眼看出后果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Danger {
    /// 提示性确认（update 前的预警）。
    Warn,
    /// 破坏性确认（revert）。
    Critical,
}

/// 确认后要执行的动作。
///
/// 做成枚举而不是闭包：闭包要捕获 `Svn`（不 Send）或 `Vec<PathBuf>`，
/// 生命周期很难摆平；枚举让 `App` 在一个 match 里集中处理，
/// 所有副作用都在一处，好推理。
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// URL 和版本在准备时冻结；确认后仍会复核工作副本状态。
    Workflow(crate::svn::workflow::WorkflowPlan),
    /// 加入版本控制；路径在确认框打开时冻结，目录遵循 svn add 的递归规则。
    Add(Vec<std::path::PathBuf>),
    /// 真正执行 revert。
    Revert(Vec<std::path::PathBuf>),
    /// 继续执行 update（已知有远端改动）。
    Update,
    /// 解决冲突：对该路径应用该策略。
    Resolve(std::path::PathBuf, crate::tui::panels::conflict::Strategy),
    /// svn delete：从版本控制删除。第二参数为 `keep_local`。
    Delete(Vec<std::path::PathBuf>, bool),
    /// 反向合并回到指定版本（历史面板里按 R）。
    /// 带版本号是为了确认框里能写清楚"要回到哪一版"。
    RevertToRev(u64),
    /// 树冲突的「恢复服务器版本」。
    ///
    /// ⚠ 树冲突**只能** resolve 到 `working` —— `mine-full` / `theirs-full`
    /// 会被 svn 拒绝（`E155027 Inapplicable conflict resolution option`）。
    /// 所以"要服务器版本"不能走 resolve，得两步：
    ///   `svn revert -R <path>`  重建文件（base 版），顺带清掉树冲突标记
    ///   `svn update  <path>`    拉到服务器最新
    RestoreFromRepo(std::path::PathBuf),
}

/// 确认面板。
pub struct ConfirmPanel {
    title: String,
    /// 主提示语。
    message: String,
    /// 受影响的文件（可滚动）。
    items: Vec<String>,
    danger: Danger,
    action: ConfirmAction,
    scroll: usize,
}

impl ConfirmPanel {
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        items: Vec<String>,
        danger: Danger,
        action: ConfirmAction,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            items,
            danger,
            action,
            scroll: 0,
        }
    }

    pub fn action(&self) -> &ConfirmAction {
        &self.action
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n as usize);
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n as usize);
    }

    fn accent(&self) -> Color {
        match self.danger {
            Danger::Warn => Color::Yellow,
            Danger::Critical => Color::LightRed,
        }
    }

    /// 居中弹框，最大 70% 宽 / 80% 高。
    fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
        let w = (area.width * pct_w / 100).max(30).min(area.width);
        let h = (area.height * pct_h / 100).max(8).min(area.height);
        Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let adding = matches!(self.action, ConfirmAction::Add(_));
        let reverting = matches!(self.action, ConfirmAction::Revert(_));
        let workflow = matches!(self.action, ConfirmAction::Workflow(_));
        let detailed = adding || reverting || workflow;
        let pop = if detailed {
            Self::centered(area, 90, 90)
        } else {
            Self::centered(area, 70, 80)
        };
        f.render_widget(Clear, pop);

        let accent = self.accent();
        let block = theme::block()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            )))
            .border_style(Style::default().fg(accent));

        let inner = block.inner(pop);
        f.render_widget(block, pop);

        let message_lines = wrap_message(&self.message, inner.width as usize);
        let message_height = if detailed {
            message_lines
                .len()
                .min(inner.height.saturating_sub(4) as usize) as u16
        } else {
            2
        };
        // 详细提示按显示宽度换行，同时为目标列表和确认键留出空间。
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(message_height), // 提示语
                Constraint::Min(3),                 // 文件列表
                Constraint::Length(1),              // 按键行
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new(if detailed {
                message_lines
                    .into_iter()
                    .map(Line::from)
                    .collect::<Vec<_>>()
            } else {
                self.message.lines().map(Line::from).collect::<Vec<_>>()
            })
            .style(Style::default().fg(accent))
            .wrap(Wrap { trim: false }),
            chunks[0],
        );

        // 路径和分支 URL 按终端宽度展开，避免关键目标被水平截断；滚动以显示行为单位。
        let wrapped_rows;
        let rows = if reverting || workflow {
            wrapped_rows = self
                .items
                .iter()
                .flat_map(|item| wrap_message(item, inner.width.saturating_sub(2) as usize))
                .collect::<Vec<_>>();
            &wrapped_rows
        } else {
            &self.items
        };
        if rows.is_empty() {
            f.render_widget(
                Paragraph::new("（无）").style(theme::Theme::dim()),
                chunks[1],
            );
        } else {
            let list_h = chunks[1].height as usize;
            // clamp：列表变短后旧的 scroll 会让画面停在空白
            let max_scroll = rows.len().saturating_sub(list_h);
            if self.scroll > max_scroll {
                self.scroll = max_scroll;
            }
            let items: Vec<ListItem> = rows
                .iter()
                .skip(self.scroll)
                .take(list_h)
                .map(|s| ListItem::new(Line::from(Span::raw(format!("  {s}")))))
                .collect();
            // ⚠️ 不能用 status_bar()：那是"黑字青底"，给顶栏设计的。
            //    用在多行列表上会变成一整片蓝底，既刺眼也盖住文字。
            //    文件列表就用普通前景色，危险性由标题和边框的红色表达。
            f.render_widget(
                List::new(items).style(Style::default().fg(Color::Gray)),
                chunks[1],
            );
        }

        // Yes / No 而不是 确认 / 取消：
        // 破坏性操作（revert / delete）用明确的 Y/N，
        // 丢弃本地改动时 Enter 取消；其他操作保留原有确认方式。
        //
        // 每个框把「按了会怎样」写死，不用通用的"确认/取消"：
        // 用户得先想"确认是干啥"，出事故的往往就是那一下想当然。
        let yes_txt = match &self.action {
            ConfirmAction::Workflow(plan) => plan.operation().title(),
            ConfirmAction::Add(_) => "加入 SVN（尚未提交）",
            ConfirmAction::Revert(_) => "丢弃本地改动",
            ConfirmAction::Delete(_, _) => "删除（本地文件一并删除）",
            ConfirmAction::Update => "继续更新",
            ConfirmAction::Resolve(_, _) => "应用该方案",
            ConfirmAction::RevertToRev(_) => "回退（需再提交一次）",
            ConfirmAction::RestoreFromRepo(_) => "恢复服务器版本（本地改动将丢失）",
        };
        let keys = Line::from(vec![
            Span::styled(
                " Y ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{}    ", yes_txt), theme::Theme::dim()),
            Span::styled(
                " N ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("不执行（Esc / q）    ", theme::Theme::dim()),
            Span::styled("j/k ", theme::Theme::dim()),
            Span::styled("滚动", theme::Theme::dim()),
        ]);
        let keys = if adding {
            Line::from(vec![
                Span::styled(
                    "Y 添加  N 取消  ",
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled("j/k 滚动", theme::Theme::dim()),
            ])
        } else if workflow {
            Line::from(Span::styled(
                "Y 执行  Enter/N/Esc 取消  j/k 滚动",
                Style::default().fg(accent),
            ))
        } else if reverting {
            Line::from(Span::styled(
                "Y 丢弃  Enter/N 取消  j/k 滚动",
                Style::default().fg(accent),
            ))
        } else {
            keys
        };
        f.render_widget(Paragraph::new(keys).alignment(Alignment::Left), chunks[2]);
    }
}

/// 按终端列宽预先换行，详细确认框据此分配说明和清单高度。
fn wrap_message(message: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for original in message.lines() {
        let mut line = String::new();
        let mut used = 0;
        for ch in original.chars() {
            let cells = Line::from(ch.to_string()).width();
            if cells > width {
                continue;
            }
            if used + cells > width {
                rows.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(ch);
            used += cells;
        }
        rows.push(line);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(panel: &mut ConfirmPanel, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| panel.render(f, f.area())).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .flat_map(|c| c.symbol().chars())
            .filter(|c| !c.is_whitespace() && *c != '│')
            .collect()
    }

    #[test]
    fn add_scope_and_cancel_keys_are_visible_in_narrow_terminal() {
        let mut panel = ConfirmPanel::new("确认加入 SVN：1 个目标",
            "范围：全部未纳管项（包含搜索过滤外的项目）\n目录递归添加未忽略子项。\n仅加入 SVN，尚未提交。",
            vec!["new.txt".into()], Danger::Warn, ConfirmAction::Add(vec![]));
        let text = render(&mut panel, 40, 20);
        for expected in [
            "过滤外",
            "目录递归",
            "尚未提交",
            "new.txt",
            "Y添加",
            "N取消",
        ] {
            assert!(text.contains(expected), "缺少 {expected}：{text}");
        }
    }

    #[test]
    fn target_list_scrolls_to_last_item_and_back() {
        let mut panel = ConfirmPanel::new(
            "添加",
            "仅加入 SVN",
            (0..40).map(|i| format!("target-{i:02}.txt")).collect(),
            Danger::Warn,
            ConfirmAction::Add(vec![]),
        );
        let first = render(&mut panel, 80, 20);
        assert!(first.contains("target-00.txt"));
        assert!(!first.contains("target-39.txt"));
        panel.scroll_down(u16::MAX);
        let last = render(&mut panel, 80, 20);
        assert!(last.contains("target-39.txt"));
        assert!(!last.contains("target-00.txt"));
        panel.scroll_up(u16::MAX);
        assert!(render(&mut panel, 80, 20).contains("target-00.txt"));
    }
    #[test]
    fn revert_scope_consequences_and_safe_default_are_visible_in_narrow_terminal() {
        let mut panel = ConfirmPanel::new("丢弃本地改动：1 项",
            "范围：当前项 目录\n目录递归撤销，包含执行时目录内的版本改动。\n丢弃的内容无法恢复；不影响仓库历史。",
            vec!["A 新增.txt\n  取消添加，保留本地文件".into()], Danger::Critical, ConfirmAction::Revert(vec![]));
        let text = render(&mut panel, 40, 20);
        for expected in [
            "范围",
            "目录递归",
            "无法恢复",
            "新增.txt",
            "保留本地文件",
            "Y丢弃",
            "Enter/N取消",
        ] {
            assert!(text.contains(expected), "缺少 {expected}：{text}");
        }
        assert!(text.contains('╭') && text.contains('╯'));
    }

    #[test]
    fn revert_long_paths_wrap_and_scroll_to_their_consequences() {
        let path = format!("目录/{}/末尾.txt", "长路径".repeat(30));
        let mut panel = ConfirmPanel::new(
            "丢弃本地改动",
            "范围：当前文件",
            vec![format!("A {path}\n  取消添加，保留本地文件")],
            Danger::Critical,
            ConfirmAction::Revert(vec![]),
        );
        let first = render(&mut panel, 40, 14);
        assert!(first.contains("目录/"));
        panel.scroll_down(u16::MAX);
        let last = render(&mut panel, 40, 14);
        assert!(last.contains("末尾.txt"), "末尾未显示：{last}");
        assert!(last.contains("保留本地文件"));
        assert!(last.contains("Enter/N取消"));
    }

}
