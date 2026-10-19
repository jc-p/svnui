//! 真机集成测试：**需要 svn + svnadmin**，默认全部跳过。
//!
//! 静态 fixture 测不了的东西只有这里能测：
//!   - `resolve` 之后状态是否真的归位
//!   - trio 文件（`.mine` / `.rOLD` / `.rNEW`）是否被清除
//!   - commit 是否真的产生 revision
//!
//! 跑法：
//! ```bash
//! just test-all                      # 有 svnadmin 的机器
//! cargo test -- --ignored            # 只跑这些
//! ```
//!
//! 默认由 `#[ignore]` 排除；显式执行时工具缺失和准备失败必须报错。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 显式执行集成测试即要求工具可用；不把准备失败误报为通过。
fn tool(name: &str) -> PathBuf {
    let path = std::env::var_os(format!("SVNR_TEST_{}", name.to_uppercase()))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(name));
    let out = Command::new(&path)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("集成测试需要 {name}: {e}"));
    assert!(out.status.success(), "集成测试工具 {name} 不可用");
    path
}

fn svn() -> PathBuf {
    tool("svn")
}
fn svnadmin() -> PathBuf {
    tool("svnadmin")
}

fn run(exe: &Path, args: &[&str], cwd: &Path) -> std::io::Result<String> {
    let out = Command::new(exe)
        .args(args)
        .arg("--non-interactive")
        .env("LC_ALL", "C")
        .current_dir(cwd)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "{} {:?} 失败 ({}): {}",
            exe.display(),
            args,
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 造一个工作副本：repo + wc，含一个已提交的文件。
struct Fixture {
    _tmp: tempfile::TempDir,
    wc: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let svnadmin = svnadmin();
        let svn = svn();
        let tmp = tempfile::TempDir::new().expect("创建集成测试临时目录");
        let repo = tmp.path().join("repo");
        let wc = tmp.path().join("wc");

        let created = Command::new(&svnadmin)
            .arg("create")
            .arg(&repo)
            .output()
            .expect("启动 svnadmin create");
        assert!(
            created.status.success(),
            "创建测试仓库失败: {}",
            String::from_utf8_lossy(&created.stderr)
        );
        let url = format!("file://{}", repo.display());
        run(
            &svn,
            &["co", &url, wc.to_str().expect("临时路径为 UTF-8")],
            tmp.path(),
        )
        .expect("检出测试工作副本");
        std::fs::write(wc.join("a.txt"), "one\n").expect("创建初始文件");
        run(&svn, &["add", "a.txt"], &wc).expect("添加初始文件");
        run(&svn, &["ci", "-m", "init"], &wc).expect("提交初始文件");
        Self { _tmp: tmp, wc }
    }
}

/// 冲突的完整生命周期：制造 → 断言 C → resolve → 断言归位 + trio 清除。
///
/// 这是整个项目里**唯一**能验证"冲突真的被处理干净"的测试。
/// 静态 fixture 只能验证解析，验证不了副作用。
#[test]
#[ignore = "需要 svn + svnadmin"]
fn conflict_lifecycle_clears_trio_files() {
    let f = Fixture::new();
    let svn = svn();

    // 本地改
    std::fs::write(f.wc.join("a.txt"), "local\n").unwrap();

    // 另一个工作副本改同一个文件并提交
    let wc2 = f.wc.parent().unwrap().join("wc2");
    let url = format!("file://{}", f.wc.parent().unwrap().join("repo").display());
    run(
        &svn,
        &["co", &url, wc2.to_str().unwrap()],
        f.wc.parent().unwrap(),
    )
    .unwrap();
    std::fs::write(wc2.join("a.txt"), "remote\n").unwrap();
    run(&svn, &["ci", "-m", "remote"], &wc2).unwrap();

    // 回来 update，应产生冲突
    run(&svn, &["up", "--accept", "postpone"], &f.wc).expect("更新并制造冲突");

    let st = run(&svn, &["status"], &f.wc).unwrap();
    assert!(
        st.lines().any(|l| l.starts_with('C')),
        "应产生冲突，实际状态：\n{st}"
    );

    // 三件套应该存在
    let trio = ["a.txt.mine", "a.txt.r1", "a.txt.r2"];
    assert!(
        f.wc.join("a.txt.mine").exists(),
        "冲突后应生成 .mine 文件（三件套之一）"
    );

    // 解决
    run(&svn, &["resolve", "--accept", "mine-full", "a.txt"], &f.wc).unwrap();

    // 断言 1：状态归位（不再是 C）
    let st2 = run(&svn, &["status"], &f.wc).unwrap();
    assert!(
        !st2.lines().any(|l| l.starts_with('C')),
        "resolve 后不应还有冲突状态，实际：\n{st2}"
    );

    // 断言 2：trio 文件被清除
    for t in trio {
        assert!(
            !f.wc.join(t).exists(),
            "resolve 后 {t} 应被 svn 自动清除，残留会污染工作副本"
        );
    }
}

/// 提交真的产生 revision，且状态清空。
#[test]
#[ignore = "需要 svn + svnadmin"]
fn commit_produces_revision_and_clears_status() {
    let f = Fixture::new();
    let svn = svn();

    std::fs::write(f.wc.join("a.txt"), "changed\n").unwrap();
    let st = run(&svn, &["status"], &f.wc).unwrap();
    assert!(st.starts_with('M'), "应先有修改状态，实际：{st}");

    let ci = run(&svn, &["ci", "-m", "change"], &f.wc).unwrap();
    assert!(
        ci.contains("Committed revision"),
        "提交应回显 revision，实际：{ci}"
    );

    let st2 = run(&svn, &["status"], &f.wc).unwrap();
    assert!(st2.trim().is_empty(), "提交后状态应清空，实际：{st2}");
}

/// 7 列解析在真实输出上不产生幽灵路径。
///
/// 树冲突会多打一行说明（第 7 列 `>`），`Summary of conflicts:` 也有统计块。
/// 静态 fixture 覆盖了，但真实输出可能还有别的边缘情况。
#[test]
#[ignore = "需要 svn + svnadmin"]
fn real_status_parses_without_phantom_paths() {
    let f = Fixture::new();
    let svn = svn();

    std::fs::write(f.wc.join("a.txt"), "x\n").unwrap();
    std::fs::write(f.wc.join("new.txt"), "y\n").unwrap();
    run(&svn, &["add", "new.txt"], &f.wc).unwrap();

    let out = run(&svn, &["status"], &f.wc).unwrap();
    let entries = svnui::svn::porcelain::parse_status(&out);

    // 每条解析出来的路径都必须真实存在（或是被删除的）
    for e in &entries {
        let p = f.wc.join(e.path.as_str());
        let exists = p.exists();
        let is_deleted = e.text == svnui::domain::StatusKind::Deleted;
        assert!(
            exists || is_deleted,
            "解析出幽灵路径 {:?}（状态 {:?}）—— 说明有行没被正确跳过",
            e.path,
            e.text
        );
    }
}
