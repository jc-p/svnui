//! 写操作：add / remove / revert / commit / update。
//!
//! **每个方法都必须先过 `policy::judge`** —— 这是硬性约定，不是建议。
//! 判定发生在执行之前，且不依赖任何外部状态，便于测试。

use std::path::PathBuf;

use crate::domain::Error;
use crate::policy::{danger_of, judge, render_targets, Op, Verdict};
use crate::svn::Svn;

/// 统一的护栏执行器。
///
/// 返回 `None` 表示"不应执行"（已拒绝/用户取消/干跑），附带要打印的文本。
/// 返回 `Some(())` 表示放行。
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
            s.push_str("\n（未实际执行。去掉 --dry-run 才会生效）");
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

fn to_strings(paths: &[PathBuf]) -> Vec<String> {
    if paths.is_empty() {
        vec!["(整个工作副本)".to_string()]
    } else {
        paths.iter().map(|p| p.to_string_lossy().to_string()).collect()
    }
}

pub fn add(svn: &Svn, paths: &[PathBuf]) -> Result<String, Error> {
    let t = to_strings(paths);
    // add 是 Low，可直接执行
    let _ = guard(Op::Add, danger_of(Op::Add), &t, false, true, false)?;
    let out = svn.add(paths)?;
    crate::cache::invalidate_for(&svn.root);
    Ok(if out.stdout.trim().is_empty() { "已加入版本控制".to_string() } else { out.stdout })
}

pub fn remove(
    svn: &Svn,
    keep_local: bool,
    yes: bool,
    paths: &[PathBuf],
    tty: bool,
) -> Result<String, Error> {
    let t = to_strings(paths);
    //
    // ⚠️ 之前的写法把 yes 硬编码成 true、tty 硬编码成 false，
    //    等于把 guard 完全架空 —— 非交互脚本里一次误调用就真删了。
    //    Remove 是中危：不给 --yes 且非 TTY 时必须拒绝。
    //
    // guard 内部会用 render_targets 列出受影响文件。
    // 默认 keep_local=false 会连本地文件一起删，这个语义在 Op::Remove 的
    // Display 里已经写明，这里不再重复渲染。
    match guard(Op::Remove, danger_of(Op::Remove), &t, false, yes, tty)? {
        Some(_) => {}
        None => return Ok(String::new()),
    }
    let out = svn.remove(paths, keep_local)?;
    crate::cache::invalidate_for(&svn.root);
    Ok(if out.stdout.trim().is_empty() {
        format!("已从版本控制移除 {} 项{}", t.len(), if keep_local { "（保留本地文件）" } else { "" })
    } else {
        out.stdout
    })
}

/// **高危**。svn revert 没有真正的 dry-run，所以干跑是我们自己实现的：
/// 先把"将要回滚"的清单打出来，不调 svn。
pub fn revert(
    svn: &Svn,
    paths: &[PathBuf],
    dry_run: bool,
    yes: bool,
    tty: bool,
) -> Result<String, Error> {
    // 干跑前需要知道会动哪些文件 —— 若用户没给路径，就取全部变更。
    let effective: Vec<String> = if paths.is_empty() {
        svn.changed(&Default::default())?.iter().map(|e| e.path.to_string()).collect()
    } else {
        to_strings(paths)
    };

    match guard(Op::Revert, danger_of(Op::Revert), &effective, dry_run, yes, tty)? {
        Some(warn) if !warn.is_empty() => return Ok(warn),
        Some(_) => {}
        None => return Ok(String::new()),
    }

    let out = svn.revert(paths, false)?;
    crate::cache::invalidate_for(&svn.root);
    if out.stdout.trim().is_empty() {
        Ok(format!("已回滚 {} 项", effective.len()))
    } else {
        Ok(out.stdout)
    }
}

pub fn commit(
    svn: &Svn,
    message: Option<&str>,
    paths: &[PathBuf],
    dry_run: bool,
    yes: bool,
    tty: bool,
) -> Result<String, Error> {
    let effective: Vec<String> = if paths.is_empty() {
        svn.changed(&Default::default())?.iter().map(|e| e.path.to_string()).collect()
    } else {
        to_strings(paths)
    };

    if effective.is_empty() {
        return Ok("没有需要提交的变更".to_string());
    }

    match guard(Op::Commit, danger_of(Op::Commit), &effective, dry_run, yes, tty)? {
        Some(text) if !text.is_empty() => return Ok(text),
        Some(_) => {}
        None => return Ok(String::new()),
    }

    let msg = message.unwrap_or("(svnui: 无提交信息)");
    let out = svn.commit(msg, paths)?;
    crate::cache::invalidate_for(&svn.root);

    // 从输出里抠出 revision，便于脚本与 yazi 回执。
    let rev = out
        .stdout
        .lines()
        .find(|l| l.contains("Committed revision"))
        .and_then(|l| l.split_whitespace().last())
        .map(|s| s.trim_end_matches('.'))
        .unwrap_or("?");

    Ok(format!("已提交 {} 项，revision {rev}", effective.len()))
}

pub fn update(
    svn: &Svn,
    rev: Option<&str>,
    paths: &[PathBuf],
    yes: bool,
    tty: bool,
) -> Result<String, Error> {
    let t = to_strings(paths);
    match guard(Op::Update, danger_of(Op::Update), &t, false, yes, tty)? {
        Some(text) if !text.is_empty() => return Ok(text),
        Some(_) => {}
        None => return Ok(String::new()),
    }

    let out = svn.update(rev, paths)?;
    crate::cache::invalidate_for(&svn.root);
    let mut s = out.stdout;

    // update 后主动体检：有冲突立刻告知，而不是等 commit 时才发现。
    let conflicts = svn.conflicts()?;
    if !conflicts.is_empty() {
        s.push_str(&format!(
            "\n⚠️  更新后有 {} 项冲突，跑 `svnui conflicts` 查看，或 `svnui resolve -a mine-full` 处理",
            conflicts.len()
        ));
    }
    Ok(s)
}

// ------------------------------------------------------------ 检出 / 认证

/// 检出一份工作副本。
pub fn checkout(
    svn: &Svn,
    url: &str,
    path: Option<&std::path::Path>,
    username: Option<&str>,
    password: Option<&str>,
    depth: &str,
    no_auth_cache: bool,
) -> Result<String, Error> {
    // 没给路径就用 URL 最后一段当目录名（和 svn 自己的行为一致）
    let default_name = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("wc")
        .to_string();
    let target = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(&default_name));

    let auth = crate::svn::client::AuthOpts {
        username: username.map(|s| s.to_string()),
        password: password.map(|s| s.to_string()),
        no_auth_cache,
    };

    let out = svn.checkout(url, &target, &auth, depth)?;

    let mut s = format!("已检出到 {}\n", target.display());
    s.push_str(&out.stdout);
    Ok(s)
}

/// 登录 / 注销。
pub fn login(
    svn: &Svn,
    url: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
    no_auth_cache: bool,
    logout: bool,
) -> Result<String, Error> {
    if logout {
        svn.logout()?;
        return Ok("已清除凭据缓存（所有仓库）".to_string());
    }

    // 没给 URL 就用当前工作副本的地址
    let target = match url {
        Some(u) => u.to_string(),
        None => svn.info()?.url,
    };

    let auth = crate::svn::client::AuthOpts {
        username: username.map(|s| s.to_string()),
        password: password.map(|s| s.to_string()),
        no_auth_cache,
    };

    svn.login(&target, &auth)?;

    Ok(if no_auth_cache {
        format!("✓ 凭据有效（未缓存）：{}", target)
    } else {
        format!("✓ 已登录并缓存凭据：{}\n   之后的操作不再需要输入密码。", target)
    })
}

/// 列出已缓存的凭据。
pub fn auth_list(svn: &Svn) -> Result<String, Error> {
    let out = svn.auth_list()?;
    if out.stdout.trim().is_empty() {
        return Ok("（没有缓存的凭据）".to_string());
    }
    Ok(out.stdout)
}
