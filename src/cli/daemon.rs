//! daemon 与快速查询的 CLI。

use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;

use crate::domain::Error;
use crate::svn::Svn;

/// 本模块的结果类型（避免与 std 的 Result 混淆时显式写出）
type R = std::result::Result<String, Error>;

// Cmd / Action 需要 Clone：cli::dispatch 匹配的是 `&cli.cmd`，
// 拿到的是引用，而这里构造新的 Cmd 需要 owned 值。
#[derive(Debug, Clone, Subcommand)]
pub enum Cmd {
    /// 常驻守护进程。
    Daemon {
        #[command(subcommand)]
        action: Action,
    },

    /// 查询某目录层的状态。优先走 daemon，不可用则直连 svn。
    Q {
        /// 要查询的目录。默认当前目录。
        #[arg(long)]
        dir: Option<PathBuf>,
        /// 已知版本戳。与 daemon 一致时只回一个几十字节的"无变化"。
        #[arg(long)]
        since: Option<u64>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum Action {
    /// 启动（幂等，已在运行则不动）。
    Start,
    /// 优雅停止。
    Stop,
    /// 查看运行状态。
    Status,
    /// 强制重新全量扫描。
    Rescan,
    /// 实际运行 daemon（由 start 内部调用，不要手动跑）。
    Run {
        #[arg(long)]
        root: PathBuf,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
}

pub fn run(cmd: Cmd, cli: &super::Cli, svn: &Svn) -> R {
    match cmd {
        Cmd::Daemon { action } => daemon(action, cli, svn),
        Cmd::Q { dir, since } => query(cli, svn, dir, since),
    }
}

fn daemon(action: Action, cli: &super::Cli, svn: &Svn) -> R {
    let root = &svn.root;
    let timeout = cli.timeout();

    match action {
        Action::Start => {
            let started = crate::daemon::start(root, timeout)?;
            if cli.json {
                return Ok(crate::output::ok(&serde_json::json!({ "started": started })));
            }
            Ok(if started {
                format!("daemon 已启动\n  socket {}", crate::daemon::protocol::socket_path(root).display())
            } else {
                "daemon 已在运行".to_string()
            })
        }

        Action::Stop => {
            let stopped = crate::daemon::stop(root)?;
            if cli.json {
                return Ok(crate::output::ok(&serde_json::json!({ "stopped": stopped })));
            }
            Ok(if stopped { "daemon 已停止".to_string() } else { "daemon 未在运行".to_string() })
        }

        Action::Status => {
            let running = crate::daemon::is_running(root);
            if cli.json {
                return Ok(crate::output::ok(&serde_json::json!({
                    "running": running,
                    "root": root,
                    "socket": crate::daemon::protocol::socket_path(root),
                })));
            }
            Ok(format!(
                "daemon   {}\nroot     {}\nsocket   {}",
                if running { "运行中" } else { "未运行" },
                root.display(),
                crate::daemon::protocol::socket_path(root).display()
            ))
        }

        Action::Rescan => {
            let ok = crate::daemon::rescan(root)?;
            if cli.json {
                return Ok(crate::output::ok(&serde_json::json!({ "rescanned": ok })));
            }
            Ok(if ok {
                "已触发全量重扫".to_string()
            } else {
                "daemon 未在运行，先跑 svnui daemon start".to_string()
            })
        }

        Action::Run { root, timeout } => {
            // 阻塞运行。start 会拉起这个子命令。
            crate::daemon::serve(&root, Duration::from_secs(timeout))?;
            Ok(String::new())
        }
    }
}

/// `svnui q`：yazi 的主要数据入口。
///
/// 无论走没走 daemon，返回结构完全一致 —— yazi 不需要知道区别。
fn query(
    cli: &super::Cli,
    svn: &Svn,
    dir: Option<PathBuf>,
    since: Option<u64>,
) -> R {
    // `--dir` 可能给相对路径（yazi 与手写命令都常见），
    // 必须基于当前目录拼成绝对路径 —— 否则 daemon 端 strip_prefix 会失败，
    // 静默退化成"查根层"，看起来像"状态全丢了"。
    let d = match dir {
        Some(p) if p.is_relative() => svn.cwd.join(p),
        Some(p) => p,
        None => svn.cwd.clone(),
    };
    let layer = crate::ipc::query(svn, &d, since)?;

    if cli.json {
        return Ok(crate::output::ok(&serde_json::json!({
            "v": layer.v,
            "root": layer.root,
            "branch": layer.branch,
            "map": layer.map,
            "ready": layer.ready,
            "changed": layer.changed,
            "from_daemon": layer.from_daemon,
        })));
    }

    // 非 JSON 时给人类可读的两字符格式
    let mut keys: Vec<_> = layer.map.keys().collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        out.push_str(&format!("{} {}\n", layer.map[k], k));
    }
    if out.is_empty() {
        out.push_str("(clean)\n");
    }
    Ok(out)
}
