//! 面板集合。
//!
//! 每个面板一个文件，各自管自己的状态和渲染。
//! 共享的条目模型放这里。

pub mod commit;
pub mod diff;
pub mod status;

use crate::domain::StatusEntry;

/// 列表里的一行。
///
/// 从 `StatusEntry` 扁平化而来 —— UI 层不该关心 7 列 porcelain 的细节，
/// 只关心"显示什么符号、什么颜色、绝对路径是什么"。
#[derive(Debug, Clone)]
pub struct Item {
    /// 相对工作副本根的路径，用于显示。
    pub rel: String,
    /// 绝对路径，传给 svn 用。
    pub abs: std::path::PathBuf,
    /// 单字符状态符号（`M` / `A` / `?` / `C` ...）。
    pub sign: char,
    /// 完整两字符 porcelain（`_M` / `A+` 这类），detail 用。
    pub xy: String,
    pub is_dir: bool,
}

impl Item {
    /// 从 domain 的条目转换。
    ///
    /// `root` 用来算相对路径；算不出来（不在 root 下）就退化为绝对路径，
    /// 宁可显示得长一点也不丢信息。
    pub fn from_entry(e: &StatusEntry, root: &std::path::Path) -> Self {
        let abs = e.path.as_std_path().to_path_buf();
        let rel = abs
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| abs.to_string_lossy().to_string());

        // 取"更严重"的那一列：属性改动（第二列）通常不如内容改动值得关注，
        // 但冲突必须优先。
        let text = e.text.sign();
        let props = e.props.sign();

        let sign = if e.tree_conflict {
            'T'
        } else if text == 'C' {
            'C'
        } else if text != ' ' && text != '_' {
            text
        } else if props != ' ' {
            // 只有属性改了 —— 显示 `_` 前缀的形式不够直观，直接用 'M' 配青色
            if props == 'M' {
                'M'
            } else {
                props
            }
        } else {
            ' '
        };

        Self {
            rel,
            abs,
            sign,
            xy: format!("{}{}", text, props),
            is_dir: matches!(e.kind, crate::domain::NodeKind::Dir),
        }
    }

    /// 能否进入提交候选。
    ///
    /// `?`（未版本化）/ `I`（忽略）/ `X`（外部）都不行 ——
    /// svn add 之前它们根本不在版本控制里。
    pub fn committable(&self) -> bool {
        !matches!(self.sign, '?' | 'I' | 'X' | ' ')
    }

    /// 显示名：目录加个尾斜杠，和命令行习惯一致。
    pub fn display(&self) -> String {
        if self.is_dir {
            format!("{}/", self.rel)
        } else {
            self.rel.clone()
        }
    }
}
