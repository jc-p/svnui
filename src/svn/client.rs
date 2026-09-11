use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::{Error, LogEntry, NodeKind, RepoInfo, Result, Snapshot, StatusEntry, StatusKind};
use crate::svn::command::{run, RawOutput, RunOpts};

use super::locator::{relative_to, svn_exe, wc_root};
use super::version::SvnVersion;

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

impl Svn {
    /// 从任意子目录发现工作副本。
    pub fn discover(start: &Path, timeout: Duration) -> Result<Self> {
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
            global: vec!["--non-interactive".to_string()],
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
    pub fn discover_bare(start: &Path, timeout: Duration) -> Result<Self> {
        let exe = svn_exe().cloned().ok_or(Error::SvnNotFound)?;
        let version = Self::probe_version(&exe, start)?;
        Ok(Self {
            exe,
            root: start.to_path_buf(),
            cwd: start.to_path_buf(),
            version,
            timeout,
            global: vec!["--non-interactive".to_string()],
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

    /// 同上，但组装成带冒泡索引的 Snapshot。yazi 侧直接消费这个。
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
    pub fn log(&self, limit: usize, paths: &[PathBuf]) -> Result<Vec<LogEntry>> {
        let owned = self.guard_paths(paths)?;
        let lim = limit.to_string();
        let mut args: Vec<&str> = vec!["log", "--xml", "-v", "-l", &lim];
        for p in &owned {
            args.push(p.as_str());
        }
        let out = run(&self.exe, &args, &self.root, &self.opts(true))?;
        super::parser::parse_log_xml(&out.stdout)
    }

    /// `svn info --xml`。
    /// 按版本查日志。`-c N` 等价于 `-r N`；`-r N:M` 取区间。
    ///
    /// 注意：这里走 `svn log -r`，**不是** `-c`。
    /// 两者的差别在合并提交上：`-c N` 显示该版本引入的变化，
    /// `-r N` 显示该版本的日志条目。svn 没有 git 那种 cherry-pick 语义，
    /// 用 `-r` 更接近用户按版本号查日志的预期。
    pub fn log_rev(&self, rev: &str, verbose: bool) -> Result<Vec<LogEntry>> {
        let mut args: Vec<String> = vec!["log".into()];
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
