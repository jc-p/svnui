//! 救援：conflicts / resolve / cleanup / purge-mine / doctor。

use std::path::PathBuf;

use crate::domain::{Error, StatusEntry, StatusKind};
use crate::policy::{cleanup_danger, danger_of, judge, render_targets, Op, Verdict};
use crate::svn::Svn;

fn guard(
    op: Op,
    danger: crate::policy::Danger,
    targets: &[String],
    dry_run: bool,
    yes: bool,
    tty: bool,
) -> Result<Option<String>, Error> {
    match judge(op, danger, dry_run, yes, tty) {
        Verdict::Proceed => Ok(Some(String::new())),
        Verdict::DryRun => {
            let mut s = render_targets(&format!("[干跑] {op} 将作用于："), targets, 50);
            s.push_str("\n（未实际执行）");
            Ok(Some(s))
        }
        Verdict::NeedConfirm { reason } => {
            let mut s = render_targets(&format!("{op} 将作用于："), targets, 50);
            s.push_str(&format!("\n{reason}\n"));
            if crate::policy::confirm(&s) {
                Ok(Some(String::new()))
            } else {
                Ok(Some("已取消".to_string()))
            }
        }
        Verdict::Refused { reason } => Err(Error::Parse(reason)),
    }
}

/// 列出冲突。这是 yazi 冲突弹框的数据源。
pub fn conflicts(cli: &super::Cli, svn: &Svn, tty: bool) -> Result<String, Error> {
    let v = svn.conflicts()?;
    if cli.json {
        let data: Vec<_> = v
            .iter()
            .map(|e| {
                serde_json::json!({
                    "path": e.path,
                    "kind": if e.tree_conflict { "tree" }
                            else if e.props == StatusKind::Conflicted { "property" }
                            else { "text" },
                    "sign": e.sign().to_string(),
                })
            })
            .collect();
        return Ok(crate::output::ok(&data));
    }

    if v.is_empty() {
        return Ok("(无冲突)".to_string());
    }

    let mut out = String::new();
    for e in &v {
        let kind = if e.tree_conflict {
            "树冲突"
        } else if e.props == StatusKind::Conflicted {
            "属性冲突"
        } else {
            "文本冲突"
        };
        let s = e.sign();
        out.push_str(&crate::output::style::by_sign(s, &s.to_string(), tty));
        out.push_str(&format!("  {kind}  {}\n", e.path));
    }
    out.push_str(&format!(
        "\n共 {} 项。处理：svnui resolve -a mine-full|theirs-full|working [PATH...]\n",
        v.len()
    ));
    Ok(out)
}

pub fn resolve(
    svn: &Svn,
    accept: &str,
    paths: &[PathBuf],
    dry_run: bool,
    yes: bool,
    tty: bool,
) -> Result<String, Error> {
    // 校验策略名 —— 拼进 --accept= 之前必须拦住非法值，
    // 否则 svn 会报一堆看不懂的错。
    const VALID: &[&str] = &[
        "mine-full",
        "theirs-full",
        "working",
        "base",
        "mine-conflict",
        "theirs-conflict",
    ];
    if !VALID.contains(&accept) {
        return Err(Error::Parse(format!(
            "未知的 resolve 策略 `{accept}`。可选：{}",
            VALID.join(" / ")
        )));
    }

    let effective: Vec<String> = if paths.is_empty() {
        svn.conflicts()?.iter().map(|e| e.path.to_string()).collect()
    } else {
        paths.iter().map(|p| p.to_string_lossy().to_string()).collect()
    };

    if effective.is_empty() {
        return Ok("(无冲突)".to_string());
    }

    match guard(Op::Resolve, danger_of(Op::Resolve), &effective, dry_run, yes, tty)? {
        Some(text) if !text.is_empty() => return Ok(text),
        Some(_) => {}
        None => return Ok(String::new()),
    }

    svn.resolve(paths, accept)?;
    crate::cache::invalidate_for(&svn.root);
    Ok(format!("已按 `{accept}` 解决 {} 项", effective.len()))
}

/// **`--remove-unversioned` 是核弹**。默认关闭，开启后强制列清单 + 确认。
pub fn cleanup(
    svn: &Svn,
    remove_unversioned: bool,
    remove_ignored: bool,
    vacuum_pristines: bool,
    dry_run: bool,
    yes: bool,
    tty: bool,
) -> Result<String, Error> {
    let danger = cleanup_danger(remove_unversioned, remove_ignored);
    let targets: Vec<String> = if remove_unversioned || remove_ignored {
        // 高危时列出**将被删除**的文件，让用户看清代价。
        let opts = crate::svn::StatusOpts { include_ignored: remove_ignored, ..Default::default() };
        svn.status(&opts)?
            .into_iter()
            .filter(|e| e.text == StatusKind::Unversioned || e.text == StatusKind::Ignored)
            .map(|e| e.path.to_string())
            .collect()
    } else {
        vec!["(仅解锁，不删任何文件)".to_string()]
    };

    match guard(Op::Cleanup, danger, &targets, dry_run, yes, tty)? {
        Some(text) if !text.is_empty() => return Ok(text),
        Some(_) => {}
        None => return Ok(String::new()),
    }

    svn.cleanup(remove_unversioned, remove_ignored, vacuum_pristines)?;
    crate::cache::invalidate_for(&svn.root);
    Ok("清理完成".to_string())
}

/// `a.txt.mine` → (`a.txt`, "mine")；`.r<版本号>` 同理。不是三件套后缀返回 `None`。
///
/// 只认这两种后缀：目录里还有别的文件，而这**是删文件的操作** ——
/// 宁可漏一个，不能错删一个。
fn split_trio_suffix(name: &str) -> Option<(&str, &str)> {
    let (base, suffix) = name.rsplit_once('.')?;
    if base.is_empty() || suffix.is_empty() {
        return None;
    }
    // `.rOLD` / `.rNEW` 是文档里的叫法，磁盘上实际是 `.r<版本号>`。
    let is_rev =
        suffix.len() > 1 && suffix.starts_with('r') && suffix[1..].chars().all(|c| c.is_ascii_digit());
    if suffix == "mine" || is_rev {
        Some((base, suffix))
    } else {
        None
    }
}

/// 工作副本里残留的冲突三件套（绝对路径）。
///
/// 三件套 svn 从不纳入版本控制，所以它们一定以"未版本化 / 被忽略"的形态
/// 出现在 status 里 —— 因此要**从它们自己找**，而不是从父文件找：
/// 父文件一旦干净（冲突已解决、改动已提交）就不出现在 status 里，
/// 而那恰恰是最常见的残留场景。从父文件找会把这类全漏掉 ——
/// 之前 `doctor` 就这么写，结果在有残留的干净仓库上一路报"健康"。
///
/// 两条排除规则：
/// - 父文件**仍在冲突中** → 那三个文件是唯一的现场证据，删了没法比对
/// - 父文件本身也未版本化 → 那是用户自己的 `xxx.mine`，不是冲突残留
fn residual_files(root: &std::path::Path, entries: &[StatusEntry]) -> Vec<PathBuf> {
    let conflicted: std::collections::HashSet<&str> = entries
        .iter()
        .filter(|e| e.is_conflicted())
        .map(|e| e.path.as_str())
        .collect();
    let unversioned: std::collections::HashSet<&str> = entries
        .iter()
        .filter(|e| e.text == StatusKind::Unversioned || e.text == StatusKind::Ignored)
        .map(|e| e.path.as_str())
        .collect();

    let mut out = Vec::new();
    for e in entries {
        if e.text != StatusKind::Unversioned && e.text != StatusKind::Ignored {
            continue;
        }
        let Some(name) = e.path.file_name() else {
            continue;
        };
        let Some((base_name, _)) = split_trio_suffix(name) else {
            continue;
        };
        let base = e.path.with_file_name(base_name);
        if conflicted.contains(base.as_str()) || unversioned.contains(base.as_str()) {
            continue;
        }
        out.push(root.join(&e.path));
    }
    out.sort();
    out
}

/// 清理已解决冲突残留的三件套文件。
///
/// `doctor` 一直在报告 `.mine 残留 N 项`，但从来没给过清理手段 ——
/// 用户只能自己 `rm`。这里补上，并且只动**已经不在冲突态**的路径：
/// 还在冲突的文件，三件套是唯一的现场证据，删了就没法三路比对了。
pub fn purge_mine(
    cli: &super::Cli,
    svn: &Svn,
    dry_run: bool,
    yes: bool,
    tty: bool,
) -> Result<String, Error> {
    let opts = crate::svn::StatusOpts { include_ignored: true, ..Default::default() };
    let entries = svn.status(&opts)?;

    let files = residual_files(&svn.root, &entries);

    if files.is_empty() {
        if cli.json {
            return Ok(crate::output::ok(&serde_json::json!({
                "purged": [],
                "failed": [],
            })));
        }
        return Ok("没有冲突残留，无需清理".to_string());
    }

    let targets: Vec<String> = files
        .iter()
        .map(|f| {
            f.strip_prefix(&svn.root)
                .unwrap_or(f.as_path())
                .display()
                .to_string()
        })
        .collect();

    // 中危：会删文件，但删的都是 svn 自己该清掉的临时副本，且可重建。
    match guard(Op::PurgeMine, danger_of(Op::PurgeMine), &targets, dry_run, yes, tty)? {
        Some(text) if !text.is_empty() => return Ok(text),
        Some(_) => {}
        None => return Ok(String::new()),
    }

    let mut purged = Vec::new();
    let mut failed = Vec::new();
    for f in &files {
        match std::fs::remove_file(f) {
            Ok(()) => purged.push(
                f.strip_prefix(&svn.root)
                    .unwrap_or(f.as_path())
                    .display()
                    .to_string(),
            ),
            Err(e) => failed.push(format!("{}（{}）", f.display(), e)),
        }
    }
    crate::cache::invalidate_for(&svn.root);

    if cli.json {
        return Ok(crate::output::ok(&serde_json::json!({
            "purged": purged,
            "failed": failed,
        })));
    }
    let mut out = format!("已清理 {} 个残留文件", purged.len());
    if !failed.is_empty() {
        out.push_str(&format!("，{} 个失败：", failed.len()));
        for f in &failed {
            out.push_str(&format!("\n  {f}"));
        }
    }
    Ok(out)
}

/// 体检报告。yazi 的 `vh` 键走这个。
pub fn doctor(cli: &super::Cli, svn: &Svn, tty: bool) -> Result<String, Error> {
    let opts = crate::svn::StatusOpts { include_ignored: true, ..Default::default() };
    let entries = svn.status(&opts)?;

    let locked: Vec<_> = entries.iter().filter(|e| e.locked).collect();
    let conflicts: Vec<_> = entries.iter().filter(|e| e.is_conflicted()).collect();
    let missing: Vec<_> = entries.iter().filter(|e| e.text == StatusKind::Missing).collect();
    let obstructed: Vec<_> = entries.iter().filter(|e| e.text == StatusKind::Obstructed).collect();
    let unversioned: Vec<_> = entries.iter().filter(|e| e.text == StatusKind::Unversioned).collect();

    // 冲突三件套残留。与 purge-mine 共用同一份判定：这里报"有残留"，
    // purge-mine 就必须能清掉，两边各算一套会互相打脸。
    let residual: Vec<String> = residual_files(&svn.root, &entries)
        .iter()
        .map(|p| {
            p.strip_prefix(&svn.root)
                .unwrap_or(p.as_path())
                .display()
                .to_string()
        })
        .collect();

    let healthy = locked.is_empty()
        && conflicts.is_empty()
        && missing.is_empty()
        && obstructed.is_empty()
        && residual.is_empty();

    if cli.json {
        return Ok(crate::output::ok(&serde_json::json!({
            "healthy": healthy,
            "locked": locked.iter().map(|e| &e.path).collect::<Vec<_>>(),
            "conflicts": conflicts.iter().map(|e| &e.path).collect::<Vec<_>>(),
            "missing": missing.iter().map(|e| &e.path).collect::<Vec<_>>(),
            "obstructed": obstructed.iter().map(|e| &e.path).collect::<Vec<_>>(),
            "unversioned_count": unversioned.len(),
            "residual_mine": residual,
        })));
    }

    let mut out = String::new();
    let mark = |ok: bool, tty: bool| -> String {
        if ok {
            crate::output::style::green("✓", tty).to_string()
        } else {
            crate::output::style::red("✗", tty).to_string()
        }
    };

    out.push_str(&format!("{} 工作副本体检\n\n", if healthy { "健康" } else { "有问题" }));
    out.push_str(&format!("{} 锁         {} 项\n", mark(locked.is_empty(), tty), locked.len()));
    out.push_str(&format!("{} 冲突       {} 项\n", mark(conflicts.is_empty(), tty), conflicts.len()));
    out.push_str(&format!("{} 缺失       {} 项\n", mark(missing.is_empty(), tty), missing.len()));
    out.push_str(&format!("{} 阻碍       {} 项\n", mark(obstructed.is_empty(), tty), obstructed.len()));
    out.push_str(&format!("{} .mine 残留 {} 项\n", mark(residual.is_empty(), tty), residual.len()));
    out.push_str(&format!("  未版本化   {} 项\n", unversioned.len()));

    if !conflicts.is_empty() {
        out.push_str("\n冲突文件：\n");
        for e in conflicts.iter().take(20) {
            out.push_str(&format!("  {}\n", e.path));
        }
    }
    if !locked.is_empty() {
        out.push_str("\n有锁 → 先跑 `svnui cleanup`\n");
    }
    if !residual.is_empty() {
        out.push_str("\n有冲突残留 → `svnui purge-mine` 可安全清掉（仍冲突的文件不会动）\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这是删文件的操作，宁可漏一个也不能错删一个 —— 所以只认
    /// `.mine` 和 `.r<数字>` 两种后缀。
    #[test]
    fn only_real_conflict_trio_suffixes_match() {
        assert_eq!(split_trio_suffix("a.txt.mine"), Some(("a.txt", "mine")));
        assert_eq!(split_trio_suffix("a.txt.r1"), Some(("a.txt", "r1")));
        assert_eq!(split_trio_suffix("a.txt.r24"), Some(("a.txt", "r24")));

        // 近似但不等价的一律不算
        for name in ["a.txt.merge", "a.txt.r", "a.txt.r1x", "a.txt.bak", "a.txt.", ".mine", "a.txt"] {
            assert_eq!(split_trio_suffix(name), None, "{name} 不该被当成三件套");
        }
    }

    /// 目录里有别的文件时不误伤：三件套是按**后缀**认的，不是按前缀扫的。
    #[test]
    fn sibling_files_are_never_treated_as_leftovers() {
        assert!(split_trio_suffix("b.txt.mine").is_some()); // 它自己确实是 .mine
        assert_eq!(split_trio_suffix("b.txt.mine").unwrap().0, "b.txt");
    }
}
