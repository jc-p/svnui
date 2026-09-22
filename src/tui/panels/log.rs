//! 提交历史。
//!
//! `l` 打开。列表里 Enter 看该版本的改动文件（`-v` 才拿得到，
//! 所以延迟到选中时再取，避免第一次打开就跑全量 verbose）。

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

use crate::domain::LogEntry;
use crate::tui::theme;

pub struct LogPanel {
    /// 完整列表（不过滤）。
    entries: Vec<LogEntry>,
    /// 过滤后可见的**下标**。搜索时用它，避免 clone 整个 Vec。
    visible: Vec<usize>,
    /// 本地过滤词（小写）。设置它会**真的过滤** visible。
    filter: Option<String>,
    /// 服务端搜索词。只用于标题显示，**不过滤**。
    ///
    /// 用 `svn log --search` 时结果已经是服务器筛过的，
    /// 再本地过滤一遍属于重复劳动，还可能因为大小写差异误杀。
    /// 但标题得让用户知道"当前看的是搜索结果"，所以单独记一个。
    server_search: Option<String>,
    state: ListState,
    /// 选中版本的改动文件（懒加载）。
    detail: Option<Vec<String>>,
    /// 当前在看的修订号，避免重复请求。
    detail_rev: Option<u64>,
}


impl LogPanel {
    pub fn new(entries: Vec<LogEntry>) -> Self {
        let mut state = ListState::default();
        if !entries.is_empty() {
            state.select(Some(0));
        }
        let visible = (0..entries.len()).collect();
        Self {
            entries,
            visible,
            filter: None,
            server_search: None,
            state,
            detail: None,
            detail_rev: None,
        }
    }

    /// 设置搜索词（None 或空串 = 不过滤）。
    ///
    /// 搜 message / 作者 / 版本号 —— 找某个提交时最常用这三个。
    pub fn set_filter(&mut self, f: Option<String>) {
        self.filter = f.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
        // 两者互斥：本地一过滤，服务端结果就不再是"当前视图"了
        if self.filter.is_some() {
            self.server_search = None;
        }
        self.rebuild_visible();
    }

    /// 标记"这批结果是服务端搜 `kw` 得来的"。
    /// 只影响标题，不过滤。
    pub fn set_server_search(&mut self, kw: Option<String>) {
        self.server_search = kw.filter(|s| !s.trim().is_empty());
        self.filter = None;
        self.rebuild_visible();
    }

    /// 追加更多历史（"加载更多"）。
    ///
    /// 去重按 revision：svn 可能在两次查询之间产生了新提交，
    /// 导致翻页时同一个版本号出现两次。
    pub fn extend(&mut self, more: Vec<LogEntry>) {
        let have: std::collections::HashSet<u64> =
            self.entries.iter().map(|e| e.revision).collect();
        for e in more {
            if !have.contains(&e.revision) {
                self.entries.push(e);
            }
        }
        self.rebuild_visible();
    }

    pub fn filter(&self) -> Option<&String> {
        self.filter.as_ref()
    }

    fn rebuild_visible(&mut self) {
        self.visible = match &self.filter {
            None => (0..self.entries.len()).collect(),
            Some(f) => self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.msg.to_lowercase().contains(f)
                        || e.author.to_lowercase().contains(f)
                        || e.revision.to_string().contains(f)
                })
                .map(|(i, _)| i)
                .collect(),
        };
        if self.visible.is_empty() {
            self.state.select(None);
        } else {
            let cur = self.state.selected().unwrap_or(0).min(self.visible.len() - 1);
            self.state.select(Some(cur));
        }
    }

    /// 当前已加载多少条。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn selected_rev(&self) -> Option<u64> {
        self.state
            .selected()
            .and_then(|i| self.visible.get(i))
            .and_then(|&i| self.entries.get(i))
            .map(|e| e.revision)
    }

    pub fn need_detail(&self) -> Option<u64> {
        let rev = self.selected_rev()?;
        if self.detail_rev == Some(rev) {
            None
        } else {
            Some(rev)
        }
    }

    pub fn set_detail(&mut self, rev: u64, lines: Vec<String>) {
        self.detail_rev = Some(rev);
        self.detail = Some(lines);
    }

    /// 选中项变了：清掉上一版的改动文件。
    ///
    /// 不清的话右栏会短暂显示**上一个版本**的改动文件，
    /// 配上新版本的标题 —— 数据张冠李戴，比白屏更难发现问题。
    fn on_selection_changed(&mut self) {
        self.detail = None;
        self.detail_rev = None;
    }

    pub fn move_up(&mut self) {
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            Some(_) => return,
            None if !self.visible.is_empty() => 0,
            None => return,
        };
        self.state.select(Some(i));
        self.on_selection_changed();
    }

    pub fn move_down(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i + 1 < self.visible.len() => i + 1,
            Some(_) => return,
            None => 0,
        };
        self.state.select(Some(i));
        self.on_selection_changed();
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        // 左栏固定 28 列：内容就是 "2026-05-12 pengjiecheng"，定宽，
        // 用百分比的话终端一宽就多余、一窄又挤。
        //   - 10 日期 + 1 空格 + 12 作者 = 23
        //   - +2 边框 + 1 滚动条 + 2 余量 = 28
        //
        // 作者给 12 是因为常见账号名就是这么长（`pengjiecheng`），
        // 之前 8 会截成 `pengjie...` —— 同一批人就分不清谁是谁了。
        //
        // 终端太窄时退回百分比：固定 28 列会把右栏挤没了，
        // 宁可左栏跟着缩。
        let left_w = if area.width >= 78 { 28 } else { (area.width / 3).max(16) };
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_w), Constraint::Min(20)])
            .split(area);

        // ── 左：提交列表
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| {
                // 只显示 日期 + 提交人。
                //
                // message 已移到右上块完整显示 —— 列表里塞它也放不下几个字，
                // 而右栏按上下分块后有整块空间，多行都能看完。
                // 这里只剩两列定宽内容，所以左栏可以用**固定宽度**（见下）。
                ListItem::new(Line::from(vec![
                    // 日期用白色，不能用 dim()：
                    // 高亮行是 bg(DarkGray)，dim 的前景色也是 DarkGray，
                    // 深灰叠深灰 = 选中行上日期直接消失。
                    Span::styled(
                        e.date.get(..10).unwrap_or(&e.date).to_string(),
                        Style::default().fg(Color::White),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:<12}", truncate(&e.author, 12)),
                        theme::Theme::modified(),
                    ),
                ]))
            })
            .collect();

        let title = match (&self.filter, &self.server_search) {
            (Some(f), _) => format!(" 提交历史（搜索：{}，{} 条）", f, self.visible.len()),
            (None, Some(kw)) => {
                format!(" 提交历史（搜索：{}，{} 条）", kw, self.entries.len())
            }
            (None, None) => format!(" 提交历史（{} 条）", self.entries.len()),
        };
        let list_block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_bottom(
                Line::from(" j/k 移动 | R 回退到此版本 | / 搜索全部历史 | N 加载更早 | Esc 返回 ")
                    .alignment(Alignment::Right),
            )
            .border_style(theme::Theme::border_active());

        let list = List::new(items)
            .block(list_block)
            .highlight_style(theme::Theme::selected());

        f.render_stateful_widget(list, cols[0], &mut self.state);

        // 滚动条：不加的话长列表根本看不出有多少条
        if self.visible.len() as u16 > cols[0].height.saturating_sub(2) {
            let mut sb = ScrollbarState::new(self.visible.len())
                .position(self.state.selected().unwrap_or(0));
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                cols[0],
                &mut sb,
            );
        }

        // ── 右：选中版本的详情
        // 搜不到时明确提示，别让人以为界面坏了
        if self.visible.is_empty() {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" 提交历史（搜索：{}）", self.filter.as_deref().unwrap_or("")))
                .border_style(theme::Theme::border_active());
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " 没有匹配的提交。按 / 换个关键词",
                    theme::Theme::dim(),
                )))
                .block(block),
                cols[0],
            );
            return;
        }

        // ── 右栏内容：本地信息立即渲染，联网部分作为补充
        //
        // 之前右栏只有"改动文件"，必须等 `svn log -r N -v` 回来才有内容，
        // 移动光标时一路白屏 —— 大仓库上就是"卡"。
        // 现在改成：author / 日期 / 完整 message 全部来自**本地已有数据**，
        // 选中即出，零延迟；改动文件异步加载完再追加到下面。
        let sel = self
            .state
            .selected()
            .and_then(|i| self.visible.get(i))
            .and_then(|&i| self.entries.get(i));

        let mut detail: Vec<Line> = Vec::new();
        if let Some(e) = sel {
            detail.push(Line::from(Span::styled(
                format!(" {}    {}", e.date.get(..16).unwrap_or(&e.date), e.author),
                theme::Theme::dim(),
            )));
            detail.push(Line::from(""));
            // 完整 message（多行都显示），不是只有首行
            for l in e.msg.lines() {
                detail.push(Line::from(Span::raw(format!(" {}", l))));
            }
            detail.push(Line::from(""));
            match &self.detail {
                Some(lines) if !lines.is_empty() => {
                    detail.push(Line::from(Span::styled(
                        format!(" 改动文件（{}）", lines.len()),
                        theme::Theme::props(),
                    )));
                    for l in lines {
                        detail.push(colorize_path(l));
                    }
                }
                Some(_) => {
                    detail.push(Line::from(Span::styled(
                        " 该版本没有文件改动记录",
                        theme::Theme::dim(),
                    )));
                }
                None => {
                    detail.push(Line::from(Span::styled(
                        " 正在读取改动文件…",
                        theme::Theme::dim(),
                    )));
                }
            }
        } else {
            detail.push(Line::from(Span::styled(" 读取中…", theme::Theme::dim())));
        }

        // 版本号移到标题：左列不显示了，但看详情时要能看到是哪个版本
        let title = match sel {
            Some(e) => format!(
                " r{} · {} ",
                e.revision,
                truncate(&e.author, 12)
            ),
            None => " 详情 ".to_string(),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(theme::Theme::border());

        // 详情也加滚动条
        let inner_h = cols[1].height.saturating_sub(2) as usize;
        // 先算长度：Text::from 会 move 掉 detail，之后就借不到了
        let detail_len = detail.len();
        let mut sb_state = ScrollbarState::new(detail_len.saturating_sub(inner_h));

        f.render_widget(
            Paragraph::new(ratatui::text::Text::from(detail)).block(block),
            cols[1],
        );
        if detail_len > inner_h {
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                cols[1],
                &mut sb_state,
            );
        }
    }
}

/// `-v` 输出形如 `   M /trunk/src/a.txt`，按动作字母着色。
fn colorize_path(l: &str) -> Line<'static> {
    let t = l.trim_start();
    let style = match t.chars().next() {
        Some('A') => theme::Theme::added(),
        Some('D') => theme::Theme::deleted(),
        Some('M') => theme::Theme::modified(),
        Some('R') => theme::Theme::structural(),
        _ => theme::Theme::dim(),
    };
    Line::from(Span::styled(l.to_string(), style))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>())
    }
}
