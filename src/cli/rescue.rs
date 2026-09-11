//! 救援：conflicts / resolve / cleanup / purge-mine / doctor。

use std::path::PathBuf;

use crate::domain::{Error, StatusKind};
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

/// 体检报告。yazi 的 `vh` 键走这个。
pub fn doctor(cli: &super::Cli, svn: &Svn, tty: bool) -> Result<String, Error> {
    let opts = crate::svn::StatusOpts { include_ignored: true, ..Default::default() };
    let entries = svn.status(&opts)?;

    let locked: Vec<_> = entries.iter().filter(|e| e.locked).collect();
    let conflicts: Vec<_> = entries.iter().filter(|e| e.is_conflicted()).collect();
    let missing: Vec<_> = entries.iter().filter(|e| e.text == StatusKind::Missing).collect();
    let obstructed: Vec<_> = entries.iter().filter(|e| e.text == StatusKind::Obstructed).collect();
    let unversioned: Vec<_> = entries.iter().filter(|e| e.text == StatusKind::Unversioned).collect();

    // .mine 残留：文件已不在冲突态，但 .mine 还在磁盘上。
    let mut residual = Vec::new();
    for e in &entries {
        if e.is_conflicted() || e.text == StatusKind::Unversioned {
            continue;
        }
        let p = svn.root.join(&e.path);
        let mine = PathBuf::from(format!("{}.mine", p.display()));
        if mine.exists() {
            residual.push(e.path.to_string());
        }
    }

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
        out.push_str("\n有 .mine 残留 → 可安全跑 `svnui cleanup` 后手动删除，或检查是否已解决\n");
    }
    Ok(out)
}
