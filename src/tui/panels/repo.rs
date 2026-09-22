//! 远端仓库浏览器（Repo 面板）。
//!
//! ## 为什么不是树
//!
//! 本地文件树（`tree.rs`）可以一次遍历完再按需展开，远端不行 ——
//! 每进一层都是一次**联网** `svn list`。做成可展开的树，意味着：
//! - 每个节点都要记"未加载 / 加载中 / 已加载 / 失败"四态
//! - 展开父目录要并发拉 N 个子目录，失败时状态机很难推回一致
//!
//! 所以这里用**面包屑式单层浏览**（ranger 的一列）：
//! 只有"当前这一层"，进目录/退目录各一次请求。
//! 代价是看不到兄弟目录的内容，收益是状态机只有一份，不会错乱。
//!
//! ## 关于 URL 编码
//!
//! `svn list --xml` 返回的 `<name>` 是**已解码**的（中文直接是 UTF-8），
//! 但拼进 URL 必须重新 percent-encode —— 否则 `A8升级包` 这种名字
//! 拼出来的 URL svn 不认。见 [`encode`]。

use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
              ScrollbarOrientation},
    Frame,
};

use crate::svn::parser::DirEntry;
use crate::tui::theme;

/// 缓存的目录层数上限。
///
/// 按"最坏一层"估算：一个 `DirEntry` 是 3 个 String（name / author / date）
/// 加 3 个整数，堆上约 150~200 字节。`svn list` 单层几千条很常见，
/// 取 5000 条 × 200B ≈ 1MB/层。32 层就是 32MB 上界，可接受；
/// 再往上涨对"退回去不用重新联网"这个收益已经没有边际提升了 ——
/// 用户真正会来回退的通常只有最近几层。
const MAX_CACHED_DIRS: usize = 32;

/// 单层展示的条目上限。
///
/// 既是内存保护，也是**渲染保护**：ratatui 每帧要把整个列表画成 `ListItem`，
/// 几万条时每帧都要构造几万个 Span，界面会卡到没法用。
/// 真有这种量级的目录也不该在 TUI 里翻 —— 截断后提示用户用更精确的入口。
const MAX_ENTRIES_SHOWN: usize = 5000;

/// 面板想让 App 做的事。
///
/// 面板自己**不持有** `Svn`，发请求、写剪贴板、开检出框都得 App 代劳 ——
/// 保持和 `log` / `preview` 面板一致的分工：面板只管状态和渲染。
pub enum RepoAction {
    /// 什么都不做（纯光标移动之类）。
    None,
    /// 去拉这个 URL 的目录列表。
    Load(String),
    /// 用这个 URL 打开检出面板。
    Checkout(String),
    /// 把这段文本写进剪贴板并提示。
    Copy(String),
    /// 关掉面板回主视图。
    Close,
}

pub struct RepoPanel {
    /// 仓库根 URL（`svn info` 的 repository root）。
    ///
    /// 用它而不是"工作副本的 URL"，是因为要能**往上去**：
    /// 从 `/trunk/A8_Patch/xxx` 退到 `/trunk` 再退到根。
    /// 工作副本的 url 只能往下走，退到头就出不去了。
    root_url: String,
    /// 面包屑：每层的**已编码** URL 片段。空 = 在根。
    trail: Vec<String>,
    /// 每层返回时光标要恢复到哪（下标和 `trail` 平行）。
    cursors: Vec<usize>,
    entries: Vec<DirEntry>,
    state: ListState,
    /// url -> 条目。退回上级时直接用，不重复联网。
    ///
    /// 大仓库里"进错目录又退回来"很常见，不缓存的话每次都等一次往返。
    ///
    /// ⚠️ 必须设上限：一个 `DirEntry` 含 3 个 String，大仓库里单层轻松几千条，
    ///    一层就几百 KB。不封顶地逛下去（尤其 A8_Patch 这种版本目录特别多的树）
    ///    内存会一路涨到几百 MB。见 [`MAX_CACHED_DIRS`]。
    cache: HashMap<String, Vec<DirEntry>>,
    /// 缓存插入顺序（FIFO 淘汰用）。
    ///
    /// `HashMap` 自身无序，没法知道"哪个最老"。额外记一个顺序表是最省的做法：
    /// 淘汰时从头部取，命中时**不**上移 —— 真 LRU 要 O(n) 挪动，
    /// 而这里 n 最多 [`MAX_CACHED_DIRS`]，FIFO 的命中率差不了多少。
    cache_order: Vec<String>,
    loading: bool,
    error: Option<(String, String)>,
    /// 导航（进目录 / 退上级）前的 trail + cursors 快照。
    ///
    /// 远端 list 是**异步**的，而原来的顺序是"先改 trail 再发请求"：
    /// 请求失败时 trail 已经变了，面板就停在一个进不去的目录里 ——
    /// 按 R 重试的还是那个 URL，按 Esc 位置全丢。
    /// 在按子树授权的仓库里往上退必然撞 forbidden，等于必踩。
    ///
    /// 所以改 trail 之前先存一份，失败时原路退回。
    rollback: Option<(Vec<String>, Vec<usize>)>,
    /// 正在导航的目标 URL（失败提示里要指名道姓，不然不知道哪层进不去）。
    rollback_url: Option<String>,
    /// 一行提示（如"无法进入 X，已返回上一层"）。
    notice: Option<String>,
}

impl RepoPanel {
    pub fn new(root_url: String) -> Self {
        let mut s = Self {
            root_url: root_url.trim_end_matches('/').to_string(),
            trail: Vec::new(),
            cursors: Vec::new(),
            entries: Vec::new(),
            state: ListState::default(),
            cache: HashMap::new(),
            cache_order: Vec::new(),
            loading: true,
            error: None,
            rollback: None,
            rollback_url: None,
            notice: None,
        };
        if !s.entries.is_empty() {
            s.state.select(Some(0));
        }
        s
    }

    // ------------------------------------------------------------ 路径

    /// 从仓库根 + 起始 URL 构造面板。
    ///
    /// 为什么不直接把"工作副本 URL"当 root？
    /// 因为 `current_url()` 是 `root + trail.join("/")`，
    /// 而退上级靠的是 `trail.pop()`。root 设成工作副本 URL 时 trail 恒为空，
    /// **一步都退不上去** —— 而"往上退几层看兄弟目录"正是这个面板的用途。
    ///
    /// 所以 root 放仓库根，把"从根到工作副本的相对路径"塞进 trail：
    /// `current_url()` 算出来仍然是工作副本 URL（有权限、打得开），
    /// 但 trail 非空，← 就能一层层退回去。
    pub fn new_at(root_url: String, start_url: String) -> Self {
        let root = root_url.trim_end_matches('/').to_string();
        let start = start_url.trim_end_matches('/').to_string();
        let mut s = Self::new(root.clone());
        // start 不是 root 的子路径（比如 info.url / repository_root 对不上）
        // 就退化成"从根起步"，不做猜测。
        if start.len() > root.len() && start.starts_with(&root) {
            let rest = &start[root.len()..];
            let trail: Vec<String> = rest
                .split('/')
                .filter(|x| !x.is_empty())
                .map(encode_seg)
                .collect();
            if !trail.is_empty() {
                s.trail = trail;
                // cursors 和 trail 平行：每层一个光标位置，全从 0 起步
                s.cursors = vec![0; s.trail.len()];
            }
        }
        s
    }

    /// 当前目录的完整 URL。
    pub fn current_url(&self) -> String {
        if self.trail.is_empty() {
            self.root_url.clone()
        } else {
            format!("{}/{}", self.root_url, self.trail.join("/"))
        }
    }

    /// 面包屑的可读形式（给用户看，已解码）。
    ///
    /// `trail` 里存的是编码片段，直接显示会是 `%E5%8D%87...` 这种鬼画符。
    fn breadcrumb(&self) -> String {
        // 解码只为了显示，不做严格校验：解不出来就原样显示，
        // 显示层不该因为一个怪字符就整个崩掉。
        let segs: Vec<String> = self.trail.iter().map(|s| decode(s)).collect();
        if segs.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", segs.join("/"))
        }
    }

    /// 选中项。
    pub fn selected(&self) -> Option<&DirEntry> {
        self.state.selected().and_then(|i| self.entries.get(i))
    }

    /// 选中项的完整 URL（用于复制 / 检出）。
    pub fn selected_url(&self) -> Option<String> {
        let e = self.selected()?;
        let base = self.current_url();
        Some(format!("{}/{}", base, encode(&e.name)))
    }

    // ------------------------------------------------------------ 状态

    /// 塞回一次 `svn list` 的结果。
    ///
    /// `url` 要一起传，用来判断"这结果还属不属于当前目录" ——
    /// 用户可能已经退到上级了，迟到的结果不能覆盖新目录。
    pub fn set_entries(&mut self, url: &str, entries: Vec<DirEntry>) {
        self.put_cache(url, entries.clone());
        // 导航成功：回滚点作废（restore_from_cache 靠它区分"要不要退"）
        self.rollback = None;
        self.rollback_url = None;
        if url != self.current_url() {
            return;
        }
        self.loading = false;
        self.error = None;
        self.entries = entries;
        // 目录排前面（和 svn / 文件管理器一致），同类型内按名字。
        //
        // 用 `sort_by_cached_key` 而不是 `sort_by` + `to_lowercase()`：
        // 后者每次比较都现算两个小写 String，5000 条要比较约 6 万次、
        // 也就是 12 万次堆分配，肉眼可见地卡。`_cached_key` 每个元素只算一次。
        self.entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
        // 超长目录截断：见 [`MAX_ENTRIES_SHOWN`]。
        if self.entries.len() > MAX_ENTRIES_SHOWN {
            self.entries.truncate(MAX_ENTRIES_SHOWN);
        }
        // cursors 和 trail 平行：cursors[i] 是"在 trail[i] 这一层时光标在哪"。
        // 进入新目录 push(0)，退回上级 pop 后 last() 自然就是上级原来的位置 ——
        // 不需要在进/退时手动记"离开时的光标"，少一个状态就少一处错。
        let cur = self.cursors.last().copied().unwrap_or(0);
        let sel = if self.entries.is_empty() {
            None
        } else {
            Some(cur.min(self.entries.len() - 1))
        };
        self.state.select(sel);
    }

    /// 写缓存，超过 [`MAX_CACHED_DIRS`] 时淘汰最老的一层。
    fn put_cache(&mut self, url: &str, entries: Vec<DirEntry>) {
        // 已在缓存里就只更新内容，不动顺序（FIFO 语义下位置没意义）。
        if self.cache.contains_key(url) {
            self.cache.insert(url.to_string(), entries);
            return;
        }
        self.cache.insert(url.to_string(), entries);
        self.cache_order.push(url.to_string());
        while self.cache_order.len() > MAX_CACHED_DIRS {
            // 淘汰最老的。顺序表里可能有已被 remove 的 url，遇到就跳过。
            let old = self.cache_order.remove(0);
            self.cache.remove(&old);
        }
    }

    /// 丢掉某一层的缓存（刷新用），顺序表同步清理。
    fn drop_cache(&mut self, url: &str) {
        self.cache.remove(url);
        self.cache_order.retain(|u| u != url);
    }

    /// 记录一次失败。返回 `Some(url)` 表示"已自动退回上一层，请去拉这个 URL"。
    ///
    /// 只有**导航**（进目录 / 退上级）失败才回滚 —— 那时面板里还有上一层的内容，
    /// 退回去比占屏报错有用得多。首屏和刷新失败没有回滚点，只能占整屏，
    /// 因为那时确实没有别的内容可显示。
    pub fn set_error(&mut self, summary: String, detail: String) -> Option<String> {
        self.loading = false;

        if self.rollback.is_some() {
            let target = self.rollback_url.take().unwrap_or_default();
            let where_ = short_target(&target);
            match self.restore_from_cache() {
                None => {
                    // 缓存里就有上一层，立刻恢复，不用再联网
                    self.notice = Some(format!("无法进入「{}」，已返回上一层", where_));
                    return None;
                }
                Some(url) => {
                    // 缓存里没有，得回去重拉（起点层一定在缓存里，
                    // 所以这条主要出现在"连着退了好几层"的情况）
                    self.notice = Some(format!("无法进入「{}」，正在返回上一层…", where_));
                    return Some(url);
                }
            }
        }

        self.error = Some((summary, detail));
        None
    }

    /// 清掉提示（下次导航开始时调用）。
    fn clear_notice(&mut self) {
        self.notice = None;
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    // ------------------------------------------------------------ 交互

    fn move_by(&mut self, d: isize) {
        if self.entries.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        let n = self.entries.len() as isize;
        let next = (cur + d).clamp(0, n - 1) as usize;
        self.state.select(Some(next));
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> RepoAction {
        use crossterm::event::KeyCode;

        // 加载中只允许退出：这时候按 Enter 会拿到空列表，
        // 体验上是"按了没反应"，不如直接忽略。
        if self.loading && !matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            return RepoAction::None;
        }

        // 失败态：只留重试和退出，别的键没意义
        if self.error.is_some() {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('q') => RepoAction::Close,
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    self.error = None;
                    self.loading = true;
                    RepoAction::Load(self.current_url())
                }
                _ => RepoAction::None,
            };
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => RepoAction::Close,

            KeyCode::Down | KeyCode::Char('j') => {
                self.move_by(1);
                RepoAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_by(-1);
                RepoAction::None
            }
            KeyCode::Home => {
                if !self.entries.is_empty() {
                    self.state.select(Some(0));
                }
                RepoAction::None
            }
            KeyCode::End => {
                if !self.entries.is_empty() {
                    self.state.select(Some(self.entries.len() - 1));
                }
                RepoAction::None
            }

            // 进入目录
            //
            // 先把名字拷出来：selected() 借着 &self，
            // 后面要改 self.cursors / self.trail（可变借用），
            // 两个借用重叠会被借用检查器挡下（E0502）。
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let picked = self
                    .selected()
                    .filter(|e| e.is_dir)
                    .map(|e| e.name.clone());
                match picked {
                    Some(name) => match self.begin_nav(Some(&name)) {
                        Some(url) => RepoAction::Load(url),
                        None => RepoAction::None,
                    },
                    // 文件不"进入"：远端文件没有下一层
                    _ => RepoAction::None,
                }
            }

            // 返回上级。
            //
            // 已经退到仓库根（trail 空）时 begin_nav 返回 None，不动作 ——
            // 别弹错误，根目录就是根目录，不是失败。
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                match self.begin_nav(None) {
                    Some(url) => RepoAction::Load(url),
                    None => RepoAction::None,
                }
            }

            KeyCode::Char('r') | KeyCode::Char('R') => {
                // 刷新 = 丢掉这一层的缓存重拉
                let url = self.current_url();
                self.drop_cache(&url);
                self.loading = true;
                self.entries.clear();
                RepoAction::Load(url)
            }

            // 复制当前目录 / 选中项
            KeyCode::Char('y') => match self.selected_url() {
                Some(u) => RepoAction::Copy(u),
                None => RepoAction::None,
            },
            KeyCode::Char('Y') => RepoAction::Copy(self.current_url()),

            // 检出选中目录
            KeyCode::Char('o') | KeyCode::Char('O') => match self.selected() {
                Some(e) if e.is_dir => match self.selected_url() {
                    Some(u) => RepoAction::Checkout(u),
                    None => RepoAction::None,
                },
                _ => RepoAction::None,
            },

            _ => RepoAction::None,
        }
    }

    /// 开始一次导航：进子目录（`Some(name)`）或退上级（`None`）。
    ///
    /// 返回 `Some(url)` = 需要联网去拉；`None` = 无需动作
    ///（命中缓存已直接回填，或已经退到根没得退）。
    ///
    /// ⚠️ 顺序很关键：**先存回滚点，再改 trail**。
    /// 远端 list 是异步的，失败时结果回来得比用户按键晚，
    /// 不存快照就没法把面板退回"用户按键之前"的位置。
    fn begin_nav(&mut self, push: Option<&str>) -> Option<String> {
        // 上一次的提示到此为止（否则"已返回上一层"会一直挂着）
        self.clear_notice();
        // 快照必须在改 trail 之前
        self.rollback = Some((self.trail.clone(), self.cursors.clone()));

        match push {
            Some(name) => {
                // 先把当前层光标写回 cursors，再为新层 push(0)。
                // cursors[i] 是"在 trail[i] 这一层时光标在哪"，
                // 退回上级 pop 后 last() 自然就是上级原来的位置。
                let cur = self.state.selected().unwrap_or(0);
                if let Some(last) = self.cursors.last_mut() {
                    *last = cur;
                }
                self.cursors.push(0);
                self.trail.push(encode(name));
            }
            None => {
                if self.trail.pop().is_none() {
                    // 已经在根，没得退。回滚点作废，别让后续失败误触发回滚。
                    self.rollback = None;
                    return None;
                }
                self.cursors.pop();
            }
        }

        let url = self.current_url();
        self.rollback_url = Some(url.clone());
        self.loading = true;
        self.entries.clear();
        self.state.select(None);

        // 命中缓存就不联网：set_entries 会把 loading 置回 false
        if let Some(cached) = self.cache.get(&url).cloned() {
            self.set_entries(&url, cached);
            return None;
        }
        Some(url)
    }

    /// 导航失败时把面板退回按键之前的位置。
    ///
    /// 返回 `Some(url)` = 回滚后的那一层需要重新拉取（缓存里没有）；
    /// `None` = 已就地恢复，或这本来就不是导航失败（首屏 / 刷新）。
    fn restore_from_cache(&mut self) -> Option<String> {
        let (trail, cursors) = self.rollback.take()?;
        self.trail = trail;
        self.cursors = cursors;
        let url = self.current_url();
        if let Some(cached) = self.cache.get(&url).cloned() {
            self.set_entries(&url, cached);
            return None;
        }
        self.loading = true;
        Some(url)
    }

    // ------------------------------------------------------------ 渲染

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(area);

        // 面包屑：顶上一行，宽度不够时截中间保住末尾（末段最有用）
        let crumb = self.breadcrumb();
        let max_w = rows[0].width.saturating_sub(10) as usize;
        let crumb_show = if crumb.chars().count() > max_w {
            let cs: Vec<char> = crumb.chars().collect();
            format!("…{}", cs[cs.len().saturating_sub(max_w)..].iter().collect::<String>())
        } else {
            crumb
        };
        let mut crumb_spans = vec![
            Span::styled(" 位置 ", theme::Theme::props()),
            Span::styled(crumb_show, theme::Theme::text()),
        ];
        // 提示跟在面包屑后面（"无法进入 X，已返回上一层"），
        // 单独占一行会把列表挤掉一行，而这类提示看过一次就够了。
        if let Some(n) = &self.notice {
            crumb_spans.push(Span::styled(
                format!("   {}", n),
                Style::default().fg(Color::LightYellow),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(crumb_spans)), rows[0]);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(rows[1]);

        self.render_list(f, cols[0]);
        self.render_detail(f, cols[1]);

        // 底行按键提示。面板能按的键有 9 个，不在屏幕上写出来
        // 就只能靠猜（或翻帮助），而这个面板本来就是为了省事才开的。
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Enter 进入 ", theme::Theme::props()),
                Span::styled("← 返回 ", theme::Theme::props()),
                Span::styled("o 检出此目录 ", theme::Theme::props()),
                Span::styled("Y 复制路径 ", theme::Theme::props()),
                Span::styled("Shift+Y 复制本目录 ", theme::Theme::props()),
                Span::styled("R 刷新 ", theme::Theme::props()),
                Span::styled("Esc 返回主界面", theme::Theme::props()),
            ])),
            rows[2],
        );
    }

    fn render_list(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|e| {
                let (mark, style) = if e.is_dir {
                    ("▸ ", Style::default().fg(Color::LightBlue))
                } else {
                    ("  ", theme::Theme::text())
                };
                let name = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };

                let mut spans = vec![
                    Span::styled(mark, theme::Theme::dim()),
                    Span::styled(name, style),
                ];

                // 目录不显示 size（svn 也不给），改成显示"最后改动"
                if let Some(r) = e.revision {
                    spans.push(Span::styled(
                        format!("  r{}", r),
                        theme::Theme::dim(),
                    ));
                }
                if let Some(d) = &e.date {
                    spans.push(Span::styled(format!("  {}", d), theme::Theme::dim()));
                }
                if let Some(a) = &e.author {
                    spans.push(Span::styled(format!("  {}", a), theme::Theme::props()));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let title = if self.loading {
            " 仓库（读取中…） ".to_string()
        } else {
            format!(" 仓库 {} 项 ", self.entries.len())
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(theme::Theme::border_active()),
            )
            .highlight_style(theme::Theme::selected());

        f.render_stateful_widget(list, area, &mut self.state);

        let visible_h = area.height.saturating_sub(2) as usize;
        if self.entries.len() > visible_h {
            let mut sb = ratatui::widgets::ScrollbarState::new(self.entries.len())
                .position(self.state.selected().unwrap_or(0));
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area,
                &mut sb,
            );
        }
    }

    fn render_detail(&mut self, f: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        if self.loading {
            lines.push(Line::from(Span::styled(
                " 正在读取目录… ",
                theme::Theme::title(),
            )));
        } else if let Some((summary, detail)) = &self.error {
            lines.push(Line::from(Span::styled(
                format!(" {}", summary),
                Style::default().fg(Color::LightRed),
            )));
            lines.push(Line::from(""));
            for l in detail.lines() {
                lines.push(Line::from(Span::styled(
                    format!(" {}", l),
                    theme::Theme::dim(),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " R 重试    Esc 返回",
                theme::Theme::title(),
            )));
        } else if let Some(e) = self.selected() {
            let url = self.selected_url().unwrap_or_default();

            let mut push = |k: &str, v: String| {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {:<6}", k), theme::Theme::dim()),
                    Span::styled(v, theme::Theme::text()),
                ]));
            };

            push("类型", if e.is_dir { "目录".to_string() } else { "文件".to_string() });
            if let Some(s) = e.size {
                push("大小", human_size(s));
            }
            if let Some(r) = e.revision {
                push("版本", format!("r{}", r));
            }
            if let Some(a) = &e.author {
                push("作者", a.clone());
            }
            if let Some(d) = &e.date {
                push("日期", d.clone());
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(" URL", theme::Theme::dim())));
            // URL 一行放不下，按宽度硬折
            for chunk in wrap(&url, area.width.saturating_sub(4) as usize) {
                lines.push(Line::from(Span::styled(
                    format!(" {}", chunk),
                    theme::Theme::props(),
                )));
            }
        } else {
            lines.push(Line::from(Span::styled(
                " 空目录",
                theme::Theme::dim(),
            )));
        }

        let p = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 详情 ")
                .border_style(theme::Theme::border()),
        );
        f.render_widget(p, area);
    }
}

// ---------------------------------------------------------------- 工具

/// 编码一个路径片段，**已编码过的原样返回**。
///
/// `svn info` 输出的 URL 里，中文通常已经是 percent-encoded
/// （`A8%E5%8D%87%E7%BA%A7%E5%8C%85`）。再 encode 一次会把 `%` 变成 `%25`，
/// 拼出来的 URL svn 认不出来 —— 双重编码是这类拼 URL 的代码最容易踩的坑，
/// 而且症状（404 / 路径不存在）看不出是编码问题。
///
/// 所以含 `%` 就认为已编码，不动它。
fn encode_seg(s: &str) -> String {
    if s.contains('%') {
        s.to_string()
    } else {
        encode(s)
    }
}

/// 把 URL 缩成"最后一段"用于提示。
///
/// 直接塞整条 URL 会把面包屑那一行撑爆，用户真正关心的就是最后一层名字。
/// 顺带做一次解码：`A8%E5%8D%87...` 这种原样显示等于没说。
fn short_target(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if last.is_empty() {
        return url.to_string();
    }
    decode(last)
}

/// 把名字编码成能拼进 URL 的形式。
///
/// 只保留 unreserved（`A-Za-z0-9-._~`）原样，其余按 UTF-8 字节 `%XX`。
/// 中文名（`A8升级包`）走字节编码，正是 svn 服务器期待的形式。
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// 解码，只为显示。解不出就原样返回（显示层不该因此崩）。
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'0' - 39),
        b'A'..=b'F' => Some(b - b'0' - 7),
        _ => None,
    }
}

fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "K", "M", "G", "T"];
    if n < 1024 {
        return format!("{} {}", n, U[0]);
    }
    let mut v = n as f64;
    let mut i = 0usize;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, U[i])
}

/// 按字符数硬折行。URL 没有空格，Paragraph 的自动折行在这里不生效。
fn wrap(s: &str, w: usize) -> Vec<String> {
    if w == 0 {
        return vec![s.to_string()];
    }
    let cs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let end = (i + w).min(cs.len());
        out.push(cs[i..end].iter().collect());
        i = end;
    }
    out
}
