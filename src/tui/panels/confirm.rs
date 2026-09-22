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
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
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
    scroll: u16,
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
        self.scroll = self.scroll.saturating_add(n);
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
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
        let pop = Self::centered(area, 70, 80);
        f.render_widget(Clear, pop);

        let accent = self.accent();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            )))
            .border_style(Style::default().fg(accent));

        let inner = block.inner(pop);
        f.render_widget(block, pop);

        // 布局：提示 / 文件列表（占满剩余）/ 底部按键
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // 提示语（可能换行）
                Constraint::Min(3),    // 文件列表
                Constraint::Length(1), // 按键行
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new(self.message.as_str())
                .style(Style::default().fg(accent))
                .wrap(Wrap { trim: false }),
            chunks[0],
        );

        if self.items.is_empty() {
            f.render_widget(
                Paragraph::new("（无）").style(theme::Theme::dim()),
                chunks[1],
            );
        } else {
            let list_h = chunks[1].height as usize;
            // clamp：列表变短后旧的 scroll 会让画面停在空白
            let max_scroll = self.items.len().saturating_sub(list_h) as u16;
            if self.scroll > max_scroll {
                self.scroll = max_scroll;
            }
            let items: Vec<ListItem> = self
                .items
                .iter()
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
        // 避免"按 Enter 手滑就执行了"。Y 和 N 是这套 UI 里
        // 所有确认框的统一语义 —— 不用记哪个框是 Enter、哪个是 Esc。
        //
        // 每个框把「按了会怎样」写死，不用通用的"确认/取消"：
        // 用户得先想"确认是干啥"，出事故的往往就是那一下想当然。
        let yes_txt = match &self.action {
            ConfirmAction::Revert(_) => "还原（本地改动将丢失）",
            ConfirmAction::Delete(_, _) => "删除（本地文件一并删除）",
            ConfirmAction::Update => "继续更新",
            ConfirmAction::Resolve(_, _) => "应用该方案",
            ConfirmAction::RevertToRev(_) => "回退（需再提交一次）",
        };
        let keys = Line::from(vec![
            Span::styled(" Y ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}    ", yes_txt), theme::Theme::dim()),
            Span::styled(" N ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::styled("不执行（Esc / q）    ", theme::Theme::dim()),
            Span::styled("j/k ", theme::Theme::dim()),
            Span::styled("滚动", theme::Theme::dim()),
        ]);
        f.render_widget(Paragraph::new(keys).alignment(Alignment::Left), chunks[2]);
    }
}
