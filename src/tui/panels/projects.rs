//! 项目切换面板（`Ctrl+P`）。
//!
//! 列出这台机器上已知的工作副本，选中直接切过去 ——
//! 换项目不用再"另开终端、cd、再敲一次"。
//!
//! # 为什么必须在打开时是瞬时的
//!
//! 这个面板的存在理由是**省时间**。它一旦要跑 `svn status` 或联网，
//! 就退化成"另一个要等的东西"，用户宁可继续开新终端。
//! 所以这里展示的一切都来自内存和几个 `stat`：
//! 项目清单由 `projects::list` 从配置和缓存目录读出，
//! 守护进程状态只是看一眼 pid 文件在不在。

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::theme;

/// 面板想让外层做什么。
pub enum ProjectsAction {
    /// 只重绘。
    None,
    /// 切到这个工作副本。
    Open(PathBuf),
    /// 关闭面板。
    Close,
    /// 打开检出界面（选中了"检出新工作副本"）。
    Checkout,
}

/// 列表里的一行。`None` 是末尾那条"检出新工作副本"。
type Row = Option<usize>;

pub struct ProjectsPanel {
    all: Vec<crate::projects::Project>,
    /// 当前所在的仓库。按 `Ctrl+P` 直接高亮它 ——
    /// 最常见的用法是"从这一个切到旁边那个"，每次都从头找太烦。
    current: Option<PathBuf>,
    /// 过滤后的行。
    rows: Vec<Row>,
    state: ListState,
    query: String,
    searching: bool,
    notice: Option<String>,
}

impl ProjectsPanel {
    pub fn new(all: Vec<crate::projects::Project>, current: Option<PathBuf>) -> Self {
        let mut p = Self {
            all,
            current,
            rows: Vec::new(),
            state: ListState::default(),
            query: String::new(),
            searching: false,
            notice: None,
        };
        p.refilter();
        // 一打开就落在当前项目上，而不是第一行。
        if let Some(c) = &p.current {
            if let Some(i) = p.all.iter().position(|x| &x.path == c) {
                if let Some(pos) = p.rows.iter().position(|r| *r == Some(i)) {
                    p.state.select(Some(pos));
                }
            }
        }
        p
    }

    /// 这是当前所在的仓库吗。
    fn is_current(&self, p: &crate::projects::Project) -> bool {
        self.current.as_deref() == Some(p.path.as_path())
    }

    pub fn set_notice(&mut self, msg: String) {
        self.notice = Some(msg);
    }

    /// 重新按当前过滤条件算可见行，并保住选中位置。
    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        let hits: Vec<usize> = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                q.is_empty()
                    || p.name.to_lowercase().contains(&q)
                    || p.path.display().to_string().to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();

        // 过滤时把"检出"这一项放在最后：它是兜底动作，不该混在项目里。
        let mut rows: Vec<Row> = hits.into_iter().map(Some).collect();
        rows.push(None);

        let old = self.rows.get(self.state.selected().unwrap_or(0)).copied().flatten();
        self.rows = rows;
        let next = match old {
            Some(idx) => self.rows.iter().position(|r| *r == Some(idx)),
            None => None,
        };
        // 原来那条 project 在新列表里没了（比如被过滤掉）就落到第一行，
        // 列表整个空了就什么都不选。
        let sel = next.or(if self.rows.is_empty() { None } else { Some(0) });
        self.state.select(sel);
    }

    /// 当前选中的项目（选中的是"检出"那条则为 `None`）。
    pub fn selected(&self) -> Option<&crate::projects::Project> {
        let i = self.state.selected()?;
        self.rows.get(i).copied().flatten().map(|k| &self.all[k])
    }

    /// 正在输入过滤词吗。App 要靠它判断 `a` / `d` 是命令还是输入字符。
    pub fn searching(&self) -> bool {
        self.searching
    }

    /// 换一份项目清单（登记/移除之后）。保留过滤词和当前项标记。
    pub fn refresh(&mut self, all: Vec<crate::projects::Project>) {
        self.all = all;
        self.refilter();
    }

    fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        let n = self.rows.len() as isize;
        let next = (cur + delta).clamp(0, n - 1) as usize;
        self.state.select(Some(next));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ProjectsAction {
        // 搜索中：除了回车/退出，其余都当输入。
        if self.searching {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.searching = false;
                    if key.code == KeyCode::Esc {
                        self.query.clear();
                        self.refilter();
                    }
                    return ProjectsAction::None;
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.refilter();
                    return ProjectsAction::None;
                }
                KeyCode::Char(c) => {
                    self.query.push(c);
                    self.refilter();
                    return ProjectsAction::None;
                }
                _ => return ProjectsAction::None,
            }
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ProjectsAction::Close,
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_by(1);
                ProjectsAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_by(-1);
                ProjectsAction::None
            }
            KeyCode::Char('/') => {
                self.searching = true;
                ProjectsAction::None
            }
            KeyCode::Enter => match self.selected() {
                Some(p) => ProjectsAction::Open(p.path.clone()),
                // 最后一条不是项目，是"检出新副本"。
                None => ProjectsAction::Checkout,
            },
            _ => ProjectsAction::None,
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let n = self.all.len();
        let block = theme::block().borders(Borders::ALL).title(theme::focus_title(
            format!(" 项目 · {n} 个 "),
            true,
        ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(inner);

        if n == 0 {
            self.render_empty(f, rows[0]);
        } else {
            self.render_list(f, rows[0]);
        }
        self.render_hints(f, rows[1]);
    }

    /// 空状态必须说清"下一步干什么" ——
    /// 只画一个空列表，用户不知道是自己还没打开过仓库，还是工具坏了。
    fn render_empty(&self, f: &mut Frame, area: Rect) {
        f.render_widget(
            theme::empty_state(
                "还没有已知的工作副本",
                &[
                    "打开过的仓库会自动记进这个列表",
                    "也可以在这里按 a 登记当前目录",
                    "或者选下面那条检出新副本",
                ],
            ),
            area,
        );
    }

    fn render_list(&mut self, f: &mut Frame, area: Rect) {
        let width = area.width as usize;
        let items: Vec<ListItem> = self
            .rows
            .iter()
            .map(|row| match row {
                None => ListItem::new(Line::from(Span::styled(
                    "  + 检出新工作副本",
                    theme::Theme::dim(),
                ))),
                Some(i) => {
                    let p = &self.all[*i];
                    let cur = self.is_current(p);
                    ListItem::new(Self::row_line(p, width, cur))
                }
            })
            .collect();

        let list = List::new(items)
            .highlight_style(theme::Theme::selected())
            .block(Block::default().borders(Borders::NONE));
        f.render_stateful_widget(list, area, &mut self.state);
    }

    /// 一行：`▸ 名称   路径   守护   状态`
    fn row_line(p: &crate::projects::Project, width: usize, current: bool) -> Line<'static> {
        let name_w = 16usize;
        // 右边留给守护标记（2）和状态文字（最多 8）。
        let tail_w = 14usize;
        let path_w = width.saturating_sub(name_w + tail_w + 3).max(8);

        let name = theme::fit(&p.name, name_w);
        let path = theme::fit(&crate::projects::short_path(&p.path), path_w);

        let mut spans = vec![
            Span::styled(format!("  {name:<name_w$}"), theme::Theme::body()),
            Span::styled(format!(" {path:<path_w$}"), theme::Theme::dim()),
        ];

        // 守护在跑说明这个仓库有人在用、且状态是新的。
        if crate::daemon::is_running(&p.path) {
            spans.push(Span::styled("● ", theme::Theme::success()));
        } else {
            spans.push(Span::styled("  ", theme::Theme::dim()));
        }

        // "我现在在哪儿"是这个面板最该先回答的问题 ——
        // 切错项目的代价是白跑一次，而这个标记是零成本的。
        if current {
            spans.push(Span::styled("当前", theme::Theme::info()));
        } else {
            match p.state {
                crate::projects::State::Ready => {}
                crate::projects::State::NotWorkingCopy => {
                    spans.push(Span::styled("不是副本", theme::Theme::warning()));
                }
                crate::projects::State::Missing => {
                    spans.push(Span::styled("已不在", theme::Theme::danger()));
                }
            }
        }
        Line::from(spans)
    }

    /// 只在**有事要说**时占这一行。
    ///
    /// 键位提示底栏已经有了，再抄一遍只是白占一行高度 ——
    /// 这一行留给"已登记 X""当前副本不能移除"这类即时反馈。
    /// 只在**有事要说**时占这一行。
    ///
    /// 键位提示底栏已经有了，再抄一遍只是白占一行高度 ——
    /// 这一行留给"已登记 X""当前副本不能移除"这类即时反馈。
    fn render_hints(&self, f: &mut Frame, area: Rect) {
        let line = if self.searching {
            format!(" 搜索: {}_", self.query)
        } else if let Some(n) = &self.notice {
            format!(" {n}")
        } else {
            return;
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                theme::fit(&line, area.width as usize),
                theme::Theme::dim(),
            ))),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{Project, Source, State};

    fn proj(name: &str, path: &str) -> Project {
        Project {
            path: PathBuf::from(path),
            name: name.to_string(),
            source: Source::Registered,
            state: State::Ready,
        }
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, crossterm::event::KeyModifiers::NONE)
    }

    /// 打开时要落在**当前所在**的项目上，而不是第一行 ——
    /// 最常见的用法是"从这一个切到旁边那个"，每次从头找太烦。
    #[test]
    fn opens_on_the_project_you_are_in() {
        let p = ProjectsPanel::new(
            vec![proj("a", "/a"), proj("b", "/b"), proj("c", "/c")],
            Some(PathBuf::from("/b")),
        );
        assert_eq!(p.selected().map(|x| x.name.as_str()), Some("b"));
    }

    /// 过滤按名字或路径都行。
    #[test]
    fn filter_matches_name_or_path() {
        let mut p = ProjectsPanel::new(
            vec![proj("alpha", "/x/alpha"), proj("beta", "/y/beta")],
            None,
        );
        p.query = "beta".into();
        p.refilter();
        assert_eq!(p.selected().map(|x| x.name.as_str()), Some("beta"));

        p.query = "/x/".into();
        p.refilter();
        assert_eq!(p.selected().map(|x| x.name.as_str()), Some("alpha"));
    }

    /// "检出新工作副本"永远在最后：它是兜底动作，不该混在项目里被误选。
    #[test]
    fn checkout_row_is_always_last() {
        let mut p = ProjectsPanel::new(vec![proj("a", "/a")], None);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows.last(), Some(&None));

        // 移到最后一行按 Enter，要的是检出而不是切换
        p.move_by(1);
        assert!(p.selected().is_none(), "最后一行不是项目");
        assert!(matches!(p.handle_key(key(KeyCode::Enter)), ProjectsAction::Checkout));
    }

    #[test]
    fn enter_opens_selected_and_esc_closes() {
        let mut p = ProjectsPanel::new(vec![proj("a", "/a")], None);
        assert!(matches!(p.handle_key(key(KeyCode::Esc)), ProjectsAction::Close));
        match p.handle_key(key(KeyCode::Enter)) {
            ProjectsAction::Open(path) => assert_eq!(path, PathBuf::from("/a")),
            _ => panic!("Enter 应该打开选中的项目"),
        }
    }

    /// 过滤到没有命中时，剩下的只有"检出"那条 ——
    /// 不能出现空列表（选不中任何东西会表现为"按了没反应"）。
    #[test]
    fn filtering_everything_out_still_leaves_the_checkout_row() {
        let mut p = ProjectsPanel::new(vec![proj("a", "/a")], None);
        p.query = "zzz".into();
        p.refilter();
        assert_eq!(p.rows, vec![None]);
        assert!(p.state.selected().is_some(), "必须还有得选");
    }
}
