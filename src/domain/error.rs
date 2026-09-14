use std::path::PathBuf;
use thiserror::Error;

/// 全 crate 统一结果类型。
pub type Result<T> = std::result::Result<T, Error>;

/// 领域错误。**刻意不用 anyhow** —— domain 层需要被 daemon、测试、CLI 分别消费，
/// 具体类型才能做差异化处理（例如 `Locked` 要触发 cleanup 提示）。
#[derive(Debug, Error)]
pub enum Error {
    /// PATH 里找不到 svn；也可能用户没装 Subversion 命令行工具。
    #[error("svn executable not found in PATH (override with SVNR_SVN=/path/to/svn)")]
    SvnNotFound,

    /// 给定路径向上遍历到文件系统根都没找到 `.svn`。
    #[error("not a subversion working copy: {0}")]
    NotWorkingCopy(PathBuf),

    /// svn 自身非零退出。stderr 原样保留，供上层翻译成人话。
    #[error("svn exited with {code}: {stderr}")]
    SvnFailed { code: i32, stderr: String },

    /// svn 自身非零退出，且 stderr 已被翻译成人话。
    ///
    /// 和 `SvnFailed` 的区别：这个变体适合直接显示给用户
    /// （"连不上服务器，检查网络"），`SvnFailed` 保留原始 stderr 用于排查。
    #[error("{summary}")]
    Explained { summary: String, detail: String },

    /// 工作副本被锁（E155004 之类），通常需要 `svn cleanup`。
    #[error("working copy is locked: {0}")]
    Locked(PathBuf),

    /// 解析 svn 输出失败。意味着样本与 svn 版本脱节，或遇到了未覆盖的格式。
    #[error("failed to parse svn output: {0}")]
    Parse(String),

    /// 超时。一定已经 kill 掉子进程，不会有孤儿进程残留。
    #[error("svn command timed out after {0}s")]
    Timeout(u64),

    #[error("operation cancelled")]
    Cancelled,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// 给 CLI 出口层用的退出码。
    ///
    /// `0` 成功 ｜ `1` 业务失败 ｜ `2` 参数错误 ｜ `3` 非工作副本 ｜ `4` svn 未安装
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::NotWorkingCopy(_) => 3,
            Error::SvnNotFound => 4,
            _ => 1,
        }
    }

    /// stderr 里出现这些特征时，说明是 WC 锁而不是别的错误。
    pub fn is_lock_error(stderr: &str) -> bool {
        stderr.contains("E155004")
            || stderr.contains("Working copy")
                && stderr.contains("locked")
            || stderr.contains("sqlite: database is locked")
    }
}

/// 把 svn 的 stderr 翻译成人话。
///
/// ## 为什么需要
///
/// svn 的原始报错是给管理员看的：
/// `E170013: Unable to connect to a repository at URL 'https://...'`
/// 用户看到这个只会想"然后呢？"。
///
/// 真正有用的信息是**下一步该干什么**：
/// 连不上 → 查网络/VPN/换镜像；没权限 → 找管理员要账号。
///
/// 匹配用错误码（E170013）而不是整句文案 —— 文案会随版本和本地化变，
/// 错误码是稳定的契约。
pub fn explain(stderr: &str) -> Option<(String, String)> {
    let code = extract_code(stderr);

    let (summary, action) = match code.as_deref() {
        // SSL 证书校验失败。
        //
        // ⚠️ 必须放在 E170013 分支**之前**：
        // 证书挂了的时候 svn 会同时报 E170013（连不上）+ E230001（证书），
        // 而 E170013 是"结果"、E230001 才是"原因"。
        // 匹配到前者就会给出"检查网络"的建议 —— 方向完全错了。
        Some("E230001") => (
            "SSL 证书校验失败",
            concat!(
                "服务器证书的域名对不上，或用的是自签名/内网 CA。\n",
                "确认是自己的内网服务器后：加 --trust-cert 重跑，\n",
                "或设环境变量 SVNR_TRUST_CERT=1（省得每次敲）。"
            ),
        ),
        // 连不上服务器 —— 最常见的"日志读不出来"原因
        Some("E170013") | Some("E170001") | Some("E000111") => (
            "连不上 SVN 服务器",
            "检查网络 / VPN 是否连通，或确认仓库地址还有效。\n如果服务器要求登录，先跑 svnui login。",
        ),
        // 认证失败
        Some("E215004") => (
            "认证失败",
            "用户名或密码不对。跑 svnui login --username <你> 重新登录。",
        ),
        Some("E215001") => (
            "没有权限",
            "这个账号没有访问该仓库/路径的权限，找管理员开通。",
        ),
        // 不是工作副本
        Some("E155007") => (
            "这里不是 SVN 工作副本",
            "当前目录没有 .svn。cd 到工作副本里，或先跑 svnui checkout。",
        ),
        // 工作副本被锁
        Some("E155004") | Some("E155009") => (
            "工作副本被锁",
            "上次操作异常中断留下了锁。跑 svn cleanup 清理。",
        ),
        // 路径不存在（可能是版本太老没有这个路径）
        Some("E160013") | Some("E170000") => (
            "路径在该版本不存在",
            "这个文件或目录在指定的版本里还没有被创建（或已被删除）。",
        ),
        _ => return None,
    };

    Some((summary.to_string(), action.to_string()))
}

/// 从 stderr 里挑出**最关键**的那个 svn 错误码。
///
/// ## 为什么不是"取第一个"
///
/// svn 经常一次报多个错，而且**结果在前、原因在后**：
///
/// ```text
/// svn: E170013: Unable to connect to a repository at URL '...'
/// svn: E230001: Server SSL certificate verification failed: ...
/// ```
///
/// 这里 E170013（连不上）只是*结果*，E230001（证书）才是*原因*。
/// 取第一个就会建议"检查网络" —— 方向完全错了，用户会去查 VPN，
/// 而真正的问题在证书上。
///
/// 所以按**原因优先级**排序，而不是出现顺序：
/// 越能指向"具体下一步动作"的码优先级越高。
fn extract_code(stderr: &str) -> Option<String> {
    // 从高到低。证书 > 认证 > 锁 > 连接 > 其他
    const PRIORITY: &[&str] = &[
        "E230001", // SSL 证书
        "E215004", // 认证失败
        "E215001", // 无权限
        "E155004", // 工作副本锁
        "E155009",
        "E155007", // 不是工作副本
        "E160013", // 路径不存在
        "E170000",
        "E170013", // 连不上（放在后面：它常常只是别的错误的结果）
        "E170001",
        "E000111",
    ];

    let codes: Vec<String> = stderr
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| {
            w.len() >= 6
                && w.len() <= 8
                && w.starts_with('E')
                && w[1..].chars().all(|c| c.is_ascii_digit())
        })
        .map(|w| w.to_string())
        .collect();

    PRIORITY
        .iter()
        .find(|p| codes.iter().any(|c| c == *p))
        .map(|p| p.to_string())
        .or_else(|| codes.into_iter().next())
}

/// 把 `SvnFailed` 升级成带解释的版本（能识别的话）。
///
/// 识别不出来就原样返回 —— 宁可显示原始报错，也不要瞎猜一个建议。
pub fn with_explanation(code: i32, stderr: String) -> Error {
    match explain(&stderr) {
        Some((summary, detail)) => Error::Explained {
            summary: format!("{}（svn 退出码 {}）", summary, code),
            detail,
        },
        None => Error::SvnFailed { code, stderr },
    }
}
