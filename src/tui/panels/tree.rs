//! 文件树（左栏）。
//!
//! ## 为什么自己写而不用 `tui-tree-widget`
//!
//! 一开始想直接用现成的，但两点不合适：
//! 1. 状态标记要画在**文件名上**（目录还要冒泡子节点的颜色），
//!    这需要完全控制每行的渲染，而现成组件的 item 渲染是固定套路。
//! 2. 全量模式下要**按需展开**才去读目录 —— 大仓库一次遍历几万个文件太慢。
//!
//! 自己实现的树其实不复杂：内部存一棵真树，渲染时**扁平化**成
//! "可见节点列表"（只含祖先都展开的），光标在这个扁平列表上移动。
//! 这样 `j/k` 就是简单的下标加减，不需要递归查找。

use std::collections::HashMap;

use ratatui::{
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Scrollbar, ScrollbarOrientation},
    Frame,
};

use super::Item;
use crate::tui::theme;

/// 树的一个节点。
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    /// 相对工作副本根的路径。
    pub rel: String,
    /// 绝对路径（全量模式下未从 svn 拿到的也有，直接拼出来）。
    pub abs: std::path::PathBuf,
    pub is_dir: bool,
    /// 状态符号。目录是子节点冒泡上来的。
    pub sign: char,
    pub xy: String,
    pub depth: usize,
    pub expanded: bool,
    /// 子节点名字（保序）。真正的节点在 `nodes` map 里。
    pub children: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TreeMode {
    /// 只显示有变更的（svn status 的结果，快）。
    Changed,
    /// 全量：按需展开时读目录。
    All,
}

pub struct TreePanel {
    /// key = 相对路径，根是 ""。
    nodes: HashMap<String, Node>,
    /// 扁平化后的可见节点（相对路径），光标在这上面移动。
    visible: Vec<String>,
    state: ListState,
    mode: TreeMode,
    /// 搜索关键词（小写，用于包含匹配）。
    filter: Option<String>,
    /// 是否已经为搜索铺开过目录（避免每敲一个字符都递归一遍）。
    search_loaded: bool,
    /// 勾选集合（相对路径）。用于提交范围选择。
    ///
    /// 放在 TreePanel 而不是 CommitPanel：
    /// 勾选是"在文件树上直接挑"的动作（Space），用户希望边看 diff 边挑，
    /// 而不是进一个弹框里挑 —— 弹框只该负责写提交信息。
    checked: std::collections::HashSet<String>,
}

/// 该状态是否可以进入提交范围。
///
/// `?`（未版本化）/ `I`（忽略）/ `X`（外部）不行：
/// - `?` 还没纳入版本控制，得先 `svn add`
/// - `I` / `X` 本来就不该进库
pub fn is_committable_sign(sign: char) -> bool {
    matches!(sign, 'M' | 'A' | 'D' | 'R' | 'C' | 'T' | '!')
}

impl TreePanel {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            visible: Vec::new(),
            state: ListState::default(),
            mode: TreeMode::Changed,
            filter: None,
            checked: std::collections::HashSet::new(),
            search_loaded: false,
        }
    }

    pub fn mode(&self) -> TreeMode {
        self.mode
    }

    /// 为什么这一项不能勾选。返回 `None` 表示**可以**勾。
    ///
    /// 这是给 UI 用的解释层：光让 toggle 返回 None 的话，
    /// 用户按了 Space 没反应却不知道为什么，会以为程序坏了。
    pub fn check_block_reason(&self) -> Option<&'static str> {
        let rel = self.visible.get(self.state.selected()?)?;
        let n = self.nodes.get(rel)?;
        if n.is_dir {
            // 提交目录：svn 会递归整棵子树，范围不可控。
            return Some("目录不能勾选，请把光标移到文件上");
        }
        if is_committable_sign(n.sign) {
            return None;
        }
        match n.sign {
            // 干净文件：勾了也没用，svn commit 会直接跳过它。
            // 给它一个能勾的框等于骗人 —— 这是这个判断存在的理由。
            ' ' => Some("这个文件没有改动，不需要提交"),
            '?' => Some("未版本化：先按 A 执行 svn add 才能提交"),
            'I' => Some("已忽略的文件（svn:ignore），不会提交"),
            'X' => Some("外部定义（svn:externals），不属于本仓库"),
            _ => Some("这个文件当前没有可提交的状态"),
        }
    }

    /// 这一项是否显示复选框。
    ///
    /// 只有真的能提交才画 `[ ]` —— 给没改动的文件画一个可勾的框，
    /// 勾了 svn 也会忽略，纯属误导。
    pub fn is_checkable(&self, rel: &str) -> bool {
        self.nodes
            .get(rel)
            .map(|n| !n.is_dir && is_committable_sign(n.sign))
            .unwrap_or(false)
    }

    /// 切换当前项的勾选状态。返回切换后是否勾选。
    pub fn toggle_check(&mut self) -> Option<bool> {
        // 不能勾的（目录 / 无改动 / 未版本化 / 忽略 / 外部）直接挡掉，
        // 具体原因让调用方用 check_block_reason 去问。
        if self.check_block_reason().is_some() {
            return None;
        }
        let rel = self.visible.get(self.state.selected()?)?.clone();
        if self.checked.contains(&rel) {
            self.checked.remove(&rel);
            Some(false)
        } else {
            self.checked.insert(rel);
            Some(true)
        }
    }

    /// 当前是否勾选。
    pub fn is_checked(&self, rel: &str) -> bool {
        self.checked.contains(rel)
    }

    /// 勾选数量。
    pub fn checked_count(&self) -> usize {
        self.checked.len()
    }

    /// 清空勾选。
    /// 直接设置某一项的勾选（用于提交失败后还原勾选）。
    ///
    /// 和 `toggle_check`（走光标、受"能不能勾"限制）不同，
    /// 这是程序化写入：仍然校验可勾性，但不依赖当前光标位置。
    pub fn set_checked(&mut self, rel: &str, on: bool) {
        if !self.is_checkable(rel) {
            return;
        }
        if on {
            self.checked.insert(rel.to_string());
        } else {
            self.checked.remove(rel);
        }
    }

    pub fn clear_check(&mut self) {
        self.checked.clear();
    }

    /// 全选 / 清空：只对"可提交"的条目生效。
    ///
    /// `?`（未版本化）永远不自动勾 —— 它还没纳入版本控制，
    /// 自动勾上会让用户误提交不该进库的东西。
    pub fn select_committable(&mut self, on: bool) {
        self.checked.clear();
        if !on {
            return;
        }
        for (rel, n) in &self.nodes {
            if !n.is_dir && is_committable_sign(n.sign) {
                self.checked.insert(rel.clone());
            }
        }
    }

    /// 勾选的绝对路径（按 visible 顺序，便于展示时稳定）。
    ///
    /// 再过滤一次 is_checkable 是保险：勾选之后状态可能变了
    /// （比如勾了 M 文件又按 R 还原，sign 变成空格），
    /// 这时它还在 checked 里，但已经不该提交。
    /// 光靠 UI 不画框不够 —— 状态变化后旧勾选还在集合里。
    pub fn checked_abs(&self) -> Vec<std::path::PathBuf> {
        self.visible
            .iter()
            .filter(|r| self.checked.contains(*r))
            .filter(|r| self.is_checkable(r))
            .filter_map(|r| self.nodes.get(r).map(|n| n.abs.clone()))
            .collect()
    }

    /// 清掉已经不可提交的勾选（revert / commit 之后状态会变）。
    ///
    /// 不清的话会留下"看不见但还在"的幽灵勾选：
    /// UI 上不画框了，集合里却还有，下次 Ctrl+A 又冒出来。
    pub fn prune_checked(&mut self) {
        // 先收集再删，不能 retain：
        // `self.checked.retain(闭包里读 self.nodes)` 会同时可变借用
        // checked、不可变借用 nodes —— 同一个 self 上两笔借用，编译器不放行。
        let dead: Vec<String> = self
            .checked
            .iter()
            .filter(|rel| {
                !self
                    .nodes
                    .get(*rel)
                    .map(|n| !n.is_dir && is_committable_sign(n.sign))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        for d in dead {
            self.checked.remove(&d);
        }
    }

    pub fn set_mode(&mut self, mode: TreeMode) {
        self.mode = mode;
        // 两种模式内容完全不同，上次的搜索词留着没有意义，
        // 还会让人以为"搜到了但列表是空的"。
        self.filter = None;
        self.search_loaded = false;
        self.rebuild_visible();
    }

    // ── 构建 ──────────────────────────────────────────────

    /// 从变更列表建树。
    ///
    /// 目录节点是凭路径**造**出来的（svn status 不一定给出目录条目），
    /// 它的 sign 由子节点冒泡 —— 父目录显示最严重的那个子状态。
    pub fn build_from_items(&mut self, items: &[Item], root: &std::path::Path) {
        self.nodes.clear();
        self.nodes.insert(
            String::new(),
            Node {
                name: String::new(),
                rel: String::new(),
                abs: root.to_path_buf(),
                is_dir: true,
                sign: ' ',
                xy: String::new(),
                depth: 0,
                expanded: true,
                children: Vec::new(),
            },
        );

        for item in items {
            let parts: Vec<&str> = item.rel.split('/').filter(|s| !s.is_empty()).collect();
            let mut cur = String::new();

            for (i, part) in parts.iter().enumerate() {
                let is_last = i + 1 == parts.len();
                let child_rel = if cur.is_empty() {
                    part.to_string()
                } else {
                    format!("{}/{}", cur, part)
                };

                // 保证节点存在
                if !self.nodes.contains_key(&child_rel) {
                    let node = Node {
                        name: part.to_string(),
                        rel: child_rel.clone(),
                        abs: root.join(&child_rel),
                        is_dir: !is_last || item.is_dir,
                        sign: if is_last { item.sign } else { ' ' },
                        xy: if is_last { item.xy.clone() } else { String::new() },
                        depth: i,
                        // 中间目录默认全展开：变更树通常不深，展开比折叠有用
                        expanded: true,
                        children: Vec::new(),
                    };
                    self.nodes.insert(child_rel.clone(), node);
                }

                // 挂到父节点（去重）
                let parent = self.nodes.get_mut(&cur).unwrap();
                if !parent.children.contains(&child_rel) {
                    parent.children.push(child_rel.clone());
                }

                if !is_last {
                    // 让父目录展开（状态树默认全展开）
                    if let Some(p) = self.nodes.get_mut(&cur) {
                        p.expanded = true;
                    }
                }
                cur = child_rel;
            }
        }

        self.bubble();
        self.search_loaded = false;
        // ⚠️ 必须同步 mode：本函数是按"变更列表"重建的，
        //    不设的话切到全量树后再刷新（比如外部编辑器改完文件回来），
        //    树的 mode 还是 All、标题显示"文件树"，内容却已是变更列表 —— 对不上。
        self.mode = TreeMode::Changed;
        self.rebuild_visible();
    }

    /// 目录的 sign 取子节点里**最严重**的。
    fn bubble(&mut self) {
        // 从最深往上：按 depth 降序处理
        let mut by_depth: Vec<(usize, String)> = self
            .nodes
            .iter()
            .map(|(k, v)| (v.depth, k.clone()))
            .collect();
        by_depth.sort_by(|a, b| b.0.cmp(&a.0));

        for (_, rel) in by_depth {
            let (is_dir, children, cur_sign) = match self.nodes.get(&rel) {
                Some(n) => (n.is_dir, n.children.clone(), n.sign),
                None => continue,
            };
            if !is_dir || children.is_empty() {
                continue;
            }

            let worst = children
                .iter()
                .filter_map(|c| self.nodes.get(c))
                .map(|c| c.sign)
                .fold(' ', |acc, s| worse_sign(acc, s));

            // 自己的显式状态优先（比如目录本身被 delete）
            let final_sign = if cur_sign != ' ' && cur_sign != '?' {
                cur_sign
            } else {
                worst
            };

            if let Some(n) = self.nodes.get_mut(&rel) {
                n.sign = final_sign;
            }
        }
    }

    /// 重新算可见节点列表（祖先都展开的）。
    fn rebuild_visible(&mut self) {
        self.visible.clear();
        // ⚠️ 先 clone 出来：直接 `match self.filter.as_deref()` 会让不可变借用
        //    一直活到 match 结束，而分支里要调 &mut self 的方法，必然冲突。
        let f = self.filter.clone();
        match f.as_deref() {
            Some(f) if !f.is_empty() => self.collect_matched(f),
            _ => self.collect_visible(String::new()),
        }

        if self.visible.is_empty() {
            self.state.select(None);
        } else {
            let i = self.state.selected().unwrap_or(0).min(self.visible.len() - 1);
            self.state.select(Some(i));
        }
    }

    /// 搜索模式下收集可见节点：**命中项 + 它们的祖先链**，仅此而已。
    ///
    /// ⚠️ 早期版本是"搜索时把所有节点 expanded = true"，结果一搜就把整棵树
    ///    全铺开 —— 几百行平铺，反而找不到东西。这里改成只保留必要路径。
    ///
    /// 匹配规则分两种，为了兼顾"搜文件名"和"搜带目录的路径"：
    ///   · 关键词含 `/` → 匹配整条路径（可写 `src/mai`）
    ///   · 否则         → 只匹配最后一段（文件名）
    ///
    /// 只匹配文件名是刻意的：否则搜 `src` 会把 src 下**所有**后代都列出来
    /// （它们的路径里都有 "src"），那是噪音不是结果。
    fn collect_matched(&mut self, f: &str) {
        let whole_path = f.contains('/');

        let direct: Vec<String> = self
            .nodes
            .keys()
            .filter(|k| {
                if k.is_empty() {
                    return false;
                }
                let hay = if whole_path {
                    k.to_lowercase()
                } else {
                    match k.rsplit('/').next() {
                        Some(name) => name.to_lowercase(),
                        None => return false,
                    }
                };
                hay.contains(f)
            })
            .cloned()
            .collect();

        // 命中项 + 祖先链（祖先让层级可见）
        let mut keep: std::collections::HashSet<String> = direct.iter().cloned().collect();
        for k in &direct {
            let mut cur = k.as_str();
            while let Some(pos) = cur.rfind('/') {
                cur = &cur[..pos];
                keep.insert(cur.to_string());
            }
        }

        let mut list: Vec<String> = keep.into_iter().collect();
        // 路径字典序天然让父节点排在子节点前（"a" < "a/b"），
        // 配合缩进就能看出层级，不需要真的展开。
        list.sort();
        self.visible = list;
    }

    fn collect_visible(&mut self, rel: String) {
        let Some(node) = self.nodes.get(&rel).cloned() else {
            return;
        };

        // ⚠️ 顺序很重要：先把自己加进可见列表，再决定要不要递归子节点。
        //    写在 `if !node.expanded { return }` 后面的话，折叠的目录连自己
        //    都不显示 —— 看起来就像"树没生效"（只剩几个展开的叶子）。
        // 根不显示。
        if !rel.is_empty() && self.matches_filter(&rel) {
            self.visible.push(rel.clone());
        }

        if !node.expanded {
            return;
        }
        for c in node.children.clone() {
            self.collect_visible(c);
        }
    }

    /// 无搜索时所有节点都可见；搜索时由 `collect_matched` 单独处理。
    fn matches_filter(&self, rel: &str) -> bool {
        match &self.filter {
            None => true,
            Some(f) => f.is_empty() || rel.to_lowercase().contains(f),
        }
    }

    // ── 光标 ──────────────────────────────────────────────

    pub fn selected(&self) -> Option<&Node> {
        self.state
            .selected()
            .and_then(|i| self.visible.get(i))
            .and_then(|r| self.nodes.get(r))
    }

    /// 返回 true 表示真的动了（调用方据此决定是否重新加载 diff）。
    pub fn move_up(&mut self) -> bool {
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => return false,
        };
        self.state.select(Some(i));
        true
    }

    pub fn move_down(&mut self) -> bool {
        let i = match self.state.selected() {
            Some(i) if i + 1 < self.visible.len() => i + 1,
            None if !self.visible.is_empty() => 0,
            _ => return false,
        };
        self.state.select(Some(i));
        true
    }

    /// 展开/折叠。返回 true 表示需要重新扫描（全量模式懒加载）。
    pub fn toggle(&mut self) -> bool {
        let Some(rel) = self.state.selected().and_then(|i| self.visible.get(i)).cloned() else {
            return false;
        };
        let Some(node) = self.nodes.get(&rel) else {
            return false;
        };
        if !node.is_dir {
            return false;
        }

        let now_expanded = !node.expanded;
        if let Some(n) = self.nodes.get_mut(&rel) {
            n.expanded = now_expanded;
        }

        // 全量模式：展开时才读目录（懒加载，避免一次遍历整个仓库）
        let need_load = now_expanded
            && self.mode == TreeMode::All
            && self
                .nodes
                .get(&rel)
                .map(|n| n.children.is_empty())
                .unwrap_or(false);

        self.rebuild_visible();
        need_load
    }

    /// 全量模式下把目录内容读进来。
    pub fn load_dir(&mut self, rel: &str) -> std::io::Result<()> {
        self.load_dir_raw(rel)?;
        self.rebuild_visible();
        Ok(())
    }

    /// 同 `load_dir`，但不重算可见列表 —— 批量加载时用，省掉 N 次遍历。
    fn load_dir_raw(&mut self, rel: &str) -> std::io::Result<()> {
        let abs = match self.nodes.get(rel) {
            Some(n) => n.abs.clone(),
            None => return Ok(()),
        };

        let mut entries: Vec<_> = std::fs::read_dir(&abs)?
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                (name, is_dir)
            })
            .collect();
        // 目录在前，其次按名。
        //
        // 用 `_cached_key`：闭包里的 `to_lowercase()` 会分配一个新 String，
        // `sort_by` 每次比较都现算，几千条的目录就是十几万次堆分配 ——
        // 切目录时能感觉到卡。`_cached_key` 每个元素只算一次。
        entries.sort_by_cached_key(|(name, is_dir)| (!*is_dir, name.to_lowercase()));

        let depth = self.nodes.get(rel).map(|n| n.depth + 1).unwrap_or(0);
        for (name, is_dir) in entries {
            if name == ".svn" {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", rel, name)
            };
            self.nodes.entry(child_rel.clone()).or_insert(Node {
                name,
                rel: child_rel.clone(),
                abs: abs.join(&child_rel.rsplit('/').next().unwrap_or(&child_rel)),
                is_dir,
                sign: ' ',
                xy: String::new(),
                depth,
                expanded: false,
                children: Vec::new(),
            });
            if let Some(p) = self.nodes.get_mut(rel) {
                if !p.children.contains(&child_rel) {
                    p.children.push(child_rel);
                }
            }
        }
        Ok(())
    }

    /// 搜索前把目录递归加载进来。
    ///
    /// 全量模式是**懒加载**的：没展开过的目录，它的子节点根本不在 `nodes` 里，
    /// 所以搜那些文件等于什么都不搜。这里 BFS 铺开到 max_depth 层。
    ///
    /// 限量是必须的 —— 对着几万文件的仓库递归到底，一次搜索能把界面卡死。
    pub fn ensure_loaded(&mut self, max_depth: usize, max_nodes: usize) {
        if self.mode != TreeMode::All {
            return;
        }
        let mut queue = vec![String::new()];

        while let Some(rel) = queue.pop() {
            if self.nodes.len() > max_nodes {
                break;
            }
            let (depth, loaded, children) = match self.nodes.get(&rel) {
                Some(n) => (n.depth, !n.children.is_empty(), n.children.clone()),
                None => continue,
            };
            if depth >= max_depth {
                continue;
            }
            if !loaded {
                let _ = self.load_dir_raw(&rel);
            }
            if let Some(n) = self.nodes.get(&rel) {
                for c in &n.children {
                    queue.push(c.clone());
                }
            }
            let _ = children;
        }
    }

    // ── 搜索 ──────────────────────────────────────────────

    pub fn set_filter(&mut self, f: Option<String>) {
        let has = f.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
        self.filter = f.map(|s| s.to_lowercase());

        if has {
            // 第一次搜索时才铺开：全量模式下未展开的目录还没加载过子节点，
            // 不加载就搜不到里面的文件。铺开一次够了，之后增量搜索不用重复。
            if !self.search_loaded {
                self.ensure_loaded(4, 5000);
                self.search_loaded = true;
            }
        }
        // ⚠️ 不要再全展开节点了：那会让搜索结果变成一整棵平铺的树。
        //    collect_matched 已经保证命中项及其祖先可见。
        self.rebuild_visible();
    }

    pub fn filter(&self) -> Option<&String> {
        self.filter.as_ref()
    }

    // ── 渲染 ──────────────────────────────────────────────

    pub fn render(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let items: Vec<ListItem> = self
            .visible
            .iter()
            // 保留 (rel, node) 对而不是只留 node：
            // 下面判断勾选要用 rel 查 self.checked，
            // 只传 node 的话 rel 在闭包里就不可见了。
            .filter_map(|rel| self.nodes.get(rel).map(|n| (rel, n)))
            .map(|(rel, node)| {
                let style = theme::for_sign(node.sign);

                let indent = "  ".repeat(node.depth);
                // 目录：三角 + 名字 + 尾斜杠
                let (mark, name) = if node.is_dir {
                    let m = if node.expanded { "▾ " } else { "▸ " };
                    (m, format!("{}/", node.name))
                } else {
                    ("  ", node.name.clone())
                };

                // 勾选框：只有"真能提交"的文件才画。
                //
                // 目录、干净文件、未版本化(?)、忽略(I)、外部(X) 一律留空：
                // 给它们画一个 [ ] 是骗人的 —— 勾上也不会进提交，
                // 用户会以为勾选生效了，结果是空提交或者漏提交。
                //
                // 留三个空格而不是不画：要保持这一列对齐，
                // 否则有框和没框的文件名会错开一列。
                let checkbox = if !self.is_checkable(rel) {
                    Span::raw("   ")
                } else if self.checked.contains(rel) {
                    Span::styled("[x] ", theme::Theme::checked())
                } else {
                    Span::styled("[ ] ", theme::Theme::unchecked())
                };

                let mut spans = vec![
                    Span::raw(indent),
                    Span::styled(mark, theme::Theme::dim()),
                    checkbox,
                    // 状态符号直接跟在名字前 —— 这是"状态放文件名上"的关键
                    Span::styled(format!("{} ", node.sign), style),
                    Span::styled(name, style),
                ];

                if !node.is_dir && !node.xy.trim().is_empty() {
                    spans.push(Span::styled(
                        format!("  {}", node.xy.trim_end()),
                        theme::Theme::dim(),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let title = match self.mode {
            TreeMode::Changed => " 变更 ",
            TreeMode::All => " 文件树 ",
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
        if self.visible.len() > visible_h {
            let mut sb = ratatui::widgets::ScrollbarState::new(self.visible.len())
                .position(self.state.selected().unwrap_or(0));
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area,
                &mut sb,
            );
        }
    }
}

/// 取更严重的那个状态。顺序基本按"会不会阻塞提交"排。
fn worse_sign(a: char, b: char) -> char {
    const ORDER: &[char] = &['C', 'T', '!', 'D', 'R', 'A', 'M', '?'];
    let ia = ORDER.iter().position(|c| *c == a);
    let ib = ORDER.iter().position(|c| *c == b);
    match (ia, ib) {
        (Some(x), Some(y)) => ORDER[x.min(y)],
        (Some(_), None) => a,
        (None, Some(_)) => b,
        (None, None) => a,
    }
}
