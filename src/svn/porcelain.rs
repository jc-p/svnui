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

/// 解析 `svn status -u` 的输出，只挑出**远端有更新**的条目。
///
/// ## 为什么单独一个函数，而不是在 `parse_status` 里加字段
///
/// `StatusEntry` 是**序列化到 yazi 插件的 JSON** 的结构，加字段会改变
/// 协议形状。而且 `-u` 要连服务器，**慢且可能超时**，不能进热路径
/// （`parse_status` 每秒可能被调很多次）。
///
/// 所以这里单独解析，返回 (路径, 远端最新版本号)。
///
/// ## 格式
///
/// `svn status -u` 比普通 status 多两列（第 8 列 `*`、第 9 起是版本号）：
///
/// ```text
/// M                4521   src/main.rs     ← 第8列空格：只有本地改动
///         *        4521   src/lib.rs      ← 第8列 `*`：远端有更新
/// Status against revision:   4521
/// ```
///
/// 路径可含空格，所以取路径时必须用"跳过后导数字再 trim"，
/// 不能用 split_whitespace。
pub fn parse_outdated(out: &str) -> Vec<(String, u64)> {
    let mut v = Vec::new();
    for line in out.lines() {
        // -u 独有的收尾行，不是状态行
        if line.starts_with("Status against revision") {
            continue;
        }
        let mut chars = line.chars();
        // 第 8 列（下标 7）是 `*` 才表示远端有更新
        match chars.nth(7) {
            Some('*') => {}
            _ => continue,
        }
        let rest: String = chars.collect();
        let rest = rest.trim_start();

        // 版本号是行首连续数字
        let digits_end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits_end == 0 {
            continue; // 没有版本号，格式不符，跳过而不是塞垃圾
        }
        let rev = rest[..digits_end].parse::<u64>().unwrap_or(0);
        let path = rest[digits_end..].trim_start().to_string();
        if path.is_empty() {
            continue;
        }
        v.push((path, rev));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outdated_only_picks_star_column() {
        let out = "M                4521   src/main.rs\n        *        4521   src/lib.rs\nStatus against revision:   4521\n";
        let v = parse_outdated(out);
        assert_eq!(v.len(), 1, "只有带 * 的那行算远端有更新");
        assert_eq!(v[0].0, "src/lib.rs");
        assert_eq!(v[0].1, 4521);
    }

    #[test]
    fn outdated_keeps_spaces_in_path() {
        let out = "        *        100   a b/c d.txt\n";
        let v = parse_outdated(out);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].0, "a b/c d.txt", "路径含空格不能被 split 掉");
    }

    #[test]
    fn outdated_ignores_malformed_line() {
        let out = "        *   notanum   x.txt\n";
        assert!(parse_outdated(out).is_empty());
    }

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
