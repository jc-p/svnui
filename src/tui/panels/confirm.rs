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

use crate::policy::{danger_of, Danger, Op};
use crate::tui::theme;

/// 确认框里的可切换开关。
///
/// TUI 里很多写操作把参数写死了（`svn rm` 恒删本地文件、cleanup 恒不删未纳管项），
/// CLI 却在同一次调用上提供了开关。把开关放回确认框，既不新增框架，
/// 又让「默认安全、想要破坏性语义要自己打开」变成显式动作。
#[derive(Debug, Clone)]
pub struct Toggle {
    /// 显示文案。写「打开会怎样」，不写参数名（`--keep-local` 对用户没意义）。
    pub label: String,
    pub on: bool,
    /// 开关对危险级的影响：`(打开时, 关闭时)`。
    ///
    /// 只有**会改变危险级**的开关才需要填。填了之后危险级随开关实时变化，
    /// 边框与标题配色跟着变 —— 用户能在按下 Y 之前看见后果升级。
    pub danger: Option<(Danger, Danger)>,
}

impl Toggle {
    /// 普通开关：不改变危险级。
    pub fn new(label: impl Into<String>, on: bool) -> Self {
        Self {
            label: label.into(),
            on,
            danger: None,
        }
    }

    /// 带危险级联动的开关。
    pub fn with_danger(label: impl Into<String>, on: bool, when_on: Danger, when_off: Danger) -> Self {
        Self {
            label: label.into(),
            on,
            danger: Some((when_on, when_off)),
        }
    }
}

/// 确认框里的单行输入（如 update 的目标版本号）。
#[derive(Debug, Clone)]
pub struct Field {
    pub label: String,
    pub value: String,
}

/// 开关/输入框的焦点位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Toggle(usize),
    Field,
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
    /// svn delete：从版本控制删除。第二参数为 `keep_local`，
    /// 实际取值以确认框里的开关为准（构造时传的只是占位）。
    Delete(Vec<std::path::PathBuf>, bool),
    /// `svn cleanup`。删除开关由确认框的 toggle 决定，不在这里带参数 ——
    /// 否则危险级无法随开关实时变化。
    Cleanup,
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
    /// `danger_of(op)` 的静态定级。开关联动后实际等级可能高于它。
    base: Danger,
    action: ConfirmAction,
    scroll: usize,
    toggles: Vec<Toggle>,
    field: Option<Field>,
    focus: Option<Focus>,
}

impl ConfirmPanel {
    /// 危险级由 `policy::danger_of(op)` 唯一决定 —— 不在调用点手写等级，
    /// 免得同一个操作在不同入口被定成两个级别。
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        items: Vec<String>,
        op: Op,
        action: ConfirmAction,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            items,
            base: danger_of(op),
            action,
            scroll: 0,
            toggles: Vec::new(),
            field: None,
            focus: None,
        }
    }

    /// 附加可切换开关。返回 Self 以便链式构造。
    pub fn with_toggles(mut self, toggles: Vec<Toggle>) -> Self {
        self.toggles = toggles;
        self
    }

    /// 附加单行输入。
    ///
    /// 没有开关时直接把焦点放进输入框 —— 只有一个输入框还要求先按 Tab
    /// 是纯粹的摩擦。有开关时保持无焦点，避免字符键被输入框吃掉。
    pub fn with_field(mut self, field: Field) -> Self {
        let autofocus = self.toggles.is_empty();
        self.field = Some(field);
        if autofocus {
            self.focus = Some(Focus::Field);
        }
        self
    }

    pub fn action(&self) -> &ConfirmAction {
        &self.action
    }

    /// 当前开关状态（按顺序）。
    pub fn toggles(&self) -> &[Toggle] {
        &self.toggles
    }

    /// 第 `i` 个开关是否打开。越界按关闭处理。
    pub fn toggle_on(&self, i: usize) -> bool {
        self.toggles.get(i).map(|t| t.on).unwrap_or(false)
    }

    /// 输入框当前内容。
    pub fn field_value(&self) -> Option<&str> {
        self.field.as_ref().map(|f| f.value.as_str())
    }

    /// 打开状态下重新定级：静态级与所有开关联动级取最高。
    ///
    /// 这样 `svn rm` 的「保留本地文件」一关，等级立刻从 中危 升到 高危，
    /// 边框先变色，用户不必按了 Y 才知道。
    pub fn danger(&self) -> Danger {
        let mut d = self.base;
        for t in &self.toggles {
            if let Some((when_on, when_off)) = t.danger {
                let applied = if t.on { when_on } else { when_off };
                if applied > d {
                    d = applied;
                }
            }
        }
        d
    }

    /// Tab：在 开关 → 输入框 → 无焦点（滚列表） 之间循环。
    pub fn focus_next(&mut self) {
        let n = self.toggles.len();
        self.focus = match self.focus {
            None if n > 0 => Some(Focus::Toggle(0)),
            None if self.field.is_some() => Some(Focus::Field),
            None => None,
            Some(Focus::Toggle(i)) if i + 1 < n => Some(Focus::Toggle(i + 1)),
            Some(Focus::Toggle(_)) if self.field.is_some() => Some(Focus::Field),
            Some(_) => None,
        };
    }

    /// 是否有可聚焦控件（决定按键行要不要提示 Tab）。
    pub fn has_controls(&self) -> bool {
        !self.toggles.is_empty() || self.field.is_some()
    }

    /// Space：切换当前聚焦的开关。未聚焦开关时无动作。
    pub fn toggle_focused(&mut self) {
        if let Some(Focus::Toggle(i)) = self.focus {
            if let Some(t) = self.toggles.get_mut(i) {
                t.on = !t.on;
            }
        }
    }

    /// 输入框聚焦时追加一个字符。
    pub fn field_push(&mut self, c: char) {
        if let (Some(Focus::Field), Some(f)) = (self.focus, self.field.as_mut()) {
            f.value.push(c);
        }
    }

    /// 输入框聚焦时退格。
    pub fn field_pop(&mut self) {
        if let (Some(Focus::Field), Some(f)) = (self.focus, self.field.as_mut()) {
            f.value.pop();
        }
    }

    /// 当前聚焦的是输入框（用于决定字符键归属）。
    pub fn field_focused(&self) -> bool {
        matches!(self.focus, Some(Focus::Field))
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n as usize);
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n as usize);
    }

    fn accent(&self) -> Color {
        match self.danger() {
            Danger::Safe => Color::Cyan,
            Danger::Low => Color::Yellow,
            Danger::Medium => Color::LightYellow,
            Danger::High => Color::LightRed,
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
        // 危险级写进标题：`svn rm` 关掉「保留本地文件」时这里从「中危」变「高危·不可撤销」。
        let block = theme::block()
            .borders(Borders::ALL)
            .title(Line::from(vec![
                Span::styled(
                    format!(" {} ", self.title),
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("[{}] ", self.danger().label()),
                    Style::default().fg(accent),
                ),
            ]))
            .border_style(Style::default().fg(accent));

        let inner = block.inner(pop);
        f.render_widget(block, pop);

        // 开关与输入框各占一行；没有控件时该段高度为 0，布局不变。
        let controls_height = self.toggles.len() as u16 + u16::from(self.field.is_some());

        let message_lines = wrap_message(&self.message, inner.width as usize);
        let reserve = 4 + controls_height;
        let message_height = if detailed {
            message_lines
                .len()
                .min(inner.height.saturating_sub(reserve) as usize) as u16
        } else {
            2
        };
        // 详细提示按显示宽度换行，同时为目标列表、选项和确认键留出空间。
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(message_height), // 提示语
                Constraint::Min(3),                 // 文件列表
                Constraint::Length(controls_height), // 开关 / 输入
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

        // 开关 / 输入框。聚焦项用反色标出来，否则用户不知道 Space 会动哪一个。
        if controls_height > 0 {
            let mut lines: Vec<Line> = Vec::new();
            for (i, t) in self.toggles.iter().enumerate() {
                let focused = matches!(self.focus, Some(Focus::Toggle(x)) if x == i);
                let box_char = if t.on { "[x]" } else { "[ ]" };
                let mut spans = vec![
                    Span::styled(
                        format!(" {box_char} "),
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        t.label.clone(),
                        if focused {
                            Style::default()
                                .fg(Color::Black)
                                .bg(accent)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(accent)
                        },
                    ),
                ];
                if focused {
                    spans.push(Span::styled("  ← Space 切换", theme::Theme::dim()));
                }
                lines.push(Line::from(spans));
            }
            if let Some(fld) = self.field.as_ref() {
                let focused = self.field_focused();
                lines.push(Line::from(vec![
                    Span::styled(format!(" {} ", fld.label), theme::Theme::dim()),
                    Span::styled(
                        if fld.value.is_empty() {
                            "（留空）".to_string()
                        } else {
                            fld.value.clone()
                        },
                        if focused {
                            Style::default()
                                .fg(Color::Black)
                                .bg(accent)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(accent)
                        },
                    ),
                    if focused {
                        Span::styled(
                            "  ← 直接输入，Backspace 删除",
                            theme::Theme::dim(),
                        )
                    } else {
                        Span::raw(String::new())
                    },
                ]));
            }
            f.render_widget(Paragraph::new(lines), chunks[2]);
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
            // 删除的后果取决于「保留本地文件」开关，文案必须跟着开关走。
            ConfirmAction::Delete(_, _) => {
                if self.toggle_on(0) {
                    "从版本控制移除（本地文件保留）"
                } else {
                    "删除（本地文件一并删除）"
                }
            }
            ConfirmAction::Update => "继续更新",
            ConfirmAction::Resolve(_, _) => "应用该方案",
            ConfirmAction::RevertToRev(_) => "回退（需再提交一次）",
            ConfirmAction::RestoreFromRepo(_) => "恢复服务器版本（本地改动将丢失）",
            ConfirmAction::Cleanup => "执行清理",
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
        let keys = if self.has_controls() {
            let mut v = vec![
                Span::styled(
                    " Tab ",
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled("选项    ", theme::Theme::dim()),
            ];
            v.extend(keys.spans);
            Line::from(v)
        } else {
            keys
        };
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
        f.render_widget(Paragraph::new(keys).alignment(Alignment::Left), chunks[3]);
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
            vec!["new.txt".into()], Op::Add, ConfirmAction::Add(vec![]));
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
            Op::Add,
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
            vec!["A 新增.txt\n  取消添加，保留本地文件".into()], Op::Revert, ConfirmAction::Revert(vec![]));
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

    /// 删本地文件是不可撤销的，所以关掉「保留本地」必须把等级顶到 High。
    #[test]
    fn keep_local_toggle_drives_danger_level() {
        let mut panel = ConfirmPanel::new(
            "确认删除",
            "将从版本控制删除 X",
            vec!["X".into()],
            Op::Remove,
            ConfirmAction::Delete(vec![], false),
        )
        .with_toggles(vec![Toggle::with_danger(
            "保留本地文件",
            false,
            Danger::Medium,
            Danger::High,
        )]);
        // Remove 静态是 Medium，但开关关闭 = 连本地文件一起删 → High。
        assert_eq!(panel.danger(), Danger::High);

        panel.focus_next();
        panel.toggle_focused();
        assert!(panel.toggle_on(0));
        assert_eq!(panel.danger(), Danger::Medium);
    }

    /// cleanup 反过来：默认只解锁（Low），打开删除开关才升到 High。
    #[test]
    fn cleanup_delete_toggles_escalate_danger() {
        let mut panel = ConfirmPanel::new(
            "清理工作副本",
            "svn cleanup",
            vec![],
            Op::Cleanup,
            ConfirmAction::Cleanup,
        )
        .with_toggles(vec![
            Toggle::with_danger("删除未纳管", false, Danger::High, Danger::Low),
            Toggle::with_danger("删除忽略项", false, Danger::High, Danger::Low),
        ]);
        assert_eq!(panel.danger(), Danger::Low);
        panel.focus_next();
        panel.toggle_focused();
        assert_eq!(panel.danger(), Danger::High);
    }

    #[test]
    fn revision_field_accepts_typed_input_and_backspace() {
        let mut panel = ConfirmPanel::new(
            "更新到指定版本",
            "填入版本号",
            vec![],
            Op::Update,
            ConfirmAction::Update,
        )
        .with_field(Field {
            label: "版本".into(),
            value: String::new(),
        });
        // 只有输入框时自动聚焦，否则用户得先按 Tab 才能打字。
        assert!(panel.field_focused());
        for c in "1200:1234".chars() {
            panel.field_push(c);
        }
        assert_eq!(panel.field_value(), Some("1200:1234"));
        panel.field_pop();
        assert_eq!(panel.field_value(), Some("1200:123"));
    }

    /// 等级必须写在标题里 —— 只靠边框颜色的话，配色主题一换就没人看见了。
    #[test]
    fn danger_label_appears_in_title() {
        let mut panel = ConfirmPanel::new(
            "丢弃本地改动",
            "范围：当前文件",
            vec![],
            Op::Revert,
            ConfirmAction::Revert(vec![]),
        );
        let text = render(&mut panel, 60, 20);
        assert!(text.contains("高危"), "标题应显示危险级：{text}");
    }

    #[test]
    fn revert_long_paths_wrap_and_scroll_to_their_consequences() {
        let path = format!("目录/{}/末尾.txt", "长路径".repeat(30));
        let mut panel = ConfirmPanel::new(
            "丢弃本地改动",
            "范围：当前文件",
            vec![format!("A {path}\n  取消添加，保留本地文件")],
            Op::Revert,
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
