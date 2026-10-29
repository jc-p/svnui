pub mod daemon;
pub mod hint;
pub mod mutate;
pub mod query;
pub mod rescue;
pub mod workflow;

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
    pub cmd: Option<Cmd>,
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

    /// 独立预览本地文件或浏览 SVN URL，无需进入工作副本。
    #[cfg(feature = "tui")]
    Preview {
        /// 本地文件路径或 SVN 目录 URL；相对路径基于 --cwd 或当前目录。
        path: PathBuf,
    },

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

    /// 从当前 URL 的仓库版本创建分支（立即提交仓库，不含本地改动）。
    Branch {
        /// 新目标的完整 URL 或 ^/branches/name；父目录必须存在。
        destination: String,
        #[arg(short, long)]
        message: String,
        /// 非负整数 / HEAD / BASE；默认 BASE。
        #[arg(short, long)]
        revision: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// 从当前 URL 的仓库版本创建标签（与 branch 同为仓库副本）。
    Tag {
        /// 新目标的完整 URL 或 ^/tags/name；父目录必须存在。
        destination: String,
        #[arg(short, long)]
        message: String,
        #[arg(short, long)]
        revision: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// 切换整个工作副本；拒绝本地改动、未纳管项或已切换子树。
    Switch {
        url: String,
        /// 非负整数 / HEAD / BASE；默认 HEAD。
        #[arg(short, long)]
        revision: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// 原生 SVN 合并预演；不会应用改动或提交。
    MergePreview {
        source: String,
        /// N:M；不指定时预演所有符合条件的合并。
        #[arg(short = 'r', long = "revision")]
        revisions: Option<String>,
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

    /// LLM 配置示例与自检：查看实际生效的预设、模型及连通性。
    ///
    /// 不依赖 SVN 工作副本；--test 才发送请求，失败返回非零退出码。
    Llm {
        /// 实发一个最小请求验证连通性（会消耗少量 token）。
        #[arg(long, conflicts_with = "preset")]
        test: bool,

        /// 输出 zhipu / ollama / deepseek 配置示例；不写文件、不联网。
        #[arg(long)]
        preset: Option<crate::config::LlmProvider>,
    },

    /// 配置文件状态与初始化。
    ///
    /// 不带参数：显示路径、是否存在、当前生效的关键项。
    /// --init：写一份默认配置（已存在时不覆盖，加 --force 才覆盖）。
    Config {
        /// 写入默认配置文件。
        #[arg(long)]
        init: bool,

        /// 配合 --init 使用：覆盖已有文件。
        #[arg(long)]
        force: bool,

        /// 只打印配置文件路径，方便 `vim $(svnui config --path)`。
        #[arg(long)]
        path: bool,
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
        if let Some(Cmd::Daemon { action }) = &self.cmd {
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
pub fn run(mut cli: Cli) -> i32 {
    // 配置文件里的 `trust_cert` 与命令行 `--trust-cert` 等价：
    // 内网自签名证书每次都敲参数太烦，配一次就行。
    if !cli.trust_cert && crate::config::Config::load().svn.trust_cert {
        cli.trust_cert = true;
    }
    // 不带子命令 → 进 TUI（主形态）
    #[cfg(feature = "tui")]
    if cli.cmd.is_none() {
        cli.cmd = Some(Cmd::Tui);
    }
    
    #[cfg(not(feature = "tui"))]
    if cli.cmd.is_none() {
        // 没编 TUI 又没给子命令：打帮助，别默默干别的事
        use clap::CommandFactory;
        let _ = Cli::command().print_help();
        return 2;
    }
    
    // 配置示例和模型自检不使用 SVN，不能被工作副本或 SVN 安装状态阻断。
    if let Some(Cmd::Llm { test, preset }) = cli.cmd.as_ref() {
        return match llm_report(*test, *preset) {
            Ok(text) => { println!("{text}"); 0 }
            Err(e) => { emit_err(&cli, &e); e.exit_code() }
        };
    }

    let start = cli.start_dir();

    // URL 保持原字符串，不能与本地目录拼接；本地预览仍绕过 SVN 探测。
    #[cfg(feature = "tui")]
    if let Some(Cmd::Preview { path }) = cli.cmd.as_ref() {
        let url = path.to_str().filter(|value| {
            ["https://", "http://", "svn://", "svn+ssh://", "file://"]
                .iter()
                .any(|scheme| value.starts_with(scheme))
        });
        let result = match url {
            Some(url) => crate::svn::Svn::discover_bare(&start, cli.timeout(), cli.trust_cert)
                .and_then(|svn| crate::tui::browse_repository(svn, url)),
            None => crate::tui::preview(&start.join(path)),
        };
        return match result {
            Ok(()) => 0,
            Err(e) => {
                emit_err(&cli, &e);
                e.exit_code()
            }
        };
    }

    // 这些命令不需要工作副本：
    //   · probe        —— 它就是用来回答"为什么找不到仓库"的，
    //                     进了非工作副本直接退出等于自废武功
    //   · checkout     —— 它的用途就是**创建**工作副本，要求先有工作副本是循环依赖
    //   · login / auth —— 凭据是全局的（~/.subversion/auth），不隶属于某个副本
    let mut needs_wc = !matches!(
        cli.cmd,
        Some(Cmd::Probe) | Some(Cmd::Checkout { .. }) | Some(Cmd::Login { .. }) | Some(Cmd::Auth)
    );
    // TUI 自己能处理"不在工作副本"的情况（进去给检出界面），
    // 所以在这一层放行，别先把它拦下来。
    #[cfg(feature = "tui")]
    if matches!(cli.cmd, Some(Cmd::Tui)) {
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

/// LLM 自检输出。不联网时只描述配置；`--test` 才实发请求。
fn llm_report(test: bool, preset: Option<crate::config::LlmProvider>) -> crate::domain::Result<String> {
    if let Some(provider) = preset {
        return Ok(crate::config::LlmConfig::preset_example(provider));
    }
    let explain = |summary| crate::domain::Error::Explained { summary, detail: String::new() };
    let cfg = crate::config::Config::load_checked().map_err(explain)?;
    let mut out = crate::llm::describe(&cfg.llm);

    if test {
        out.push_str("\n\n");
        out.push_str(&crate::llm::ping(&cfg.llm)
            .map_err(|e| explain(format!("连通性测试失败：{e}")))?);
    }
    Ok(out)
}

/// 配置文件自检。`--init` 写入默认配置，`--path` 只打路径。
fn config_report(
    init: bool,
    force: bool,
    path_only: bool,
) -> Result<String, crate::domain::Error> {
    // --path 也顺手补一份：否则 vim 打开的是不存在的文件，白高兴一场。
    if path_only {
        crate::config::Config::ensure_file();
        return Ok(crate::config::Config::path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "（拿不到路径：HOME 未设置）".to_string()));
    }

    if init {
        let p = crate::config::Config::write_default(force)?;
        return Ok(format!(
            "配置文件已写入 {}\n{}\n\n改完可用 `svnui config` 复核。",
            p.display(),
            if force {
                "（已覆盖原有文件）"
            } else {
                "（原文件已存在则未改动）"
            }
        ));
    }

    Ok(crate::config::Config::load().describe())
}

fn dispatch(cli: &Cli, svn: &crate::svn::Svn, tty: bool) -> Result<String, crate::domain::Error> {
    use self::daemon as daemon_cmd;

    match cli.cmd.as_ref().expect("cmd 已由 run() 填充") {
        #[cfg(feature = "tui")]
        Cmd::Tui => {
            // TUI 自己接管终端（raw mode + alternate screen），
            // 不走 output 层，也不该往 stdout 打东西。
            return crate::tui::run(svn.clone()).map(|_| String::new());
        }

        #[cfg(feature = "tui")]
        Cmd::Preview { .. } => unreachable!("preview 在 SVN 探测前已分发"),

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
        Cmd::Llm { test, preset } => llm_report(*test, *preset),
        Cmd::Config { init, force, path } => config_report(*init, *force, *path),

        Cmd::Add { paths } => mutate::add(svn, paths),
        Cmd::Remove { keep_local, yes, paths } => {
            mutate::remove(svn, *keep_local, *yes, paths, tty)
        }
        Cmd::Revert { paths, dry_run, yes } => mutate::revert(svn, paths, *dry_run, *yes, tty),
        Cmd::Commit { message, paths, dry_run, yes } => {
            mutate::commit(svn, message.as_deref(), paths, *dry_run, *yes, tty)
        }
        Cmd::Update { rev, paths, yes } => mutate::update(svn, rev.as_deref(), paths, *yes, tty),
        Cmd::Branch { destination, message, revision, dry_run, yes }
        | Cmd::Tag { destination, message, revision, dry_run, yes } => {
            use crate::svn::workflow::{WorkflowOperation, WorkflowRequest};
            let operation = if matches!(cli.cmd, Some(Cmd::Branch { .. })) {
                WorkflowOperation::Branch
            } else {
                WorkflowOperation::Tag
            };
            workflow::run(svn, WorkflowRequest {
                operation, url: destination.clone(), revision: revision.clone(), message: message.clone(),
            }, *dry_run, *yes, tty, cli.json)
        }
        Cmd::Switch { url, revision, dry_run, yes } => workflow::run(svn,
            crate::svn::workflow::WorkflowRequest {
                operation: crate::svn::workflow::WorkflowOperation::Switch,
                url: url.clone(), revision: revision.clone(), message: String::new(),
            }, *dry_run, *yes, tty, cli.json),
        Cmd::MergePreview { source, revisions } => workflow::run(svn,
            crate::svn::workflow::WorkflowRequest {
                operation: crate::svn::workflow::WorkflowOperation::MergePreview,
                url: source.clone(), revision: revisions.clone(), message: String::new(),
            }, false, false, tty, cli.json),
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
