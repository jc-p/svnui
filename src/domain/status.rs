use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// 节点类型。文本解析（porcelain）拿不到，只有 XML 或 fs stat 能确定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
    /// 未探测（热路径默认不 stat，避免大仓库下几万次文件系统调用）。
    Unknown,
}

/// 仓库锁令牌（第 6 列）。带 `-u` 时语义更丰富。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LockToken {
    /// `K` —— 本工作副本持有锁。
    Present,
    /// `O` —— 锁在别的工作副本（仅 `-u`）。
    Other,
    /// `T` —— 锁被窃取（仅 `-u`）。
    Stolen,
    /// `B` —— 锁已失效（仅 `-u`）。
    Broken,
}

impl LockToken {
    pub fn sign(self) -> char {
        match self {
            LockToken::Present => 'K',
            LockToken::Other => 'O',
            LockToken::Stolen => 'T',
            LockToken::Broken => 'B',
        }
    }

    pub fn from_sign(c: char) -> Option<Self> {
        match c {
            'K' => Some(LockToken::Present),
            'O' => Some(LockToken::Other),
            'T' => Some(LockToken::Stolen),
            'B' => Some(LockToken::Broken),
            _ => None,
        }
    }
}

/// svn 的 item 状态全集（与 `svn status --xml` 的 `@item` 取值一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusKind {
    /// `none` —— 第 1/2 列为空格的含义：无改动。
    None,
    Normal,
    Unversioned,
    Added,
    Missing,
    Deleted,
    Replaced,
    Modified,
    Merged,
    Conflicted,
    Ignored,
    Obstructed,
    External,
    Incomplete,
}

impl StatusKind {
    /// 单字符符号，与 `svn status` 输出一致。
    pub fn sign(self) -> char {
        match self {
            StatusKind::None | StatusKind::Normal => ' ',
            StatusKind::Unversioned => '?',
            StatusKind::Added => 'A',
            StatusKind::Missing | StatusKind::Incomplete => '!',
            StatusKind::Deleted => 'D',
            StatusKind::Replaced => 'R',
            StatusKind::Modified => 'M',
            StatusKind::Merged => 'G',
            StatusKind::Conflicted => 'C',
            StatusKind::Ignored => 'I',
            StatusKind::Obstructed => '~',
            StatusKind::External => 'X',
        }
    }

    /// 从第 1 列字符反查。返回 `None` 表示这不是合法状态字符 —— 调用方应跳过该行。
    pub fn from_sign(c: char) -> Option<Self> {
        match c {
            ' ' => Some(StatusKind::None),
            '?' => Some(StatusKind::Unversioned),
            'A' => Some(StatusKind::Added),
            '!' => Some(StatusKind::Missing),
            'D' => Some(StatusKind::Deleted),
            'R' => Some(StatusKind::Replaced),
            'M' => Some(StatusKind::Modified),
            'G' => Some(StatusKind::Merged),
            'C' => Some(StatusKind::Conflicted),
            'I' => Some(StatusKind::Ignored),
            '~' => Some(StatusKind::Obstructed),
            'X' => Some(StatusKind::External),
            _ => None,
        }
    }

    /// 第 2 列（属性状态）只允许 ` ` / `M` / `C`。
    pub fn from_prop_sign(c: char) -> Option<Self> {
        match c {
            ' ' => Some(StatusKind::None),
            'M' => Some(StatusKind::Modified),
            'C' => Some(StatusKind::Conflicted),
            _ => None,
        }
    }

    /// 冒泡优先级：父目录显示子树里"最严重"的状态。数值越大越严重。
    pub fn priority(self) -> u8 {
        match self {
            StatusKind::Conflicted => 100,
            StatusKind::Missing | StatusKind::Incomplete => 90,
            StatusKind::Obstructed => 80,
            StatusKind::Replaced => 70,
            StatusKind::Deleted => 60,
            StatusKind::Added => 50,
            StatusKind::Modified => 40,
            StatusKind::Merged => 35,
            StatusKind::Unversioned => 20,
            StatusKind::Ignored => 10,
            StatusKind::External => 5,
            StatusKind::None | StatusKind::Normal => 0,
        }
    }

    /// 是否算"本地有变更"（`svnui changed` 的过滤依据）。
    ///
    /// 注意：`Ignored` 与 `External` 不计入 —— 它们不是待提交内容。
    pub fn is_changed(self) -> bool {
        matches!(
            self,
            StatusKind::Added
                | StatusKind::Deleted
                | StatusKind::Replaced
                | StatusKind::Modified
                | StatusKind::Merged
                | StatusKind::Conflicted
                | StatusKind::Missing
                | StatusKind::Incomplete
                | StatusKind::Obstructed
        )
    }

    pub fn is_conflicted(self) -> bool {
        matches!(self, StatusKind::Conflicted)
    }
}

/// 一条状态记录。路径相对工作副本根。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEntry {
    pub path: Utf8PathBuf,
    /// 第 1 列 —— 文本/内容状态。
    pub text: StatusKind,
    /// 第 2 列 —— 属性状态。`_M` 表示只有属性改了。
    pub props: StatusKind,
    /// 第 3 列 `L` —— 工作副本目录被锁。
    pub locked: bool,
    /// 第 4 列 `+` —— 带历史的添加（copy 而来）。
    pub copied: bool,
    /// 第 5 列 `S` —— 切换到别的分支。
    pub switched: bool,
    /// 第 6 列 —— 锁令牌。
    pub lock_token: Option<LockToken>,
    /// 第 7 列 `C` —— 树冲突。
    pub tree_conflict: bool,
    pub kind: NodeKind,
}

impl StatusEntry {
    /// 展示用的单字符（取文本状态，属性变更时回落到 `M`/`C`）。
    pub fn sign(&self) -> char {
        if self.tree_conflict {
            return 'T';
        }
        if self.text.sign() != ' ' {
            return self.text.sign();
        }
        self.props.sign()
    }

    /// 两字符 porcelain，与 git 手感一致：`AM` / `_M` / `A+`。
    ///
    /// 这里第二字符直接取属性符号，空格替换为 `_` 让"仅属性改动"在终端里可见。
    pub fn porcelain(&self) -> String {
        let a = if self.tree_conflict { 'T' } else { self.text.sign() };
        let b = self.props.sign();
        let a = if a == ' ' { '_' } else { a };
        let b = if b == ' ' { '_' } else { b };
        format!("{a}{b}")
    }

    /// 综合优先级，用于目录冒泡取最大值。
    pub fn priority(&self) -> u8 {
        let mut p = self.text.priority().max(self.props.priority());
        if self.tree_conflict {
            p = p.max(100);
        }
        p
    }

    pub fn is_changed(&self) -> bool {
        self.text.is_changed() || self.props.is_changed() || self.tree_conflict
    }

    pub fn is_conflicted(&self) -> bool {
        self.text.is_conflicted() || self.props.is_conflicted() || self.tree_conflict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_roundtrip_covers_all_kinds() {
        let all = [
            StatusKind::None,
            StatusKind::Normal,
            StatusKind::Unversioned,
            StatusKind::Added,
            StatusKind::Missing,
            StatusKind::Deleted,
            StatusKind::Replaced,
            StatusKind::Modified,
            StatusKind::Merged,
            StatusKind::Conflicted,
            StatusKind::Ignored,
            StatusKind::Obstructed,
            StatusKind::External,
            StatusKind::Incomplete,
        ];
        for k in all {
            let c = k.sign();
            // 两处有意的归一化：
            // - Normal 与 None 都是 ' '，反查统一得 None（"无改动"只有一种含义）
            // - Incomplete 与 Missing 都是 '!'，反查统一得 Missing
            // 因此这两个不能要求 roundtrip 回原值，只断言符号正确。
            if matches!(k, StatusKind::Incomplete | StatusKind::Normal) {
                continue;
            }
            assert_eq!(StatusKind::from_sign(c), Some(k), "sign({k:?}) = {c:?}");
        }
        assert_eq!(StatusKind::Normal.sign(), ' ');
        assert_eq!(StatusKind::Incomplete.sign(), '!');
    }

    #[test]
    fn invalid_sign_returns_none() {
        for c in ['>', 'Z', '1', '\t'] {
            assert_eq!(StatusKind::from_sign(c), None);
        }
    }

    #[test]
    fn priority_conflict_wins() {
        // 完整顺序：Conflicted > Missing > Obstructed > Replaced > Deleted > Added > Modified
        assert!(StatusKind::Conflicted.priority() > StatusKind::Missing.priority());
        assert!(StatusKind::Missing.priority() > StatusKind::Obstructed.priority());
        assert!(StatusKind::Deleted.priority() > StatusKind::Added.priority());
        assert!(StatusKind::Added.priority() > StatusKind::Modified.priority());
        assert!(StatusKind::Modified.priority() > StatusKind::Unversioned.priority());
        assert_eq!(StatusKind::None.priority(), 0);
        assert_eq!(StatusKind::Normal.priority(), 0);
    }

    #[test]
    fn prop_only_change_shows_underscore_m() {
        let e = StatusEntry {
            path: Utf8PathBuf::from("src/lib.rs"),
            text: StatusKind::None,
            props: StatusKind::Modified,
            locked: false,
            copied: false,
            switched: false,
            lock_token: None,
            tree_conflict: false,
            kind: NodeKind::File,
        };
        assert_eq!(e.porcelain(), "_M");
        assert_eq!(e.sign(), 'M');
        assert!(e.is_changed());
    }

    #[test]
    fn tree_conflict_beats_text_status() {
        let e = StatusEntry {
            path: Utf8PathBuf::from("src/x.rs"),
            text: StatusKind::Modified,
            props: StatusKind::None,
            locked: false,
            copied: false,
            switched: false,
            lock_token: None,
            tree_conflict: true,
            kind: NodeKind::File,
        };
        assert_eq!(e.sign(), 'T');
        assert_eq!(e.priority(), 100);
    }
}
