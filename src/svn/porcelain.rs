use camino::Utf8PathBuf;

use crate::domain::{LockToken, NodeKind, StatusEntry, StatusKind};

/// 解析 `svn status`（**不带** `-u`）的 7 列文本输出。
///
/// 这是状态查询的**热路径**：大仓库下 `status` 文本比 `--xml` 快一个量级
/// （XML 体积是文本的 5–10 倍，还要走完整反序列化）。
///
/// # 7 列语义
///
/// | 列 | 含义 | 取值 |
/// |---|---|---|
/// | 1 | 文本状态 | ` ` `A` `C` `D` `I` `M` `R` `X` `?` `!` `~` `G` |
/// | 2 | 属性状态 | ` ` `M` `C` |
/// | 3 | 工作副本锁 | ` ` `L` |
/// | 4 | 带历史的添加 | ` ` `+` |
/// | 5 | 切换 / 文件外部 | ` ` `S` `X` |
/// | 6 | 仓库锁令牌 | ` ` `K` `O` `T` `B` |
/// | 7 | 树冲突 | ` ` `C` |
///
/// 第 8 列恒为空格，路径从第 9 个字符开始。**路径可含空格，永远不能用 split。**
///
/// # 会被跳过的行
/// - 树冲突说明行（第 7 列是 `>`）
/// - `Summary of conflicts:` 及其后的 `Text conflicts: 1` 等统计行
/// - `Status against revision: N`（`-u` 才有）
/// - 任何列取值非法的内容 —— 宁可漏一行，也不能把垃圾塞进状态表产生"幽灵路径"
pub fn parse_status(out: &str) -> Vec<StatusEntry> {
    let mut v = Vec::new();
    for line in out.lines() {
        if let Some(e) = parse_line(line) {
            v.push(e);
        }
    }
    v
}

/// 解析单行。返回 `None` 表示这不是状态行（见模块文档）。
pub fn parse_line(line: &str) -> Option<StatusEntry> {
    if line.is_empty() {
        return None;
    }

    // 快路径：ASCII 行按字节切，避免每行都做 char 分配（大仓库几十万行很敏感）。
    // 慢路径兜底多字节内容，保证不 panic。
    let (cols, rest) = if line.len() >= 8 && line.is_char_boundary(7) {
        let (c, r) = line.split_at(7);
        let cols: Vec<char> = c.chars().collect();
        (cols, r)
    } else {
        let mut chars = line.chars();
        let cols: Vec<char> = chars.by_ref().take(7).collect();
        if cols.len() < 7 {
            return None;
        }
        (cols, "")
    };

    let text = StatusKind::from_sign(cols[0])?;
    let props = StatusKind::from_prop_sign(cols[1])?;

    // 树冲突说明行：第 7 列是 '>'，例如
    //   "      >   local delete, incoming edit upon update"
    if cols[6] == '>' {
        return None;
    }

    // 其余列取值校验 —— 抓出 "Text conflicts: 1" 这类統計行。
    if !matches!(cols[2], ' ' | 'L') {
        return None;
    }
    if !matches!(cols[3], ' ' | '+') {
        return None;
    }
    if !matches!(cols[4], ' ' | 'S' | 'X') {
        return None;
    }
    if !matches!(cols[5], ' ' | 'K' | 'O' | 'T' | 'B') {
        return None;
    }
    if !matches!(cols[6], ' ' | 'C') {
        return None;
    }

    // 第 8 列恒为一个空格，跳过它之后就是路径（路径可含空格，不能再 trim）。
    let path = rest.strip_prefix(' ')?;
    if path.is_empty() {
        return None;
    }

    Some(StatusEntry {
        path: Utf8PathBuf::from(path),
        text,
        props,
        locked: cols[2] == 'L',
        copied: cols[3] == '+',
        switched: cols[4] == 'S',
        lock_token: LockToken::from_sign(cols[5]),
        tree_conflict: cols[6] == 'C',
        // 文本格式拿不到节点类型；需要时由 client 层补 stat。yazi 染色不需要它。
        kind: NodeKind::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> StatusEntry {
        parse_line(line).unwrap_or_else(|| panic!("应当解析成功: {line:?}"))
    }

    #[test]
    fn parses_modified_and_added() {
        let e = one("M       src/main.rs");
        assert_eq!(e.path, "src/main.rs");
        assert_eq!(e.text, StatusKind::Modified);
        assert_eq!(e.props, StatusKind::None);

        let e = one("A       src/new.rs");
        assert_eq!(e.text, StatusKind::Added);
    }

    #[test]
    fn parses_prop_only_change() {
        let e = one(" M      src/lib.rs");
        assert_eq!(e.text, StatusKind::None);
        assert_eq!(e.props, StatusKind::Modified);
        assert_eq!(e.porcelain(), "_M");
    }

    #[test]
    fn parses_all_column_flags() {
        // 列位（0-based）：0'A' 文本=Added，1' ' 属性，2' ' 锁，3'+' copied，
        // 4' ' switched，5'K' 锁令牌，6'C' 树冲突，7' ' 分隔，8+ 路径。
        // 注意 5 和 6 之间**没有**空格 —— 每列严格一个字符宽，多一个就整行错位。
        let e = one("A  + KC src/tc.rs");

        // 反例：把 'C' 挤到第 8 列会被静默丢弃（解析失败而不是报错），
        // 这正是为什么这个断言必须存在。
        assert!(parse_line("A  + K C src/tc.rs").is_none());
        assert_eq!(e.text, StatusKind::Added);
        assert!(e.copied);
        assert_eq!(e.lock_token, Some(LockToken::Present));
        assert!(e.tree_conflict);
        assert_eq!(e.sign(), 'T');
    }

    #[test]
    fn path_with_spaces_is_preserved() {
        let e = one("M       src/my file name.rs");
        assert_eq!(e.path, "src/my file name.rs");
    }

    #[test]
    fn utf8_path_is_preserved() {
        let e = one("M       src/中文 文件.rs");
        assert_eq!(e.path, "src/中文 文件.rs");
    }

    #[test]
    fn tree_conflict_detail_line_is_skipped() {
        assert!(parse_line("      >   local delete, incoming edit upon update").is_none());
    }

    #[test]
    fn summary_block_is_skipped() {
        assert!(parse_line("Summary of conflicts:").is_none());
        assert!(parse_line("  Text conflicts: 1").is_none());
        assert!(parse_line("  Tree conflicts: 1").is_none());
        assert!(parse_line("Status against revision: 981").is_none());
    }

    #[test]
    fn missing_obstructed_ignored_external() {
        assert_eq!(one("!       a.rs").text, StatusKind::Missing);
        assert_eq!(one("~       a").text, StatusKind::Obstructed);
        assert_eq!(one("I       target").text, StatusKind::Ignored);
        assert_eq!(one("X       ext").text, StatusKind::External);
        assert_eq!(one("?       new.rs").text, StatusKind::Unversioned);
        assert_eq!(one("R       r.rs").text, StatusKind::Replaced);
        assert_eq!(one("D       d.rs").text, StatusKind::Deleted);
        assert_eq!(one("C       c.rs").text, StatusKind::Conflicted);
    }

    #[test]
    fn batch_parse_filters_noise() {
        let out = concat!(
            "M       src/main.rs\n",
            "?       src/new.rs\n",
            "!     C src/tc.rs\n",
            "      >   local delete, incoming edit upon update\n",
            "Summary of conflicts:\n",
            "  Tree conflicts: 1\n",
        );
        let v = parse_status(out);
        assert_eq!(v.len(), 3, "说明行与统计块必须被过滤掉");
        assert!(v.iter().any(|e| e.tree_conflict));
    }

    #[test]
    fn empty_and_short_lines_are_skipped() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
        assert!(parse_line("M").is_none());
    }
}
