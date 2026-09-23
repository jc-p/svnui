//! 提交信息生成 —— **纯本地规则，不联网、不调任何外部服务**。
//!
//! ## 为什么只做规则
//!
//! 代码是要提交到公司仓库的，把 diff 发给外部模型这件事不能默认做。
//! 规则能给出的其实只有两件确定的事：**改动的性质**（type）和**改动的
//! 位置**（scope）。"为什么改"只有人知道 —— 所以这里生成的是**可编辑的
//! 骨架**，不是最终答案：生成完光标落在末尾，接着写就行。
//!
//! ## 规则一览
//!
//! | 判定 | 取值 |
//! |---|---|
//! | 全是文档 | `docs` |
//! | 全是测试 | `test` |
//! | 全是构建/配置/CI | `chore` |
//! | 含删除且有其他改动 | `refactor` |
//! | 有新增 | `feat` |
//! | 只有删除 | `chore` |
//! | 其余（以修改为主） | `fix` |
//!
//! scope 取**公共目录去掉通用前缀后的最后一段**（`src/tui/panels` → `panels`），
//! 没有可用公共目录就不写 scope —— 硬凑一个比不写更误导。

/// 一次改动。生成提交信息的唯一输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// 相对工作副本根的路径（`src/tui/app.rs`）。
    pub rel: String,
    /// 状态符号（`M` / `A` / `D` / `R` / `C` / `T` / `!`）。
    pub sign: char,
}

impl Change {
    pub fn new(rel: impl Into<String>, sign: char) -> Self {
        Self {
            rel: rel.into(),
            sign,
        }
    }
}

/// 开头的"通用目录名"：几乎所有仓库都有，做 scope 没有信息量。
/// （`src/tui/panels` 里 `src` 没用，`tui`/`panels` 才有。）
const GENERIC_DIRS: [&str; 13] = [
    "src", "source", "sources", "lib", "libs", "app", "apps", "trunk", "branch", "branches",
    "tags",
    // `tests/` 也没信息量：type 已经是 `test` 了，再写一遍 scope 是废话
    "test", "tests",
];

/// 路径归一：Windows 也用 `/` 分隔，后面一律按 `/` 切。
fn norm(path: &str) -> String {
    path.replace('\\', "/")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// 扩展名（小写），没有点或点在最开头则返回空串（`.gitignore` 算没有扩展名）。
fn ext_of(path: &str) -> String {
    let name = file_name(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn is_doc(path: &str) -> bool {
    let p = norm(path);
    if p.split('/')
        .any(|s| s.eq_ignore_ascii_case("docs") || s.eq_ignore_ascii_case("doc"))
    {
        return true;
    }
    matches!(
        ext_of(&p).as_str(),
        "md" | "markdown" | "rst" | "adoc" | "txt" | "pdf"
    )
}

fn is_test(path: &str) -> bool {
    let lower = norm(path).to_ascii_lowercase();
    let name = file_name(&lower).to_string();
    if name.starts_with("test_") || name.starts_with("tests.") {
        return true;
    }
    if name.contains("_test.") || name.contains(".test.") || name.contains("_spec.") {
        return true;
    }
    lower
        .split('/')
        .any(|s| s == "test" || s == "tests" || s == "__tests__")
}

fn is_build(path: &str) -> bool {
    let lower = norm(path).to_ascii_lowercase();
    if lower.starts_with(".github/")
        || lower.contains("/.github/")
        || lower.starts_with(".gitlab/")
        || lower.contains("/.gitlab/")
    {
        return true;
    }
    let name = file_name(&lower).to_string();
    matches!(
        name.as_str(),
        "cargo.toml"
            | "cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "makefile"
            | "cmakelists.txt"
            | "go.mod"
            | "go.sum"
            | "pom.xml"
            | "build.gradle"
            | "requirements.txt"
            | "pyproject.toml"
            | "setup.py"
            | "justfile"
            | "dockerfile"
            | ".gitignore"
            | ".editorconfig"
            | "flake.nix"
    )
}

/// 改动归类：`add` / `del` / `mod`。
///
/// `R`（替换）算新增 —— 对用户来说"冒出来一个新东西"比"内部结构变了"直观。
fn group_of(sign: char) -> &'static str {
    match sign {
        'A' | 'R' => "add",
        'D' | '!' => "del",
        _ => "mod",
    }
}

fn counts(changes: &[Change]) -> (usize, usize, usize) {
    let (mut add, mut del, mut modify) = (0usize, 0usize, 0usize);
    for c in changes {
        match group_of(c.sign) {
            "add" => add += 1,
            "del" => del += 1,
            _ => modify += 1,
        }
    }
    (add, del, modify)
}

/// 改动性质 → conventional commit 的 type。
fn kind_of(changes: &[Change]) -> &'static str {
    if changes.is_empty() {
        return "chore";
    }
    if changes.iter().all(|c| is_doc(&c.rel)) {
        return "docs";
    }
    if changes.iter().all(|c| is_test(&c.rel)) {
        return "test";
    }
    if changes.iter().all(|c| is_build(&c.rel)) {
        return "chore";
    }
    let (add, del, modify) = counts(changes);
    if del > 0 && (add > 0 || modify > 0) {
        "refactor"
    } else if add > 0 {
        "feat"
    } else if del > 0 {
        "chore"
    } else {
        "fix"
    }
}

/// scope 清洗：空白变 `-`，只留字母数字（含中文）和 `_ - . /`，最长 24。
fn sanitize(s: &str) -> Option<String> {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_whitespace() {
            out.push('-');
        } else if ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/') {
            out.push(ch);
        }
    }
    let out = out.trim_matches(|c| c == '-' || c == '/').to_string();
    if out.is_empty() {
        None
    } else {
        Some(out.chars().take(24).collect())
    }
}

/// 公共目录 → scope。取**去掉通用前缀后**的最后一段。
///
/// 单文件 `src/tui/panels/repo.rs` → `panels`；
/// 多文件同处 `A8升级包/A8_V26.2.2.0/*` → `A8_V26.2.2.0`；
/// 跨模块乱改 → `None`（不硬凑）。
fn scope_of(changes: &[Change]) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    let mut common: Option<Vec<String>> = None;
    for c in changes {
        let p = norm(&c.rel);
        let segs: Vec<&str> = p.split('/').collect();
        let dirs: Vec<String> = segs[..segs.len().saturating_sub(1)]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        match &mut common {
            None => common = Some(dirs),
            Some(cur) => {
                let n = cur
                    .iter()
                    .zip(dirs.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                cur.truncate(n);
            }
        }
    }
    let mut dirs = common.unwrap_or_default();
    while let Some(first) = dirs.first() {
        let generic = GENERIC_DIRS
            .iter()
            .any(|g| first.eq_ignore_ascii_case(g));
        if generic {
            dirs.remove(0);
        } else {
            break;
        }
    }
    let last = dirs.last()?;
    sanitize(last)
}

/// 中文摘要。单文件点名，多文件给构成 —— 规则推断不出"为什么改"，
/// 不编造意图，只把**改了什么、改了多少**说清楚。
fn subject_of(changes: &[Change]) -> String {
    if changes.is_empty() {
        return "更新若干文件".to_string();
    }
    if changes.len() == 1 {
        let c = &changes[0];
        let name = file_name(&norm(&c.rel)).to_string();
        let verb = match group_of(c.sign) {
            "add" => "新增",
            "del" => "删除",
            _ => "修改",
        };
        return format!("{} {}", verb, name);
    }
    let (add, del, modify) = counts(changes);
    let mut parts: Vec<String> = Vec::new();
    if add > 0 {
        parts.push(format!("新增 {} 个", add));
    }
    if modify > 0 {
        parts.push(format!("修改 {} 个", modify));
    }
    if del > 0 {
        parts.push(format!("删除 {} 个", del));
    }
    format!("更新 {} 个文件（{}）", changes.len(), parts.join("、"))
}

/// 候选列表，按"最可能直接用"排序：
///
/// 1. `type(scope): subject`
/// 2. `type: subject`（1 有 scope 时才给，方便不喜欢 scope 的人）
/// 3. 纯中文 `subject`（svn 提交信息不强求 conventional）
/// 4. 带文件清单的版本（2~8 个文件时才给，多了清单没意义）
pub fn suggest(changes: &[Change]) -> Vec<String> {
    let kind = kind_of(changes);
    let subject = subject_of(changes);
    let scope = scope_of(changes);

    let head = match &scope {
        Some(s) => format!("{}({}): {}", kind, s, subject),
        None => format!("{}: {}", kind, subject),
    };

    let mut out = vec![head.clone()];
    if scope.is_some() {
        out.push(format!("{}: {}", kind, subject));
    }
    out.push(subject.clone());

    if changes.len() >= 2 && changes.len() <= 8 {
        let mut body = String::new();
        for c in changes {
            body.push_str(&format!("\n- {} {}", c.sign, norm(&c.rel)));
        }
        out.push(format!("{}\n{}", head, body));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(rel: &str, sign: char) -> Change {
        Change::new(rel, sign)
    }

    #[test]
    fn 全是新增是feat() {
        let v = vec![c("src/tui/panels/repo.rs", 'A')];
        assert_eq!(kind_of(&v), "feat");
        assert_eq!(scope_of(&v), Some("panels".into()));
    }

    #[test]
    fn 文档改动是docs() {
        let v = vec![c("docs/authentication.md", 'M'), c("README.md", 'M')];
        assert_eq!(kind_of(&v), "docs");
    }

    #[test]
    fn 混合删除是refactor() {
        let v = vec![c("src/svn/client.rs", 'M'), c("src/svn/old.rs", 'D')];
        assert_eq!(kind_of(&v), "refactor");
    }

    #[test]
    fn 跨模块没有scope() {
        let v = vec![c("src/tui/app.rs", 'M'), c("docs/guide.md", 'M')];
        assert_eq!(scope_of(&v), None);
    }

    #[test]
    fn 候选项不为空且含带scope版本() {
        let v = vec![c("src/tui/app.rs", 'M'), c("src/tui/panels/repo.rs", 'A')];
        let s = suggest(&v);
        assert!(s.len() >= 3);
        assert!(s[0].starts_with("feat(tui"));
    }
}
