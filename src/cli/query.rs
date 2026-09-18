//! 只读子命令：probe / status / log / info / diff。

use std::path::PathBuf;

use crate::domain::Error;
use crate::svn::Svn;

use super::Cli;

pub fn probe(cli: &Cli, svn: &Svn) -> Result<String, Error> {
    let info = svn.info().ok();
    let data = serde_json::json!({
        "svn": svn.svn_path().to_string_lossy(),
        "version": svn.version.to_string(),
        "root": svn.root.to_string_lossy(),
        "branch": info.as_ref().and_then(|i| i.branch_name()),
        "revision": info.as_ref().map(|i| i.revision),
        "vacuum_pristines": svn.version.supports_vacuum_pristines(),
        "show_item": svn.version.supports_show_item(),
    });

    if cli.json {
        return Ok(crate::output::ok(&data));
    }
    Ok(format!(
        "svn        {}\nversion    {}\nroot       {}\nbranch     {}\nrevision   {}",
        data["svn"].as_str().unwrap_or("-"),
        data["version"].as_str().unwrap_or("-"),
        data["root"].as_str().unwrap_or("-"),
        data["branch"].as_str().unwrap_or("-"),
        data["revision"].as_u64().map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
    ))
}

/// 把一次实时扫描的结果写进缓存。
fn save_cache(c: &crate::cache::Cache, svn: &Svn, entries: &[crate::domain::StatusEntry]) -> Result<(), Error> {
    let mut snap = crate::domain::Snapshot::new(svn.root.to_string_lossy().to_string());
    snap.ready = true;
    for e in entries {
        snap.insert(e.clone());
    }
    snap.build_bubbles();
    c.save(&snap)
}

/// 缓存只对"默认查询"生效。
///
/// 原因：缓存里存的是**完整**状态快照，而 `--ignored` / `-u` 会改变 `svn status` 的输出
/// 语义（`-u` 还要连服务器）。与其为每种组合各存一份，不如非默认组合一律走实时查询 ——
/// 这些本来就是低频操作。
fn is_cacheable(ignored: bool, unversioned: bool, updates: bool) -> bool {
    unversioned && !ignored && !updates
}

#[allow(clippy::too_many_arguments)]
pub fn status(
    cli: &Cli,
    svn: &Svn,
    ignored: bool,
    unversioned: bool,
    updates: bool,
    changed: bool,
    conflicts: bool,
    tty: bool,
) -> Result<String, Error> {
    let opts = crate::svn::StatusOpts {
        include_ignored: ignored,
        include_unversioned: unversioned,
        show_updates: updates,
    };

    let entries = if cli.no_cache || !is_cacheable(ignored, unversioned, updates) {
        svn.status(&opts)?
    } else {
        match crate::cache::Cache::open(&svn.root) {
            Some(c) => match c.load(std::time::Duration::from_secs(cli.cache_ttl)) {
                Some(snap) => {
                    if cli.verbose {
                        eprintln!("[cache] 命中，跳过 svn status");
                    }
                    snap.entries.values().cloned().collect()
                }
                None => {
                    let e = svn.status(&opts)?;
                    if let Err(err) = save_cache(&c, svn, &e) {
                        // 缓存写失败不该让命令失败 —— 它只是优化。
                        if cli.verbose {
                            eprintln!("[cache] 写入失败（忽略）：{err}");
                        }
                    }
                    e
                }
            },
            None => svn.status(&opts)?,
        }
    };

    let mut entries = entries;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    if changed {
        entries.retain(|e| e.is_changed());
    }
    if conflicts {
        entries.retain(|e| e.is_conflicted());
    }

    if cli.json {
        let v: Vec<_> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "path": e.path,
                    "xy": e.porcelain(),
                    "sign": e.sign().to_string(),
                    "tree_conflict": e.tree_conflict,
                    "locked": e.locked,
                    "copied": e.copied,
                })
            })
            .collect();
        return Ok(crate::output::ok(&v));
    }

    let mut out = String::new();
    for e in &entries {
        if cli.porcelain {
            out.push_str(&e.porcelain());
        } else {
            let s = e.sign();
            out.push_str(&crate::output::style::by_sign(s, &s.to_string(), tty));
        }
        out.push(' ');
        out.push_str(e.path.as_str());
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("(clean)\n");
    }
    Ok(out)
}

pub fn log(
    cli: &Cli,
    svn: &Svn,
    limit: usize,
    rev: Option<&str>,
    oneline: bool,
    paths: &[PathBuf],
) -> Result<String, Error> {
    // 给了 --rev 就走按版本查，--limit 对单版本没意义
    if let Some(r) = rev {
        return log_rev(cli, svn, r, oneline);
    }
    let entries = svn.log(limit, paths)?;
    if cli.json {
        return Ok(crate::output::ok(&entries));
    }

    let mut out = String::new();
    for e in &entries {
        if oneline || cli.porcelain {
            out.push_str(&e.oneline());
        } else {
            out.push_str(&format!("{}\n", crate::output::style::bold(&format!("r{}", e.revision), false)));
            out.push_str(&format!("{} | {}\n\n", e.author, e.date));
            for line in e.msg.lines() {
                out.push_str(&format!("    {line}\n"));
            }
        }
        out.push('\n');
    }
    Ok(out)
}

pub fn info(cli: &Cli, svn: &Svn) -> Result<String, Error> {
    let info = svn.info()?;
    if cli.json {
        return Ok(crate::output::ok(&info));
    }
    Ok(format!(
        "root      {}\nurl       {}\nbranch    {}\nrevision  {}\nuuid      {}",
        info.root,
        info.url,
        info.branch_name().unwrap_or_else(|| "-".into()),
        info.revision,
        info.uuid.unwrap_or_else(|| "-".into()),
    ))
}

/// diff 走**分页器**。短 diff 直接打印（否则看三行还要按 q，是负体验）。
pub fn diff(
    cli: &Cli,
    svn: &Svn,
    stat: bool,
    rev: Option<&str>,
    force_page: bool,
    paths: &[PathBuf],
) -> Result<String, Error> {
    let out = svn.diff(paths, rev, stat)?;

    if cli.json {
        return Ok(crate::output::ok(&serde_json::json!({ "diff": out.stdout })));
    }

    // 分页器接管时直接返回空串：内容已经写进 pager，再 println 就重复了。
    if force_page || out.stdout.lines().count() > 40 {
        let _ = crate::output::page(&out.stdout);
        return Ok(String::new());
    }

    Ok(out.stdout)
}

/// 按版本查日志。`--rev` 与 `--limit` 互斥，前者优先。
///
/// ⚠️ 注意 svn 的 `-c N` 与 `-r N` 语义不同：
///   - `-c N` 显示"该版本引入的变化"（类似 git show）
///   - `-r N` 显示"该版本的日志条目"
/// 用户按版本号查日志时想要的是后者，所以统一用 `-r`。
fn log_rev(cli: &Cli, svn: &Svn, rev: &str, oneline: bool) -> Result<String, Error> {
    let entries = svn.log_rev(rev, false)?;

    if cli.json {
        return Ok(crate::output::ok(&entries));
    }

    let mut out = String::new();
    for e in &entries {
        if oneline || cli.porcelain {
            out.push_str(&e.oneline());
        } else {
            out.push_str(&format!("r{} | {} | {}\n\n", e.revision, e.author, e.date));
            for line in e.msg.lines() {
                out.push_str(&format!("    {line}\n"));
            }
        }
        out.push('\n');
    }
    if out.trim().is_empty() {
        out = format!("(r{rev} 无匹配的提交记录)\n");
    }
    Ok(out)
}

/// blame 输出原文，不解析 —— 行数可能上万，解析再格式化纯属浪费，
/// 而且用户要的就是 svn 原始的那份对齐格式。
pub fn blame(svn: &Svn, path: &PathBuf) -> Result<String, Error> {
    svn.blame(path)
}
