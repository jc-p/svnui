use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::{Error, LogEntry, NodeKind, RepoInfo, Result, Snapshot, StatusEntry, StatusKind};
use crate::svn::command::{run, run_streaming, run_with_stdin, RawOutput, RunOpts};
use crate::svn::parser::DirEntry;

use super::locator::{relative_to, svn_exe, wc_root};
use super::version::SvnVersion;

/// `svn list --xml` 的输出上限（解析前的闸门）。
///
/// 8MB 的 XML 大约对应 3~4 万个条目 —— 远超 TUI 能流畅渲染的量级，
/// 再往上就是纯粹的浪费。见 [`Svn::list`] 里的拦截。
const MAX_LIST_XML_BYTES: usize = 8 * 1024 * 1024;

/// 一个工作副本的句柄。
///
/// 构造走 [`Svn::discover`]：一次性完成「找 svn → 找工作副本根 → 探测版本」，
/// 之后每次调用只负责拼参数。
#[derive(Debug, Clone)]
pub struct Svn {
    exe: PathBuf,
    /// 工作副本根，所有命令都在这里执行。
    pub root: PathBuf,
    /// 调用者给的起始目录（可能是 root 的子目录），用于 `--depth` 优化。
    pub cwd: PathBuf,
    pub version: SvnVersion,
    timeout: Duration,
    global: Vec<String>,
    /// 是否放行 SSL 证书校验失败。
    ///
    /// 存下来是为了"再 discover 一次"的场景能继承：
    /// TUI 启动时可能拿着 discover_bare 的 Svn（root 不是真工作副本根），
    /// 之后要重新 discover，如果不继承这个标记，
    /// 用户配了信任也会被悄悄丢掉 —— 表现为"命令行为什么行、TUI 不行"。
    pub trust_cert: bool,
}

/// `status` 的查询选项。
///
/// `Default` 必须手写：派生的默认值会把 `include_unversioned` 设成 false，
/// 而"看见未版本化的新文件"是这个工具的核心用途之一。
#[derive(Debug, Clone)]
pub struct StatusOpts {
    /// 是否包含被忽略的文件（`--no-ignore`）。默认关 —— 大仓库下能少一半输出。
    pub include_ignored: bool,
    /// 是否包含未版本化文件。默认开。
    pub include_unversioned: bool,
    /// `svn status -u`：连服务器查是否有更新。**慢且可能超时，默认关。**
    pub show_updates: bool,
}

impl Default for StatusOpts {
    fn default() -> Self {
        Self { include_ignored: false, include_unversioned: true, show_updates: false }
    }
}

/// 认证参数（`--username` / `--password` / `--no-auth-cache`）。
///
/// ## 关于 login
///
/// svn 没有独立的"登录"命令 —— 凭据是在**第一次需要认证的操作**里
/// 顺带缓存的（存到 `~/.subversion/auth/`）。所以 `login` 的实现就是
/// 拿这组参数对远端跑一次 `svn info`：成功即代表凭据有效且已被缓存，
/// 之后所有操作都不用再输。
#[derive(Debug, Clone, Default)]
pub struct AuthOpts {
    pub username: Option<String>,
    pub password: Option<String>,
    /// 不给 svn 缓存。默认 false —— 缓存正是 login 的目的。
    pub no_auth_cache: bool,
}

impl AuthOpts {
    /// 拼成 svn 全局参数。
    ///
    /// 注意：密码走命令行参数，同主机的其他用户能从 `ps` 看到。
    /// svn 官方也这样（`--password` 只有这一种传法），
    /// 所以这里额外提供从 stdin 读的入口，见 `Svn::login`。
    fn global_args(&self, base: &[String]) -> Vec<String> {
        let mut g = base.to_vec();
        if let Some(u) = &self.username {
            g.push("--username".to_string());
            g.push(u.clone());
        }
        if let Some(p) = &self.password {
            g.push("--password".to_string());
            g.push(p.clone());
        }
        if self.no_auth_cache {
            g.push("--no-auth-cache".to_string());
        }
        g
    }
}

/// 放行所有 SSL 证书校验失败类型。
///
/// 对应 svn 的 `--trust-server-cert-failures`：
/// - `unknown-ca`  未知 CA（自签名 / 内网 CA）
/// - `cn-mismatch` 证书签给了别的主机名 ← 最常见，公司网关 SSL 中间人
/// - `expired`     已过期
/// - `not-yet-valid` 还没生效
/// - `other`       其他（svn 归类不了的一律进这）
///
/// ⚠️ 必须**全给**：很多服务器同时触发多个失败类型。
/// 比如 `certificate issued for a different hostname, and other reason(s)`
/// 是 `cn-mismatch` + `other` 两个，只给 cn-mismatch 仍然失败。
pub const TRUST_CERT_ARG: &str =
    "--trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other";

/// 是否启用证书信任。
///
/// 优先级：参数（`--trust-cert`）> 环境变量 > 默认 false。
/// 环境变量是为了不用每次敲 —— 内网仓库配一次即可。
/// 环境变量名 `SVNUI_TRUST_CERT`，老名字 `SVNR_TRUST_CERT` 也认。
pub fn trust_cert_enabled(explicit: bool) -> bool {
    if explicit {
        return true;
    }
    // 两个名字都认：SVNUI_ 跟二进制名一致（新），
    // SVNR_ 是项目改名前的老名字，留着兼容已配好的环境。
    env_flag("SVNUI_TRUST_CERT") || env_flag("SVNR_TRUST_CERT")
}

/// 读布尔型环境变量：`1 / true / yes / on` 视为开。
fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// 构造全局参数。
///
/// `--non-interactive` 是必须的：否则 svn 会停在
/// `(R)eject / accept (t)emporarily / accept (p)ermanently?` 提示上，
/// 而我们没法在 TUI 里回答这个提示。
/// 更麻烦的是，对于归类为 `other` 的证书失败，svn **根本不提供 (p)**，
/// 所以永久接受这条路对这些服务器无效，只能靠 --trust-server-cert-failures。
fn global_args(trust_cert: bool) -> Vec<String> {
    let mut g = vec!["--non-interactive".to_string()];
    if trust_cert {
        g.push(TRUST_CERT_ARG.to_string());
    }
    g
}

impl Svn {
    /// 从任意子目录发现工作副本。
    ///
    /// `trust_cert`：是否放行 SSL 证书校验失败（内网自签名/网关中间人）。
    pub fn discover(start: &Path, timeout: Duration, trust_cert: bool) -> Result<Self> {
        let exe = svn_exe().cloned().ok_or(Error::SvnNotFound)?;
        let root = wc_root(start).ok_or_else(|| Error::NotWorkingCopy(start.to_path_buf()))?;

        // cwd 必须是绝对路径：`svnui q --dir src` 这类相对路径要以它为基准拼接，
        // 而 CLI 传进来的常常是 "."。
        let cwd = start
            .canonicalize()
            .or_else(|_| std::env::current_dir())
            .unwrap_or_else(|_| start.to_path_buf());

        let version = Self::probe_version(&exe, &root)?;

        Ok(Self {
            exe,
            root,
            cwd,
            version,
            timeout,
            global: global_args(trust_cert_enabled(trust_cert)),
            trust_cert: trust_cert_enabled(trust_cert),
        })
    }

    /// 只探测 svn 可执行文件与版本，**不要求处于工作副本内**。
    ///
    /// `probe` 与 `install-yazi` 用它 —— 这两个命令的用途就是诊断环境：
    /// `probe` 要回答"为什么找不到仓库"，进了非工作副本直接报错等于自废武功；
    /// `install-yazi` 压根不需要仓库。
    ///
    /// 此时 `root` 被设为 `start`（不是真正的工作副本根），
    /// 调用方不应依赖它的值。
    pub fn discover_bare(start: &Path, timeout: Duration, trust_cert: bool) -> Result<Self> {
        let exe = svn_exe().cloned().ok_or(Error::SvnNotFound)?;
        let version = Self::probe_version(&exe, start)?;
        Ok(Self {
            exe,
            root: start.to_path_buf(),
            cwd: start.to_path_buf(),
            version,
            timeout,
            global: global_args(trust_cert_enabled(trust_cert)),
            trust_cert: trust_cert_enabled(trust_cert),
        })
    }

    fn probe_version(exe: &Path, cwd: &Path) -> Result<SvnVersion> {
        let opts = RunOpts { timeout: Duration::from_secs(5), global: vec![], ..Default::default() };
        let out = run(exe, &["--version", "--quiet"], cwd, &opts)?;
        SvnVersion::parse(&out.stdout).ok_or_else(|| Error::Parse(format!("无法解析版本号: {:?}", out.stdout)))
    }

    fn opts(&self, long: bool) -> RunOpts {
        RunOpts {
            timeout: if long { self.timeout * 6 } else { self.timeout },
            allow_fail: false,
            global: self.global.clone(),
        }
    }

    // ------------------------------------------------------------- 只读

    /// `svn status`。走**文本**解析（热路径，比 XML 快一个量级）。
    pub fn status(&self, o: &StatusOpts) -> Result<Vec<StatusEntry>> {
        let mut args: Vec<&str> = vec!["status"];
        if o.include_ignored {
            args.push("--no-ignore");
        }
        if o.show_updates {
            args.push("-u");
        }

        let out = run(&self.exe, &args, &self.root, &self.opts(true))?;
        let mut entries = super::porcelain::parse_status(&out.stdout);

        if !o.include_unversioned {
            entries.retain(|e| e.text != StatusKind::Unversioned);
        }
        // status 在有部分错误时可能返回非 0 但仍输出可用结果，这里不因空输出报错。
        Ok(entries)
    }

    /// 同上，但组装成带冒泡索引的 Snapshot。调用方直接消费这个。
    pub fn snapshot(&self, o: &StatusOpts) -> Result<Snapshot> {
        let root_str = self.root.to_string_lossy().to_string();
        let mut snap = Snapshot::new(root_str);
        for e in self.status(o)? {
            snap.insert(e);
        }
        snap.build_bubbles();
        Ok(snap)
    }

    /// 只对指定路径跑 `svn status`。
    ///
    /// daemon 的增量刷新靠这个：单文件/小批路径的 status 是毫秒级，
    /// 而全量在大仓库上是秒到几十秒级。
    ///
    /// ⚠️ 注意返回的是**这些路径当前的状态**；如果某个路径已恢复正常，
    /// 它不会出现在结果里 —— 调用方必须据此把它从快照里**删掉**，
    /// 只 insert 会让状态永远残留。
    pub fn status_paths(&self, paths: &[PathBuf]) -> Result<Vec<StatusEntry>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut args: Vec<&str> = vec!["status"];
        let owned: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
        for p in &owned {
            args.push(p.as_str());
        }
        let out = run(&self.exe, &args, &self.root, &self.opts(false))?;
        Ok(super::porcelain::parse_status(&out.stdout))
    }

    /// 只取本地变更（`svnui changed`）。
    pub fn changed(&self, o: &StatusOpts) -> Result<Vec<StatusEntry>> {
        Ok(self.status(o)?.into_iter().filter(|e| e.is_changed()).collect())
    }

    /// `svn status -u`：远端有更新的文件（本地绝对路径, 远端最新版本号）。
    ///
    /// ⚠️ **慢**：要连服务器，大仓库上可能几秒到几十秒。
    ///    只在用户显式要求时调用（update 前的冲突预警），
    ///    绝不进自动刷新路径。
    ///
    /// 返回的路径是绝对的（`self.root` + 相对路径），方便直接喂给别的命令。
    pub fn outdated(&self) -> Result<Vec<(PathBuf, u64)>> {
        let out = run(&self.exe, &["status", "-u"], &self.root, &self.opts(true))?;
        Ok(super::porcelain::parse_outdated(&out.stdout)
            .into_iter()
            // self.root 是 std PathBuf，join 直接就是 PathBuf，不用再转
            .map(|(p, rev)| (self.root.join(p), rev))
            .collect())
    }

    /// 冲突文件的三份内容。
    ///
    /// SVN 冲突后在工作副本里生成：
    /// - `f.mine` —— 我本地改动后的版本
    /// - `f.r<OLD>` —— 更新前的 BASE
    /// - `f.r<NEW>` —— 服务器最新版本（theirs）
    /// - `f` 本身 —— 带 `<<<<<<<` 冲突标记的合并结果
    ///
    /// 优先读这些副产品文件：它们就是 svn 自己生成的，比 `svn cat` 再算一遍
    /// 更准确（尤其是 `svn cat -r BASE` 在属性冲突时行为诡异）。
    /// 读不到才回落到 `svn cat`。
    pub fn conflict_versions(&self, abs: &Path) -> Result<ConflictVersions> {
        let dir = abs.parent().unwrap_or(Path::new("."));
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // 找 name.mine / name.r<N>
        let mut mine_path = None;
        let mut theirs_path = None;
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut revs: Vec<(u64, std::path::PathBuf)> = Vec::new();
            for e in rd.flatten() {
                let fname = e.file_name().to_string_lossy().to_string();
                if fname == format!("{name}.mine") {
                    mine_path = Some(e.path());
                } else if let Some(rest) = fname.strip_prefix(&format!("{name}.r")) {
                    if let Ok(r) = rest.parse::<u64>() {
                        revs.push((r, e.path()));
                    }
                }
            }
            // .rNEW 是版本号最大的那个
            revs.sort_by_key(|(r, _)| *r);
            theirs_path = revs.pop().map(|(_, p)| p);
        }

        let rel = self.rel(abs);
        let rel_s = rel.to_string_lossy();

        Ok(ConflictVersions {
            mine: read_or_cat(mine_path.as_deref(), || self.cat(&rel_s, Some("BASE"))),
            theirs: read_or_cat(theirs_path.as_deref(), || self.cat(&rel_s, Some("HEAD"))),
            working: std::fs::read_to_string(abs).unwrap_or_default(),
        })
    }

    /// 所有冲突（文本 + 属性 + 树冲突）。
    pub fn conflicts(&self) -> Result<Vec<StatusEntry>> {
        Ok(self.status(&StatusOpts::default())?.into_iter().filter(|e| e.is_conflicted()).collect())
    }

    /// `svn diff`。原文直通，不做解析（颜色/格式交给 pager）。
    pub fn diff(&self, paths: &[PathBuf], rev: Option<&str>, stat: bool) -> Result<RawOutput> {
        let mut args: Vec<&str> = vec!["diff"];
        if stat {
            args.push("--stat");
        } else {
            args.push("--force");
        }
        if let Some(r) = rev {
            args.push("-c");
            args.push(r);
        }
        let owned: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
        for p in &owned {
            args.push(p.as_str());
        }
        let out = run(&self.exe, &args, &self.root, &self.opts(true))?;
        Ok(out)
    }

    /// `svn log --xml -v`。
    /// `svn diff -c REV`：看某个版本改了什么（完整 diff 文本）。
    ///
    /// 和 `diff()` 的区别：那个是"工作副本 vs BASE"（本地未提交的改动），
    /// 这个是"历史上某个版本 vs 它的上一版"（已提交的改动）。
    ///
    /// 输出里的路径是仓库相对路径，不需要 shorten 处理。
    pub fn rev_diff(&self, rev: u64) -> Result<RawOutput> {
        let r = rev.to_string();
        self.op_no_targets(&["diff".to_string(), "-c".to_string(), r])
    }

    pub fn log(&self, limit: usize, paths: &[PathBuf]) -> Result<Vec<LogEntry>> {
        self.log_search(limit, paths, None)
    }

    /// `svn log`，可选服务端搜索。
    ///
    /// `search` 非空时加 `--search`（svn 1.8+，**在服务器端过滤**）。
    /// 这一条很关键：默认只加载 100 条，靠翻页永远够不到更早的提交；
    /// 而 `--search` 让服务器在**全部历史**里找，真正解决"找不到了"。
    ///
    /// 旧版本 svn 不认识 `--search`，会直接报错；
    /// 调用方负责探测版本并决定是否传入（见 `Svn::supports_log_search`）。
    ///
    /// 另外 `-v` 只在**不搜索**时加：搜索结果通常很多，
    /// 全量带 `-v` 会让服务端返回体积翻好几倍，拖慢首屏。
    /// 需要改动文件时再按版本单独查（`log_rev(rev, true)`）。
    pub fn log_search(
        &self,
        limit: usize,
        paths: &[PathBuf],
        search: Option<&str>,
    ) -> Result<Vec<LogEntry>> {
        let owned = self.guard_paths(paths)?;
        let lim = limit.to_string();
        // 必须显式给 `-r`：不写时 svn 从「工作副本 revision」往回列，
        // 而刚提交过的工作副本是混合 revision（提交过的文件是新号、
        // 其余还是旧号），svn 取**最小的那个**当起点 —— 刚提交的条目
        // 就不在列表里，表现是"提交完了看历史还是旧的"。
        let mut args: Vec<&str> = vec!["log", "--xml", "-r", "HEAD:0", "-l", &lim];
        if let Some(kw) = search.filter(|s| !s.trim().is_empty()) {
            args.push("--search");
            args.push(kw);
        } else {
            args.push("-v");
        }
        for p in &owned {
            args.push(p.as_str());
        }
        let out = run(&self.exe, &args, &self.root, &self.opts(true))?;
        super::parser::parse_log_xml(&out.stdout)
    }

    /// 是否支持 `svn log --search`（1.8+）。
    pub fn supports_log_search(&self) -> bool {
        (self.version.major, self.version.minor) >= (1, 8)
    }

    /// `svn info --xml`。
    /// 按版本查日志。`-c N` 等价于 `-r N`；`-r N:M` 取区间。
    ///
    /// 注意：这里走 `svn log -r`，**不是** `-c`。
    /// 两者的差别在合并提交上：`-c N` 显示该版本引入的变化，
    /// `-r N` 显示该版本的日志条目。svn 没有 git 那种 cherry-pick 语义，
    /// 用 `-r` 更接近用户按版本号查日志的预期。
    pub fn log_rev(&self, rev: &str, verbose: bool) -> Result<Vec<LogEntry>> {
        let mut args: Vec<String> = vec!["log".into(), "--xml".into()];
        // ⚠️ --xml 必须加：下面用 parse_log_xml 解析。
        //    不加的话 svn 输出纯文本（------ 分隔线那种），
        //    quick_xml 必然解析失败 → "failed to parse svn output"。
        //    `log()` 有这个参数，这里之前漏了。
        if verbose {
            args.push("-v".into());
        }
        args.push("-r".into());
        args.push(rev.to_string());
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = run(&self.exe, &refs, &self.root, &self.opts(false))?;
        super::parser::parse_log_xml(&out.stdout)
    }

    /// 逐行追溯作者与版本。
    ///
    /// 输出是原文，不解析 —— blame 的行数可能上万，
    /// 解析成结构体再格式化纯属浪费，且用户要看的就是原始对齐格式。
    pub fn blame(&self, path: &Path) -> Result<String> {
        let owned = self.guard_paths(&[path.to_path_buf()])?;
        let out = run(&self.exe, &["blame", owned[0].as_str()], &self.root, &self.opts(false))?;
        Ok(out.stdout)
    }

    pub fn info(&self) -> Result<RepoInfo> {
        let out = run(&self.exe, &["info", "--xml"], &self.root, &self.opts(false))?;
        super::parser::parse_info_xml(&out.stdout, &self.root.to_string_lossy())
    }

    /// 列远端目录（`svn list --xml`）。
    ///
    /// `url` 是**完整** URL（含 `https://`），不像其他方法那样传相对路径 ——
    /// 远端浏览没有工作副本可言，cwd 对它没意义。
    ///
    /// 用 XML 而非文本：文本输出只有名字，拿不到 revision / 作者 / 日期，
    /// 而"这个目录最后是谁在什么时候改的"正是浏览仓库时最想知道的。
    pub fn list(&self, url: &str) -> Result<Vec<DirEntry>> {
        let out = run(&self.exe, &["list", "--xml", url], &self.root, &self.opts(true))?;
        // 在**解析之前**挡一道。quick_xml 是全量反序列化，
        // 解析期的内存峰值大约是 XML 文本的 3~5 倍（每个条目都要建 3 个 String）。
        // 让 100MB 的 XML 进去，出来就是几百 MB 的 Vec —— 必须提前拦。
        if out.stdout.len() > MAX_LIST_XML_BYTES {
            return Err(crate::domain::Error::Explained {
                summary: "该目录条目过多，已中止读取".to_string(),
                detail: format!(
                    "svn 返回了约 {:.1} MB 的目录列表，超过 {:.0} MB 上限。\n\
                     这种量级在 TUI 里逐行翻也没有意义（每帧都要重画上万行）。\n\
                     建议：直接用更精确的子目录 URL 打开，或先用 svn list 在命令行里筛。",
                    out.stdout.len() as f64 / 1024.0 / 1024.0,
                    MAX_LIST_XML_BYTES as f64 / 1024.0 / 1024.0,
                ),
            });
        }
        super::parser::parse_list_xml(&out.stdout)
    }

    /// `svn cat` —— 预览已删除文件的原始内容时很有用。
    pub fn cat(&self, rel: &str, rev: Option<&str>) -> Result<String> {
        let mut args: Vec<&str> = vec!["cat"];
        if let Some(r) = rev {
            args.push("-r");
            args.push(r);
        }
        args.push(rel);
        let out = run(&self.exe, &args, &self.root, &self.opts(true))?;
        Ok(out.stdout)
    }

    /// 某个路径当前的节点类型。文本解析拿不到 kind，需要时按需 stat（不在热路径上）。
    pub fn node_kind(&self, abs: &Path) -> NodeKind {
        if abs.is_dir() {
            NodeKind::Dir
        } else if abs.is_symlink() {
            NodeKind::Symlink
        } else if abs.is_file() {
            NodeKind::File
        } else {
            NodeKind::Unknown
        }
    }

    /// 相对工作副本根的路径。
    pub fn rel(&self, abs: &Path) -> PathBuf {
        relative_to(&self.root, abs)
    }

    // ------------------------------------------------------------- 写操作
    //
    // 这一组全部**只负责执行**，不做危险判定 —— 那是 `policy` 的职责。
    // 调用方（cli）必须先过 `crate::policy::judge` 再进来。

    /// `svn add`。
    pub fn add(&self, paths: &[PathBuf]) -> Result<RawOutput> {
        self.paths_op("add", paths, false)
    }

    /// `svn rm`。默认**同时删除本地文件**（svn 原生语义）。
    ///
    /// 想保留本地文件要显式传 `keep_local = true`（`--keep-local`）。
    pub fn remove(&self, paths: &[PathBuf], keep_local: bool) -> Result<RawOutput> {
        let mut args: Vec<String> = vec!["rm".to_string()];
        if keep_local {
            args.push("--keep-local".to_string());
        }
        self.paths_op_owned(&args, paths)
    }

    /// `svn revert -R`。**丢本地改动，不可撤销。**
    ///
    /// ⚠️ svn 的 revert 没有 `--dry-run`。这里 `dry_run = true` 时**直接返回空结果**，
    /// 由上层负责打印"将要回滚这些文件"。不要误以为 svn 会帮你干跑。
    pub fn revert(&self, paths: &[PathBuf], dry_run: bool) -> Result<RawOutput> {
        if dry_run {
            return Ok(RawOutput { stdout: String::new(), stderr: String::new(), code: Some(0) });
        }
        let args = vec!["revert".to_string(), "-R".to_string()];
        self.paths_op_owned(&args, paths)
    }

    /// `svn commit -F -`。提交信息走 stdin，避免 shell 转义问题。
    ///
    /// ⚠️ 这里**故意不用** `command::run` —— 那条路径强制 `stdin=null`，
    /// 而 commit 必须喂 stdin。为此单开一个 `run_with_stdin`。
    pub fn commit(&self, message: &str, paths: &[PathBuf]) -> Result<RawOutput> {
        let mut args: Vec<String> = vec!["commit".to_string(), "-F".to_string(), "-".to_string()];
        for p in paths {
            args.push(p.to_string_lossy().to_string());
        }
        super::command::run_with_stdin(&self.exe, &args, &self.root, message, &self.opts(true))
    }

    /// 反向合并：`svn merge -r HEAD:{rev} .`
    ///
    /// 把 HEAD 到 `rev` 之间的差异**反向**应用到工作副本，
    /// 等价于"回到 `rev` 那一刻"；`rev` 之后的提交从工作副本里被撤销
    /// （仓库里那些提交还在，只是本地不再包含）。
    ///
    /// ⚠️ 执行完**还要再提交一次**才会真正写进仓库。
    /// ⚠️ 固定 `--accept postpone`：冲突不自动解决，留给 resolve 面板。
    ///
    /// `dry_run = true` 时只输出"会动哪些文件"而不落盘。
    /// （`svn revert` 没有 `--dry-run`，merge 有 —— 这是回退走 merge 的原因之一。）
    pub fn merge_to(&self, rev: u64, dry_run: bool) -> Result<RawOutput> {
        let mut args: Vec<String> = vec![
            "merge".to_string(),
            "--accept".to_string(),
            "postpone".to_string(),
        ];
        if dry_run {
            args.push("--dry-run".to_string());
        }
        args.push("-r".to_string());
        args.push(format!("HEAD:{rev}"));
        args.push(".".to_string());
        self.op_no_targets(&args)
    }

    /// `svn update`。默认 `--accept postpone` —— 不自动合并，把决定权留给人。
    ///
    /// 这是刻意的：自动合并（如 `theirs-full`）可能静默吃掉本地改动。
    pub fn update(&self, rev: Option<&str>, paths: &[PathBuf]) -> Result<RawOutput> {
        let mut args: Vec<String> = vec!["update".to_string(), "--accept".to_string(), "postpone".to_string()];
        if let Some(r) = rev {
            args.push("-r".to_string());
            args.push(r.to_string());
        }
        self.paths_op_owned(&args, paths)
    }

    /// `svn resolve --accept=STRATEGY`。
    ///
    /// `strategy` 常见取值：`mine-full` / `theirs-full` / `working` / `base`
    /// / `mine-conflict` / `theirs-conflict`。
    pub fn resolve(&self, paths: &[PathBuf], strategy: &str) -> Result<RawOutput> {
        let mut args: Vec<String> =
            vec!["resolve".to_string(), format!("--accept={strategy}"), "-R".to_string()];
        for p in paths {
            args.push(p.to_string_lossy().to_string());
        }
        self.op_no_targets(&args)
    }

    /// `svn cleanup`。`remove_unversioned` 是核弹开关，只在这个方法里出现一次。
    pub fn cleanup(
        &self,
        remove_unversioned: bool,
        remove_ignored: bool,
        vacuum_pristines: bool,
    ) -> Result<RawOutput> {
        let mut args: Vec<String> = vec!["cleanup".to_string()];
        if remove_unversioned {
            args.push("--remove-unversioned".to_string());
        }
        if remove_ignored {
            args.push("--remove-ignored".to_string());
        }
        if vacuum_pristines {
            if !self.version.supports_vacuum_pristines() {
                return Err(Error::Parse(format!(
                    "svn {} 不支持 --vacuum-pristines（需要 1.10+）",
                    self.version
                )));
            }
            args.push("--vacuum-pristines".to_string());
        }
        self.op_no_targets(&args)
    }

    /// `svn lock` / `svn unlock`。
    pub fn lock(&self, paths: &[PathBuf], unlock: bool) -> Result<RawOutput> {
        let sub = if unlock { "unlock" } else { "lock" };
        self.paths_op(sub, paths, false)
    }

// ---------------------------------------------------- 检出 / 认证

    /// `svn checkout`。
    ///
    /// ⚠️ 这是**唯一不需要工作副本**的操作 —— 它的目的就是创建工作副本。
    /// 所以不能依赖 `self.root`（`self` 是 discover 出来的，discover 要求
    /// 已经在工作副本里）。这里只在 PATH 的父目录执行，`self.root` 不参与。
    pub fn checkout(
        &self,
        url: &str,
        path: &Path,
        auth: &AuthOpts,
        depth: &str,
    ) -> Result<RawOutput> {
        self.checkout_progress(url, path, auth, depth, |_| {})
    }

    /// `svn checkout`，**逐行把进度交给回调**。
    ///
    /// 检出大仓库要几十分钟，界面上只有个不动的"检出 …"看着跟卡死一样。
    /// svn 每落一个文件就打一行（`A   trunk/foo.c`），这里把它递出去，
    /// UI 就能显示"已检出 1284 项"—— 有数字在动，才知道它活着。
    ///
    /// ⚠️ 回调在读线程里跑，且**必须由调用方节流**：几万行的仓库不节流
    ///    会把 channel 打满，UI 收消息比干活还忙。
    pub fn checkout_progress<F>(
        &self,
        url: &str,
        path: &Path,
        auth: &AuthOpts,
        depth: &str,
        on_line: F,
    ) -> Result<RawOutput>
    where
        F: FnMut(&str) + Send + 'static,
    {
        // 绝对路径：svn 会在 cwd 下创建它，相对路径容易搞错位置。
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
        };

        // cwd 取父目录，父目录不存在就退到当前目录
        let cwd = abs
            .parent()
            .filter(|p| p.exists())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let opts = RunOpts {
            // 大仓库检出可能几十分钟。这里给足，超时机制仍会 kill 掉。
            timeout: Duration::from_secs(30 * 60),
            allow_fail: false,
            global: auth.global_args(&self.global),
        };

        let abs_s = abs.to_string_lossy().to_string();
        run_streaming(
            &self.exe,
            &["checkout", "--depth", depth, url, &abs_s],
            &cwd,
            &opts,
            on_line,
        )
    }

    /// 对远端跑一次 `svn info`，让 svn 缓存凭据。
    ///
    /// 密码优先从 stdin 给（`--password-from-stdin`，svn 1.12+），
    /// 退而求其次才走命令行参数 —— 后者会被 `ps` 看到。
    pub fn login(&self, url: &str, auth: &AuthOpts) -> Result<RawOutput> {
        let global = auth.global_args(&self.global);
        let opts = RunOpts { timeout: self.timeout * 4, allow_fail: false, global: global.clone() };

        if let (Some(pwd), true) = (&auth.password, self.version.supports_password_from_stdin()) {
            // 注意：不能同时给 --password 和 --password-from-stdin
            let mut g = opts.global.clone();
            g.retain(|a| a != "--password");
            let stdin_opts = RunOpts { global: g, ..opts.clone() };
            let args = vec!["info".to_string(), url.to_string(), "--password-from-stdin".to_string()];
            return run_with_stdin(&self.exe, &args, &self.root, pwd, &stdin_opts);
        }

        run(&self.exe, &["info", url], &self.root, &opts)
    }

    /// 注销：清掉 svn 缓存的凭据。
    ///
    /// 不走 `svn auth --remove` —— 它接受的是缓存**文件路径**，
    /// 得先列一遍再逐个删，而这里要的就是"全清"。直接删
    /// `~/.subversion/auth/` 更直接，也是 svn 官方认可的清缓存方式。
    ///
    /// ⚠️ 会清掉**所有仓库**的凭据（svn 的缓存是按 realm 分目录，
    /// 不是按仓库）。要只删一个，用 `svn auth --remove <路径>`。
    pub fn logout(&self) -> Result<RawOutput> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| Error::Parse("找不到 HOME，无法定位凭据缓存目录".to_string()))?;

        let auth_dir = PathBuf::from(home).join(".subversion").join("auth");
        let existed = auth_dir.exists();
        if existed {
            std::fs::remove_dir_all(&auth_dir)?;
        }

        Ok(RawOutput {
            stdout: if existed {
                "已清除 svn 凭据缓存".to_string()
            } else {
                "本来就没有缓存的凭据".to_string()
            },
            stderr: String::new(),
            code: Some(0),
        })
    }

    /// 列出已缓存的凭据（`svn auth`，1.9+）。
    pub fn auth_list(&self) -> Result<RawOutput> {
        if !self.version.supports_show_item() {
            return Err(Error::Parse(format!(
                "svn {} 不支持 svn auth（需要 1.9+）",
                self.version
            )));
        }
        let opts = RunOpts { timeout: self.timeout, allow_fail: true, global: self.global.clone() };
        run(&self.exe, &["auth"], &self.root, &opts)
    }

    // ------------------------------------------------------------- 环境

    pub fn svn_path(&self) -> &Path {
        &self.exe
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    // ------------------------------------------------------- 内部拼装辅助

    /// `[sub, path...]`。路径为空时表示作用于整个工作副本（cwd = root）。
    /// 校验目标路径都在工作副本内。
    ///
    /// 为什么必须校验：yazi 传进来的是**用户选中的**路径，而选中集合里
    /// 可能混进工作副本外的文件（比如从别的标签页复制过来的选中状态，
    /// 或者路径含 `..`）。svn 对越界路径的行为是"静默地部分成功"——
    /// 比如 `svn add /etc/passwd` 会报 skipped，但同一个命令里的其他
    /// 合法路径照常执行。这种半成功状态极难排查，不如入口就拦住。
    fn guard_paths(&self, paths: &[PathBuf]) -> Result<Vec<String>> {
        let root = &self.root;
        let owned: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();

        for (raw, p) in paths.iter().enumerate().map(|(i, p)| (&owned[i], p)) {
            // 相对路径先基于 cwd 解析，否则 `../x` 的判定会失真
            let abs = if p.is_relative() { self.cwd.join(p) } else { p.clone() };
            let abs = match abs.canonicalize() {
                Ok(a) => a,
                // 不存在的文件（比如刚删掉的）无法 canonicalize。
                // 用它的父目录判定 —— 父目录在工作副本内就放行。
                Err(_) => match abs.parent() {
                    Some(parent) => parent
                        .canonicalize()
                        .map(|c| c.join(abs.file_name().unwrap_or_default()))
                        .unwrap_or_else(|_| abs.clone()),
                    None => abs.clone(),
                },
            };

            if !abs.starts_with(root) {
                return Err(Error::Parse(format!(
                    "路径不在工作副本内，已拒绝：{raw}\n  工作副本根：{}",
                    root.display()
                )));
            }
        }
        Ok(owned)
    }

    fn paths_op(&self, sub: &str, paths: &[PathBuf], long: bool) -> Result<RawOutput> {
        let owned = self.guard_paths(paths)?;
        let mut args: Vec<&str> = vec![sub];
        for p in &owned {
            args.push(p.as_str());
        }
        if args.len() == 1 {
            args.push(".");
        }
        run(&self.exe, &args, &self.root, &self.opts(long))
    }

    /// 需要在子命令后插入额外 flag 时用这个（如 `update --accept postpone`）。
    ///
    /// ⚠️ `owned` 必须活到 `run()` 返回：它持有路径的真实字符串，
    ///    而 `args` 只是借用它们。放进内层块会 E0597（借用值活得不够久）。
    fn paths_op_owned(&self, base: &[String], paths: &[PathBuf]) -> Result<RawOutput> {
        let owned = self.guard_paths(paths)?;

        let mut args: Vec<&str> = Vec::with_capacity(base.len() + paths.len() + 1);
        for a in base {
            args.push(a.as_str());
        }
        if paths.is_empty() {
            args.push(".");
        } else {
            for p in &owned {
                args.push(p.as_str());
            }
        }
        run(&self.exe, &args, &self.root, &self.opts(true))
    }

    /// 已有完整参数列表，直接跑（用于 resolve / cleanup 这类带 `--flag=value` 的）。
    fn op_no_targets(&self, args: &[String]) -> Result<RawOutput> {
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run(&self.exe, &refs, &self.root, &self.opts(true))
    }
}

// ---------------------------------------------------- 冲突三路内容

/// 冲突文件的三份内容。任何一份都可能为空（文件不存在或读取失败）。
#[derive(Debug, Clone, Default)]
pub struct ConflictVersions {
/// 我本地改动后的版本。
pub mine: String,
/// 服务器最新版本。
pub theirs: String,
/// 当前工作副本里带冲突标记的内容。
pub working: String,
}

/// 优先读文件，读不到就执行 `f()`；`f()` 失败也给空串而不是报错。
///
/// 冲突三路里任何一路缺失都不该让整个面板挂掉 —— 少看一路用户也能做决定。
fn read_or_cat<F>(path: Option<&Path>, f: F) -> String
where
F: FnOnce() -> Result<String>,
{
if let Some(p) = path {
    if let Ok(s) = std::fs::read_to_string(p) {
        return s;
    }
}
f().unwrap_or_default()
}

