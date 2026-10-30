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

use std::collections::{HashMap, HashSet};

use ratatui::{
    text::{Line, Span},
    widgets::{Borders, List, ListItem, ListState, Paragraph, ScrollbarOrientation},
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
    /// 压缩行只改变展示，节点与操作目标仍使用真实相对路径。
    compact_rows: HashMap<String, (String, usize)>,
    /// 有变更子节点不代表读过目录，必须单独记录磁盘加载状态。
    loaded_dirs: HashSet<String>,
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
            compact_rows: HashMap::new(),
            loaded_dirs: HashSet::new(),
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
            return if self.directory_targets(rel).is_empty() {
                Some("此目录在当前筛选内没有可选的文件改动；未纳管项请先按 a 添加")
            } else {
                None
            };
        }
        if is_committable_sign(n.sign) {
            return None;
        }
        match n.sign {
            // 干净文件：勾了也没用，svn commit 会直接跳过它。
            // 给它一个能勾的框等于骗人 —— 这是这个判断存在的理由。
            ' ' => Some("这个文件没有改动，不需要提交"),
            '?' => Some("未纳管：先按 a 添加当前项，或 A 批量添加，才能提交"),
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
        if self.check_block_reason().is_some() {
            return None;
        }
        let rel = self.visible.get(self.state.selected()?)?.clone();
        if self.nodes.get(&rel)?.is_dir {
            let targets = self.directory_targets(&rel);
            let on = !targets.iter().all(|p| self.checked.contains(p));
            // 目录只是选择分组，绝不把目录路径作为递归提交目标。
            for path in targets {
                if on {
                    self.checked.insert(path);
                } else {
                    self.checked.remove(&path);
                }
            }
            return Some(on);
        }
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

    fn directory_targets(&self, rel: &str) -> Vec<String> {
        let prefix = format!("{rel}/");
        self.scope_committable()
            .into_iter()
            .filter(|p| p.starts_with(&prefix))
            .collect()
    }

    /// 搜索约束选择范围；目录折叠只影响展示，不影响批选。
    fn scope_committable(&self) -> Vec<String> {
        self.nodes
            .keys()
            .filter(|p| self.is_checkable(p) && self.matches_filter(p))
            .cloned()
            .collect()
    }

    pub fn check_scope(&mut self) -> usize {
        let before = self.checked.len();
        let targets = self.scope_committable();
        self.checked.extend(targets);
        self.checked.len() - before
    }

    pub fn invert_scope(&mut self) -> usize {
        for path in self.scope_committable() {
            if !self.checked.remove(&path) {
                self.checked.insert(path);
            }
        }
        self.checked.len()
    }

    pub fn hidden_checked_count(&self) -> usize {
        let visible: HashSet<&String> = self.visible.iter().collect();
        self.checked.iter().filter(|p| !visible.contains(p)).count()
    }

    /// 一次按祖先累加，避免渲染每个目录时反复扫描整棵树。
    fn directory_counts(&self) -> HashMap<String, (usize, usize)> {
        let mut counts = HashMap::new();
        for path in self.scope_committable() {
            let selected = usize::from(self.checked.contains(&path));
            let mut parent = path.as_str();
            while let Some((ancestor, _)) = parent.rsplit_once('/') {
                let count = counts.entry(ancestor.to_string()).or_insert((0, 0));
                count.0 += selected;
                count.1 += 1;
                parent = ancestor;
            }
        }
        counts
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

    /// 勾选全部可提交项（含折叠、搜索过滤之外的）。返回新勾上的数量。
    ///
    /// `?`（未版本化）永远不自动勾 —— 它还没纳入版本控制，
    /// 自动勾上会让用户误提交不该进库的东西。
    pub fn check_all_committable(&mut self) -> usize {
        let mut added = 0;
        let targets: Vec<String> = self
            .nodes
            .iter()
            .filter(|(_, n)| !n.is_dir && is_committable_sign(n.sign))
            .map(|(rel, _)| rel.clone())
            .collect();
        for rel in targets {
            if self.checked.insert(rel) {
                added += 1;
            }
        }
        added
    }

    /// 全部有效勾选按路径排序，折叠和过滤不能让已选项漏出提交清单。
    ///
    /// 再过滤一次 is_checkable 是保险：勾选之后状态可能变了
    /// （比如勾了 M 文件又按 R 还原，sign 变成空格），
    /// 这时它还在 checked 里，但已经不该提交。
    /// 光靠 UI 不画框不够 —— 状态变化后旧勾选还在集合里。
    pub fn checked_abs(&self) -> Vec<std::path::PathBuf> {
        self.checked_changes()
            .into_iter()
            .filter_map(|(rel, _)| self.nodes.get(&rel).map(|n| n.abs.clone()))
            .collect()
    }

    /// 勾选项的「相对路径 + 状态符号」，供提交信息生成（`Ctrl+G`）用。
    ///
    /// 与 `checked_abs` 同序（按相对路径排序）—— 清单和待提交路径
    /// 顺序一致，对照着看不会错位。
    pub fn checked_changes(&self) -> Vec<(String, char)> {
        let mut changes: Vec<_> = self
            .checked
            .iter()
            .filter(|r| self.is_checkable(r))
            .filter_map(|r| self.nodes.get(r).map(|n| (n.rel.clone(), n.sign)))
            .collect();
        changes.sort_by(|a, b| a.0.cmp(&b.0));
        changes
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
        self.search_loaded = false;
        self.rebuild_visible();
    }

    pub fn switch_mode(&mut self, mode: TreeMode, items: &[Item], root: &std::path::Path) {
        self.mode = mode;
        self.refresh_from_items(items, root);
    }

    // ── 构建 ──────────────────────────────────────────────

    /// 刷新以路径恢复位置，保留折叠、过滤与有效勾选；全量树只重读已加载的目录。
    pub fn refresh_from_items(&mut self, items: &[Item], root: &std::path::Path) {
        let selected = self.selected().map(|n| n.rel.clone());
        let index = self.state.selected();
        let offset = self.state.offset();
        let mode = self.mode;
        let filter = self.filter.clone();
        let mut dirs: Vec<_> = self
            .nodes
            .values()
            .filter(|n| n.is_dir)
            .map(|n| {
                (
                    n.rel.clone(),
                    n.expanded,
                    self.loaded_dirs.contains(&n.rel),
                    n.depth,
                )
            })
            .collect();
        dirs.sort_by_key(|d| d.3);

        self.build_from_items(items, root);
        self.mode = mode;
        if mode == TreeMode::All {
            let _ = self.load_dir_raw("");
            for (rel, expanded, loaded, _) in &dirs {
                if *expanded || *loaded {
                    let _ = self.load_dir_raw(rel);
                }
            }
            // 搜索仍沿用原来的深度/节点限额，让新目录中的匹配项也可见。
            if filter.is_some() {
                self.ensure_loaded(4, 5000);
                self.search_loaded = true;
            }
        }
        for (rel, expanded, _, _) in dirs {
            if let Some(node) = self.nodes.get_mut(&rel) {
                node.expanded = expanded;
            }
        }
        self.filter = filter;
        self.rebuild_visible();
        let restored = selected.and_then(|rel| self.visible.iter().position(|p| p == &rel));
        self.state.select(restored.or_else(|| {
            (!self.visible.is_empty()).then(|| index.unwrap_or(0).min(self.visible.len() - 1))
        }));
        let row = index.unwrap_or(0).saturating_sub(offset);
        *self.state.offset_mut() = self.state.selected().unwrap_or(0).saturating_sub(row);
        self.prune_checked();
    }

    /// 从变更列表建树。
    ///
    /// 目录节点是凭路径**造**出来的（svn status 不一定给出目录条目），
    /// 它的 sign 由子节点冒泡 —— 父目录显示最严重的那个子状态。
    pub fn build_from_items(&mut self, items: &[Item], root: &std::path::Path) {
        self.nodes.clear();
        self.loaded_dirs.clear();
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
                        xy: if is_last {
                            item.xy.clone()
                        } else {
                            String::new()
                        },
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
        self.compact_rows.clear();
        // ⚠️ 先 clone 出来：直接 `match self.filter.as_deref()` 会让不可变借用
        //    一直活到 match 结束，而分支里要调 &mut self 的方法，必然冲突。
        let f = self.filter.clone();
        match f.as_deref() {
            Some(f) if !f.is_empty() => self.collect_matched(f),
            _ => self.collect_visible(String::new(), 0),
        }

        if self.visible.is_empty() {
            self.state.select(None);
        } else {
            let i = self
                .state
                .selected()
                .unwrap_or(0)
                .min(self.visible.len() - 1);
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

    fn collect_visible(&mut self, rel: String, depth: usize) {
        let Some(mut node) = self.nodes.get(&rel).cloned() else {
            return;
        };
        if rel.is_empty() {
            for child in node.children {
                self.collect_visible(child, depth);
            }
            return;
        }
        let mut label = node.name.clone();
        // 改动树已知完整状态，可以压缩单目录链；完整工程树保留逐级导航，
        // 避免把尚未加载的干净兄弟节点误当作不存在。
        while self.mode == TreeMode::Changed
            && node.expanded
            && node.is_dir
            && node.xy.trim().is_empty()
            && node.children.len() == 1
        {
            let Some(child) = self.nodes.get(&node.children[0]) else {
                break;
            };
            if !child.is_dir || !child.xy.trim().is_empty() {
                break;
            }
            label.push('/');
            label.push_str(&child.name);
            node = child.clone();
        }
        self.compact_rows.insert(node.rel.clone(), (label, depth));
        self.visible.push(node.rel.clone());
        if !node.expanded {
            return;
        }
        for c in node.children.clone() {
            self.collect_visible(c, depth + 1);
        }
    }

    /// 相对路径是否命中当前过滤词。
    ///
    /// 与 `collect_matched` 同一套规则：关键词含 `/` 匹配整条路径，
    /// 否则只匹配最后一段（文件名）。别处自己写一句 `contains`
    /// 会和树上看到的对不上 —— 「按搜索预选」就成了假的所见即所得。
    ///
    /// 无搜索时返回 true（所有节点都可见）。
    pub fn matches_filter(&self, rel: &str) -> bool {
        let Some(f) = self.filter.as_deref().filter(|s| !s.is_empty()) else {
            return true;
        };
        let hay = if f.contains('/') {
            rel.to_lowercase()
        } else {
            rel.rsplit('/').next().unwrap_or(rel).to_lowercase()
        };
        hay.contains(f)
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
        let Some(rel) = self
            .state
            .selected()
            .and_then(|i| self.visible.get(i))
            .cloned()
        else {
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
        let need_load =
            now_expanded && self.mode == TreeMode::All && !self.loaded_dirs.contains(&rel);

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
        self.loaded_dirs.insert(rel.to_string());
        let mut children = self
            .nodes
            .get(rel)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        children.sort_by_cached_key(|path| {
            let node = &self.nodes[path];
            (!node.is_dir, node.name.to_lowercase(), node.name.clone())
        });
        if let Some(parent) = self.nodes.get_mut(rel) {
            parent.children = children;
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
                Some(n) => (n.depth, self.loaded_dirs.contains(&rel), n.children.clone()),
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

    fn title(&self) -> &'static str {
        match self.mode {
            TreeMode::Changed => " [改动] / 全部文件 · T切换 ",
            TreeMode::All => " 改动 / [全部文件] · T切换 ",
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: ratatui::layout::Rect, focused: bool) {
        let hidden = self.hidden_checked_count();
        let selection = if hidden > 0 {
            format!(" 已选 {} · 隐藏 {} ", self.checked_count(), hidden)
        } else {
            format!(" 已选 {} ", self.checked_count())
        };
        if self.visible.is_empty() {
            let message = if self.filter.is_some() {
                "无匹配项，按 / 修改过滤词，Esc 清空过滤"
            } else if self.mode == TreeMode::Changed {
                "暂无变更，按 T 查看全部文件，R 重新扫描"
            } else {
                "目录为空，按 R 重新扫描"
            };
            let block = theme::block()
                .borders(Borders::ALL)
                .title(theme::focus_title(self.title(), focused))
                .title_bottom(Line::from(selection))
                .border_style(if focused {
                    theme::Theme::border_active()
                } else {
                    theme::Theme::border()
                });
            f.render_widget(
                Paragraph::new(message)
                    .style(theme::Theme::dim())
                    .block(block),
                area,
            );
            return;
        }
        let counts = self.directory_counts();
        let items: Vec<ListItem> = self
            .visible
            .iter()
            // 保留 (rel, node) 对而不是只留 node：
            // 下面判断勾选要用 rel 查 self.checked，
            // 只传 node 的话 rel 在闭包里就不可见了。
            .filter_map(|rel| self.nodes.get(rel).map(|n| (rel, n)))
            .map(|(rel, node)| {
                let style = theme::for_sign(node.sign);

                let (label, depth) = self
                    .compact_rows
                    .get(rel)
                    .map(|(name, depth)| (name.as_str(), *depth))
                    .unwrap_or((&node.name, node.depth));
                let indent = "  ".repeat(depth);
                // 目录：三角 + 名字 + 尾斜杠
                let (mark, name) = if node.is_dir {
                    let m = if node.expanded { "▾ " } else { "▸ " };
                    (m, format!("{label}/"))
                } else {
                    ("  ", label.to_string())
                };

                let (selected, total) = counts.get(rel).copied().unwrap_or_default();
                let checkbox = if node.is_dir && total > 0 {
                    match selected {
                        0 => Span::styled("[ ] ", theme::Theme::unchecked()),
                        n if n == total => Span::styled("[x] ", theme::Theme::checked()),
                        _ => Span::styled("[-] ", theme::Theme::checked()),
                    }
                } else if self.is_checkable(rel) {
                    if self.checked.contains(rel) {
                        Span::styled("[x] ", theme::Theme::checked())
                    } else {
                        Span::styled("[ ] ", theme::Theme::unchecked())
                    }
                } else {
                    Span::raw("    ")
                };

                let mut spans = vec![
                    Span::raw(indent),
                    Span::styled(mark, theme::Theme::dim()),
                    checkbox,
                    // 状态符号直接跟在名字前 —— 这是"状态放文件名上"的关键
                    Span::styled(format!("{} ", node.sign), style),
                    Span::styled(name, style),
                ];

                if node.is_dir && total > 0 {
                    spans.push(Span::styled(
                        format!("  {selected}/{total}"),
                        theme::Theme::dim(),
                    ));
                }
                // 单一状态已在名前显示；仅补充属性等不同的第二状态。
                if !node.is_dir
                    && node
                        .xy
                        .chars()
                        .any(|c| c != ' ' && c != '_' && c != node.sign)
                {
                    spans.push(Span::styled(
                        format!("  {}", node.xy.trim_end()),
                        theme::Theme::dim(),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items)
            .block(
                theme::block()
                    .borders(Borders::ALL)
                    .title(theme::focus_title(self.title(), focused))
                    .title_bottom(Line::from(selection))
                    .border_style(if focused {
                        theme::Theme::border_active()
                    } else {
                        theme::Theme::border()
                    }),
            )
            .highlight_style(theme::Theme::selected());

        f.render_stateful_widget(list, area, &mut self.state);

        let visible_h = area.height.saturating_sub(2) as usize;
        if self.visible.len() > visible_h {
            let mut sb = ratatui::widgets::ScrollbarState::new(self.visible.len())
                .position(self.state.selected().unwrap_or(0));
            f.render_stateful_widget(
                theme::scrollbar(ScrollbarOrientation::VerticalRight),
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

#[cfg(test)]
mod refresh_state_tests {
    use super::*;
    use std::path::Path;

    fn item(rel: &str, sign: char) -> Item {
        Item {
            rel: rel.into(),
            abs: Path::new("/wc").join(rel),
            sign,
            xy: format!("{} ", sign),
            is_dir: false,
        }
    }

    fn select(tree: &mut TreePanel, rel: &str) {
        tree.state
            .select(Some(tree.visible.iter().position(|p| p == rel).unwrap()));
    }

    #[test]
    fn refresh_preserves_path_fold_checks_and_viewport() {
        let mut tree = TreePanel::new();
        let root = Path::new("/wc");
        let mut items = vec![
            item("src/a.txt", 'M'),
            item("src/c.txt", 'M'),
            item("other/z.txt", 'M'),
        ];
        tree.build_from_items(&items, root);
        tree.nodes.get_mut("other").unwrap().expanded = false;
        tree.rebuild_visible();
        select(&mut tree, "src/c.txt");
        tree.set_checked("src/c.txt", true);
        *tree.state.offset_mut() = tree.state.selected().unwrap() - 1;
        items.insert(1, item("src/b.txt", '?'));
        tree.refresh_from_items(&items, root);
        assert_eq!(tree.selected().unwrap().rel, "src/c.txt");
        assert_eq!(tree.state.selected().unwrap() - tree.state.offset(), 1);
        assert!(!tree.nodes["other"].expanded);
        assert!(tree.is_checked("src/c.txt"));
        assert!(!tree.is_checkable("src/b.txt"));
    }

    #[test]
    fn refresh_keeps_full_tree_filter_and_lazy_directory_state() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src/nested")).unwrap();
        std::fs::create_dir(tmp.path().join("untouched")).unwrap();
        std::fs::write(tmp.path().join("src/match.txt"), "old").unwrap();
        let mut tree = TreePanel::new();
        tree.build_from_items(&[], tmp.path());
        tree.load_dir("").unwrap();
        tree.set_mode(TreeMode::All);
        tree.nodes.get_mut("src").unwrap().expanded = true;
        tree.load_dir("src").unwrap();
        tree.set_filter(Some("match".into()));
        select(&mut tree, "src/match.txt");
        std::fs::write(tmp.path().join("src/match-new.txt"), "new").unwrap();
        tree.refresh_from_items(&[], tmp.path());
        assert_eq!(tree.mode(), TreeMode::All);
        assert_eq!(tree.filter().map(String::as_str), Some("match"));
        assert_eq!(tree.selected().unwrap().rel, "src/match.txt");
        assert!(tree.visible.iter().any(|p| p == "src/match-new.txt"));
        assert!(!tree.nodes["src/nested"].expanded);
        assert!(!tree.nodes["untouched"].expanded);
        // 清空搜索后仍只展示原先展开的层级。
        tree.set_filter(None);
        assert!(tree.nodes["src"].expanded);
        assert!(!tree.nodes["src/nested"].expanded);
    }

    /// 全选受搜索约束，重复按不扩大范围；显式全量方法仍供无勾选提交使用。
    #[test]
    fn check_scope_respects_search_and_keeps_existing_picks() {
        let mut tree = TreePanel::new();
        let root = Path::new("/wc");
        tree.build_from_items(
            &[
                item("src/a.txt", 'M'),
                item("src/b.txt", 'M'),
                item("docs/c.txt", 'M'),
                item("tmp/d.tmp", '?'),
            ],
            root,
        );
        assert_eq!(
            tree.scope_committable().len(),
            3,
            "未纳管的 tmp/d.tmp 不算可提交"
        );

        // 关键词含 `/` 才按整条路径匹配；只写 "src" 匹配的是文件名。
        tree.set_filter(Some("src/".into()));
        assert_eq!(tree.scope_committable().len(), 2);
        assert_eq!(tree.check_scope(), 2);
        assert!(tree.is_checked("src/a.txt"));
        assert!(!tree.is_checked("docs/c.txt"), "过滤外的不该被勾上");

        // 重复全选不扩大搜索范围；提交入口仍可显式选择全部。
        assert_eq!(tree.check_scope(), 0);
        assert_eq!(tree.check_all_committable(), 1);
        assert!(tree.is_checked("docs/c.txt"));
        assert!(!tree.is_checked("tmp/d.tmp"), "未纳管永远不自动勾");

        // 重复调用不重复计数。
        assert_eq!(tree.check_all_committable(), 0);
        assert_eq!(tree.checked_count(), 3);
    }

    /// 反选遵守搜索范围，不影响搜索之外的勾选。
    #[test]
    fn invert_only_flips_the_search_range() {
        let mut tree = TreePanel::new();
        tree.build_from_items(
            &[item("a.txt", 'M'), item("b.txt", 'M'), item("c.txt", 'M')],
            Path::new("/wc"),
        );
        tree.set_filter(Some("a".into()));
        assert_eq!(tree.invert_scope(), 1);
        assert!(tree.is_checked("a.txt"));

        // 再反一次：a 取消，b/c 不受影响（它们不在可见范围内）。
        assert_eq!(tree.invert_scope(), 0);
        assert!(!tree.is_checked("a.txt"));
    }

    /// 已经用 Space 挑好的不该被「全选」冲掉 —— 全选只补勾。
    #[test]
    fn check_scope_adds_without_clearing_manual_picks() {
        let mut tree = TreePanel::new();
        tree.build_from_items(
            &[item("a.txt", 'M'), item("b.txt", 'M'), item("c.txt", 'M')],
            Path::new("/wc"),
        );
        tree.set_checked("b.txt", true);
        tree.set_filter(Some("a".into()));
        assert_eq!(tree.check_scope(), 1);
        assert!(tree.is_checked("b.txt"), "手动勾的还在");
        assert!(tree.is_checked("a.txt"));
    }

    #[test]
    fn disappearing_selection_falls_back_and_expired_checks_are_pruned() {
        let mut tree = TreePanel::new();
        let root = Path::new("/wc");
        tree.build_from_items(&[item("a.txt", 'M'), item("b.txt", 'M')], root);
        select(&mut tree, "b.txt");
        tree.set_checked("b.txt", true);
        tree.refresh_from_items(&[item("a.txt", '?')], root);
        assert_eq!(tree.selected().unwrap().rel, "a.txt");
        assert_eq!(tree.checked_count(), 0);
        tree.refresh_from_items(&[], root);
        assert!(tree.selected().is_none());
        assert_eq!(tree.state.offset(), 0);
    }

    #[test]
    fn directory_space_selects_folded_descendants_without_siblings_or_untracked() {
        let mut tree = TreePanel::new();
        tree.build_from_items(
            &[
                item("src/a.txt", 'M'),
                item("src/nested/b.txt", 'A'),
                item("src/new.txt", '?'),
                item("src/ignored.txt", 'I'),
                item("src/external.txt", 'X'),
                item("src2/c.txt", 'M'),
            ],
            Path::new("/wc"),
        );
        select(&mut tree, "src");
        tree.toggle();
        assert!(!tree.visible.contains(&"src/nested/b.txt".into()));
        assert_eq!(tree.toggle_check(), Some(true));
        assert_eq!(tree.checked_count(), 2);
        assert_eq!(
            tree.checked_abs(),
            vec![
                Path::new("/wc/src/a.txt"),
                Path::new("/wc/src/nested/b.txt")
            ]
        );
        assert_eq!(
            tree.checked_changes(),
            vec![("src/a.txt".into(), 'M'), ("src/nested/b.txt".into(), 'A')]
        );
        assert_eq!(tree.hidden_checked_count(), 2);
        assert_eq!(tree.toggle_check(), Some(false));
        assert_eq!(tree.checked_count(), 0);
    }

    #[test]
    fn partial_directory_selection_and_search_do_not_expand_scope() {
        let mut tree = TreePanel::new();
        tree.build_from_items(
            &[
                item("src/a.sql", 'M'),
                item("src/b.sql", 'D'),
                item("src/keep.txt", 'M'),
                item("other/c.sql", 'M'),
            ],
            Path::new("/wc"),
        );
        tree.set_checked("src/a.sql", true);
        assert_eq!(tree.directory_counts()["src"], (1, 3));
        tree.set_filter(Some(".sql".into()));
        select(&mut tree, "src");
        assert_eq!(tree.toggle_check(), Some(true));
        assert_eq!(tree.directory_counts()["src"], (2, 2));
        assert!(!tree.is_checked("src/keep.txt"));
        assert!(!tree.is_checked("other/c.sql"));
        tree.set_filter(Some("keep".into()));
        assert_eq!(tree.checked_abs().len(), 2, "搜索不丢失先前的勾选");
        assert_eq!(tree.hidden_checked_count(), 2);
        assert_eq!(tree.check_scope(), 1);
        assert_eq!(tree.check_scope(), 0);
        assert_eq!(tree.invert_scope(), 2);
        assert!(tree.is_checked("src/a.sql"));
    }

    #[test]
    fn check_scope_includes_folds_and_empty_directory_cannot_be_checked() {
        let mut tree = TreePanel::new();
        tree.build_from_items(
            &[item("src/a.txt", 'M'), item("new/b.txt", '?')],
            Path::new("/wc"),
        );
        select(&mut tree, "src");
        tree.toggle();
        assert_eq!(tree.check_scope(), 1);
        assert_eq!(tree.invert_scope(), 0);
        select(&mut tree, "new");
        assert!(tree.check_block_reason().is_some());
        assert_eq!(tree.toggle_check(), None);
    }

    #[test]
    fn compact_tree_renders_partial_counts_and_keeps_actual_file_targets() {
        let mut tree = TreePanel::new();
        tree.build_from_items(
            &[
                item("db/mysql/omc/a.sql", 'M'),
                item("db/mysql/omc/b.sql", 'M'),
            ],
            Path::new("/wc"),
        );
        assert_eq!(
            tree.visible,
            vec!["db/mysql/omc", "db/mysql/omc/a.sql", "db/mysql/omc/b.sql"]
        );
        tree.set_checked("db/mysql/omc/a.sql", true);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(70, 10)).unwrap();
        let frame = terminal.draw(|f| tree.render(f, f.area(), true)).unwrap();
        let text: String = frame.buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("[-] M db/mysql/omc/  1/2"));
        select(&mut tree, "db/mysql/omc");
        assert_eq!(tree.toggle_check(), Some(true));
        assert_eq!(tree.checked_count(), 2);
        tree.toggle();
        assert_eq!(tree.checked_abs().len(), 2);
        assert!(tree.checked_abs().iter().all(|p| p.extension().is_some()));
    }

    #[test]
    fn full_tree_loads_clean_siblings_even_when_changed_children_exist() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/changed.txt"), "changed").unwrap();
        std::fs::write(tmp.path().join("src/clean.txt"), "clean").unwrap();
        std::fs::create_dir(tmp.path().join("src/subdir")).unwrap();
        let mut tree = TreePanel::new();
        let mut changed = item("src/changed.txt", 'M');
        changed.abs = tmp.path().join(&changed.rel);
        let items = [changed];
        tree.build_from_items(&items, tmp.path());
        tree.set_checked("src/changed.txt", true);
        select(&mut tree, "src/changed.txt");
        tree.switch_mode(TreeMode::All, &items, tmp.path());
        assert_eq!(tree.selected().unwrap().rel, "src/changed.txt");
        assert!(tree.visible.contains(&"src/clean.txt".into()));
        assert!(!tree.is_checkable("src/clean.txt"));
        assert_eq!(tree.nodes["src"].children[0], "src/subdir");
        tree.set_filter(Some("changed".into()));
        tree.switch_mode(TreeMode::Changed, &items, tmp.path());
        assert_eq!(tree.selected().unwrap().rel, "src/changed.txt");
        assert_eq!(tree.filter().map(String::as_str), Some("changed"));
        assert_eq!(tree.checked_count(), 1);
        assert!(!tree.nodes.contains_key("src/clean.txt"));
    }

    #[test]
    fn full_tree_expands_unloaded_directory_with_changed_children() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/clean.txt"), "clean").unwrap();
        let mut tree = TreePanel::new();
        tree.build_from_items(&[item("src/changed.txt", 'M')], tmp.path());
        tree.set_mode(TreeMode::All);
        select(&mut tree, "src");
        assert!(!tree.toggle(), "折叠不加载");
        assert!(tree.toggle(), "展开时即使已有改动子节点也要读磁盘");
        tree.load_dir("src").unwrap();
        assert!(tree.visible.contains(&"src/clean.txt".into()));
    }
}
