pub mod daemon;
pub mod hint;
pub mod mutate;
pub mod query;
pub mod rescue;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "svnui", version, about = "快速、可脚本化的 Subversion 封装")]
pub struct Cli {
    /// 工作目录（默认当前目录）。
    #[arg(long, global = true, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// JSON 输出（信封格式 `{"ok":..}`）。
    #[arg(long, global = true)]
    pub json: bool,

    /// 紧凑两字符格式，如 `AM src/main.rs`。
    #[arg(long, global = true)]
    pub porcelain: bool,

    /// 超时秒数。
    #[arg(long, global = true, default_value_t = 30)]
    pub timeout: u64,

    /// 禁用状态缓存（每次都真跑 svn status）。
    #[arg(long, global = true)]
    pub no_cache: bool,

    /// 缓存有效秒数。默认 300 —— 严格校验已保证正确性，TTL 只是安全阀；
    /// 设为 0 等价于总是重新扫描。
    #[arg(long, global = true, default_value_t = 300)]
    pub cache_ttl: u64,

    /// 详细日志（打到 stderr）。
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// 环境自检：svn 路径、版本、工作副本根、能力开关。
    Probe,

    /// 交互式 TUI：浏览变更、看 diff、勾选提交。
    ///
    /// 这是主形态 —— yazi 插件做不到的交互（可勾选的提交面板、
    /// 可滚动的长 diff、多行提交信息）都在这里。
    #[cfg(feature = "tui")]
    Tui,

    /// 工作副本状态。
    Status {
        #[arg(long)]
        ignored: bool,
        /// 排除未版本化文件。**默认包含** —— 看见刚写的新文件是这个工具的核心用途，
        /// 与 `StatusOpts::default()` 的语义保持一致。
        #[arg(long)]
        no_unversioned: bool,
        /// 连服务器检查是否有更新（慢，默认关）。
        #[arg(long, short = 'u')]
        updates: bool,
        #[arg(long)]
        changed: bool,
        #[arg(long)]
        conflicts: bool,
    },

    /// 提交历史。
    Log {
        #[arg(long, short = 'l', default_value_t = 20)]
        limit: usize,
        /// 按版本查：`-r 1234` 或区间 `-r 1200:1234`。
        /// 给了这个参数就忽略 `--limit`。
        #[arg(long, short = 'r')]
        rev: Option<String>,
        #[arg(long)]
        oneline: bool,
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },

    /// 仓库信息。
    Info,

    /// 查看差异。
    Diff {
        #[arg(long)]
        stat: bool,
        /// `-c REV` 查看某次提交的差异。
        #[arg(long, short = 'c')]
        rev: Option<String>,
        /// 强制走分页器（短 diff 默认直接打印）。
        #[arg(long)]
        page: bool,
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },

    /// 加入版本控制。
    /// 逐行追溯作者与版本。
    Blame {
        #[arg(value_name = "PATH", required = true)]
        path: PathBuf,
    },

    Add {
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<PathBuf>,
    },

    /// 从版本控制移除（默认同时删除本地文件）。
    Remove {
        /// 只从版本库移除，保留本地文件（默认会连本地文件一起删）。
        #[arg(long)]
        keep_local: bool,
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<PathBuf>,
    },

    /// 回滚本地改动。**高危：不可撤销。**
    Revert {
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// 只打印将被回滚的文件，不执行。
        #[arg(long)]
        dry_run: bool,
        /// 跳过确认（非交互环境必须显式给）。
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// 提交。
    Commit {
        /// 提交信息。不给则用默认文案（仅用于脚本）。
        #[arg(short, long)]
        message: Option<String>,
        /// 只提交这些路径；不给则提交全部变更。
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// 更新工作副本。默认 `--accept postpone`（冲突留给人工判断）。
    Update {
        /// 更新到指定版本。
        #[arg(long, short = 'r')]
        rev: Option<String>,
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// 解决冲突。
    Resolve {
        /// mine-full / theirs-full / working / base / mine-conflict / theirs-conflict
        #[arg(long, short = 'a', default_value = "working")]
        accept: String,
        /// 不指定则作用于所有冲突项。
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// 清理工作副本。
    Cleanup {
        /// **危险**：删除所有未纳入版本的文件（包括你刚写了一半的新文件）。
        #[arg(long)]
        remove_unversioned: bool,
        /// **危险**：删除所有被忽略的文件（通常是 build 产物）。
        #[arg(long)]
        remove_ignored: bool,
        /// 清理 .svn/pristine 垃圾（svn 1.10+）。
        #[arg(long)]
        vacuum_pristines: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// 工作副本体检：锁 / 冲突 / 缺失 / 阻碍 / .mine 残留。
    Doctor,

    /// 列出所有冲突。
    Conflicts,

    /// 常驻守护进程：让切目录不再跑 svn status。
    Daemon {
        #[command(subcommand)]
        action: daemon::Action,
    },

    /// 查询某目录层状态（yazi 主入口）。优先 daemon，不可用则直连 svn。
    Q {
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        since: Option<u64>,
    },

    /// 把 yazi 插件装进配置目录。
    InstallYazi {
        /// 只检查会装到哪里、不写盘。
        #[arg(long)]
        check: bool,
        /// 覆盖已存在的插件文件。
        #[arg(long)]
        force: bool,
        /// 自动把 require("svnui"):setup {} 追加进 init.lua。
        #[arg(long)]
        patch_init: bool,
        /// 打印键位片段（不安装）。
        #[arg(long)]
        print_keymap: bool,
    },

    /// 查看 / 清理状态缓存。
    Cache {
        /// 删除缓存文件。
        #[arg(long)]
        clear: bool,
    },
}

impl Cli {
    pub fn start_dir(&self) -> PathBuf {
        // `daemon run --root X` 是由 `daemon start` 在后台拉起的，
        // 它的 cwd 继承自调用者，**不一定**在工作副本里。
        // 若这里不做特殊处理，discover 会因为 cwd 不对而直接失败退出。
        if let Cmd::Daemon { action } = &self.cmd {
            if let daemon::Action::Run { root, .. } = action {
                return root.clone();
            }
        }
        self.cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout)
    }
}

/// 分发。**出口层**：把领域错误翻译成人类可读提示 + 退出码。
pub fn run(cli: Cli) -> i32 {
    let start = cli.start_dir();

    // 这两个命令不需要工作副本：
    //   · install-yazi —— 装插件时人可能在任何目录，甚至还没装 svn
    //   · probe        —— 它就是用来回答"为什么找不到仓库"的，
    //                     进了非工作副本直接退出等于自废武功
    let needs_wc = !matches!(cli.cmd, Cmd::InstallYazi { .. } | Cmd::Probe);

    let svn = match crate::svn::Svn::discover(&start, cli.timeout()) {
        Ok(s) => s,
        Err(e) => {
            if needs_wc {
                emit_err(&cli, &e);
                return e.exit_code();
            }
            // 降级：只要能找到 svn 本体就继续（报出路径+版本也是有用的诊断）
            match crate::svn::Svn::discover_bare(&start, cli.timeout()) {
                Ok(s) => s,
                Err(bare_err) => {
                    emit_err(&cli, &bare_err);
                    return bare_err.exit_code();
                }
            }
        }
    };

    let tty = !cli.json && crate::output::is_tty();
    let res = dispatch(&cli, &svn, tty);

    match res {
        Ok(text) => {
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }
        Err(e) => {
            emit_err(&cli, &e);
            e.exit_code()
        }
    }
}

/// 统一错误出口：JSON 走信封，人类走可读提示。
fn emit_err(cli: &Cli, e: &crate::domain::Error) {
    if cli.json {
        println!(
            "{}",
            crate::output::err(crate::output::kind_of(e), &e.to_string())
        );
    } else {
        eprintln!("{}", crate::cli::hint::human_hint(e));
    }
}

fn dispatch(cli: &Cli, svn: &crate::svn::Svn, tty: bool) -> Result<String, crate::domain::Error> {
    use self::daemon as daemon_cmd;

    match &cli.cmd {
        #[cfg(feature = "tui")]
        Cmd::Tui => {
            // TUI 自己接管终端（raw mode + alternate screen），
            // 不走 output 层，也不该往 stdout 打东西。
            return crate::tui::run(svn.clone()).map(|_| String::new());
        }

        Cmd::Probe => query::probe(cli, svn),
        // 反转成"包含"语义：默认 true，与 StatusOpts::default() 一致。
        Cmd::Status {
            ignored,
            no_unversioned,
            updates,
            changed,
            conflicts,
        } => query::status(
            cli,
            svn,
            *ignored,
            !*no_unversioned,
            *updates,
            *changed,
            *conflicts,
            tty,
        ),
        Cmd::Log {
            limit,
            rev,
            oneline,
            paths,
        } => query::log(cli, svn, *limit, rev.as_deref(), *oneline, paths),
        Cmd::Blame { path } => query::blame(svn, path),
        Cmd::Info => query::info(cli, svn),
        Cmd::Diff {
            stat,
            rev,
            page,
            paths,
        } => query::diff(cli, svn, *stat, rev.as_deref(), *page, paths),
        Cmd::Conflicts => rescue::conflicts(cli, svn, tty),

        Cmd::Add { paths } => mutate::add(svn, paths),
        Cmd::Remove {
            keep_local,
            yes,
            paths,
        } => mutate::remove(svn, *keep_local, *yes, paths, tty),
        Cmd::Revert {
            paths,
            dry_run,
            yes,
        } => mutate::revert(svn, paths, *dry_run, *yes, tty),
        Cmd::Commit {
            message,
            paths,
            dry_run,
            yes,
        } => mutate::commit(svn, message.as_deref(), paths, *dry_run, *yes, tty),
        Cmd::Update { rev, paths, yes } => mutate::update(svn, rev.as_deref(), paths, *yes, tty),
        Cmd::Resolve {
            accept,
            paths,
            dry_run,
            yes,
        } => rescue::resolve(svn, accept, paths, *dry_run, *yes, tty),
        Cmd::Cleanup {
            remove_unversioned,
            remove_ignored,
            vacuum_pristines,
            dry_run,
            yes,
        } => rescue::cleanup(
            svn,
            *remove_unversioned,
            *remove_ignored,
            *vacuum_pristines,
            *dry_run,
            *yes,
            tty,
        ),
        Cmd::Doctor => rescue::doctor(cli, svn, tty),

        // 注意：这里匹配的是 `&cli.cmd`，字段都是引用，必须 clone 成 owned 值。
        Cmd::Daemon { action } => daemon_cmd::run(
            daemon_cmd::Cmd::Daemon {
                action: action.clone(),
            },
            cli,
            svn,
        ),
        Cmd::Q { dir, since } => daemon_cmd::run(
            daemon_cmd::Cmd::Q {
                dir: dir.clone(),
                since: *since,
            },
            cli,
            svn,
        ),

        Cmd::InstallYazi {
            check,
            force,
            patch_init,
            print_keymap,
        } => {
            if *print_keymap {
                return Ok(crate::install::keymap_snippet().to_string());
            }
            let rep = crate::install::install(crate::install::Opts {
                check: *check,
                force: *force,
                patch_init: *patch_init,
            })?;
            Ok(crate::install::render(&rep))
        }

        Cmd::Cache { clear } => {
            let c = match crate::cache::Cache::open(&svn.root) {
                Some(c) => c,
                None => {
                    return Err(crate::domain::Error::Parse(
                        "该工作副本不支持缓存（缺少 .svn/wc.db，可能是 SVN 1.6 及更早）"
                            .to_string(),
                    ))
                }
            };
            if *clear {
                c.invalidate();
                return Ok("缓存已清除".to_string());
            }
            let st = c.stats();
            if cli.json {
                return Ok(crate::output::ok(&serde_json::json!({
                    "path": st.path,
                    "exists": st.exists,
                    "bytes": st.bytes,
                    "age_secs": st.age_secs,
                    "entries": st.entries,
                    "tree_nodes": st.tree_nodes,
                    "root": st.root,
                })));
            }
            if !st.exists {
                return Ok(format!(
                    "缓存文件不存在：{}\n（跑一次 svnui status 会生成）",
                    st.path.display()
                ));
            }
            Ok(format!(
                "缓存文件  {}\n大小      {} 字节\n年龄      {} 秒\n状态条目  {}\n跟踪节点  {}\n工作副本  {}",
                st.path.display(),
                st.bytes,
                st.age_secs.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                st.entries.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                st.tree_nodes.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                st.root.unwrap_or_else(|| "-".into()),
            ))
        }
    }
}
