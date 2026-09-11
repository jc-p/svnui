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
