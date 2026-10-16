//! 应用状态机。
//!
//! ## 布局
//!
//! ```text
//! ┌────────────────────────────────────────────┐
//! │ SVN trunk@r4521   C1 M3 A2 ?1   ⟳ 扫描…    │  顶栏
//! ├──────────────────┬─────────────────────────┤
//! │ ▾ src/           │ b.txt                   │
//! │   M main.rs      │ -let old = 1;           │
//! │   ? new.rs       │ +let new = 2;           │  左40% / 右60%
//! │ ▾ docs/          │                         │
//! │   M guide.md     │                         │
//! ├──────────────────┴─────────────────────────┤
//! │ ↑↓ 选择  Enter 看改动  C 提交  / 搜索  ? 帮助 │
//! └────────────────────────────────────────────┘
//! ```
//!
//! 左栏是文件树（状态标记直接画在文件名上），右栏跟随光标实时预览 diff。
//!
//! ## 为什么所有 svn 调用都走后台线程
//!
//! 早期版本把 `self.svn.update(...)` 直接写在事件循环里。大仓库上
//! `svn update` 要几十秒，这期间**事件循环完全不转** —— 界面冻住、
//! 按键无响应，看起来就是"卡死"。

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::svn::{StatusOpts, Svn};
use crate::tui::panels::checkout::CheckoutPanel;
use crate::tui::panels::loading;
use crate::tui::panels::commit::CommitPanel;
use crate::tui::panels::commit_gen::Change;
use crate::tui::panels::preview::{read_file, BarHit, Kind as PreviewKind, PreviewPanel};
use crate::tui::panels::help::HelpPanel;
use crate::tui::panels::log::LogPanel;
use crate::tui::panels::confirm::{ConfirmAction, ConfirmPanel, Danger};
use crate::tui::panels::conflict::{ConflictItem, ConflictPanel, Side, Strategy};
use crate::tui::panels::tree::{TreeMode, TreePanel};
use crate::tui::panels::repo::{RepoAction, RepoPanel};
use crate::tui::theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// 主视图（左树 + 右预览）。
    Browse,
    /// 提交面板。
    Commit,
    /// diff 全屏。
    Diff,
    /// 提交历史。
    Log,
    /// 二次确认（revert / update 预警）。
    Confirm,
    /// 冲突解决。
    Conflict,
    /// 检出新工作副本（不在工作副本里启动时进这个）。
    Checkout,
    /// 远端仓库浏览（`svn list --xml`，可逐层下钻）。
    Repo,
}

/// 把路径压成相对工作副本根的显示名；不在 root 下就原样返回。
///
/// 用于通知和确认面板 —— 绝对路径太长，占满一行还看不出重点。
fn short_rel(p: &str, root: &std::path::Path) -> String {
    pretty_path(p, root, 80)
}

/// 变更列表的排序方式（`S` 键循环）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortMode {
    /// 按状态严重度：冲突 → 缺失 → 删除 → 替换 → 新增 → 修改 → 未版本化。
    Status,
    /// 按相对路径字典序。
    Path,
    /// 按文件名（忽略目录）。
    Name,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            SortMode::Status => SortMode::Path,
            SortMode::Path => SortMode::Name,
            SortMode::Name => SortMode::Status,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortMode::Status => "状态",
            SortMode::Path => "路径",
            SortMode::Name => "文件名",
        }
    }
}

/// 搜索输入用在哪。
/// 正在拖拽哪个面板的哪根滚动条。
///
/// 要区分 preview / diff 两块面板：全屏 diff 打开时两块同时存在，
/// 但只有 diff 是可见的，拖错一块等于"拖了看不见的东西"。
#[derive(Debug, Clone, Copy, PartialEq)]
enum DragBar {
    Preview(BarHit),
    Diff(BarHit),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SearchFor {
    /// 过滤文件树
    Tree,
    /// 在提交历史里搜
    Log,
}

enum BgDone {
    Preview(u64, String, PreviewPanel),
    /// (变更条目, 工作副本根)
    Status(bool, crate::domain::Result<(Vec<crate::tui::panels::Item>, PathBuf, String)>),
    Msg(crate::domain::Result<String>),
    /// 日志加载/搜索完成：(结果, 搜索词, 是否追加到现有列表)。
    ///
    /// 带搜索词是因为：搜索结果回来时要把它设进面板的 filter，
    /// 让标题显示"搜索：xxx，N 条"。不记住的话无法区分
    /// "用户清空了搜索"还是"结果属于旧搜索"。
    LogLoad(
        crate::domain::Result<Vec<crate::domain::LogEntry>>,
        Option<String>,
        bool,
    ),
    LogDetail(u64, Vec<String>),
    /// 检出的**中间进度**：(已落盘条目数, 最后一项)。
    ///
    /// 只在检出过程中发，由调用方节流（约 10 次/秒）。
    /// 大仓库几万行，不节流会把 channel 打满。
    CheckoutProgress(usize, String),
    /// 检出完成，带结果消息。
    Checkout(crate::domain::Result<String>),
    /// 提交信息候选项（模型候选在前，规则候选在后）。
    ///
    /// 规则候选一起带回来是为了兜底：模型没配、超时、返回怪东西时，
    /// 按 Ctrl+G 还能轮换到规则给的版本，不会变成"按了没反应"。
    CommitSug(Vec<String>),
    /// 本地状态检查：(用户选择的范围, 固定的操作路径, 最新状态)。
    RevertCheck(
        String,
        Vec<PathBuf>,
        crate::domain::Result<Vec<crate::domain::StatusEntry>>,
    ),
    /// 回退干跑：(目标版本, 当前 HEAD, 将被撤销的提交摘要, 受影响文件)。
    ///
    /// 带"被撤销的提交"是因为：光看文件列表不知道自己回到了哪，
    /// 看到"这几个提交会被撤销"才敢按 Y —— 这是回退确认里最关键的一屏。
    RevDry(u64, u64, Vec<String>, Vec<String>),
    /// update 前的远端检查：(远端有更新的文件, 本地改动的相对路径)。
    OutdatedCheck(crate::domain::Result<(Vec<(PathBuf, u64)>, Vec<String>)>),
    /// 远端目录列表：(请求的 URL, 结果)。
    ///
    /// 带 URL 是因为结果可能**迟到** —— 用户已经退到上级了，
    /// 这时候拿旧 URL 的结果覆盖当前目录，界面会显示错层的内容。
    RepoList(String, crate::domain::Result<Vec<crate::svn::parser::DirEntry>>),
    /// 冲突文件列表加载完成。
    Conflicts(crate::domain::Result<Vec<ConflictItem>>),
    /// 某个历史版本的完整 diff。
    RevDiff(u64, crate::domain::Result<String>),
    /// 冲突三路内容加载完成。
    ConflictContent(crate::domain::Result<String>),
}

pub struct App {
    svn: Svn,
    mode: Mode,

    tree: TreePanel,
    commit: Option<CommitPanel>,
    diff: Option<PreviewPanel>,
    /// 右栏内嵌预览。
    preview: Option<PreviewPanel>,
    /// 最近一帧是否展示内嵌预览，避免缩窗后命中旧滚动条坐标。
    preview_visible: bool,
    logpanel: Option<LogPanel>,
    checkout: Option<CheckoutPanel>,
    confirm: Option<ConfirmPanel>,
    conflict: Option<ConflictPanel>,
    /// 远端仓库浏览面板（`V` 打开）。
    repo: Option<RepoPanel>,
    /// 提交范围（打开提交面板时锁定，避免弹框期间勾选变化导致不一致）。
    commit_paths: Vec<PathBuf>,
    /// 是否有一个 svn commit 在后台跑。
    ///
    /// `spawn_busy` 的成功/失败回调是所有操作共用的（Msg(Ok) / Msg(Err)），
    /// 光看回调分不清"这次成功的是提交还是 revert"，
    /// 而只有提交成功才该清掉草稿和勾选 —— 所以要单独记一笔。
    commit_inflight: bool,
    /// 提交信息草稿。
    ///
    /// 退出提交框时把内容存在这里，下次按 C 打开**自动恢复**。
    /// 这是"不弹二次确认框"的前提 —— 不存档就必须问一句"确定放弃吗"，
    /// 而那个框会让"我只想退出去改点东西"变成出不来。
    ///
    /// 提交成功才清空；提交失败时保留，方便改完直接重试。
    commit_draft: Option<String>,
    /// 日志读取失败的原因（人话摘要 + 建议）。
    ///
    /// 单独存而不只弹 notice：
    /// notice 只有一行且在底部容易被忽略，而"连不上服务器"这种错误
    /// 用户需要看到**下一步该干什么**，值得占一整个面板。
    log_error: Option<(String, String)>,
    show_help: bool,
    /// 鼠标滚轮是否接管（对应 crossterm 的 EnableMouseCapture）。
    mouse_on: bool,
    /// 待生效的鼠标捕获开关。
    ///
    /// App 拿不到终端 backend，改不了捕获状态，所以只记一笔"想改成什么"，
    /// 由事件循环取走后真正 enable/disable —— 和 `editor_req` 一个套路。
    mouse_pending: Option<bool>,
    /// 正在拖拽的滚动条（按下左键命中后记住，拖动时跟随，松开清空）。
    drag: Option<DragBar>,

    /// 搜索状态。
    searching: Option<SearchFor>,
    query: String,

    preview_gen: u64,
    tx: Sender<BgDone>,
    rx: Receiver<BgDone>,
    busy: Option<String>,
    refresh_watch: Option<super::refresh::RefreshWatch>,
    watch_error: Option<String>,
    last_refresh: Option<Instant>,
    scan_failed: bool,
    /// busy 的起始时刻。所有设 `busy` 的地方都只赋字符串，计时统一在这里补 ——
    /// 否则要改十几个调用点，漏一处就是"这个操作的时间不走"。
    busy_start: Option<Instant>,
    /// 检出的实时进度：(已落盘条目数, 最后一项)。只有 checkout 会填。
    busy_prog: Option<(usize, String)>,

    notice: Option<String>,
    title: String,
    sort: SortMode,
    /// 待打开的外部编辑器请求（路径）。由 `mod.rs` 拿到后让出终端。
    editor_req: Option<(String, PathBuf)>,
    /// 工作副本根，建树要用。
    root: PathBuf,
    /// 扁平的变更列表（提交面板用）。
    items: Vec<crate::tui::panels::Item>,
    should_quit: bool,
}

impl App {
    pub fn new(svn: Svn) -> crate::domain::Result<Self> {
        let (tx, rx) = channel();
        // ⚠️ 必须在构造 Self 之前取：svn 马上要 move 进去，之后借不到了
        let self_trust = svn.trust_cert;

        // ⚠️ 必须在构造 Self 之前取：下面 `svn` 会 move 进去，之后就借不到了
        let start_cwd = svn.cwd.clone();
        let timeout = svn.timeout();

        let mut app = Self {
            svn,
            mode: Mode::Browse,
            tree: TreePanel::new(),
            commit: None,
            diff: None,
            preview: None,
            preview_visible: false,
            logpanel: None,
            checkout: None,
            confirm: None,
            conflict: None,
            repo: None,
            commit_paths: Vec::new(),
            commit_draft: None,
            commit_inflight: false,
            log_error: None,
            show_help: false,
            // 默认关闭：开启后终端不再处理鼠标，选中文本复制会失效，
            // 而且鼠标移动会持续产生事件、把键盘事件挤到队列后面（表现为"敲键没反应"）。
            // 需要拖拽滚动条时按 M 打开。
            mouse_on: crate::config::Config::load().ui.mouse,
            mouse_pending: None,
            drag: None,
            searching: None,
            query: String::new(),
            preview_gen: 0,
            tx,
            rx,
            busy: None,
            refresh_watch: None,
            watch_error: None,
            last_refresh: None,
            scan_failed: false,
            busy_start: None,
            busy_prog: None,
            notice: None,
            sort: SortMode::Status,
            editor_req: None,
            title: "SVN".to_string(),
            root: PathBuf::new(),
            items: Vec::new(),
            should_quit: false,
        };

        // 传进来的 svn 可能是 discover_bare 得到的（root 不是真工作副本根）。
        // 这里再试一次真正的 discover，失败就进检出界面。
        // 继承 trust_cert：cli 已经处理过环境变量/命令行开关，
        // 这里不能写死 false，否则 TUI 会丢失信任配置。
        let real = crate::svn::Svn::discover(&start_cwd, timeout, self_trust);

        match real {
            Ok(real_svn) => {
                app.svn = real_svn;
                // 先监听再初次扫描，避免启动扫描期间丢失外部修改。
                match super::refresh::RefreshWatch::new(app.svn.root.as_path()) {
                    Ok(watch) => app.refresh_watch = Some(watch),
                    Err(e) => app.watch_error = Some(e.to_string()),
                }
                // 首帧必须有内容，所以第一次同步做。
                app.reload_sync()?;
                app.title = Self::fetch_title(&app.svn);
                if app.tree.selected().is_some() {
                    app.spawn_preview();
                }
            }
            Err(_) => {
                app.checkout = Some(CheckoutPanel::new(
                    start_cwd.to_string_lossy().to_string(),
                ));
                app.mode = Mode::Checkout;
            }
        }
        Ok(app)
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    fn fetch_title(svn: &Svn) -> String {
        let mut t = "SVN".to_string();
        if let Ok(info) = svn.info() {
            if let Some(rel) = info.relative_url.as_deref() {
                t.push_str(&format!(" {}", rel.trim_start_matches("^/")));
            }
            t.push_str(&format!("@r{}", info.revision));
        }
        t
    }

    // ── 扫描 ──────────────────────────────────────────────

    fn sort_items(items: &mut [crate::tui::panels::Item], mode: SortMode) {
        match mode {
            SortMode::Status => items.sort_by_key(|i| match i.sign {
                'C' | 'T' => 0,
                '!' | 'D' => 1,
                'R' => 2,
                'A' => 3,
                'M' => 4,
                '?' => 5,
                _ => 6,
            }),
            SortMode::Path => items.sort_by(|a, b| a.rel.cmp(&b.rel)),
            SortMode::Name => items.sort_by(|a, b| {
                let na = a.rel.rsplit('/').next().unwrap_or(&a.rel);
                let nb = b.rel.rsplit('/').next().unwrap_or(&b.rel);
                na.cmp(nb)
            }),
        }
    }

    fn fetch(svn: &Svn, sort: SortMode) -> crate::domain::Result<(Vec<crate::tui::panels::Item>, PathBuf)> {
        let snap = svn.snapshot(&StatusOpts::default())?;
        let root = PathBuf::from(snap.root.as_str());
        // 浏览列表也要展示未版本化文件，用户才能按 A 加入 SVN；
        // 能否勾选提交仍由 Item / TreePanel 的提交资格规则控制。
        let mut items: Vec<crate::tui::panels::Item> = snap
            .entries
            .values()
            .filter(|e| e.is_changed() || e.text == crate::domain::StatusKind::Unversioned)
            .map(|e| crate::tui::panels::Item::from_entry(e, &root))
            .collect();
        Self::sort_items(&mut items, sort);
        Ok((items, root))
    }

    fn rebuild_tree_keep_mode(&mut self) {
        self.tree.refresh_from_items(&self.items, &self.root);
    }

    fn reload_sync(&mut self) -> crate::domain::Result<()> {
        let (items, root) = Self::fetch(&self.svn, self.sort)?;
        self.items = items;
        self.root = root;
        self.rebuild_tree_keep_mode();
        // 状态变了，之前勾的可能已经不能提交（比如刚 revert 掉）。
        // 不清理会留下"UI 上看不见、集合里还在"的幽灵勾选。
        self.tree.prune_checked();
        self.last_refresh = Some(Instant::now());
        self.scan_failed = false;
        Ok(())
    }

    fn spawn_reload(&mut self) {
        self.spawn_status(false);
    }

    fn spawn_status(&mut self, automatic: bool) {
        // 扫描与写操作共享 busy；避免旧扫描完成后清掉另一个任务的 busy。
        if self.busy.is_some() {
            return;
        }
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        let sort = self.sort;
        self.busy = Some(if automatic { "自动扫描" } else { "扫描" }.into());
        std::thread::spawn(move || {
            let result = Self::fetch(&svn, sort).map(|(items, root)| {
                // info 同样可能超时；不能在 UI 线程更新标题时阻塞按键。
                (items, root, Self::fetch_title(&svn))
            });
            let _ = tx.send(BgDone::Status(automatic, result));
        });
    }

    // ── diff 预览 ─────────────────────────────────────────

    pub fn set_notice(&mut self, msg: impl Into<String>) {
        self.notice = Some(msg.into());
    }

    /// 外部编辑器返回后调用：重新拉一次预览。
    ///
    /// 文件可能已经被改了，缓存的 diff / 内容就过期了。
    pub fn refresh_preview_after_edit(&mut self) {
        self.spawn_preview();
        // 顺带让树上的状态标记也更新（新改动要显示出来）。
        // 这里用同步版：编辑器刚退出，界面还没重绘，用户能容忍一次短阻塞；
        // 用 spawn_reload 的话状态会晚一拍才出现。
        if let Err(e) = self.reload_sync() {
            self.notice = Some(format!("重新扫描失败：{}", e));
        }
    }

    /// 取出"请用外部编辑器打开这个文件"的请求。
    ///
    /// ⚠️ 为什么不在 App 里直接 spawn 编辑器：TUI 占着 raw mode 和
    ///    alternate screen，vim 在里面会拿不到正常的终端行为（方向键、
    ///    退格、Ctrl-C 全乱）。必须由 `mod.rs` 先让出终端再启动。
    ///    所以这里只是"声明"，真正的让渡在事件循环里做。
    pub fn take_editor_request(&mut self) -> Option<(String, PathBuf)> {
        self.editor_req.take()
    }

    /// 编辑器默认值：`$VISUAL` → `$EDITOR` → `vim` → `vi`。
    ///
    /// 尊重用户自己的配置比硬编码 vim 重要 —— 有人用 nvim / code / emacs。
    pub fn default_editor() -> String {
        // ⚠️ Result 不是 Iterator，不能链式 .filter()。
        //    而且这里要的是"取第一个非空值"：VISUAL 设了但为空串时
        //    应该继续看 EDITOR，而不是直接返回空串。
        for var in ["VISUAL", "EDITOR"] {
            if let Ok(v) = std::env::var(var) {
                if !v.trim().is_empty() {
                    return v;
                }
            }
        }
        "vim".to_string()
    }

    /// 滚动右栏预览。正数向下，负数向上。
    /// 纵向滚动。正数向下，负数向上。
    ///
    /// 全屏模式下滚 diff，否则滚右栏预览 —— **不两个都滚**：
    /// 全屏时背景预览根本看不见，跟着滚没意义，
    /// 退出全屏后位置却变了，反而莫名其妙。
    ///（鼠标滚轮也走这里，所以这一步是"全屏里滚轮能用"的前提。）
    fn scroll_preview(&mut self, delta: i32) {
        if let Some(p) = self.diff.as_mut() {
            if delta > 0 {
                p.scroll_down(delta as u16);
            } else {
                p.scroll_up((-delta) as u16);
            }
            return;
        }
        if let Some(p) = self.preview.as_mut() {
            if delta > 0 {
                p.scroll_down(delta as u16);
            } else {
                p.scroll_up((-delta) as u16);
            }
        }
    }

    /// 横向滚动右栏预览。正数向右，负数向左。
    ///
    /// 同 `scroll_preview`：全屏时只滚 diff，不连带滚背景。
    fn scroll_preview_h(&mut self, delta: i32) {
        if let Some(p) = self.diff.as_mut() {
            if delta > 0 {
                p.scroll_right(delta as u16);
            } else {
                p.scroll_left((-delta) as u16);
            }
            return;
        }
        if let Some(p) = self.preview.as_mut() {
            if delta > 0 {
                p.scroll_right(delta as u16);
            } else {
                p.scroll_left((-delta) as u16);
            }
        }
    }

    /// 横向翻页。dir > 0 向右，< 0 向左。
    ///
    /// 步长取当前预览的 3/4 屏宽（见 `PreviewPanel::page_step`）：
    /// 两块面板宽度通常不同，各自按自己的算才对得上。
    fn page_preview_h(&mut self, dir: i32) {
        if let Some(p) = self.diff.as_mut() {
            let step = p.page_step();
            if dir > 0 {
                p.scroll_right(step);
            } else {
                p.scroll_left(step);
            }
            return;
        }
        if let Some(p) = self.preview.as_mut() {
            let step = p.page_step();
            if dir > 0 {
                p.scroll_right(step);
            } else {
                p.scroll_left(step);
            }
        }
    }

    /// 回到最左（列 0）。
    fn preview_home(&mut self) {
        if let Some(p) = self.preview.as_mut() {
            p.scroll_home();
        }
        if let Some(p) = self.diff.as_mut() {
            p.scroll_home();
        }
    }

    /// 切换鼠标滚轮接管。
    fn toggle_mouse(&mut self) {
        self.mouse_on = !self.mouse_on;
        self.mouse_pending = Some(self.mouse_on);
        self.notice = Some(if self.mouse_on {
            "鼠标：已开启（可拖拽滚动条；要选中文本复制按 M 关闭，或按住 Option 拖选）".into()
        } else {
            "鼠标：已关闭（可以正常选中文本复制了；滚动条拖拽也不可用）".into()
        });
    }

    /// 取走待生效的鼠标开关（由事件循环执行真正的 enable/disable）。
    pub fn take_mouse_toggle(&mut self) -> Option<bool> {
        self.mouse_pending.take()
    }

    pub fn mouse_on(&self) -> bool {
        self.mouse_on
    }

    /// 是否有后台任务在跑。事件循环用它把轮询间隔调密（动画更顺）。
    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    /// 鼠标事件处理：**只做滚动条拖拽，不接管滚轮**。
    ///
    /// 不做滚轮是刻意的 —— 滚轮一动就滚，很容易在看 diff 时误触把位置冲掉，
    /// 而拖拽是"明确指着滚动条拖"的主动动作，不会误触。
    /// 需要滚轮的话按 M 打开（连带影响见 `toggle_mouse`）。
    ///
    /// 点击选中树节点不做：要维护坐标→节点的命中测试，
    /// 树在滚动/折叠后换算容易错位，收益不抵风险。
    pub fn handle_mouse(&mut self, ev: MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        // 只在这两种模式下响应。
        // 提交框 / 确认框 / 冲突面板里滚预览是错的行为
        // （用户在看的是那块面板，不是背后的预览）。
        if !matches!(self.mode, Mode::Browse | Mode::Diff) {
            // 顺手清掉拖拽：例如拖到一半按 Esc 退了全屏，
            // 残留状态会让之后的鼠标移动继续滚一块已经不可见的面板。
            self.drag = None;
            return;
        }
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag = self.hit_bar(ev.column, ev.row);
            }
            // Drag 是按下后的移动；Moved 是未按键的移动。
            // 两个都接：macOS 上按住左键拖动，iTerm2 报的是 Moved 而不是 Drag。
            // 只在 drag 非 None 时处理，所以 Moved 不会造成误滚。
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved => {
                if let Some(bar) = self.drag {
                    self.drag_to(bar, ev.column, ev.row);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.drag = None,
            _ => {}
        }
    }

    /// 坐标命中了哪块面板的滚动条。
    ///
    /// 全屏 diff 优先：它盖在右栏预览上面，用户看得见的是它。
    fn hit_bar(&self, x: u16, y: u16) -> Option<DragBar> {
        if let Some(p) = self.diff.as_ref() {
            return p.hit_bar(x, y).map(DragBar::Diff);
        }
        if self.preview_visible {
            if let Some(p) = self.preview.as_ref() {
                return p.hit_bar(x, y).map(DragBar::Preview);
            }
        }
        None
    }

    /// 跟着鼠标更新滚动位置。
    fn drag_to(&mut self, bar: DragBar, x: u16, y: u16) {
        let target = match bar {
            DragBar::Diff(_) => self.diff.as_mut(),
            DragBar::Preview(_) => self.preview.as_mut(),
        };
        if let Some(p) = target {
            match bar {
                DragBar::Diff(BarHit::Vertical) | DragBar::Preview(BarHit::Vertical) => {
                    p.drag_v(y)
                }
                DragBar::Diff(BarHit::Horizontal) | DragBar::Preview(BarHit::Horizontal) => {
                    p.drag_h(x)
                }
            }
        }
    }

    fn spawn_preview(&mut self) {
        let Some(node) = self.tree.selected().cloned() else {
            self.preview = None;
            return;
        };

        // 目录：显示它汇总了什么，不做 diff
        if node.is_dir {
            self.preview = Some(PreviewPanel::notice(
                format!("{}/", node.rel),
                format!(
                    "目录 · {} 个子项 · 汇总状态 {} · 按 Enter 展开",
                    node.children.len(),
                    if node.sign == ' ' { "无改动".to_string() } else { node.sign.to_string() }
                ),
            ));
            return;
        }

        self.preview_gen += 1;
        let gen = self.preview_gen;
        let rel = node.rel.clone();
        let abs = node.abs.clone();
        // 标题用美化后的路径：去掉工作副本根前缀、解掉 percent 编码。
        // 绝对路径会占满整个标题栏，看不出重点。
        let name = pretty_path(&node.rel, &self.root, 60);
        let root_str = self.root.to_string_lossy().to_string();

        // 未版本化：没有 BASE 可比，svn diff 必然 E155010。
        // 直接读文件内容 + 顶部提示，别去惊动 svn。
        if node.sign == '?' {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let panel = match read_file(&abs) {
                    Ok(c) => PreviewPanel::file_with_banner(
                        name,
                        &c,
                        "未纳管 · 按 a 添加当前项，或 A 批量添加后才有 diff",
                    ),
                    Err(e) => PreviewPanel::notice(name, format!("读取失败：{}", e)),
                };
                let _ = tx.send(BgDone::Preview(gen, rel, panel));
            });
            return;
        }

        // 干净文件（无状态符号）：直接读内容，别跑 svn diff
        if node.sign == ' ' || node.sign == '\0' {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let panel = match read_file(&abs) {
                    Ok(c) => PreviewPanel::file(name, &c),
                    Err(e) => PreviewPanel::notice(name, format!("读取失败：{}", e)),
                };
                let _ = tx.send(BgDone::Preview(gen, rel, panel));
            });
            return;
        }

        // 有改动：跑 svn diff
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let panel = match svn.diff(&[abs], None, false) {
                Ok(out) => PreviewPanel::diff(name, &shorten_paths(&out.stdout, &root_str)),
                Err(e) => PreviewPanel::notice(name, format!("diff 失败：{}", e)),
            };
            let _ = tx.send(BgDone::Preview(gen, rel, panel));
        });
    }

    pub fn tick(&mut self) {
        if let Some(watch) = self.refresh_watch.as_mut() {
            watch.observe(Instant::now());
        }
        // busy 的计时起点在这里统一补：设 busy 的调用点有十几个，
        // 让它们各自记时间必然会漏。动画的帧号也由这个 elapsed 推出来。
        if self.busy.is_some() {
            if self.busy_start.is_none() {
                self.busy_start = Some(Instant::now());
            }
        } else {
            self.busy_start = None;
            self.busy_prog = None;
        }

        while let Ok(done) = self.rx.try_recv() {
            match done {
                BgDone::Preview(gen, _rel, panel) => {
                    if gen == self.preview_gen {
                        self.preview = Some(panel);
                    }
                }
                BgDone::Status(automatic, Ok((items, root, title))) => {
                    let n = items.len();
                    self.items = items;
                    self.root = root;
                    self.rebuild_tree_keep_mode();
                    // 后台刷新同样要清幽灵勾选：
                    // 比如勾了 M 文件、又在别处 revert 了，状态变了但勾选还在。
                    self.tree.prune_checked();
                    self.busy = None;
                    self.last_refresh = Some(Instant::now());
                    self.scan_failed = false;
                    if !automatic {
                        self.notice = Some(format!("已刷新：{} 项状态（含未纳管）", n));
                    }
                    // 顶栏的 @rNNN 只在启动时读过一次，提交/更新后不重读
                    // 就会一直停在启动那一刻的版本号。
                    self.title = title;
                    self.spawn_preview();
                }
                BgDone::Status(_, Err(e)) => {
                    self.busy = None;
                    self.scan_failed = true;
                    self.notice = Some(format!("扫描失败：{}", e));
                }
                BgDone::Msg(Ok(msg)) => {
                    self.busy = None;
                    // 提交成功才清草稿 + 清勾选。
                    // 失败时**必须都留着**：改一句就能重试，
                    // 清掉的话用户得重新写信息、重新勾一遍文件。
                    if self.commit_inflight {
                        self.commit_inflight = false;
                        self.commit_draft = None;
                        self.commit_paths.clear();
                        self.tree.clear_check();
                    }
                    // 提交完历史里要能立刻看到这一版：svn log 默认从工作副本
                    // revision 往回列，混合 revision 下会漏掉刚提交的条目。
                    if self.mode == Mode::Log {
                        self.spawn_log_load(None, Self::LOG_PAGE, false);
                    }
                    self.notice = Some(msg);
                    self.spawn_reload();
                }
                BgDone::Msg(Err(e)) => {
                    self.busy = None;
                    // 提交失败：草稿和勾选保留，提示里说清楚怎么重试
                    if self.commit_inflight {
                        self.commit_inflight = false;
                        // 勾选取的是"打开提交框那一刻"的快照，失败后要还原回树上，
                        // 否则用户按 C 会发现"我勾的文件都没了"。
                        for p in self.commit_paths.iter() {
                            if let Ok(r) = p.strip_prefix(&self.root) {
                                self.tree.set_checked(&r.to_string_lossy(), true);
                            }
                        }
                        self.notice =
                            Some(format!("提交失败：{}（信息已保留，按 C 重试）", e));
                    } else {
                        self.notice = Some(format!("失败：{}", e));
                    }
                }
                BgDone::LogLoad(Ok(entries), search, append) => {
                    self.busy = None;
                    let n = entries.len();
                    match (&mut self.logpanel, append) {
                        // 追加：加载更多
                        (Some(p), true) => {
                            p.extend(entries);
                        }
                        _ => {
                            self.logpanel = Some(LogPanel::new(entries));
                        }
                    }
                    if let Some(p) = self.logpanel.as_mut() {
                        // 服务端搜过的只设标题、不再本地过滤：
                        // 结果已经是筛过的，重复过滤可能因大小写误杀。
                        if search.is_some() {
                            p.set_server_search(search.clone());
                        }
                    }
                    if n == 0 {
                        self.notice = Some(match &search {
                            Some(kw) => format!("没有找到包含“{}”的提交", kw),
                            None => "没有提交历史".into(),
                        });
                    }
                    self.spawn_log_detail();
                }
                BgDone::LogLoad(Err(e), search, _) => {
                    self.busy = None;
                    // 搜索失败要特别处理：可能是旧 svn 不支持 --search。
                    // 明确告诉用户降级到"只搜已加载的"，而不是笼统说"失败"。
                    if let Some(kw) = search {
                        self.notice = Some(format!("搜索“{}”失败：{}（将只搜索已加载的部分）", kw, e));
                        if let Some(p) = self.logpanel.as_mut() {
                            p.set_filter(Some(kw));
                        }
                    } else {
                        self.logpanel = None;
                        self.log_error = Some(humanize_error(&e));
                        self.notice = Some(format!("读取历史失败：{}", e));
                    }
                }
                BgDone::RevertCheck(scope, paths, result) => {
                    self.busy = None;
                    match result {
                        Err(e) => self.notice = Some(format!("检查撤销范围失败：{}", e)),
                        Ok(entries) => {
                            // 清单来自真实状态；执行路径沿用用户选择，不能从展示文字反推。
                            let items: Vec<String> = entries
                                .iter()
                                .filter_map(|entry| {
                                    let path = self.root.join(entry.path.as_std_path());
                                    if !paths.iter().any(|target| path.starts_with(target)) {
                                        return None;
                                    }
                                    let consequence = match entry.sign() {
                                        'A' if !entry.copied => "取消添加，保留本地文件",
                                        'A' => "取消复制/移动，新增侧可能删除；请先备份",
                                        'D' | '!' => "恢复到工作副本基线版本",
                                        'M' => "丢弃内容及属性改动",
                                        'R' => "撤销替换，新增内容可能删除；请先备份",
                                        'C' | 'T' => "丢弃本地改动并清除冲突状态",
                                        '~' => "尝试还原版本状态；阻碍项可能导致失败",
                                        _ => return None,
                                    };
                                    Some(format!(
                                        "{} {}\n  {}",
                                        if entry.sign() == 'A' && entry.copied { "A+".into() } else { entry.sign().to_string() },
                                        path.strip_prefix(&self.root).unwrap_or(&path).display(),
                                        consequence
                                    ))
                                })
                                .collect();
                            if items.is_empty() {
                                self.notice =
                                    Some("所选范围没有可撤销的版本改动；未纳管文件会保留".into());
                            } else {
                                self.confirm = Some(ConfirmPanel::new(
                                    format!("丢弃本地改动：{} 项", items.len()),
                                    format!("范围：{}\n目录递归撤销，包含执行时目录内的版本改动。\n丢弃的内容无法恢复；不影响仓库历史。", scope),
                                    items,
                                    Danger::Critical,
                                    ConfirmAction::Revert(paths),
                                ));
                                self.mode = Mode::Confirm;
                            }
                        }
                    }
                }

                BgDone::RevDry(rev, head, commits, files) => {
                    self.busy = None;
                    let mut items: Vec<String> = Vec::new();
                    items.push(format!("将被撤销的提交（r{} 之后到 r{}）：", rev, head));
                    for c in &commits {
                        items.push(format!("  {}", c));
                    }
                    if commits.is_empty() {
                        items.push("  （无）".to_string());
                    }
                    items.push(String::new());
                    items.push(format!("受影响文件（{}）：", files.len()));
                    if files.is_empty() {
                        items.push("  （无）".to_string());
                    } else {
                        for f in &files {
                            items.push(format!("  {}", f));
                        }
                    }
                    let n = commits.len();
                    self.confirm = Some(ConfirmPanel::new(
                        format!("确认回到 r{}", rev),
                        format!(
                            "将撤销 r{} 之后的 {} 个提交。执行后工作副本回到 r{} 的状态，\n仍需再提交一次才会写进仓库。本地未提交的改动会与回退结果合并，有冲突会标出来。",
                            rev, n, rev
                        ),
                        items,
                        Danger::Critical,
                        ConfirmAction::RevertToRev(rev),
                    ));
                    self.mode = Mode::Confirm;
                }

                BgDone::OutdatedCheck(Ok((remote, local))) => {
                    self.busy = None;
                    if remote.is_empty() {
                        // 远端没更新，直接 update 不会撞冲突。
                        // tick 返回 ()，这里不能 return Ok(()) —— 用完 continue 跳过剩余逻辑。
                        self.do_update_confirmed();
                        continue;
                    }
                    // 重叠 = 远端有更新 && 本地也改了 → 大概率冲突
                    let local_set: std::collections::HashSet<&str> =
                        local.iter().map(|s| s.as_str()).collect();
                    let overlaps: Vec<String> = remote
                        .iter()
                        .filter(|(p, _)| {
                            p.strip_prefix(&self.root)
                                .ok()
                                .and_then(|r| r.to_str())
                                .map(|r| local_set.contains(r))
                                .unwrap_or(false)
                        })
                        .map(|(p, rev)| {
                            format!(
                                "{}（远端 r{}）",
                                short_rel(&p.to_string_lossy(), &self.root),
                                rev
                            )
                        })
                        .collect();

                    if overlaps.is_empty() {
                        // 远端有更新但和本地改动不重叠，安全
                        self.do_update_confirmed();
                        continue;
                    }

                    let n = remote.len();
                    self.confirm = Some(ConfirmPanel::new(
                        "更新前预警",
                        format!(
                            "远端有 {} 项更新，其中 {} 项你本地也改了，更新后可能产生冲突：",
                            n,
                            overlaps.len()
                        ),
                        overlaps,
                        Danger::Warn,
                        ConfirmAction::Update,
                    ));
                    self.mode = Mode::Confirm;
                }

                BgDone::OutdatedCheck(Err(e)) | BgDone::Conflicts(Err(e)) => {
                    self.busy = None;
                    self.notice = Some(format!("失败：{}", e));
                }

                BgDone::Conflicts(Ok(files)) => {
                    self.busy = None;
                    if files.is_empty() {
                        self.notice = Some("没有冲突".into());
                    } else {
                        let n = files.len();
                        self.conflict = Some(ConflictPanel::new(files));
                        self.mode = Mode::Conflict;
                        self.load_conflict_content();
                        self.notice = Some(format!("{} 处冲突", n));
                    }
                }

                BgDone::RevDiff(rev, res) => {
                    self.busy = None;
                    match res {
                        Ok(text) => {
                            if text.trim().is_empty() {
                                self.notice = Some(format!("r{} 没有内容改动（可能只改了属性）", rev));
                            } else {
                                // 彩色 diff：+ 绿、- 红、@@ 青
                                let lines: Vec<Line> = text
                                    .lines()
                                    .take(5000)
                                    .map(|l| {
                                        let st = if l.starts_with('+') && !l.starts_with("+++") {
                                            Style::default().fg(Color::Green)
                                        } else if l.starts_with('-') && !l.starts_with("---") {
                                            Style::default().fg(Color::Red)
                                        } else if l.starts_with("@@") {
                                            Style::default().fg(Color::Cyan)
                                        } else if l.starts_with("Index:") {
                                            theme::Theme::title()
                                        } else {
                                            theme::Theme::text()
                                        };
                                        Line::from(Span::styled(l.to_string(), st))
                                    })
                                    .collect();
                                let title = format!("r{} 的改动", rev);
                                self.diff = Some(PreviewPanel::with_lines(
                                    PreviewKind::Diff,
                                    title,
                                    lines,
                                    None,
                                ));
                                self.mode = Mode::Diff;
                            }
                        }
                        Err(e) => {
                            let (s, _d) = humanize_error(&e);
                            self.notice = Some(format!("读取 r{} 失败：{}", rev, s));
                        }
                    }
                }

                BgDone::ConflictContent(res) => {
                    if let Some(p) = self.conflict.as_mut() {
                        p.set_content(res.unwrap_or_default());
                    }
                }

                BgDone::CheckoutProgress(n, last) => {
                    // 只更新显示，不动 busy —— 检出还没结束。
                    self.busy_prog = Some((n, last));
                }
                BgDone::Checkout(Ok(msg)) => {
                    self.busy = None;
                    self.notice = Some(format!("{} — 按 Q 退出后 cd 进去", msg.lines().next().unwrap_or("已检出")));
                }
                BgDone::Checkout(Err(e)) => {
                    self.busy = None;
                    self.checkout = Some(CheckoutPanel::new(
                        self.root.to_string_lossy().to_string(),
                    ));
                    self.mode = Mode::Checkout;
                    if let Some(p) = self.checkout.as_mut() {
                        p.set_hint(format!("检出失败：{}", e));
                    }
                }
                BgDone::LogDetail(rev, lines) => {
                    if let Some(p) = self.logpanel.as_mut() {
                        p.set_detail(rev, lines);
                    }
                }
                BgDone::CommitSug(cands) => {
                    self.busy = None;
                    if let Some(panel) = self.commit.as_mut() {
                        if cands.is_empty() {
                            panel.set_notice("没能生成候选".to_string());
                        } else {
                            panel.set_candidates(cands);
                            let msg = panel.next_suggestion();
                            panel.set_notice(msg);
                        }
                    }
                }
                BgDone::RepoList(url, res) => {
                    self.busy = None;
                    if let Some(p) = self.repo.as_mut() {
                        match res {
                            Ok(entries) => p.set_entries(&url, entries),
                            Err(e) => {
                                // 导航失败（进目录 / 退上级）时面板会自己退回上一层，
                                // 并返回"需要重新拉的 URL" —— 这时不能占屏报错：
                                // 用户还在面板里，位置已经退回去了，占屏反而把
                                // 上一层的内容盖掉了。
                                //
                                // 只有首屏 / 刷新失败才占整屏，因为那时确实
                                // 没有别的内容可显示（和 log 面板一个套路）。
                                let retry = p.set_error(
                                    e.to_string(),
                                    crate::cli::hint::human_hint(&e),
                                );
                                if let Some(u) = retry {
                                    self.spawn_repo_list(&u);
                                }
                            }
                        }
                    }
                }
            }
        }
        // 输入提交信息、确认危险操作、搜索时只积累事件，回到空闲主界面再刷新。
        if self.mode == Mode::Browse && self.busy.is_none()
            && self.searching.is_none() && !self.show_help
        {
            if self.refresh_watch.as_mut().is_some_and(|w| w.take_due(Instant::now())) {
                self.spawn_status(true);
            }
        }
    }

    // ── 输入 ──────────────────────────────────────────────

    /// 粘贴整段文本。
    ///
    /// 走 `Event::Paste`（需要终端开启 bracketed paste），
    /// **不**逐字符模拟按键 —— 那样粘贴内容里的 `q` / `Esc` / `/`
    /// 会被当成快捷键触发，界面直接被点掉。
    pub fn handle_paste(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        // 搜索输入中：粘贴进搜索框
        if let Some(target) = self.searching {
            self.query.push_str(&text);
            let q = self.query.clone();
            self.apply_query(&q, target);
            return;
        }
        match self.mode {
            // 提交框：插到光标处。TextArea 自己会处理多行粘贴。
            Mode::Commit => {
                if let Some(p) = self.commit.as_mut() {
                    p.insert_str(&text);
                    // 之前可能挂着"提交信息不能为空"，粘完就过期了
                    p.clear_hint();
                }
            }
            // 检出面板：插到当前字段。
            //
            // 这个必须接 —— 仓库 URL 长且难敲，没人手打，
            // 全靠 Cmd+V。之前落进下面的 `_ => {}` 被静默忽略，
            // 表现为"粘贴没反应"，看着就像输入框是坏的。
            Mode::Checkout => {
                if let Some(p) = self.checkout.as_mut() {
                    p.push_str(&text);
                }
            }
            // 其余模式没有输入框，粘贴没有落点。
            // 静默忽略比报错好 —— 用户只是按错了地方。
            _ => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        if self.show_help {
            self.show_help = false;
            return Ok(());
        }
        if self.searching.is_none() && matches!(key.code, KeyCode::Char('?') | KeyCode::F(1)) {
            self.show_help = true;
            return Ok(());
        }

        // 搜索输入中：只处理 Enter（确认）/ Esc（取消）/ 退格 / 字符
        if let Some(target) = self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = None;
                    self.query.clear();
                    self.tree.set_filter(None);
                    if let Some(p) = self.logpanel.as_mut() {
                        p.set_filter(None);
                    }
                    return Ok(());
                }
                KeyCode::Enter => {
                    self.searching = None;
                    // 提交历史：Enter 确认时再走**服务端**搜索。
                    //
                    // 输入过程中只做本地过滤（免费、即时），
                    // 因为每敲一个字母都跑一次联网搜索会严重卡顿；
                    // 而本地只能搜已加载的 100 条 —— 所以确认时
                    // 必须补一次服务端搜索，否则永远够不到更早的提交。
                    if target == SearchFor::Log && !self.query.trim().is_empty() {
                        let kw = self.query.clone();
                        self.spawn_log_load(Some(kw), Self::LOG_PAGE, false);
                    }
                    return Ok(());
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    let q = self.query.clone();
                    self.apply_query(&q, target);
                    return Ok(());
                }
                KeyCode::Char(c) => {
                    self.query.push(c);
                    let q = self.query.clone();
                    self.apply_query(&q, target);
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }

        match self.mode {
            Mode::Browse => self.handle_browse_key(key),
            Mode::Commit => self.handle_commit_key(key),
            Mode::Diff => self.handle_diff_key(key),
            Mode::Log => self.handle_log_key(key),
            Mode::Confirm => self.handle_confirm_key(key),
            Mode::Conflict => self.handle_conflict_key(key),
            Mode::Checkout => self.handle_checkout_key(key),
            Mode::Repo => self.handle_repo_key(key),
        }
    }

    /// 把查询词应用到对应目标（实时过滤）。
    fn apply_query(&mut self, q: &str, target: SearchFor) {
        let f = if q.is_empty() { None } else { Some(q.to_string()) };
        match target {
            SearchFor::Tree => self.tree.set_filter(f),
            SearchFor::Log => {
                if let Some(p) = self.logpanel.as_mut() {
                    p.set_filter(f);
                }
            }
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        // 后台工作时仍可浏览和退出；写操作及打开其他后台面板等待当前任务结束。
        let navigation = !key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code,
            KeyCode::Char('q' | 'j' | 'k' | 'J' | 'K' | 'M' | '<' | '>' | ',' | '.' | '0' | ' ')
            | KeyCode::Esc | KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right);
        if self.busy.is_some() && !navigation {
            self.notice = Some("后台任务进行中，请稍候再操作".into());
            return Ok(());
        }
        match key.code {
            KeyCode::Esc if self.tree.filter().is_some() => {
                self.tree.set_filter(None);
                self.query.clear();
                self.spawn_preview();
            }
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,

            KeyCode::Char('j') | KeyCode::Down => {
                if self.tree.move_down() {
                    self.spawn_preview();
                }
            }
            KeyCode::Char('k') | KeyCode::Up if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.tree.move_up() {
                    self.spawn_preview();
                }
            }

            // 右栏预览滚动：j/k 被树占用了，所以大写 J/K 给预览。
            // Ctrl+d / Ctrl+u 翻页（和 vim 一致）。
            KeyCode::Char('J') => self.scroll_preview(1),
            KeyCode::Char('K') => self.scroll_preview(-1),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_preview(10)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_preview(-10)
            }

            // 横向滚动：长行超出右边界时用。
            //
            // 只留两套：← → 慢移（2 列，看长行末尾时精调），
            // < > 翻页（约 3/4 屏宽，跨大段时一步到位）。
            //
            // 之前还有 Shift+←/→ 一套，砍掉了 —— 三套按键做同一件事，
            // 而且 Shift+方向键在很多终端里根本投不进来（被终端自己吃掉），
            // 结果就是"按了没反应"，比没有更让人困惑。
            KeyCode::Left => self.scroll_preview_h(-2),
            KeyCode::Right => self.scroll_preview_h(2),
            KeyCode::Char('>') | KeyCode::Char('.') => self.page_preview_h(1),
            KeyCode::Char('<') | KeyCode::Char(',') => self.page_preview_h(-1),
            KeyCode::Char('0') => self.preview_home(),

            // M 切换鼠标滚轮。开了鼠标捕获后**终端的文本选中复制会失效**
            // （所有鼠标事件都被本程序接管），所以要能随时关掉。
            KeyCode::Char('M') => self.toggle_mouse(),

            // S 切换排序
            KeyCode::Char('s') | KeyCode::Char('S') => {
                let next = self.sort.next();
                self.sort = next;
                Self::sort_items(&mut self.items, next);
                if self.tree.mode() == TreeMode::Changed {
                    self.rebuild_tree_keep_mode();
                }
                self.notice = Some(format!("排序：{}", next.label()));
            }

            // 展开/折叠。全量模式下展开会触发懒加载。
            //
            // ⚠️ 必须放在 `KeyCode::Char(' ')` 和 'e' 之前：match 按顺序匹配，
            //    放后面会被前面的分支截走。
            // Enter = "打开"：目录展开/折叠，文件进全屏预览。
            // 不用为全屏单独记一个键 —— 和 lazygit 一致，新人不用学两套。
            //
            // ⚠️ Space 从这里拆出去了：它现在是"勾选提交范围"，
            //    两个动作语义不同，混在一个分支里会让 Space 在
            //    目录上变成展开、在文件上变成全屏 —— 无法勾选。
            KeyCode::Enter => {
                let is_dir = self.tree.selected().map(|n| n.is_dir).unwrap_or(false);
                if is_dir {
                    let need_load = self.tree.toggle();
                    if need_load {
                        if let Some(rel) = self.tree.selected().map(|n| n.rel.clone()) {
                            let _ = self.tree.load_dir(&rel);
                        }
                    }
                    self.spawn_preview();
                } else if let Some(p) = self.preview.take().filter(|p| !p.is_empty()) {
                    self.diff = Some(p);
                    self.mode = Mode::Diff;
                }
            }

            // Space：勾选/取消当前文件（提交范围选择）
            //
            // 勾选放在文件树而不是提交弹框里：
            // 用户要"边看 diff 边挑"，弹框会挡住预览区，反复进出很累。
            // 这是 lazygit 的习惯，也是用户明确要求的交互。
            KeyCode::Char(' ') => match self.tree.toggle_check() {
                Some(true) => {
                    let n = self.tree.checked_count();
                    self.notice = Some(format!("已勾选 {} 项（按 C 提交）", n));
                }
                Some(false) => {
                    let n = self.tree.checked_count();
                    self.notice = Some(if n == 0 {
                        "已取消勾选".into()
                    } else {
                        format!("已取消，当前 {} 项", n)
                    });
                }
                None => {
                    // 不能勾的原因有很多（目录/无改动/未版本化/忽略），
                    // 统一问 tree 要，别在这儿写死一句"目录不能勾选"——
                    // 那样用户看到的是错误的原因。
                    self.notice = Some(
                        self.tree
                            .check_block_reason()
                            .unwrap_or("请先选择要提交的文件")
                            .to_string(),
                    )
                }
            },

            // Ctrl+A 全选可提交的 / Ctrl+R 清空勾选
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree.select_committable(true);
                let n = self.tree.checked_count();
                self.notice = Some(format!("已全选可提交项（{} 项）", n));
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree.clear_check();
                self.notice = Some("已清空勾选".into());
            }

            // Ctrl+K = svn cleanup：只解锁、清中断的操作，不删任何文件。
            // 三 false 是刻意的（不删未版本化、不删忽略项、不真空 pristine），
            // 危险开关留在 CLI `svnui cleanup --remove-unversioned`。
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.spawn_busy("svn cleanup", |svn| {
                    svn.cleanup(false, false, false)?;
                    Ok("已清理工作副本（解锁 + 清除中断的操作）".to_string())
                });
            }

            // E = 用外部编辑器打开（$VISUAL / $EDITOR / vim）
            KeyCode::Char('e') | KeyCode::Char('E') => self.request_editor(),

            // T：变更树 ↔ 全量文件树
            KeyCode::Char('t') | KeyCode::Char('T') => {
                let next = match self.tree.mode() {
                    TreeMode::Changed => {
                        // 切到全量：先读根目录
                        let _ = self.tree.load_dir("");
                        TreeMode::All
                    }
                    TreeMode::All => {
                        self.tree.build_from_items(&self.items, &self.root);
                        TreeMode::Changed
                    }
                };
                self.tree.set_mode(next);
                self.spawn_preview();
                self.notice = Some(match next {
                    TreeMode::Changed => "已切换到变更视图".into(),
                    TreeMode::All => "已切换到全量文件树".into(),
                });
            }

            // X：冲突解决面板
            //
            // ⚠️ 原来是 V，但 V 的认知度更像"查看/浏览"，让给远端仓库面板了。
            //    resolve 这个词在 svn 里就是 `svn resolve`，取首字母 X 也不算生造
            //    （C 已给 commit、R 给 revert，能用的单字母本来就不多）。
            KeyCode::Char('x') | KeyCode::Char('X') => self.open_conflict(),

            // D = svn delete（大写，和小写 d 的 vim 翻页习惯区分开）
            KeyCode::Char('d') | KeyCode::Char('D') => self.do_delete(),

            // /：搜索文件名
            KeyCode::Char('/') => {
                self.searching = Some(SearchFor::Tree);
                self.query.clear();
            }

            KeyCode::Char('c') | KeyCode::Char('C') => self.open_commit(),
            KeyCode::Char('a') => self.add_current(),
            KeyCode::Char('A') => self.confirm_add_all(),
            // 小写 r 还原，大写 R 刷新；不能合并大小写匹配，否则刷新会误入还原。
            KeyCode::Char('r') => self.do_revert(),
            // V = 远端仓库浏览（view）：不用切网页就能翻目录、复制路径、直接检出。
            // 原来挂在 R 上，但 R 和小写 r（revert）只差一个 Shift，
            // 而 revert 是破坏性操作 —— 手滑按成大写的代价太大。
            KeyCode::Char('v') | KeyCode::Char('V') => self.open_repo(),
            KeyCode::Char('R') | KeyCode::F(5) => self.spawn_reload(),
            KeyCode::Char('u') | KeyCode::Char('U') => self.do_update(),
            KeyCode::Char('h') | KeyCode::Char('H') => self.do_doctor(),
            KeyCode::Char('l') | KeyCode::Char('L') => self.open_log(),
            _ => {}
        }
        Ok(())
    }

    fn handle_commit_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        if self.commit.is_none() {
            self.mode = Mode::Browse;
            return Ok(());
        }

        // ⚠️ Enter 是**换行**，不是提交。
        //
        // 之前把 Enter 让给提交，理由是"提交信息通常单行"。
        // 但这是个多行编辑器，Enter=换行是所有编辑器的肌肉记忆，
        // 按下去发现变成提交了，正在写的东西直接飞走 —— 太危险。
        //
        // 提交用 Ctrl+S（和"写完了保存"一个语义）。
        // F2 曾经也接，但同一个动作两个入口只会让人记不住，已去掉。
        // 另外 Ctrl+Enter / Alt+Enter 也接（部分终端能投递修饰位，收不到也不影响）。
        let wants_commit = matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('s'), KeyModifiers::CONTROL)
                | (KeyCode::F(2), _)
                | (KeyCode::Enter, KeyModifiers::CONTROL)
                | (KeyCode::Enter, KeyModifiers::ALT)
        );

        if wants_commit {
            return self.do_commit();
        }

        // Ctrl+G：生成提交信息。
        //
        // 走后台线程，因为可能要联网问模型 —— 在主线程发请求会冻住界面。
        // 规则候选始终一并算出来兜底：模型没配 / 超时 / 返回怪东西，都还有得用。
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('g'), KeyModifiers::CONTROL)
        ) {
            // 先在 mut 借 commit 之前收清单，否则借用冲突。
            let changes: Vec<Change> = self
                .tree
                .checked_changes()
                .into_iter()
                .map(|(rel, sign)| Change::new(rel, sign))
                .collect();

            if let Some(panel) = self.commit.as_mut() {
                // 已有候选就就地轮换，不重复联网。
                if panel.has_candidates() {
                    let msg = panel.next_suggestion();
                    panel.set_notice(msg);
                    return Ok(());
                }
                if self.busy.is_some() {
                    panel.set_notice("正在生成，请稍候".to_string());
                    return Ok(());
                }

                let tx = self.tx.clone();
                self.busy = Some("生成提交信息".to_string());
                std::thread::spawn(move || {
                    let cfg = crate::config::Config::load();
                    let items: Vec<(String, char)> = changes
                        .iter()
                        .map(|c| (c.rel.clone(), c.sign))
                        .collect();
                    let mut out = crate::llm::suggest(&items, None, &cfg.llm).unwrap_or_default();
                    out.extend(crate::tui::panels::commit_gen::suggest(&changes));
                    let _ = tx.send(BgDone::CommitSug(out));
                });
            }
            return Ok(());
        }

        // Esc：直接退出，**不弹二次确认**。
        //
        // 之前有内容时会弹"确定放弃吗"，结果用户按 N 想退出却被送回提交框，
        // 再按 Esc 又弹 —— 出不去。根因是"退出"和"不放弃"共用一个键。
        //
        // 现在改成：Esc 永远能走，内容存进 commit_draft，
        // 下次按 C 打开自动恢复。既然不会丢，就没必要再问一句。
        if matches!(key.code, KeyCode::Esc) {
            self.close_commit();
            return Ok(());
        }

        // 其余按键全部交给 TextArea（含方向键、退格、Ctrl+J 换行等）
        if let Some(panel) = self.commit.as_mut() {
            panel.textarea_mut().input(key);
            // 一编辑就让旧提示过期（多半是"提交信息不能为空"）。
            // 重新按提交键时如果还有问题会再提示一次，不会漏。
            panel.clear_hint();
        }
        Ok(())
    }

    fn handle_diff_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        let Some(panel) = self.diff.as_mut() else {
            self.mode = Mode::Browse;
            return Ok(());
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.diff = None;
                self.mode = Mode::Browse;
                self.spawn_preview();
            }
            KeyCode::Char('j') | KeyCode::Down => panel.scroll_down(1),
            KeyCode::Char('k') | KeyCode::Up => panel.scroll_up(1),
            KeyCode::Char('d') | KeyCode::PageDown => panel.scroll_down(10),
            KeyCode::Char('u') | KeyCode::PageUp => panel.scroll_up(10),
            KeyCode::Char('g') => panel.scroll_top(),
            // 和主界面保持一致：h/l 与 ←/→ 慢移，< > 翻页。
            // 这里 h/l 是从 less/vim 沿用的，比方向键顺手，所以两套都留。
            KeyCode::Char('h') | KeyCode::Left => panel.scroll_left(2),
            KeyCode::Char('l') | KeyCode::Right => panel.scroll_right(2),
            // ⚠️ 这里不能调 self.page_preview_h()：
            // `panel` 还借着 self.diff，再借一次会冲突。
            // 直接在本面板上取步长再滚，效果一样。
            KeyCode::Char('>') | KeyCode::Char('.') => {
                let step = panel.page_step();
                panel.scroll_right(step);
            }
            KeyCode::Char('<') | KeyCode::Char(',') => {
                let step = panel.page_step();
                panel.scroll_left(step);
            }
            KeyCode::Char('0') => panel.scroll_home(),
            _ => {}
        }
        Ok(())
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        let Some(panel) = self.confirm.as_ref() else {
            self.mode = Mode::Browse;
            return Ok(());
        };
        // 丢弃本地改动必须明确按 Y；Enter 保持保守默认，直接取消。
        if key.code == KeyCode::Enter && matches!(panel.action(), ConfirmAction::Revert(_)) {
            self.confirm = None;
            self.mode = Mode::Browse;
            self.notice = Some("已取消丢弃本地改动".into());
            return Ok(());
        }
        // 进 match 前先把动作 clone 出来：
        // arm 里再借用 panel 会和 `self.confirm = None` 打架。
        //
        // 提交已经不走确认框了（Enter 改换行后不会误触），
        // 这里剩下的都是不可逆的动作：revert / delete / resolve / update。
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.confirm = None;
                self.mode = Mode::Browse;
                self.notice = Some("已取消".into());
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                // 先 clone 出动作再关面板：确认过程中可能又要 spawn，
                // 借着 panel 不放手会在下面 self.spawn_busy 时冲突。
                let action = panel.action().clone();
                self.confirm = None;
                self.mode = Mode::Browse;
                match action {
                    ConfirmAction::Add(paths) => self.spawn_add(paths),
                    ConfirmAction::Revert(paths) => {
                        if paths.is_empty() {
                            self.notice = Some("没有要撤销的项目".into());
                        } else {
                            self.do_revert_confirmed(paths);
                        }
                    }
                    ConfirmAction::RevertToRev(rev) => self.do_revert_to_rev_confirmed(rev),
                    ConfirmAction::Update => self.do_update_confirmed(),
                    ConfirmAction::Resolve(path, strategy) => {
                        self.do_resolve_confirmed(path, strategy)
                    }
                    ConfirmAction::RestoreFromRepo(path) => {
                        self.do_restore_from_repo_confirmed(path)
                    }
                    ConfirmAction::Delete(paths, keep_local) => {
                        if paths.is_empty() {
                            self.notice = Some("没有要删除的项".into());
                        } else {
                            self.do_delete_confirmed(paths, keep_local);
                        }
                    }
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(p) = self.confirm.as_mut() {
                    p.scroll_down(1)
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(p) = self.confirm.as_mut() {
                    p.scroll_up(1)
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_conflict_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.conflict = None;
                self.mode = Mode::Browse;
                self.spawn_preview();
                return Ok(());
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(p) = self.conflict.as_mut() {
                    if p.move_down() {
                        self.load_conflict_content();
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(p) = self.conflict.as_mut() {
                    if p.move_up() {
                        self.load_conflict_content();
                    }
                }
            }
            KeyCode::Char('1') => {
                if let Some(p) = self.conflict.as_mut() {
                    p.set_side(Side::Mine);
                }
                self.load_conflict_content();
            }
            KeyCode::Char('2') => {
                if let Some(p) = self.conflict.as_mut() {
                    p.set_side(Side::Theirs);
                }
                self.load_conflict_content();
            }
            KeyCode::Char('3') => {
                if let Some(p) = self.conflict.as_mut() {
                    p.set_side(Side::Working);
                }
                self.load_conflict_content();
            }
            KeyCode::Tab => {
                if let Some(p) = self.conflict.as_mut() {
                    p.cycle_side();
                }
                self.load_conflict_content();
            }
            // m = 用我的，t = 用服务器的，Enter = 保留当前文件
            // 树冲突的「恢复服务器版本」。
            //
            // ⚠ 树冲突**只能** resolve 到 working，mine-full / theirs-full
            //   会被 svn 拒绝（E155027），按了没反应就是这么来的。
            //   所以树冲突下 r 不走 resolve，改走 revert + update。
            //   普通冲突仍用 theirs-full（t 键同义）。
            KeyCode::Char('r') => {
                if self.conflict_tree_selected() {
                    self.do_restore_from_repo();
                } else {
                    self.do_resolve(Strategy::TheirsFull);
                }
            }
            // 树冲突的「坚持我的删除」== 磁盘现状，只能表达成 working。
            KeyCode::Char('m') | KeyCode::Char('M') => {
                if self.conflict_tree_selected() {
                    self.do_resolve(Strategy::Working);
                } else {
                    self.do_resolve(Strategy::MineFull);
                }
            }
            KeyCode::Char('t') | KeyCode::Char('T') => self.do_resolve(Strategy::TheirsFull),
            KeyCode::Enter => self.do_resolve(Strategy::Working),
            KeyCode::Char('d') | KeyCode::PageDown => {
                if let Some(p) = self.conflict.as_mut() {
                    p.scroll_down(10)
                }
            }
            KeyCode::Char('u') | KeyCode::PageUp => {
                if let Some(p) = self.conflict.as_mut() {
                    p.scroll_up(10)
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_log_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        // 日志没读出来时（比如连不上服务器），也要能操作：
        // 否则面板上写着"按 R 重试"，按了却没反应。
        if self.logpanel.is_none() {
            match key.code {
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    self.log_error = None;
                    self.open_log();
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('l') | KeyCode::Char('L') => {
                    self.log_error = None;
                    self.mode = Mode::Browse;
                }
                _ => {}
            }
            return Ok(());
        }

        let Some(panel) = self.logpanel.as_mut() else {
            self.mode = Mode::Browse;
            return Ok(());
        };
        match key.code {
            // Enter：看这个版本改了什么（完整 diff）
            KeyCode::Enter => {
                if let Some(rev) = panel.selected_rev() {
                    self.open_rev_diff(rev);
                }
                return Ok(());
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('l') | KeyCode::Char('L') => {
                self.logpanel = None;
                self.mode = Mode::Browse;
                return Ok(());
            }
            // /：在历史里搜
            KeyCode::Char('/') => {
                self.searching = Some(SearchFor::Log);
                self.query.clear();
                return Ok(());
            }
            // N：再加载一页更早的历史。
            //
            // 默认只加载 100 条是为了首屏快；想往回翻就按这个。
            // 更早/更精确的定位靠 / 搜索（走服务端 --search）。
            KeyCode::Char('n') | KeyCode::Char('N') => {
                let have = panel.len();
                self.spawn_log_load(None, have + Self::LOG_PAGE, false);
                return Ok(());
            }
            // R：反向合并回到选中版本。
            //
            // 放在历史面板里而不是主界面，是因为"回退到哪一版"这个决定
            // 只能看着历史做 —— 在主界面按 R 得先记住版本号再输一遍，
            // 链路断了，功能等于没有。
            KeyCode::Char('r') | KeyCode::Char('R') => return self.do_revert_to_rev(),
            KeyCode::Char('j') | KeyCode::Down => panel.move_down(),
            KeyCode::Char('k') | KeyCode::Up => panel.move_up(),
            _ => {}
        }
        self.spawn_log_detail();
        Ok(())
    }

    fn handle_checkout_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        let Some(panel) = self.checkout.as_mut() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Tab => panel.next_field(),
            KeyCode::BackTab => panel.prev_field(),
            KeyCode::Backspace => panel.pop_char(),
            KeyCode::Up => panel.prev_field(),
            KeyCode::Down => panel.next_field(),
            // Ctrl+U 清空当前字段：粘错一长串 URL 时逐字符退格太慢。
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                panel.clear_field()
            }
            KeyCode::Char(c) => panel.push_char(c),
            KeyCode::Enter => {
                let at_last = panel.focus() == 3;
                if at_last {
                    return self.do_checkout();
                }
                panel.next_field();
            }
            _ => {}
        }
        Ok(())
    }

    // ── 远端仓库浏览 ───────────────────────────────────────

    /// 打开 Repo 面板。
    ///
    /// 仓库根 URL 来自 `svn info` 的 repository root —— **同步**拿，
    /// 因为它是纯本地读取（不加 `-u` 不联网），一次几毫秒，
    /// 没必要为此多一套异步状态。真正的目录列表才走后台。
    fn open_repo(&mut self) {
        // 起点用**工作副本当前目录的 URL**，不用 repository root。
        //
        // 很多 SVN 是「按子树授权」的：账号只对 /trunk/A8_Patch/... 这一支有权限，
        // 拿 repository root（/JR/KAMP）去 svn list 会被拒 ——
        // svn 报 E170013（连不上）+ E175013（forbidden），
        // 而 170013 是结果、175013 才是原因，看起来像"服务器挂了"，
        // 实际是"这个账号看不了仓库根"。
        //
        // 从工作副本 URL 起步，能一路向上退到还有权限的那一层；
        // 需要更上层就找管理员开通，而不是让人去查 VPN。
        let (root_url, start_url) = match self.svn.info() {
            Ok(info) => {
                let root = info
                    .repository_root
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .to_string();
                let wc = info.url.trim_end_matches('/').to_string();
                // repository root 拿不到（少见）就退化成"从工作副本起步"，
                // 代价是退不上去，但至少面板能开。
                if root.is_empty() {
                    (wc.clone(), wc)
                } else {
                    (root, wc)
                }
            }
            Err(e) => {
                self.notice = Some(format!("读取仓库信息失败：{}", e));
                return;
            }
        };

        // root 归 root、起点靠 trail —— 直接把工作副本 URL 当 root 的话
        // trail 恒为空，← 一步都退不上去。见 RepoPanel::new_at。
        self.repo = Some(RepoPanel::new_at(root_url, start_url));
        self.mode = Mode::Repo;
        let url = self.repo.as_ref().unwrap().current_url();
        self.spawn_repo_list(&url);
    }

    fn spawn_repo_list(&mut self, url: &str) {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        let url = url.to_string();
        // 远端 list 是联网操作，别用默认的 30s —— 大目录 + 慢服务器很容易超。
        self.busy = Some("读取仓库目录".to_string());
        std::thread::spawn(move || {
            let _ = tx.send(BgDone::RepoList(url.clone(), svn.list(&url)));
        });
    }

    fn handle_repo_key(&mut self, key: KeyEvent) -> crate::domain::Result<()> {
        // 先把 action 取成 owned 再动 self：
        // 下面几个分支都要 &mut self（spawn / 开检出框），
        // 如果让 `panel` 这个 &mut 借用活到 match 里，就会撞上借用检查。
        let action = match self.repo.as_mut() {
            Some(p) => p.handle_key(key),
            None => {
                self.mode = Mode::Browse;
                return Ok(());
            }
        };

        match action {
            RepoAction::None => {}
            RepoAction::Load(url) => self.spawn_repo_list(&url),
            RepoAction::Close => {
                self.repo = None;
                self.mode = Mode::Browse;
            }
            RepoAction::Copy(text) => match copy_to_clipboard(&text) {
                Ok(()) => self.notice = Some(format!("已复制：{}", text)),
                Err(e) => {
                    // 复制失败别静默：用户以为复制了，粘出来还是旧内容。
                    // 兜底把 URL 直接显示出来，至少能手动选中。
                    self.notice = Some(format!("复制失败（{}）：{}", e, text));
                }
            },
            RepoAction::Checkout(url) => {
                // 直接复用检出面板并预填 URL —— 别自己再写一套检出逻辑
                self.repo = None;
                self.mode = Mode::Browse;
                self.open_checkout_with(Some(url));
            }
        }
        Ok(())
    }

    /// 打开检出面板，`url` 非空时预填。
    fn open_checkout_with(&mut self, url: Option<String>) {
        let mut p = CheckoutPanel::new(self.root.to_string_lossy().to_string());
        if let Some(u) = url {
            p.set_field(0, &u);
            p.set_hint("URL 已从仓库面板带入；本地路径确认后按 Enter 开始检出");
        }
        self.checkout = Some(p);
        self.mode = Mode::Checkout;
    }

    fn do_checkout(&mut self) -> crate::domain::Result<()> {
        let Some(panel) = self.checkout.as_ref() else {
            return Ok(());
        };

        let url = panel.url().trim().to_string();
        if url.is_empty() {
            if let Some(p) = self.checkout.as_mut() {
                p.set_hint("仓库 URL 不能为空");
            }
            return Ok(());
        }

        let path = std::path::PathBuf::from(panel.path().trim());
        let username = panel.username().map(|s| s.to_string());
        let password = panel.password().map(|s| s.to_string());
        let url2 = url.clone();

        self.checkout = None;
        self.busy = Some("检出".into());

        let svn = self.svn.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let auth = crate::svn::client::AuthOpts {
                username,
                password,
                no_auth_cache: false,
            };
            // 进度回传：svn 每落一个文件打一行，这里数行数、留最后一项。
            // 节流到 ~8 次/秒 —— 大仓库几万行，不节流 channel 会被打满，
            // UI 光收消息就够忙了。
            let tx_p = tx.clone();
            let mut n = 0usize;
            let mut last = String::new();
            let mut sent = Instant::now();

            let r = svn
                .checkout_progress(&url2, &path, &auth, "infinity", move |line| {
                    let t = line.trim();
                    if t.is_empty() {
                        return;
                    }
                    n += 1;
                    // "A    trunk/foo.c" —— 跳过状态列，只留路径
                    if let Some(p) = t.split_whitespace().nth(1) {
                        last = p.to_string();
                    }
                    if sent.elapsed() >= Duration::from_millis(120) {
                        sent = Instant::now();
                        let _ = tx_p.send(BgDone::CheckoutProgress(n, last.clone()));
                    }
                })
                .map(|out| {
                    format!("已检出到 {}
{}", path.display(), out.stdout)
                });
            let _ = tx.send(BgDone::Checkout(r));
        });

        // 检出期间显示"进行中"，完成后提示退出重进
        self.mode = Mode::Browse;
        Ok(())
    }

    /// 请求用外部编辑器打开选中文件。
    fn request_editor(&mut self) {
        let Some(node) = self.tree.selected().cloned() else {
            return;
        };
        if node.is_dir {
            self.notice = Some("目录不能用编辑器打开".to_string());
            return;
        }
        if !node.abs.exists() {
            self.notice = Some("文件不存在（可能是已删除）".to_string());
            return;
        }
        self.editor_req = Some((Self::default_editor(), node.abs.clone()));
    }

    // ── 动作 ──────────────────────────────────────────────

    fn spawn_busy<F>(&mut self, label: &str, f: F)
    where
        F: FnOnce(Svn) -> crate::domain::Result<String> + Send + 'static,
    {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        self.busy = Some(label.to_string());
        std::thread::spawn(move || {
            let _ = tx.send(BgDone::Msg(f(svn)));
        });
    }

    /// 首次打开历史时加载多少条。
    ///
    /// 100 而不是 200：svn log 是**联网**操作，每多一条都要等服务器。
    /// 首屏 100 条足够翻找，更早的靠搜索（服务端 `--search`）定位。
    const LOG_PAGE: usize = 100;

    fn open_log(&mut self) {
        self.spawn_log_load(None, Self::LOG_PAGE, false);
        self.mode = Mode::Log;
    }

    /// 拉日志。`search` 非空时走服务端 `--search`。
    ///
    /// `append`：是否把结果追加到现有列表（搜索是替换，不是追加）。
    fn spawn_log_load(&mut self, search: Option<String>, limit: usize, append: bool) {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        // 版本不够就退回本地过滤：旧 svn 不认识 --search，
        // 直接传会报错。这种情况下至少还能搜已加载的 100 条。
        let use_search = search.is_some() && svn.supports_log_search();
        self.busy = Some(match (&search, use_search) {
            (Some(kw), true) => format!("搜索“{}”…", kw),
            (Some(_), false) => "搜索（本地）…".into(),
            (None, _) => "读取历史".into(),
        });
        std::thread::spawn(move || {
            let r = if use_search {
                svn.log_search(limit, &[], search.as_deref())
            } else {
                svn.log(limit, &[])
            };
            let _ = tx.send(BgDone::LogLoad(r, search.clone(), append));
        });
    }

    /// 查看某个历史版本改了什么（`svn diff -c REV`）。
    ///
    /// 用在日志面板里按 Enter：右侧已经能看到"改了哪些文件"（`-v` 摘要），
    /// 但要看**具体改了什么**得拉完整 diff。
    fn open_rev_diff(&mut self, rev: u64) {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        self.busy = Some(format!("读取 r{} 的改动", rev));
        std::thread::spawn(move || {
            let r = svn.rev_diff(rev).map(|o| o.stdout);
            let _ = tx.send(BgDone::RevDiff(rev, r));
        });
    }

    fn spawn_log_detail(&mut self) {
        let Some(rev) = self.logpanel.as_ref().and_then(|p| p.need_detail()) else {
            return;
        };
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let lines = match svn.log_rev(&rev.to_string(), true) {
                Ok(es) => es
                    .into_iter()
                    .flat_map(|e| {
                        e.paths
                            .iter()
                            .map(|p| format!(" {} {}", p.action, p.path))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
                Err(e) => vec![format!("读取失败：{}", e)],
            };
            let _ = tx.send(BgDone::LogDetail(rev, lines));
        });
    }

    /// 打开提交面板。
    ///
    /// ## 勾选来源
    ///
    /// - 用户已在文件树上 Space 勾选过 → 用勾选的
    /// - 一个都没勾 → **默认全选可提交项**（`?` 未版本化除外）
    ///
    /// 第二种是便利：多数时候就是"全提交"，不该强制一个个勾。
    /// 但 `?` 永远不自动进 —— 那需要先 `svn add`。
    fn open_commit(&mut self) {
        if self.items.is_empty() {
            self.notice = Some("没有可提交的变更".into());
            return;
        }

        // 一个都没勾就默认全选可提交的
        if self.tree.checked_count() == 0 {
            self.tree.select_committable(true);
        }

        let paths = self.tree.checked_abs();
        if paths.is_empty() {
            self.notice = Some("没有可提交的文件（未纳管项请先按 a 或 A 加入）".into());
            return;
        }

        // 展示名：美化路径（去根前缀 + percent 解码）
        let files: Vec<String> = paths
            .iter()
            .map(|p| pretty_path(&p.to_string_lossy(), &self.root, 70))
            .collect();

        // 提交信息生成用的改动清单（相对路径 + 状态符号）。
        // 和 paths 同序（都按 visible 走），清单对得上。
        let changes: Vec<Change> = self
            .tree
            .checked_changes()
            .into_iter()
            .map(|(rel, sign)| Change::new(rel, sign))
            .collect();

        // 草稿在这里生效：上次按 Esc 退出时存的内容自动回来。
        // 用 take() 而不是 clone —— 恢复之后草稿就归提交框管了，
        // 留一份副本会在"提交成功"和"再次 Esc"之间产生两份不一致的状态。
        self.commit = Some(CommitPanel::new(files, self.commit_draft.take(), changes));
        self.commit_paths = paths;
        self.mode = Mode::Commit;
    }

    /// 关闭提交框：内容存草稿，下次打开自动恢复。
    ///
    /// 不弹"确定放弃吗" —— 内容不丢，问那一句就是纯打扰。
    fn close_commit(&mut self) {
        let mut saved = false;
        if let Some(p) = self.commit.take() {
            let msg = p.message();
            if msg.is_empty() {
                self.commit_draft = None;
            } else {
                self.commit_draft = Some(msg);
                saved = true;
            }
        }
        self.mode = Mode::Browse;
        // 没写东西就别提"已保留" —— 会让人以为有什么东西在等他
        self.notice = Some(
            if saved {
                "已退出提交（内容已保留，按 C 可继续）"
            } else {
                "已取消提交"
            }
            .into(),
        );
    }

    fn add_current(&mut self) {
        let Some(selected) = self.tree.selected() else {
            self.notice = Some("请先选中要添加的文件或目录".into());
            return;
        };
        // 文本 status 的 kind 可能为 Unknown；添加前确认真实类型，目录不能绕过确认。
        let is_dir = match std::fs::symlink_metadata(&selected.abs) {
            Ok(metadata) => metadata.file_type().is_dir(),
            Err(e) => {
                self.notice = Some(format!("无法读取所选项类型：{}", e));
                return;
            }
        };
        let rel = selected.rel.clone();
        // 目录状态可能由子项冒泡而来：只添加真实未纳管项，不能对已纳管父目录重复 add。
        let targets: Vec<PathBuf> = self
            .items
            .iter()
            .filter(|item| {
                item.sign == '?'
                    && (item.rel == rel
                        || (is_dir && std::path::Path::new(&item.rel).starts_with(&rel)))
            })
            .map(|item| self.root.join(&item.rel))
            .collect();
        if targets.is_empty() {
            self.notice = Some("当前项没有可添加的未纳管内容，按 A 可查看全部新增项".into());
        } else if is_dir {
            self.confirm_add(targets, format!("当前目录：{}/", rel));
        } else {
            self.spawn_add(targets);
        }
    }

    fn confirm_add_all(&mut self) {
        let targets: Vec<PathBuf> = self
            .items
            .iter()
            .filter(|item| item.sign == '?')
            .map(|item| self.root.join(&item.rel))
            .collect();
        self.confirm_add(targets, "全部未纳管项（包含搜索过滤外的项目）".into());
    }

    fn confirm_add(&mut self, mut targets: Vec<PathBuf>, scope: String) {
        if targets.is_empty() {
            self.notice = Some("没有可添加的未纳管项".into());
            return;
        }
        // 冻结显式目标；目标目录的子项仍由 svn add 执行时按递归规则发现。
        targets.sort();
        targets.dedup();
        let entries = targets
            .iter()
            .map(|p| {
                p.strip_prefix(&self.root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        self.confirm = Some(ConfirmPanel::new(
            format!("确认加入 SVN：{} 个目标", targets.len()),
            format!("范围：{scope}\n目录递归添加未忽略子项。\n仅加入 SVN，尚未提交。"),
            entries,
            Danger::Warn,
            ConfirmAction::Add(targets),
        ));
        self.mode = Mode::Confirm;
    }

    fn spawn_add(&mut self, targets: Vec<PathBuf>) {
        if targets.is_empty() {
            return;
        }
        let n = targets.len();
        self.spawn_busy("svn add", move |svn| {
            svn.add(&targets)?;
            Ok(format!("已加入 SVN：{} 个目标（尚未提交）", n))
        });
    }

    /// SVN revert 没有干跑能力，先读取最新本地状态，再显示范围和后果。
    fn do_revert(&mut self) {
        let (scope, mut paths) = if self.tree.checked_count() > 0 {
            // 勾选集合独立于可见树；过滤或折叠不能悄悄缩小操作范围。
            let paths: Vec<PathBuf> = self
                .items
                .iter()
                .filter(|item| self.tree.is_checked(&item.rel))
                .map(|item| self.root.join(&item.rel))
                .collect();
            (
                format!("已勾选 {} 项（含过滤/折叠隐藏项）", paths.len()),
                paths,
            )
        } else if let Some(node) = self.tree.selected() {
            (format!("当前项 {}", node.rel), vec![node.abs.clone()])
        } else {
            self.notice = Some("请先选中文件、目录或勾选要撤销的项目".into());
            return;
        };
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            self.notice = Some("勾选范围已失效，请刷新后重新选择".into());
            return;
        }
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        self.busy = Some("检查本地改动".into());
        std::thread::spawn(move || {
            let result = svn.status_paths(&paths);
            let _ = tx.send(BgDone::RevertCheck(scope, paths, result));
        });
    }

    /// 历史面板按 R：反向合并回到选中版本（先干跑）。
    ///
    /// 干跑除了列出"会动哪些文件"，还会把**将被撤销的那几个提交**列出来。
    /// 这是刻意的：回退的风险不在"改了哪些文件"，而在"我到底退到了哪"，
    /// 后者只有看到被撤销的提交列表才能确认。
    fn do_revert_to_rev(&mut self) -> crate::domain::Result<()> {
        let rev = match self.logpanel.as_ref().and_then(|p| p.selected_rev()) {
            Some(r) => r,
            None => {
                self.notice = Some("请先用 ↑↓ 选中一个版本".into());
                return Ok(());
            }
        };
        let root = self.root.clone();
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        self.busy = Some(format!("检查回到 r{} 的影响", rev));
        std::thread::spawn(move || {
            let head = match svn.info() {
                Ok(i) => i.revision,
                Err(e) => {
                    let _ = tx.send(BgDone::Msg(Err(e)));
                    return;
                }
            };
            if rev >= head {
                let _ = tx.send(BgDone::Msg(Err(crate::domain::Error::Explained {
                    summary: format!("r{} 不早于当前版本 r{}", rev, head),
                    detail: "回退只能回到更早的版本。".into(),
                })));
                return;
            }
            // 将被撤销的提交：rev+1 .. HEAD（log 默认倒序，翻转成正序更好读）
            let commits: Vec<String> = match svn.log_rev(&format!("{}:{}", rev + 1, head), false) {
                Ok(es) => es
                    .iter()
                    .rev()
                    .map(|e| {
                        format!(
                            "r{}  {}  {}",
                            e.revision,
                            e.date.get(..10).unwrap_or(&e.date),
                            e.author
                        )
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
            let files = match svn.merge_to(rev, true) {
                Ok(out) => out
                    .stdout
                    .lines()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty() && !l.starts_with("---"))
                    .filter_map(|l| l.split_whitespace().nth(1))
                    .filter(|p| *p != ".")
                    .map(|p| short_rel(p, &root))
                    .collect::<Vec<_>>(),
                Err(e) => {
                    let _ = tx.send(BgDone::Msg(Err(e)));
                    return;
                }
            };
            let _ = tx.send(BgDone::RevDry(rev, head, commits, files));
        });
        Ok(())
    }

    /// 真正执行回退（确认面板里按 Y 之后）。
    fn do_revert_to_rev_confirmed(&mut self, rev: u64) {
        self.spawn_busy(&format!("回退到 r{}", rev), move |svn| {
            svn.merge_to(rev, false)?;
            Ok(format!("已回退到 r{}，改动在工作副本里 —— 还需提交一次才会写进仓库", rev))
        });
    }

    /// 真正执行 revert（仅在确认面板里明确按 Y 之后）。
    fn do_revert_confirmed(&mut self, paths: Vec<PathBuf>) {
        let n = paths.len();
        self.spawn_busy("svn revert", move |svn| {
            svn.revert(&paths, false)?;
            Ok(format!("已撤销 {} 个目标的本地改动", n))
        });
    }

    /// update 第一阶段：先查远端有没有更新（`svn status -u`）。
    ///
    /// 为什么多这一步：`svn update` 撞上别人改过的文件时会产生冲突，
    /// 事后救火比重事前看一眼贵得多。有重叠就先预警。
    ///
    /// ⚠️ `-u` 要连服务器，可能几秒到几十秒，所以走后台线程。
    fn do_update(&mut self) {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        let local: Vec<String> = self
            .items
            .iter()
            .filter(|i| i.sign != ' ' && i.sign != '?')
            .map(|i| i.rel.clone())
            .collect();
        self.busy = Some("检查远端更新".into());
        std::thread::spawn(move || {
            let res = svn.outdated().map(|v| (v, local));
            let _ = tx.send(BgDone::OutdatedCheck(res));
        });
    }

    fn do_update_confirmed(&mut self) {
        self.spawn_busy("svn update", |svn| {
            let out = svn.update(None, &[])?;
            Ok(out
                .stdout
                .lines()
                .last()
                .unwrap_or("已更新")
                .trim()
                .to_string())
        });
    }

    /// 打开冲突解决面板。
    fn open_conflict(&mut self) {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        self.busy = Some("扫描冲突".into());
        std::thread::spawn(move || {
            let res = svn.conflicts().map(|v| {
                v.into_iter()
                    .map(|e| ConflictItem {
                        // 第 7 列的 `C`：树冲突。三路对比对它没意义，
                        // 面板据此换一套界面。
                        tree: e.tree_conflict,
                        path: e.path.into_std_path_buf(),
                    })
                    .collect::<Vec<_>>()
            });
            let _ = tx.send(BgDone::Conflicts(res));
        });
    }

    /// 冲突面板里切换文件后，重新加载对应那一侧的内容。
    fn load_conflict_content(&mut self) {
        let Some(panel) = self.conflict.as_ref() else {
            return;
        };
        let Some(path) = panel.selected().map(|it| it.path.clone()) else {
            return;
        };
        let side = panel.side();

        let svn = self.svn.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let res = svn.conflict_versions(&path).map(|v| match side {
                Side::Mine => v.mine,
                Side::Theirs => v.theirs,
                Side::Working => v.working,
            });
            let _ = tx.send(BgDone::ConflictContent(res));
        });
    }

    /// 执行冲突解决。不可逆，所以走确认面板。
    fn do_resolve(&mut self, strategy: Strategy) {
        let Some(panel) = self.conflict.as_ref() else {
            return;
        };
        let Some(item) = panel.selected().cloned() else {
            return;
        };
        let path = item.path.clone();
        let rel = short_rel(&path.to_string_lossy(), &self.root);
        let mut body =
            format!("对 {} 执行「{}」。\n{}", rel, strategy.label(), strategy.warning());
        if item.tree {
            body.push_str("\n\n这是树冲突：没有可合并的内容，选完即定。");
        }
        self.confirm = Some(ConfirmPanel::new(
            format!("确认：{}", strategy.label()),
            body,
            vec![rel],
            Danger::Critical,
            ConfirmAction::Resolve(path, strategy),
        ));
        self.mode = Mode::Confirm;
    }

    /// 真的调 svn resolve。
    fn do_resolve_confirmed(&mut self, path: PathBuf, strategy: Strategy) {
        let arg = strategy.arg().to_string();
        self.spawn_busy("svn resolve", move |svn| {
            svn.resolve(&[path], &arg)?;
            Ok("已标记为已解决".to_string())
        });
    }

    /// 当前选中的冲突项是不是树冲突。
    ///
    /// 树冲突的解决路径和普通冲突完全不同（只能 working），
    /// 所以每个执行键都得先问一句。
    fn conflict_tree_selected(&self) -> bool {
        self.conflict
            .as_ref()
            .and_then(|p| p.selected())
            .map(|it| it.tree)
            .unwrap_or(false)
    }

    /// 树冲突的「恢复服务器版本」。
    ///
    /// 不走 `svn resolve` —— 树冲突只能 resolve 到 working，
    /// 想拿服务器版本只能 revert 掉本地的删除/改名再 update。
    fn do_restore_from_repo(&mut self) {
        let Some(path) = self
            .conflict
            .as_ref()
            .and_then(|p| p.selected())
            .map(|it| it.path.clone())
        else {
            return;
        };
        let rel = short_rel(&path.to_string_lossy(), &self.root);
        self.confirm = Some(ConfirmPanel::new(
            "确认：恢复服务器版本（树冲突）",
            format!(
                "放弃你对 {rel} 的本地删除/改名，从服务器重新拉取。\n\
                 执行：svn revert -R 然后 svn update。\n\n\
                 ⚠ 你 mv 改名生成的新文件不受影响，但旧路径会恢复；\n\
                   改名历史无法保留（要保留请用 svn mv）。"
            ),
            vec![rel],
            Danger::Critical,
            ConfirmAction::RestoreFromRepo(path),
        ));
        self.mode = Mode::Confirm;
    }

    /// 真的执行 revert + update。
    fn do_restore_from_repo_confirmed(&mut self, path: PathBuf) {
        self.spawn_busy("恢复服务器版本", move |svn| {
            let ps = vec![path];
            svn.revert(&ps, false)?;
            svn.update(None, &ps)?;
            Ok("已恢复为服务器版本（本地删除/改名已撤销）".to_string())
        });
    }

    /// `svn delete`：把光标下的文件/目录从版本控制里删除。
    ///
    /// ## 几个必须挡住的坑
    ///
    /// 1. **未版本化文件不能 svn delete** —— 它压根不在版本控制里，
    ///    `svn rm` 会报 E200009。这种文件直接系统删除即可。
    /// 2. **目录是递归删除** —— `svn rm docs/` 会把整个子树都删掉，
    ///    确认文案里必须说清楚，不能只写"删除 docs/"。
    /// 3. **默认同时删本地文件**（`svn rm` 的语义）。
    ///    这是不可逆的，所以走确认面板。
    fn do_delete(&mut self) {
        // ⚠️ 先在不可变借用里取值再断开：
        //    下面要给 self.notice / self.confirm 赋值（可变借用），
        //    持有 tree 的不可变借用会冲突。
        if self.tree.selected().is_none() {
            self.notice = Some("请先把光标移到要删除的项上".into());
            return;
        }
        let (is_dir, sign, rel, abs) = {
            let n = match self.tree.selected() {
                Some(n) => n,
                None => return,
            };
            (n.is_dir, n.sign, n.rel.clone(), n.abs.clone())
        };

        if sign == '?' {
            self.notice = Some(format!("{} 未纳入版本控制，直接删除文件即可（不需要 svn）", rel));
            return;
        }
        if sign == 'I' || sign == 'X' {
            self.notice = Some(format!("{} 是忽略/外部项，不需要删除", rel));
            return;
        }
        if sign == 'D' {
            self.notice = Some(format!("{} 已经标记为删除了（提交后生效）", rel));
            return;
        }

        // 显示名：解掉 percent 编码、去掉根前缀（svn 输出的路径可能带编码）
        let show = pretty_path(&rel, &self.root, 70);
        let message = if is_dir {
            format!("将递归删除目录 {} 及其下所有文件，本地文件一并删除：", show)
        } else {
            format!("将从版本控制删除 {}，同时删除本地文件：", show)
        };

        self.confirm = Some(ConfirmPanel::new(
            "确认删除",
            message,
            vec![if is_dir {
                format!("{}/（整个目录）", show)
            } else {
                show
            }],
            Danger::Critical,
            ConfirmAction::Delete(vec![abs], false),
        ));
        self.mode = Mode::Confirm;
    }

    /// 真正执行 svn delete。
    fn do_delete_confirmed(&mut self, paths: Vec<PathBuf>, keep_local: bool) {
        let n = paths.len();
        self.spawn_busy("svn delete", move |svn| {
            svn.remove(&paths, keep_local)?;
            Ok(if keep_local {
                format!("已从版本控制移除 {} 项（本地文件保留）", n)
            } else {
                format!("已删除 {} 项", n)
            })
        });
    }

    fn do_doctor(&mut self) {
        self.spawn_busy("体检", |svn| match svn.conflicts() {
            Ok(c) if c.is_empty() => Ok("✓ 无冲突，工作副本健康".into()),
            Ok(c) => Ok(format!("⚠ {} 处冲突，需要处理", c.len())),
            Err(e) => Err(e),
        });
    }

    fn do_commit(&mut self) -> crate::domain::Result<()> {
        let Some(panel) = self.commit.as_ref() else {
            return Ok(());
        };

        // 边界①：提交进行中再按提交键 —— 直接忽略。
        // 连按两下 Ctrl+S 会起两个 svn commit，第二次必然失败
        // （文件已提交），报错还会吓一跳。busy 就是天然的互斥锁。
        if self.busy.is_some() {
            return Ok(());
        }

        let msg = panel.message();
        // 范围来自文件树上的勾选（open_commit 时存下来的），
        // 不在这里重新读 tree —— 用户可能在弹框打开期间改了勾选，
        // 但提交范围必须锁定为"打开弹框那一刻看到的那些"，
        // 否则会出现"我看到的清单和实际提交的不一致"。
        let paths = self.commit_paths.clone();

        if msg.is_empty() {
            if let Some(p) = self.commit.as_mut() {
                p.set_hint("提交信息不能为空");
            }
            return Ok(());
        }
        if paths.is_empty() {
            if let Some(p) = self.commit.as_mut() {
                p.set_hint("没有勾选任何文件");
            }
            return Ok(());
        }

        // 边界②：超长提交信息。
        // 走的是 stdin（`-F -`），没有命令行长度限制，
        // 但服务端 pre-commit hook 或日志系统可能有自己的上限，
        // 与其让它报错后不知道为什么，不如提前说一句。
        if msg.chars().count() > 20_000 {
            if let Some(p) = self.commit.as_mut() {
                p.set_hint(
                    "提交信息超过 20000 字，可能触发服务端限制",
                );
            }
            return Ok(());
        }

        // 直接提交，不再弹"确认提交"框。
        //
        // 文件清单在提交框顶部本来就列着（"提交（N 个文件）"+ 列表），
        // 写信息时一直能看到，不需要再确认一次。
        //
        // 弹框本来是为了防手滑，但 Enter 已经改成换行了，
        // 提交必须用 Ctrl+S —— 会是手滑吗？不会，那是明确的动作。
        // 多一层的代价是每次提交都要多按一次键、多看一屏，
        // 而它拦住的那种误触已经不存在了。
        //
        // 提交失败时草稿和勾选都保留（见 BgDone::Msg(Err)），
        // 那才是真正需要兜底的地方。
        self.do_commit_confirmed(msg, paths);
        Ok(())
    }

    /// 真正执行提交。
    fn do_commit_confirmed(&mut self, msg: String, paths: Vec<std::path::PathBuf>) {
        let n = paths.len();
        self.commit = None;
        self.mode = Mode::Browse;

        // 提交前先把信息存成草稿：
        // 失败时（比如网络断了、hook 拒了）用户改一句就想重试，
        // 信息没了得整个重打一遍。存下来，失败后按 C 直接回到这里。
        //
        // 成功时由 BgDone::Msg(Ok) 那侧清掉 —— 那里才知道到底成没成。
        self.commit_draft = Some(msg.clone());
        self.commit_paths = paths.clone();
        self.commit_inflight = true;

        self.spawn_busy("svn commit", move |svn| {
            // 缺失项（!）svn commit 会静默跳过 —— 先 svn rm 调度删除，
            // 否则表现为"提示已提交、仓库没变"。
            let selected: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let missing: Vec<std::path::PathBuf> = svn
                .changed(&Default::default())?
                .into_iter()
                .filter(|e| e.text == crate::domain::StatusKind::Missing)
                .filter(|e| selected.iter().any(|s| s == e.path.as_str()))
                .map(|e| std::path::PathBuf::from(e.path.as_str()))
                .collect();
            if !missing.is_empty() {
                svn.remove(&missing, true)?;
            }

            let out = svn.commit(&msg, &paths)?;
            let tail = out
                .stdout
                .lines()
                .find(|l| l.contains("Committed"))
                .unwrap_or("完成")
                .trim()
                .to_string();

            // 实际提交数以 svn 输出为准，不用勾选数 —— 勾选了不代表真提交了。
            let actual = out
                .stdout
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("Sending")
                        || t.starts_with("Adding")
                        || t.starts_with("Deleting")
                        || t.starts_with("Replacing")
                })
                .count();

            if actual == 0 {
                return Ok(format!(
                    "⚠️ 勾选 {n} 项，但 svn 实际提交了 0 项（{tail}）。\n\
                     缺失文件（!）需先 svn rm；未版本化的新文件需先 svn add。"
                ));
            }
            Ok(format!("已提交 {actual} 项 — {tail}"))
        });
    }

    // ── 渲染 ──────────────────────────────────────────────

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let (_, keys) = self.footer_hint();
        let footer_rows = wrap_hints(keys, area.width.saturating_sub(2) as usize);
        // 保留主体空间；底栏最多三行内容，极矮窗口只用一行。
        let max_rows = area.height.saturating_sub(8).clamp(1, 3) as usize;
        let footer_height = footer_rows.len().min(max_rows).max(1) as u16 + 2;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(3),
                Constraint::Length(footer_height),
            ])
            .split(area);

        self.render_top(f, chunks[0]);
        self.render_body(f, chunks[1]);
        self.render_bottom(f, chunks[2], footer_rows);

        if let Some(notice) = &self.notice {
            let y = chunks[2].y.saturating_sub(1);
            let rect = Rect::new(area.x + 1, y, area.width.saturating_sub(2), 1);
            f.render_widget(Clear, rect);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    notice.as_str(),
                    theme::Theme::title(),
                ))),
                rect,
            );
        }

        // 搜索输入条：浮在底部上方
        if let Some(target) = self.searching {
            let y = chunks[2].y.saturating_sub(2);
            let rect = Rect::new(area.x + 2, y, area.width.saturating_sub(4), 1);
            let label = match target {
                SearchFor::Tree => "搜索文件",
                SearchFor::Log => "搜索提交",
            };
            f.render_widget(Clear, rect);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {}：", label), theme::Theme::props()),
                    Span::raw(self.query.clone()),
                    Span::styled("█", theme::Theme::title()),
                    Span::styled("   Enter 确认 / Esc 取消", theme::Theme::dim()),
                ])),
                rect,
            );
        }

        if let Some(panel) = self.commit.as_mut() {
            panel.render(f, centered_rect(80, 85, area));
        }
        if let Some(panel) = self.diff.as_mut() {
            panel.render(f, centered_rect(92, 92, area));
        }
        if let Some(panel) = self.logpanel.as_mut() {
            panel.render(f, centered_rect(92, 88, area));
        }

        // 日志：加载中 / 失败 / 正常，三种状态都要有明确画面
        if self.mode == Mode::Log && self.logpanel.is_none() {
            let rect = centered_rect(60, 40, area);
            f.render_widget(Clear, rect);

            // 失败优先于"加载中"：错误已经有了，别再转圈
            if let Some((summary, detail)) = &self.log_error {
                let mut lines = vec![
                    Line::from(Span::styled(
                        format!(" {}", summary),
                        Style::default().fg(Color::LightRed),
                    )),
                    Line::from(""),
                ];
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
                f.render_widget(
                    Paragraph::new(lines).block(
                        theme::block()
                            .borders(Borders::ALL)
                            .title(" 读取提交历史失败 ")
                            .border_style(
                                Style::default().fg(Color::LightRed),
                            ),
                    ),
                    rect,
                );
            } else {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        " 正在读取提交历史… ",
                        theme::Theme::title(),
                    )))
                    .block(
                        theme::block()
                            .borders(Borders::ALL)
                            .title(" 提交历史 ")
                            .border_style(theme::Theme::border_active()),
                    ),
                    rect,
                );
            }
        }

        if let Some(panel) = self.confirm.as_mut() {
            panel.render(f, area);
        }
        // 冲突面板自己按 area 算居中，不套 centered_rect
        if let Some(panel) = self.conflict.as_mut() {
            let root = self.root.clone();
            panel.render(f, area, &root);
        }
        if self.mode == Mode::Conflict && self.conflict.is_none() {
            let rect = centered_rect(60, 25, area);
            f.render_widget(Clear, rect);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " 正在扫描冲突… ",
                    theme::Theme::title(),
                )))
                .block(
                    theme::block()
                        .borders(Borders::ALL)
                        .title(" 解决冲突 ")
                        .border_style(theme::Theme::border_active()),
                ),
                rect,
            );
        }

        if let Some(panel) = self.checkout.as_ref() {
            // 16 行是硬下限：外框 2 + 4 个字段 ×3 + 凭证 1 + 提示 1。
            // 终端太矮时按百分比算出的高度会把字段压没（内容区归零，
            // 表现为"输入了却看不见"，极容易误判成键盘坏了）。
            panel.render(f, centered_rect_min(70, 55, 16, area));
        }

        // 仓库浏览：整屏浮层（要显示层级和详情，弹出框尺寸不够）
        if self.mode == Mode::Repo {
            match self.repo.as_mut() {
                Some(p) => p.render(f, centered_rect(92, 90, area)),
                None => {
                    // panel 还没建好（理论上不会），至少别黑屏
                    let rect = centered_rect(60, 25, area);
                    f.render_widget(Clear, rect);
                    f.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            " 正在连接仓库… ",
                            theme::Theme::title(),
                        )))
                        .block(
                            theme::block()
                                .borders(Borders::ALL)
                                .title(" 仓库 ")
                                .border_style(theme::Theme::border_active()),
                        ),
                        rect,
                    );
                }
            }
        }

        if self.show_help {
            HelpPanel::render(f, centered_rect(70, 80, area));
        }
    }

    fn render_top(&mut self, f: &mut Frame, area: Rect) {
        let watch = if self.refresh_watch.is_some() {
            "自动刷新"
        } else {
            "手动刷新"
        };
        let age = self
            .last_refresh
            .map(|t| format!(" · {}秒前", t.elapsed().as_secs()))
            .unwrap_or_default();
        let mut header = vec![Span::styled(
            format!(" {}{} ", watch, age),
            theme::Theme::dim(),
        )];
        if self.scan_failed {
            header.push(Span::styled(" 扫描失败 · R 重试 ", theme::for_sign('C')));
        }
        if let Some(error) = self.watch_error.as_ref() {
            header.push(Span::styled(
                format!(" 监听不可用：{} · R 刷新 ", tail_of(error, 32)),
                theme::for_sign('!'),
            ));
        }

        let mut spans = Vec::new();

        let mut counts: Vec<(char, usize)> = Vec::new();
        for sign in ['C', 'T', '!', 'D', 'R', 'A', 'M', '?'] {
            let n = self.items.iter().filter(|i| i.sign == sign).count();
            if n > 0 {
                counts.push((sign, n));
            }
        }
        if counts.is_empty() {
            let text = if self.scan_failed {
                " 上次扫描无变更 "
            } else {
                " 工作副本干净 "
            };
            spans.push(Span::styled(text, theme::Theme::clean()));
        } else {
            for (sign, n) in counts {
                spans.push(Span::styled(
                    format!(
                        " {}{} ",
                        match sign {
                            'C' => "冲突",
                            'T' => "树冲突",
                            '!' => "缺失",
                            'D' => "删除",
                            'R' => "替换",
                            'A' => "新增",
                            'M' => "修改",
                            '?' => "未纳管",
                            _ => "其他",
                        },
                        n
                    ),
                    theme::for_sign(sign),
                ));
            }
        }

        spans.push(Span::styled(
            format!(" 已勾选{} ", self.tree.checked_count()),
            theme::Theme::checked(),
        ));

        // 搜索中：顶栏显示当前过滤
        if let Some(f) = self.tree.filter() {
            if !f.is_empty() {
                spans.push(Span::styled(
                    format!(" 过滤：{} ", f),
                    theme::Theme::props(),
                ));
            }
        }

        spans.push(Span::styled(
            format!(" 排序：{} ", self.sort.label()),
            theme::Theme::dim(),
        ));

        if let Some(busy) = &self.busy {
            // 帧号只由"过了多少毫秒"推出来，不存计数器 —— 理由见 loading 模块。
            let ms = self
                .busy_start
                .map(|t| t.elapsed().as_millis())
                .unwrap_or(0);

            // 检出才有：已落盘条目数 + 最后一项（别的后台任务没这个信息）。
            // 拼成 detail 交给组件，顶栏不再自己拼动画。
            let detail = self.busy_prog.as_ref().map(|(n, last)| {
                if last.is_empty() {
                    format!("{} 项", n)
                } else {
                    format!("{} 项 …{}", n, tail_of(last, 24))
                }
            });

            // 之前这里是手写的拼接，还用过静止的 `⟳`（跟色块没区别）。
            // 现在交给统一的 loading 组件：别处（整屏等待框）调用同一份
            // 帧计算，动起来必然是同一个节奏。
            header.extend(
                loading::Loading::new(busy.as_str(), ms)
                    .detail(detail.as_deref())
                    .spans(),
            );
        }

        header.push(Span::styled(
            format!(" {} ", tail_of(&self.title, 40)),
            theme::Theme::status_bar(),
        ));
        f.render_widget(
            Paragraph::new(vec![Line::from(header), Line::from(spans)]),
            area,
        );
    }

    fn render_body(&mut self, f: &mut Frame, area: Rect) {
        let (tree_area, preview_area) = body_regions(area);
        self.tree.render(f, tree_area);
        self.preview_visible = preview_area.is_some();
        let Some(preview_area) = preview_area else {
            if matches!(self.drag, Some(DragBar::Preview(_))) {
                self.drag = None;
            }
            return;
        };

        match self.preview.as_mut() {
            Some(p) => p.render_pane(f, preview_area),
            None => {
                let block = theme::block()
                    .borders(Borders::ALL)
                    .title(" 预览 ")
                    .border_style(theme::Theme::border());
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        " 选中文件查看内容或改动",
                        theme::Theme::dim(),
                    )))
                    .block(block),
                    preview_area,
                );
            }
        }
    }

    /// 按选中项给出操作提示；底栏在渲染时按宽度换行。
    fn browse_hint(&self) -> &'static str {
        if self.searching.is_some() {
            return "输入过滤词   Enter 确认   Esc 清空   自动刷新暂缓";
        }
        if self.busy.is_some() {
            return "后台任务中   ↑↓ 选择   Space 勾选   q 退出   其他操作请稍候";
        }
        match self.tree.selected() {
            Some(n) if n.sign == '?' => {
                "a 添加当前   A 批量添加   Enter 打开   R 刷新   ? 帮助   q 退出"
            }
            Some(n) if n.is_dir => {
                "Enter 展开/折叠   r 撤销目录改动   a 添加目录内新增   T 切换视图   R 刷新   ? 帮助   q 退出"
            }
            Some(n) if matches!(n.sign, 'C' | 'T') => {
                "冲突 · X 处理冲突   Space 勾选   Enter 查看   R 刷新   ? 帮助"
            }
            Some(n) if crate::tui::panels::tree::is_committable_sign(n.sign) => {
                "Space 勾选   C 提交   r 撤销改动   Enter 改动   E 编辑   R 刷新   ? 帮助   q 退出"
            }
            Some(_) => "Enter 查看   E 编辑   T 变更视图   R 刷新   / 搜索   ? 帮助   q 退出",
            None if self.tree.filter().is_some() => {
                "无匹配项 · / 修改过滤词   Esc 清空过滤   R 刷新   ? 帮助"
            }
            None => "暂无变更 · T 全部文件   R 刷新   V 浏览仓库   ? 帮助   q 退出",
        }
    }

    fn footer_hint(&self) -> (&'static str, &'static str) {
        match self.mode {
            Mode::Browse => (" 操作 ", self.browse_hint()),
            Mode::Repo => (
                " 仓库 ",
                "↑↓ 选择   Enter 进入   ←/退格 返回   o 检出   Y 复制路径   R 刷新   Esc 返回",
            ),
            // "Esc 取消" 改成 "Esc 退出"：
            // 有内容时 Esc 会先弹确认框问一句，但**退出永远是可达的**
            // （确认框里按 Y 就走）。写"取消"会让人以为按了没反应。
            Mode::Commit => (
                " 提交 ",
                "Ctrl+S 提交   Esc 退出   范围：文件树上用 Space 勾",
            ),
            Mode::Diff => {
                let what = match self.diff.as_ref().map(|p| p.kind()) {
                    Some(PreviewKind::File) => " 内容 ",
                    _ => " 改动 ",
                };
                (what, "↑↓ 滚动   H/L 左右   < > 翻页   0 回最左   Esc 返回")
            }
            Mode::Log => (" 历史 ", "↑↓ 选版本   R 回退到此版本   / 搜索   Esc 返回"),
            // 必须和确认面板内部的 Y/N 一致。
            // 之前这里写"Enter 确认 Esc 取消"，跟面板里的"Y/N"是两套说法，
            // 用户按 Esc 以为能退出，实际被送回上一层 —— 两边矛盾最害人。
            Mode::Confirm
                if self.confirm.as_ref().is_some_and(|panel| {
                    matches!(panel.action(), ConfirmAction::Revert(_))
                }) =>
            {
                (" 丢弃本地改动 ", "Y 丢弃   Enter/N/Esc 取消   j/k 滚动")
            }
            Mode::Confirm => (" 确认 ", "Y 确认   N 取消（Esc）   j/k 滚动"),
            Mode::Conflict => (
                " 解决冲突 ",
                "j/k 选文件   1/2/3 切版本   M 用我的   T 用服务器的   Enter 保留当前   Esc 返回",
            ),
            Mode::Checkout => (" 检出 ", "Tab 切换   Enter 确认   Ctrl+U 清空   Esc 退出"),
        }
    }

    fn render_bottom(&mut self, f: &mut Frame, area: Rect, rows: Vec<String>) {
        let (title, _) = self.footer_hint();
        let visible_rows = area.height.saturating_sub(2) as usize;
        let title = if rows.len() > visible_rows {
            format!("{} · ? 帮助 ", title.trim_end())
        } else {
            title.to_string()
        };
        let block = theme::block()
            .borders(Borders::ALL)
            .title(title)
            .border_style(theme::Theme::border());

        f.render_widget(
            Paragraph::new(
                rows.into_iter()
                    .take(visible_rows)
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            )
            .style(theme::Theme::dim())
            .block(block),
            area,
        );
    }
}

/// 宽窗口保持左右布局；窄窗口上下排列，空间不足时保留完整文件树。
fn body_regions(area: Rect) -> (Rect, Option<Rect>) {
    let direction = if area.width >= 100 && area.height >= 6 {
        Direction::Horizontal
    } else if area.width >= 40 && area.height >= 12 {
        Direction::Vertical
    } else {
        return (area, None);
    };
    let regions = Layout::default()
        .direction(direction)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    (regions[0], Some(regions[1]))
}

/// 优先保持“按键 + 含义”为一组；组本身过长时才按显示宽度拆开，兼容中文。
fn wrap_hints(keys: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for group in keys.split("   ").filter(|g| !g.is_empty()) {
        let group_width = Line::from(group).width();
        if !line.is_empty() && used + 3 + group_width > width {
            rows.push(std::mem::take(&mut line));
            used = 0;
        }
        if !line.is_empty() {
            line.push_str("   ");
            used += 3;
        }
        for ch in group.chars() {
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
    }
    if !line.is_empty() {
        rows.push(line);
    }
    rows
}

/// 把 svn 输出里的绝对路径前缀去掉，让 diff 更好读。
///
/// 我们给 svn 传的是绝对路径（保证一定能找到文件），但 svn 会把它原样写进
/// `Index:` / `---` / `+++` 行，于是一屏 diff 全是长路径。这里换成相对路径。
///
/// macOS 上要多处理一种：工作副本根可能是 `/private/var/...`（canonicalize 后），
/// 而 svn 输出用 `/var/...`。两种前缀都替换。
/// 截尾：只留最后 `n` 个字符（`pretty_path` 相反，那个留头）。
///
/// ⚠️ 必须按 `char` 切：路径里常有中文，`&s[s.len()-n..]` 会切在 UTF-8
///    字符中间，**直接 panic**。这是顶栏每帧都跑的代码，panic 就是界面崩掉。
fn tail_of(s: &str, n: usize) -> String {
    let v: Vec<char> = s.chars().collect();
    if v.len() <= n {
        return s.to_string();
    }
    v[v.len() - n..].iter().collect()
}

fn shorten_paths(text: &str, root: &str) -> String {
    if root.is_empty() {
        return text.to_string();
    }
    let mut out = text.replace(&format!("{}/", root), "");

    // macOS：/private/var → /var
    if let Some(stripped) = root.strip_prefix("/private") {
        out = out.replace(&format!("{}/", stripped), "");
    }
    out
}

/// 把任意错误转成（摘要, 建议）两行。
///
/// `Error::Explained` 已经是翻译过的（svn 层接了 `with_explanation`），
/// 直接用；其他类型在这里补一句"该干什么"。
/// 关键是**每条都给下一步动作** —— 只说"失败了"没有任何帮助。
fn humanize_error(e: &crate::domain::Error) -> (String, String) {
    use crate::domain::Error;
    match e {
        Error::Explained { summary, detail } => (summary.clone(), detail.clone()),
        Error::NotWorkingCopy(p) => (
            "这里不是 SVN 工作副本".into(),
            format!("{} 及其上级都没有 .svn。cd 到工作副本里再试。", p.display()),
        ),
        Error::Locked(_) => (
            "工作副本被锁".into(),
            "上次操作异常中断留下了锁。退出后跑 svn cleanup，再重新打开。".into(),
        ),
        Error::Timeout(n) => (
            format!("操作超时（{} 秒）", n),
            "仓库太大或服务器太慢。可以先跑 svnui tui 时避开高峰，或让管理员放宽超时。".into(),
        ),
        Error::SvnNotFound => (
            "找不到 svn 命令".into(),
            "没装 Subversion 命令行工具，或不在 PATH 里。\n用 SVNR_SVN=/path/to/svn 指定。".into(),
        ),
        Error::SvnFailed { code, stderr } => {
            // 兜底：svn 层没识别出来的错误码，这里再试一次
            match crate::domain::explain(stderr) {
                Some((summary, detail)) => (summary, detail),
                None => (
                    format!("svn 失败（退出码 {}）", code),
                    if stderr.trim().is_empty() {
                        "没有错误输出。".to_string()
                    } else {
                        stderr.trim().to_string()
                    },
                ),
            }
        }
        other => (other.to_string(), "".to_string()),
    }
}

/// 把路径整理成"人看的形态"。
///
/// 处理三件事，顺序有讲究：
///
/// 1. **percent-decode** —— svn 对非 ASCII 路径会输出百分号编码
///    （`%E4%B8%AD%E6%96%87.txt`），必须先解回来，否则后面的
///    前缀匹配和省略都会作用在乱码上。
/// 2. **去掉工作副本根前缀** —— `/Users/me/proj/src/a.rs` → `src/a.rs`。
///    macOS 上 svn 会把 `/var` 规范成 `/private/var`，两种写法都要认。
/// 3. **中段省略** —— 剩下的还是太长就压中间，保留头尾。
///    头尾比中间有用：头部是目录（知道在哪），尾部是文件名（知道是什么）。
pub fn pretty_path(p: &str, root: &std::path::Path, max: usize) -> String {
    // ① percent-decode
    let mut s = percent_decode(p);

    // ② 去根前缀
    let root_s = root.to_string_lossy().to_string();
    if !root_s.is_empty() {
        let mut cands = vec![format!("{}/", root_s)];
        // macOS：svn canonicalize 过，可能是 /private 开头
        if let Some(stripped) = root_s.strip_prefix("/private") {
            cands.push(format!("{}/", stripped));
        } else {
            cands.push(format!("/private{}/", root_s));
        }
        for c in cands {
            if let Some(rest) = s.strip_prefix(&c) {
                s = rest.to_string();
                break;
            }
        }
        // 完全等于 root 本身
        if s == root_s {
            s = ".".to_string();
        }
    }

    // ③ 中段省略
    if s.chars().count() <= max {
        return s;
    }
    let cs: Vec<char> = s.chars().collect();
    let head = max / 3;
    let tail = max.saturating_sub(head + 3); // 3 = "…" 的显示宽度（这里按 1 字符算）
    let tail = tail.max(1);
    if head + tail >= cs.len() {
        return s;
    }
    let h: String = cs[..head].iter().collect();
    let t: String = cs[cs.len() - tail..].iter().collect();
    format!("{}…{}", h, t)
}

/// 最小 percent-decode：`%XX` → 字节，再按 UTF-8 拼回来。
///
/// 不引 percent-encoding crate —— 只需要这一种转换，手写 30 行
/// 比多一个依赖划算。解码失败（非法序列）就保留原样，
/// 宁可显示得难看也不能丢信息。
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 写进系统剪贴板。
///
/// macOS 有 `pbcopy`；Linux 按优先级试 `wl-copy` / `xclip` / `xsel`，
/// 都没有就返回错误 —— 交给调用方决定怎么提示，别静默失败
/// （用户以为复制了，粘出来还是旧内容，比直接说失败更糟）。
fn copy_to_clipboard(text: &str) -> crate::domain::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };

    let mut last: Option<String> = None;
    for (prog, args) in candidates {
        let mut child = match Command::new(prog)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => continue,
        };
        let ok = child
            .stdin
            .as_mut()
            .map(|s| s.write_all(text.as_bytes()).is_ok())
            .unwrap_or(false);
        if ok && child.wait().map(|s| s.success()).unwrap_or(false) {
            return Ok(());
        }
        last = Some(format!("{} 执行失败", prog));
    }
    Err(crate::domain::Error::Explained {
        summary: "复制失败：系统里没有可用的剪贴板工具".to_string(),
        detail: format!(
            "试过：{}{}。\nmacOS 自带 pbcopy；Linux 请装 wl-copy 或 xclip。\n要复制的内容是：\n{}",
            candidates.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(" / "),
            last.map(|s| format!("（{}）", s)).unwrap_or_default(),
            text,
        ),
    })
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

/// 同 `centered_rect`，但保证至少 `min_h` 行。
///
/// 按百分比算出来的高度在矮终端上会不够，而 checkout 面板的字段是
/// 固定 3 行一块（Border 上下各 1 + 内容 1），压一点就整块消失。
/// 这里在百分比结果不够时竖向撑到 min_h，横向不动。
fn centered_rect_min(percent_x: u16, percent_y: u16, min_h: u16, r: Rect) -> Rect {
    let out = centered_rect(percent_x, percent_y, r);
    if out.height >= min_h || r.height <= out.height {
        return out;
    }
    let h = min_h.min(r.height);
    let y = r.y + (r.height - h) / 2;
    Rect { y, height: h, ..out }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn decodes_percent_encoding() {
        // svn 对非 ASCII 路径会输出百分号编码
        assert_eq!(percent_decode("%E4%B8%AD%E6%96%87.txt"), "中文.txt");
        assert_eq!(percent_decode("a%20b.txt"), "a b.txt");
    }

    #[test]
    fn decode_keeps_invalid_sequences() {
        // 非法序列不能丢信息，保留原样
        assert_eq!(percent_decode("a%ZZb"), "a%ZZb");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn strips_root_prefix() {
        let root = std::path::Path::new("/Users/me/proj");
        assert_eq!(
            pretty_path("/Users/me/proj/src/a.rs", root, 80),
            "src/a.rs"
        );
    }

    #[test]
    fn handles_macos_private_prefix() {
        // svn canonicalize 后是 /private/var，输入可能是 /var
        let root = std::path::Path::new("/private/var/wc");
        assert_eq!(pretty_path("/var/wc/a.txt", root, 80), "a.txt");
    }

    #[test]
    fn elides_middle_when_too_long() {
        let root = std::path::Path::new("/wc");
        let long = "/wc/aaaa/bbbb/cccc/dddd/eeee/ffff/gggg/hhhh.txt";
        let out = pretty_path(long, root, 20);
        assert!(out.chars().count() <= 20, "超长了: {}", out);
        assert!(out.contains('…'), "应该有省略号: {}", out);
        assert!(out.ends_with("hhhh.txt"), "尾部文件名要保留: {}", out);
        assert!(out.starts_with("aaaa"), "头部目录要保留: {}", out);
    }

    #[test]
    fn short_path_is_untouched() {
        let root = std::path::Path::new("/wc");
        assert_eq!(pretty_path("/wc/a.txt", root, 80), "a.txt");
    }

    #[test]
    fn root_itself_becomes_dot() {
        let root = std::path::Path::new("/wc");
        assert_eq!(pretty_path("/wc", root, 80), ".");
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn wide_windows_use_columns_and_narrow_windows_use_rows() {
        let (tree, preview) = body_regions(Rect::new(5, 3, 140, 24));
        let preview = preview.unwrap();
        assert_eq!(tree.y, preview.y);
        assert_eq!(tree.right(), preview.x);
        assert!(tree.width >= 40);
        let (tree, preview) = body_regions(Rect::new(5, 3, 80, 16));
        let preview = preview.unwrap();
        assert_eq!(tree.width, 80);
        assert_eq!(preview.width, 80);
        assert_eq!(tree.bottom(), preview.y);
        for area in [Rect::new(0, 0, 140, 5), Rect::new(0, 0, 30, 20)] {
            assert_eq!(body_regions(area), (area, None));
        }
    }

    #[test]
    fn regions_stay_inside_the_terminal_through_resizes() {
        for width in [1, 20, 39, 40, 80, 99, 100, 140] {
            for height in [1, 5, 6, 11, 12, 24] {
                let area = Rect::new(2, 3, width, height);
                let (tree, preview) = body_regions(area);
                for region in std::iter::once(tree).chain(preview) {
                    assert!(region.x >= area.x && region.y >= area.y);
                    assert!(region.right() <= area.right() && region.bottom() <= area.bottom());
                }
            }
        }
    }

    #[test]
    fn hints_wrap_by_display_width_without_losing_keys_or_chinese() {
        let keys = "未纳管 · A 添加全部新增项   Enter 打开   R 刷新   ? 帮助   q 退出";
        let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        for width in [10, 18, 28, 38, 78, 138] {
            let rows = wrap_hints(keys, width);
            assert!(rows
                .iter()
                .all(|row| Line::from(row.as_str()).width() <= width));
            assert_eq!(compact(&rows.join("")), compact(keys));
        }
        assert_eq!(wrap_hints("R 刷新   ? 帮助", 10), vec!["R 刷新", "? 帮助"]);
        assert!(wrap_hints(keys, 0).is_empty());
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn run(exe: &str, args: &[&str], cwd: &Path) {
        let out = Command::new(exe).args(args).current_dir(cwd).output().unwrap();
        assert!(out.status.success(), "{exe}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn wait_for_scan(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.is_busy() {
            assert!(Instant::now() < deadline, "扫描未在期限内结束");
            app.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
    fn unversioned_files_are_visible_before_add_and_committable_after_add() {
        let tmp = tempfile::tempdir().unwrap();
        run("svnadmin", &["create", "repo"], tmp.path());
        let url = format!("file://{}/repo", tmp.path().display());
        run("svn", &["checkout", "--non-interactive", &url, "wc"], tmp.path());
        let wc = tmp.path().join("wc");
        let plugin = wc.join("hardware-plugin");
        std::fs::create_dir(&plugin).unwrap();
        run("svn", &["add", "hardware-plugin"], &wc);
        run("svn", &["commit", "--non-interactive", "-m", "init"], &wc);

        std::fs::write(plugin.join("kingdom.ico"), b"test icon").unwrap();
        let svn = Svn::discover(&wc, Duration::from_secs(5), false).unwrap();
        let mut app = App::new(svn).unwrap();
        assert_eq!(app.items.len(), 1, "启动时必须显示未版本化的新文件");
        assert_eq!(app.items[0].sign, '?');

        std::fs::write(plugin.join("升级说明.md"), "新增说明\n").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        assert_eq!(app.items.len(), 2, "刷新必须显示运行期间新增的文件");
        for item in &app.items {
            assert_eq!(item.sign, '?');
            assert!(!item.committable());
            assert!(!app.tree.is_checkable(&item.rel));
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE)).unwrap();
        assert_eq!(app.mode, Mode::Browse, "未版本化文件不能直接进入提交面板");
        assert_eq!(app.tree.checked_count(), 0);

        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE)).unwrap();
        assert_eq!(app.mode, Mode::Confirm);
        assert!(!app.is_busy(), "确认前不能执行 svn add");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        assert_eq!(app.items.len(), 2);
        for item in &app.items {
            assert_eq!(item.sign, 'A', "加入 SVN 后必须更新为 A");
            assert!(app.tree.is_checkable(&item.rel));
        }
        assert_eq!(std::fs::read_to_string(plugin.join("升级说明.md")).unwrap(), "新增说明\n");
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
    fn refresh_keys_detect_external_edits_and_clear_restored_files() {
        let tmp = tempfile::tempdir().unwrap();
        run("svnadmin", &["create", "repo"], tmp.path());
        let url = format!("file://{}/repo", tmp.path().display());
        run("svn", &["checkout", "--non-interactive", &url, "wc"], tmp.path());
        let wc = tmp.path().join("wc");
        let file = wc.join("a.txt");
        std::fs::write(&file, "original\n").unwrap();
        run("svn", &["add", "a.txt"], &wc);
        run("svn", &["commit", "--non-interactive", "-m", "init"], &wc);
        let svn = Svn::discover(&wc, Duration::from_secs(5), false).unwrap();
        let mut app = App::new(svn).unwrap();
        assert!(app.items.is_empty());

        // 走真实按键分发和后台扫描，防止文档有快捷键、实际分支却未绑定。
        for key in [KeyCode::Char('R'), KeyCode::F(5)] {
            std::fs::write(&file, "changed externally\n").unwrap();
            app.handle_key(KeyEvent::new(key, KeyModifiers::NONE)).unwrap();
            assert!(app.is_busy(), "{key:?} 必须启动状态扫描");
            wait_for_scan(&mut app);
            assert_eq!(app.items.len(), 1, "{key:?} 必须显示外部改动");
            assert_eq!(app.items[0].rel, "a.txt");
            assert_eq!(app.items[0].sign, 'M');
            assert_eq!(app.mode, Mode::Browse, "刷新不能触发还原面板");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "changed externally\n");
            app.tree.set_checked("a.txt", true);

            std::fs::write(&file, "original\n").unwrap();
            app.handle_key(KeyEvent::new(key, KeyModifiers::NONE)).unwrap();
            wait_for_scan(&mut app);
            assert!(app.items.is_empty(), "恢复原内容后必须清除 M 状态");
            assert_eq!(app.tree.checked_count(), 0, "清除过期提交勾选");
        }
    }

    fn temporary_app() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        run("svnadmin", &["create", "repo"], tmp.path());
        let url = format!("file://{}/repo", tmp.path().display());
        run(
            "svn",
            &["checkout", "--non-interactive", &url, "wc"],
            tmp.path(),
        );
        let wc = tmp.path().join("wc");
        std::fs::write(wc.join("a.txt"), "original\n").unwrap();
        run("svn", &["add", "a.txt"], &wc);
        run("svn", &["commit", "--non-interactive", "-m", "init"], &wc);
        let svn = Svn::discover(&wc, Duration::from_secs(5), false).unwrap();
        let app = App::new(svn).unwrap();
        assert!(app.refresh_watch.is_some(), "临时工作副本必须成功启动监听");
        (tmp, app)
    }

    fn wait_until(app: &mut App, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.tick();
            if done(app) {
                return;
            }
            assert!(Instant::now() < deadline, "自动刷新未在期限内完成");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn tick_for(app: &mut App, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            app.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；验证真实文件事件，仅操作临时仓库"]
    fn automatic_refresh_detects_edits_new_files_and_external_svn_add() {
        let (_tmp, mut app) = temporary_app();
        let root = app.root.clone();
        std::fs::write(root.join("a.txt"), "changed externally\n").unwrap();
        wait_until(&mut app, |a| {
            a.items.iter().any(|i| i.rel == "a.txt" && i.sign == 'M')
        });
        app.tree.set_checked("a.txt", true);
        std::fs::write(root.join("新增说明.txt"), "new\n").unwrap();
        wait_until(&mut app, |a| {
            a.items
                .iter()
                .any(|i| i.rel == "新增说明.txt" && i.sign == '?')
        });
        assert_eq!(app.tree.selected().unwrap().rel, "a.txt");
        assert!(app.tree.is_checked("a.txt"));
        assert_eq!(
            app.browse_hint(),
            "Space 勾选   C 提交   r 撤销改动   Enter 改动   E 编辑   R 刷新   ? 帮助   q 退出"
        );
        run("svn", &["add", "新增说明.txt"], &root);
        wait_until(&mut app, |a| {
            a.items
                .iter()
                .any(|i| i.rel == "新增说明.txt" && i.sign == 'A')
        });
        assert!(app.tree.is_checkable("新增说明.txt"));
        std::fs::write(root.join("a.txt"), "original\n").unwrap();
        wait_until(&mut app, |a| !a.items.iter().any(|i| i.rel == "a.txt"));
        assert_eq!(app.tree.checked_count(), 0);
        // 扫描和预览的读访问不应形成自动刷新循环。
        tick_for(&mut app, Duration::from_millis(1200));
        let scanned = app.last_refresh;
        tick_for(&mut app, Duration::from_millis(800));
        assert_eq!(app.last_refresh, scanned, "空闲时不应不断重扫");
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；验证提交范围冻结及后台操作互斥"]
    fn automatic_refresh_waits_for_commit_and_busy_operations() {
        let (_tmp, mut app) = temporary_app();
        let root = app.root.clone();
        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        wait_until(&mut app, |a| !a.items.is_empty());
        app.tree.set_checked("a.txt", true);
        app.handle_key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.mode, Mode::Commit);
        let paths = app.commit_paths.clone();
        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        tick_for(&mut app, Duration::from_millis(800));
        assert!(!app.items.iter().any(|i| i.rel == "new.txt"));
        assert_eq!(app.commit_paths, paths);
        assert!(!app.is_busy());

        app.mode = Mode::Browse;
        app.busy = Some("svn update".into());
        tick_for(&mut app, Duration::from_millis(500));
        for code in [KeyCode::Char('A'), KeyCode::Char('C'), KeyCode::Char('R')] {
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
                .unwrap();
        }
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.busy.as_deref(), Some("svn update"));
        assert!(!app.items.iter().any(|i| i.rel == "new.txt"));
        app.busy = None;
        wait_until(&mut app, |a| a.items.iter().any(|i| i.rel == "new.txt"));
        assert_eq!(app.commit_paths, paths);
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；验证搜索延后刷新和终端渲染"]
    fn search_defers_refresh_and_narrow_terminals_render() {
        let (_tmp, mut app) = temporary_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .unwrap();
        app.handle_paste("new".into());
        std::fs::write(app.root.join("new.txt"), "new").unwrap();
        tick_for(&mut app, Duration::from_millis(800));
        assert!(app.items.is_empty());
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        wait_until(&mut app, |a| !a.items.is_empty());
        assert_eq!(app.tree.filter().map(String::as_str), Some("new"));
        assert!(app.browse_hint().contains("a 添加当前"));
        app.notice = None;
        for (width, height) in [(140, 24), (100, 20), (80, 24), (40, 20), (30, 10)] {
            let backend = ratatui::backend::TestBackend::new(width, height);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|f| app.render(f, f.area())).unwrap();
            // 宽字符的续格在 TestBackend 中是空格，去掉空白后检查可见文本。
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .flat_map(|c| c.symbol().chars())
                .filter(|c| !c.is_whitespace())
                .collect();
            assert!(text.contains("自动刷新"));
            assert!(text.contains("未纳管1"));
            if width >= 40 {
                assert!(text.contains("a添加当前"));
                assert!(text.contains("A批量添加"));
                assert!(text.contains("?帮助"));
                assert!(text.contains("q退出"));
            }
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(app.tree.filter().is_none());
        assert!(!app.should_quit());
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.should_quit());
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；验证缩窗后的鼠标与全屏预览"]
    fn hidden_preview_does_not_capture_mouse_and_remains_openable() {
        let (_tmp, mut app) = temporary_app();
        std::fs::write(app.root.join("new.txt"), "new").unwrap();
        wait_until(&mut app, |a| !a.items.is_empty() && a.preview.is_some());
        // 用足够多的行保证旧布局中确实存在可命中的滚动条。
        app.preview = Some(PreviewPanel::file("new.txt", &"preview line\n".repeat(100)));
        app.notice = None;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 24)).unwrap();
        terminal.draw(|f| app.render(f, f.area())).unwrap();
        assert!(app.preview_visible);
        let old_hit = (0..24)
            .find_map(|y| {
                (0..140).find_map(|x| {
                    matches!(app.hit_bar(x, y), Some(DragBar::Preview(_))).then_some((x, y))
                })
            })
            .expect("宽窗口应有预览滚动条");
        app.drag = Some(DragBar::Preview(BarHit::Vertical));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 10)).unwrap();
        terminal.draw(|f| app.render(f, f.area())).unwrap();
        assert!(!app.preview_visible);
        assert!(app.drag.is_none());
        assert!(app.hit_bar(old_hit.0, old_hit.1).is_none());
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.mode, Mode::Diff);
        terminal.draw(|f| app.render(f, f.area())).unwrap();
    }


    fn select_path(app: &mut App, rel: &str) {
        while app.tree.move_up() {}
        loop {
            if app.tree.selected().is_some_and(|n| n.rel == rel) {
                return;
            }
            assert!(app.tree.move_down(), "找不到可见目标：{rel}");
        }
    }

    fn fresh_sign(app: &App, rel: &str) -> char {
        App::fetch(&app.svn, app.sort)
            .unwrap()
            .0
            .into_iter()
            .find(|i| i.rel == rel)
            .unwrap_or_else(|| panic!("找不到状态：{rel}"))
            .sign
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；只添加当前文件，不添加其他新增项"]
    fn lowercase_add_only_adds_the_selected_unversioned_file() {
        let (_tmp, mut app) = temporary_app();
        std::fs::write(app.root.join("first.txt"), "first").unwrap();
        std::fs::write(app.root.join("second.txt"), "second").unwrap();
        app.reload_sync().unwrap();
        app.tree.set_filter(Some("first".into()));
        select_path(&mut app, "first.txt");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.is_busy());
        wait_for_scan(&mut app);
        assert_eq!(fresh_sign(&app, "first.txt"), 'A');
        assert_eq!(fresh_sign(&app, "second.txt"), '?');
        assert_eq!(app.tree.filter().map(String::as_str), Some("first"));
        // 已加入的文件再次按 a，不应把别的新增项一起添加。
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .unwrap();
        assert!(!app.is_busy());
        assert_eq!(fresh_sign(&app, "second.txt"), '?');
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；批量取消不写入，确认只执行冻结的显式目标"]
    fn uppercase_add_confirms_all_scanned_targets_even_outside_filter() {
        let (_tmp, mut app) = temporary_app();
        std::fs::write(app.root.join("first.txt"), "first").unwrap();
        std::fs::write(app.root.join("second.txt"), "second").unwrap();
        app.reload_sync().unwrap();
        app.tree.set_filter(Some("first".into()));
        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.mode, Mode::Confirm);
        let expected = vec![app.root.join("first.txt"), app.root.join("second.txt")];
        match app.confirm.as_ref().unwrap().action() {
            ConfirmAction::Add(paths) => assert_eq!(paths, &expected),
            _ => panic!("应为添加确认"),
        }
        assert!(!app.is_busy());
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(fresh_sign(&app, "first.txt"), '?');
        assert_eq!(fresh_sign(&app, "second.txt"), '?');

        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE))
            .unwrap();
        std::fs::write(app.root.join("late.txt"), "late").unwrap();
        tick_for(&mut app, Duration::from_millis(500));
        match app.confirm.as_ref().unwrap().action() {
            ConfirmAction::Add(paths) => assert_eq!(paths, &expected),
            _ => panic!("确认期间不能替换添加目标"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        wait_for_scan(&mut app);
        assert_eq!(fresh_sign(&app, "first.txt"), 'A');
        assert_eq!(fresh_sign(&app, "second.txt"), 'A');
        assert_eq!(fresh_sign(&app, "late.txt"), '?');
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；已纳管父目录只添加范围内新增项"]
    fn directory_add_confirms_descendants_without_adding_sibling_prefixes() {
        let (_tmp, mut app) = temporary_app();
        let root = app.root.clone();
        std::fs::create_dir_all(root.join("group/inner")).unwrap();
        run("svn", &["add", "group"], &root);
        run(
            "svn",
            &["commit", "--non-interactive", "-m", "directories"],
            &root,
        );
        for rel in ["group/one.txt", "group/inner/说明.md", "group2.txt"] {
            std::fs::write(root.join(rel), "new").unwrap();
        }
        app.reload_sync().unwrap();
        select_path(&mut app, "group");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.mode, Mode::Confirm);
        let mut expected = vec![root.join("group/one.txt"), root.join("group/inner/说明.md")];
        expected.sort();
        match app.confirm.as_ref().unwrap().action() {
            ConfirmAction::Add(paths) => assert_eq!(paths, &expected),
            _ => panic!("应确认目录内新增项"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(fresh_sign(&app, "group/one.txt"), '?');
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        wait_for_scan(&mut app);
        assert_eq!(fresh_sign(&app, "group/one.txt"), 'A');
        assert_eq!(fresh_sign(&app, "group/inner/说明.md"), 'A');
        assert_eq!(fresh_sign(&app, "group2.txt"), '?');
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；未纳管目录必须确认，确认后递归添加"]
    fn unversioned_directory_is_detected_and_confirmed_before_recursive_add() {
        let (_tmp, mut app) = temporary_app();
        std::fs::create_dir_all(app.root.join("bundle/nested")).unwrap();
        std::fs::write(app.root.join("bundle/nested/new.txt"), "new").unwrap();
        app.reload_sync().unwrap();
        select_path(&mut app, "bundle");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.mode, Mode::Confirm, "未知 kind 的目录不能直接执行添加");
        assert_eq!(fresh_sign(&app, "bundle"), '?');
        match app.confirm.as_ref().unwrap().action() {
            ConfirmAction::Add(paths) => assert_eq!(paths, &vec![app.root.join("bundle")]),
            _ => panic!("应为目录添加确认"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        wait_for_scan(&mut app);
        assert_eq!(fresh_sign(&app, "bundle/nested/new.txt"), 'A');
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；只操作临时仓库，验证新鲜状态、取消和明确确认"]
    fn revert_current_uses_fresh_status_and_enter_cancels() {
        let (_tmp, mut app) = temporary_app();
        let path = app.root.join("a.txt");
        std::fs::write(&path, "changed\n").unwrap();
        app.reload_sync().unwrap();
        select_path(&mut app, "a.txt");
        // 扫描后外部恢复原内容：r 必须重新查询，不能用旧的 M 状态生成确认框。
        std::fs::write(&path, "original\n").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.confirm.is_none());
        assert!(app.notice.as_ref().unwrap().contains("没有可撤销"));

        std::fs::write(&path, "keep until confirmed\n").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        assert_eq!(app.mode, Mode::Confirm);
        match app.confirm.as_ref().unwrap().action() {
            ConfirmAction::Revert(paths) => assert_eq!(paths, &vec![path.clone()]),
            _ => panic!("应为本地改动确认"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "keep until confirmed\n"
        );
        assert!(!app.is_busy());
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE))
            .unwrap();
        wait_for_scan(&mut app);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");
        assert!(app.items.is_empty());
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；勾选包含过滤外项目，光标项不扩大范围"]
    fn revert_checked_includes_hidden_items_and_keeps_unchecked_changes() {
        let (_tmp, mut app) = temporary_app();
        let root = app.root.clone();
        for rel in ["b.txt", "c.txt"] {
            std::fs::write(root.join(rel), "original\n").unwrap();
        }
        run("svn", &["add", "b.txt", "c.txt"], &root);
        run(
            "svn",
            &["commit", "--non-interactive", "-m", "files"],
            &root,
        );
        for rel in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(root.join(rel), "changed\n").unwrap();
        }
        app.reload_sync().unwrap();
        app.tree.set_checked("a.txt", true);
        app.tree.set_checked("b.txt", true);
        app.tree.set_filter(Some("c.txt".into()));
        select_path(&mut app, "c.txt");
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        match app.confirm.as_ref().unwrap().action() {
            ConfirmAction::Revert(paths) => {
                assert_eq!(paths, &vec![root.join("a.txt"), root.join("b.txt")])
            }
            _ => panic!("应固定勾选范围"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(fresh_sign(&app, "a.txt"), 'M');
        assert_eq!(fresh_sign(&app, "b.txt"), 'M');
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        wait_for_scan(&mut app);
        for rel in ["a.txt", "b.txt"] {
            assert_eq!(
                std::fs::read_to_string(root.join(rel)).unwrap(),
                "original\n"
            );
        }
        assert_eq!(fresh_sign(&app, "c.txt"), 'M');
        assert_eq!(app.tree.checked_count(), 0);
    }

    #[test]
    #[ignore = "需要 svn + svnadmin；目录撤销恢复修改/删除，普通新增保留文件且不碰相似前缀"]
    fn revert_directory_restores_baseline_and_preserves_added_and_unversioned_files() {
        let (_tmp, mut app) = temporary_app();
        let root = app.root.clone();
        std::fs::create_dir_all(root.join("group/inner")).unwrap();
        for rel in [
            "group/modified.txt",
            "group/deleted.txt",
            "group/missing.txt",
            "group2.txt",
        ] {
            std::fs::write(root.join(rel), "original\n").unwrap();
        }
        run("svn", &["add", "group", "group2.txt"], &root);
        run(
            "svn",
            &["commit", "--non-interactive", "-m", "directory"],
            &root,
        );
        for rel in ["group/modified.txt", "group2.txt"] {
            std::fs::write(root.join(rel), "changed\n").unwrap();
        }
        run("svn", &["propset", "svnui-test", "local", "group"], &root);
        run("svn", &["delete", "group/deleted.txt"], &root);
        std::fs::remove_file(root.join("group/missing.txt")).unwrap();
        std::fs::create_dir_all(root.join("group/new/nested")).unwrap();
        std::fs::write(root.join("group/new/nested/added.txt"), "keep added\n").unwrap();
        run("svn", &["add", "group/new"], &root);
        std::fs::write(
            root.join("group/inner/unversioned.txt"),
            "keep unversioned\n",
        )
        .unwrap();
        app.reload_sync().unwrap();
        select_path(&mut app, "group");
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        assert_eq!(app.mode, Mode::Confirm);
        match app.confirm.as_ref().unwrap().action() {
            ConfirmAction::Revert(paths) => assert_eq!(paths, &vec![root.join("group")]),
            _ => panic!("应确认当前目录范围"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(!root.join("group/deleted.txt").exists());
        assert_eq!(fresh_sign(&app, "group/new"), 'A');
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)).unwrap();
        wait_for_scan(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        wait_for_scan(&mut app);
        for rel in [
            "group/modified.txt",
            "group/deleted.txt",
            "group/missing.txt",
        ] {
            assert_eq!(
                std::fs::read_to_string(root.join(rel)).unwrap(),
                "original\n"
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.join("group/new/nested/added.txt")).unwrap(),
            "keep added\n"
        );
        assert_eq!(fresh_sign(&app, "group/new"), '?');
        assert_eq!(
            std::fs::read_to_string(root.join("group/inner/unversioned.txt")).unwrap(),
            "keep unversioned\n"
        );
        assert_eq!(fresh_sign(&app, "group2.txt"), 'M');
        assert!(!app.items.iter().any(|item| item.rel == "group"), "目录属性改动也应撤销");
    }

}
