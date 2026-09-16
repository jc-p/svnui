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

    /// 放行 SSL 证书校验失败（内网自签名 CA、网关 SSL 中间人、
    /// 证书签给了别的主机名）。
    ///
    /// 对应 svn 的
    /// `--trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other`。
    ///
    /// ⚠️ 这会跳过证书校验，等于放弃了对中间人攻击的防护。
    /// 只在确认是**自己的内网服务器**时启用。
    /// 也可以设环境变量 SVNUI_TRUST_CERT=1，省得每次敲
    /// （老名字 SVNR_TRUST_CERT 也认）。
    #[arg(long, global = true)]
    pub trust_cert: bool,

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

    /// 检出一份工作副本。
    ///
    /// 唯一不需要先有工作副本的命令 —— 它的用途就是创建工作副本。
    Checkout {
        /// 仓库地址（`svn://` / `http://` / `file://`）。
        url: String,
        /// 本地路径，默认当前目录下的仓库名。
        path: Option<PathBuf>,
        #[arg(long)]
        username: Option<String>,
        /// 不安全：会短暂出现在 `ps` 输出里。svn 1.12+ 优先走 stdin。
        #[arg(long)]
        password: Option<String>,
        /// 目录深度。`infinity`（默认，全量）/ `empty` / `files` / `immediates`。
        #[arg(long, default_value = "infinity")]
        depth: String,
        /// 不让 svn 记住凭据。
        #[arg(long)]
        no_auth_cache: bool,
    },

    /// 登录：验证凭据并交给 svn 缓存。
    ///
    /// svn 没有独立的登录命令，凭据是在第一次需要认证的操作里顺带缓存的。
    /// 所以这里对远端跑一次 `svn info` —— 成功即代表凭据有效且已记住，
    /// 之后所有操作都不用再输。
    Login {
        /// 要登录的仓库地址。省略则用当前工作副本的 URL。
        url: Option<String>,
        #[arg(long, short = 'u')]
        username: Option<String>,
        #[arg(long, short = 'p')]
        password: Option<String>,
        /// 不让 svn 记住凭据（只验证这一次）。
        #[arg(long)]
        no_auth_cache: bool,
        /// 注销：清掉已缓存的凭据。
        #[arg(long, conflicts_with_all = ["username", "password"])]
        logout: bool,
    },

    /// 列出 svn 已缓存的凭据（需要 svn 1.9+）。
    Auth,

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

    /// 查询某目录层状态。优先 daemon，不可用则直连 svn。
    Q {
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        since: Option<u64>,
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

    // 这些命令不需要工作副本：
    //   · probe        —— 它就是用来回答"为什么找不到仓库"的，
    //                     进了非工作副本直接退出等于自废武功
    //   · checkout     —— 它的用途就是**创建**工作副本，要求先有工作副本是循环依赖
    //   · login / auth —— 凭据是全局的（~/.subversion/auth），不隶属于某个副本
    let mut needs_wc = !matches!(
        cli.cmd,
        Cmd::Probe | Cmd::Checkout { .. } | Cmd::Login { .. } | Cmd::Auth
    );
    // TUI 自己能处理"不在工作副本"的情况（进去给检出界面），
    // 所以在这一层放行，别先把它拦下来。
    #[cfg(feature = "tui")]
    if matches!(cli.cmd, Cmd::Tui) {
        needs_wc = false;
    }

    let svn = match crate::svn::Svn::discover(&start, cli.timeout(), cli.trust_cert) {
        Ok(s) => s,
        Err(e) => {
            if needs_wc {
                emit_err(&cli, &e);
                return e.exit_code();
            }
            // 降级：只要能找到 svn 本体就继续（报出路径+版本也是有用的诊断）
            match crate::svn::Svn::discover_bare(&start, cli.timeout(), cli.trust_cert) {
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
        println!("{}", crate::output::err(crate::output::kind_of(e), &e.to_string()));
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
        Cmd::Checkout { url, path, username, password, depth, no_auth_cache } => mutate::checkout(
            svn,
            url,
            path.as_deref(),
            username.as_deref(),
            password.as_deref(),
            depth,
            *no_auth_cache,
        ),
        Cmd::Login { url, username, password, no_auth_cache, logout } => mutate::login(
            svn,
            url.as_deref(),
            username.as_deref(),
            password.as_deref(),
            *no_auth_cache,
            *logout,
        ),
        Cmd::Auth => mutate::auth_list(svn),
        // 反转成"包含"语义：默认 true，与 StatusOpts::default() 一致。
        Cmd::Status { ignored, no_unversioned, updates, changed, conflicts } => {
            query::status(cli, svn, *ignored, !*no_unversioned, *updates, *changed, *conflicts, tty)
        }
        Cmd::Log { limit, rev, oneline, paths } => {
            query::log(cli, svn, *limit, rev.as_deref(), *oneline, paths)
        }
        Cmd::Blame { path } => query::blame(svn, path),
        Cmd::Info => query::info(cli, svn),
        Cmd::Diff { stat, rev, page, paths } => query::diff(cli, svn, *stat, rev.as_deref(), *page, paths),
        Cmd::Conflicts => rescue::conflicts(cli, svn, tty),

        Cmd::Add { paths } => mutate::add(svn, paths),
        Cmd::Remove { keep_local, yes, paths } => {
            mutate::remove(svn, *keep_local, *yes, paths, tty)
        }
        Cmd::Revert { paths, dry_run, yes } => mutate::revert(svn, paths, *dry_run, *yes, tty),
        Cmd::Commit { message, paths, dry_run, yes } => {
            mutate::commit(svn, message.as_deref(), paths, *dry_run, *yes, tty)
        }
        Cmd::Update { rev, paths, yes } => mutate::update(svn, rev.as_deref(), paths, *yes, tty),
        Cmd::Resolve { accept, paths, dry_run, yes } => {
            rescue::resolve(svn, accept, paths, *dry_run, *yes, tty)
        }
        Cmd::Cleanup { remove_unversioned, remove_ignored, vacuum_pristines, dry_run, yes } => {
            rescue::cleanup(svn, *remove_unversioned, *remove_ignored, *vacuum_pristines, *dry_run, *yes, tty)
        }
        Cmd::Doctor => rescue::doctor(cli, svn, tty),

        // 注意：这里匹配的是 `&cli.cmd`，字段都是引用，必须 clone 成 owned 值。
        Cmd::Daemon { action } => {
            daemon_cmd::run(daemon_cmd::Cmd::Daemon { action: action.clone() }, cli, svn)
        }
        Cmd::Q { dir, since } => {
            daemon_cmd::run(daemon_cmd::Cmd::Q { dir: dir.clone(), since: *since }, cli, svn)
        }

        Cmd::Cache { clear } => {
            let c = match crate::cache::Cache::open(&svn.root) {
                Some(c) => c,
                None => {
                    return Err(crate::domain::Error::Parse(
                        "该工作副本不支持缓存（缺少 .svn/wc.db，可能是 SVN 1.6 及更早）".to_string(),
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
                return Ok(format!("缓存文件不存在：{}\n（跑一次 svnui status 会生成）", st.path.display()));
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
