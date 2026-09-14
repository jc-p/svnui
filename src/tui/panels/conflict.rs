//! 冲突解决面板。
//!
//! ## 为什么值得单独做
//!
//! 冲突是 SVN 最痛的操作。命令行下你要：
//! 找到冲突文件 → 打开看 `<<<<<<<` 标记 → 决定留哪份 →
//! 敲 `svn resolve --accept=xxx` → 再确认。
//!
//! 参数还记不住（`mine-full` / `theirs-full` / `working` 到底哪个是哪个）。
//! 这里把三份内容并排放，选一个按数字键就完事。
//!
//! ## 三路内容从哪来
//!
//! `svn` 冲突后在工作副本里生成 `f.mine` / `f.rOLD` / `f.rNEW`，
//! 直接用它们，比 `svn cat` 再算一遍准确（见 `client::conflict_versions`）。

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::theme;

/// 正在看哪一份。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Mine,
    Theirs,
    Working,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Mine => "我的（本地改动）",
            Side::Theirs => "服务器的（远端最新）",
            Side::Working => "当前文件（带冲突标记）",
        }
    }

    fn next(self) -> Self {
        match self {
            Side::Mine => Side::Theirs,
            Side::Theirs => Side::Working,
            Side::Working => Side::Mine,
        }
    }
}

/// `svn resolve --accept=` 的策略。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Strategy {
    /// `mine-full`：完全用我的，丢弃服务器的改动。
    MineFull,
    /// `theirs-full`：完全用服务器的，丢弃我的改动。
    TheirsFull,
    /// `working`：保留当前文件内容（通常是手动编辑后标记已解决）。
    Working,
}

impl Strategy {
    pub fn arg(self) -> &'static str {
        match self {
            Strategy::MineFull => "mine-full",
            Strategy::TheirsFull => "theirs-full",
            Strategy::Working => "working",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Strategy::MineFull => "用我的",
            Strategy::TheirsFull => "用服务器的",
            Strategy::Working => "保留当前文件",
        }
    }

    /// 这个策略会丢什么。确认前必须说清楚。
    pub fn warning(self) -> &'static str {
        match self {
            Strategy::MineFull => "服务器对这部分的改动将被丢弃",
            Strategy::TheirsFull => "你对这部分的本地改动将被丢弃（不可恢复）",
            Strategy::Working => "按文件当前内容标记为已解决",
        }
    }
}

/// 冲突面板。
///
/// 左：冲突文件列表。右：当前文件的三路内容之一。
pub struct ConflictPanel {
    files: Vec<std::path::PathBuf>,
    state: ListState,
    side: Side,
    /// 当前文件的三路内容（切换文件时重新加载）。
    content: String,
    scroll: u16,
}

impl ConflictPanel {
    pub fn new(files: Vec<std::path::PathBuf>) -> Self {
        let mut state = ListState::default();
        if !files.is_empty() {
            state.select(Some(0));
        }
        Self {
            files,
            state,
            side: Side::Working,
            content: String::new(),
            scroll: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 相对于工作副本根的显示名。
    fn display(path: &std::path::Path, root: &std::path::Path) -> String {
        path.strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string())
    }

    pub fn move_down(&mut self) -> bool {
        let i = match self.state.selected() {
            Some(i) if i + 1 < self.files.len() => i + 1,
            Some(i) => i,
            None if !self.files.is_empty() => 0,
            None => return false,
        };
        self.state.select(Some(i));
        true
    }

    pub fn move_up(&mut self) -> bool {
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            Some(i) => i,
            None if !self.files.is_empty() => 0,
            None => return false,
        };
        self.state.select(Some(i));
        true
    }

    pub fn cycle_side(&mut self) {
        self.side = self.side.next();
        self.scroll = 0;
    }

    pub fn set_side(&mut self, s: Side) {
        self.side = s;
        self.scroll = 0;
    }

    pub fn set_content(&mut self, c: String) {
        self.content = c;
        self.scroll = 0;
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n);
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn selected(&self) -> Option<&std::path::PathBuf> {
        self.state.selected().and_then(|i| self.files.get(i))
    }

    pub fn side(&self) -> Side {
        self.side
    }

    fn centered(area: Rect) -> Rect {
        let w = (area.width * 85 / 100).max(40).min(area.width);
        let h = (area.height * 85 / 100).max(10).min(area.height);
        Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, root: &std::path::Path) {
        let pop = Self::centered(area);
        f.render_widget(Clear, pop);

        let n = self.files.len();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                format!(" 解决冲突（{} 个） ", n),
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::BOLD),
            )))
            .border_style(Style::default().fg(Color::LightRed));

        let inner = block.inner(pop);
        f.render_widget(block, pop);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
            .split(inner);

        // ── 左：文件列表 ──
        let items: Vec<ListItem> = self
            .files
            .iter()
            .map(|p| {
                ListItem::new(Line::from(Span::raw(format!(
                    " {}",
                    Self::display(p, root)
                ))))
            })
            .collect();
        f.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" 冲突文件 "))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            cols[0],
            &mut self.state,
        );

        // ── 右：三路内容 ──
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
            .split(cols[1]);

        // 三个页签：当前的高亮
        let tabs = ["1 我的", "2 服务器的", "3 当前文件"];
        let active = match self.side {
            Side::Mine => 0,
            Side::Theirs => 1,
            Side::Working => 2,
        };
        let spans: Vec<Span> = tabs
            .iter()
            .enumerate()
            .flat_map(|(i, t)| {
                let style = if i == active {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme::Theme::dim()
                };
                [
                    Span::styled(format!(" {t} "), style),
                    Span::raw(" "),
                ]
            })
            .collect();
        f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);

        let body: Vec<Line> = if self.content.is_empty() {
            vec![Line::from(Span::styled(
                "（无内容）",
                theme::Theme::dim(),
            ))]
        } else {
            self.content.lines().take(2000).map(|l| colorize(l)).collect()
        };
        f.render_widget(
            Paragraph::new(body).scroll((self.scroll, 0)),
            rows[1],
        );

        let keys = Line::from(vec![
            Span::styled(" j/k ", theme::Theme::dim()),
            Span::styled("选文件  ", theme::Theme::dim()),
            Span::styled("1/2/3 ", theme::Theme::dim()),
            Span::styled("切版本  ", theme::Theme::dim()),
            Span::styled("m ", Style::default().fg(Color::Yellow)),
            Span::styled("用我的  ", theme::Theme::dim()),
            Span::styled("t ", Style::default().fg(Color::Yellow)),
            Span::styled("用服务器的  ", theme::Theme::dim()),
            Span::styled("Enter ", Style::default().fg(Color::Yellow)),
            Span::styled("保留当前  ", theme::Theme::dim()),
            Span::styled("Esc ", theme::Theme::dim()),
            Span::styled("返回", theme::Theme::dim()),
        ]);
        f.render_widget(Paragraph::new(keys).alignment(Alignment::Left), rows[2]);
    }
}

/// 冲突标记着色。
fn colorize(l: &str) -> Line<'static> {
    let style = if l.starts_with("<<<<<<<") {
        Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)
    } else if l.starts_with("=======") {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if l.starts_with(">>>>>>>") {
        Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)
    } else {
        theme::Theme::status_bar()
    };
    Line::from(Span::styled(l.to_string(), style))
}
