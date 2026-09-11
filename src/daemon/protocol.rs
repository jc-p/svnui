//! daemon 通信协议。
//!
//! 单行 JSON，请求与响应各一行、以 `\n` 结尾。
//!
//! 为什么不用 bincode：这个协议每帧只有几百字节到几十 KB，
//! JSON 的编解码开销完全不是瓶颈，但**可调试性**天差地别 ——
//! 出问题时 `nc -U /path/to.sock` 敲一行 JSON 就能看到daemon 返回什么。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 客户端 → daemon。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    /// 要查询的目录（绝对路径）。
    pub dir: String,
    /// 客户端已知的版本戳。与 daemon 当前一致时，`changed` 为 false
    /// 且 `map` 为空 —— 客户端直接用本地缓存即可。
    pub since: Option<u64>,
}

/// daemon → 客户端。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Reply {
    /// 当前版本戳。任何状态变化都会让它自增。
    pub v: u64,
    /// false 表示"与 since 一致，没变化"。此时 `map` 为空。
    pub changed: bool,
    pub root: Option<String>,
    pub branch: Option<String>,
    /// 绝对路径 → porcelain 两字符。
    /// 只包含**该目录的直接子项**（目录的标记已冒泡聚合）。
    #[serde(default)]
    pub map: HashMap<String, String>,
    /// 全量扫描是否已完成。false 时 map 可能不完整。
    pub ready: bool,
}

impl Reply {
    /// 无变化时的极简响应（几十字节）。
    pub fn unchanged(v: u64, ready: bool) -> Self {
        Self {
            v,
            changed: false,
            ready,
            ..Default::default()
        }
    }
}

/// 控制命令（`svnui daemon start/stop` 之外的进程内控制）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Cmd {
    Ping,
    /// 强制重新全量扫描。
    Rescan,
    /// 优雅退出。
    Shutdown,
}

/// socket 路径。每个工作副本一个（用 root 路径哈希区分）。
pub fn socket_path(root: &std::path::Path) -> PathBuf {
    let name = format!("svnui-{}.sock", crate::cache::hash_root(root));
    runtime_dir().join(name)
}

/// pid 文件路径。
pub fn pid_path(root: &std::path::Path) -> PathBuf {
    let name = format!("svnui-{}.pid", crate::cache::hash_root(root));
    runtime_dir().join(name)
}

/// 单行帧的最大字节数。
///
/// 没有上限的话：一个写坏的客户端（或恶意进程）发一行 1GB 的 JSON，
/// daemon 会 `read_line` 到内存耗尽。查询请求真实大小只有几十字节，
/// 1MB 留了极大余量。
pub const MAX_FRAME: u64 = 1024 * 1024;

/// 校验 socket 文件的所有者是当前用户。
///
/// UDS 没有内置的认证机制，文件权限是唯一屏障。
/// 在 `/tmp` 这种可写目录里，**任何用户都能预先创建一个同名 socket**
/// 然后冒充 daemon —— 客户端连上去会收到伪造的状态，
/// 进而诱导用户提交/回滚错误的文件。
///
/// 返回 false 时必须拒绝连接并提示用户手动清理。
#[cfg(unix)]
pub fn socket_is_ours(sock: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(md) = std::fs::symlink_metadata(sock) else {
        return false;
    };
    // socket 类型都不对 → 肯定不是我们的
    if md.file_type().is_symlink() {
        return false;
    }
    md.uid() == libc_getuid()
}

/// 取当前 uid。不引入 libc 依赖：直接调系统调用。
#[cfg(unix)]
fn libc_getuid() -> u32 {
    // getuid 在 Linux/macOS 都不会失败，且 musl/glibc 都通过 libc 导出。
    // 这里用 `nix` 太重，用 `users` 又要多一个依赖 —— 直接 extern C 最省。
    extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

/// 运行时目录：`XDG_RUNTIME_DIR` → `/tmp`（回退）。
///
/// 不放在缓存目录里：socket 有 108 字节路径长度上限（sun_path），
/// 而缓存目录（尤其是 macOS 的 `~/Library/Caches`）可能很长。
fn runtime_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return p;
        }
    }
    std::env::temp_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_reply_roundtrip() {
        let q = Query {
            dir: "/repo/src".into(),
            since: Some(42),
        };
        let s = serde_json::to_string(&q).unwrap();
        let back: Query = serde_json::from_str(&s).unwrap();
        assert_eq!(back.dir, "/repo/src");
        assert_eq!(back.since, Some(42));
    }

    #[test]
    fn unchanged_reply_is_tiny() {
        let r = Reply::unchanged(7, true);
        let s = serde_json::to_string(&r).unwrap();
        assert!(!r.changed);
        assert!(r.map.is_empty());
        // 无变化响应必须是几十字节级别 —— 这是"每次切目录都问一次"能成立的前提
        assert!(s.len() < 120, "无变化响应过大: {s}");
    }

    #[test]
    fn reply_defaults_map_to_empty() {
        // 反序列化的老响应可能没有 map 字段，必须能容错
        let r: Reply = serde_json::from_str(r#"{"v":1,"changed":true,"ready":true}"#).unwrap();
        assert!(r.map.is_empty());
        assert_eq!(r.v, 1);
    }

    #[test]
    fn socket_and_pid_share_prefix() {
        let s = socket_path(std::path::Path::new("/repo"));
        let p = pid_path(std::path::Path::new("/repo"));
        assert!(s.to_string_lossy().contains("svnui-"));
        assert!(p.to_string_lossy().contains("svnui-"));
        assert!(s.to_string_lossy().ends_with(".sock"));
        assert!(p.to_string_lossy().ends_with(".pid"));
    }

    #[cfg(unix)]
    #[test]
    fn socket_path_respects_length_limit() {
        // sun_path 上限 108 字节。超了 bind() 会失败。
        let deep = "/".to_string() + &"very-long-directory-name/".repeat(6);
        let s = socket_path(std::path::Path::new(&deep));
        let len = s.to_string_lossy().len();
        assert!(len < 108, "socket 路径 {} 字节，超过 108 会 bind 失败", len);
    }
}
