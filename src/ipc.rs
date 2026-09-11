//! daemon 客户端与查询入口。
//!
//! 设计目标：把「换目录看状态」这件事从 **每次跑一次 `svn status`**
//! 变成 **一次 UDS 往返**。
//!
//! # 降级策略
//!
//! daemon 不可用（没启动、socket 文件残留、非 unix 平台）时，**静默退回直连 svn**。
//! 调用方拿到的结果结构完全一致 —— daemon 只是加速器，不是依赖。
//! 这条很重要：daemon 挂了不应该让 `svnui` 不可用。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::{Error, Result, Snapshot};
use crate::svn::Svn;

pub use crate::daemon::protocol::{socket_path, pid_path};

// 以下是 unix-only：非 unix 下 daemon 整个不可用（见 `cfg_if_unix` 的 no-op 分支），
// 放外面会触发 unused import 警告。
#[cfg(unix)]
use crate::daemon::protocol::{Query, Reply};
#[cfg(unix)]
use std::time::Duration;

/// 查询结果。与 daemon 是否运行无关，调用方不用区分。
#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub root: String,
    /// 版本戳。daemon 未运行时恒为 0（客户端据此放弃增量判断）。
    pub v: u64,
    /// 分支名。
    pub branch: Option<String>,
    /// 绝对路径 → porcelain 两字符。
    pub map: HashMap<String, String>,
    /// 全量扫描是否已完成。
    pub ready: bool,
    /// **状态是否变化过**。false 表示"与 since 一致，map 为空"。
    ///
    /// 调用方必须据此判断：map 为空 + changed=false = 没有变化，
    /// **不能**拿空 map 去覆盖本地缓存 —— 那会让所有状态凭空消失。
    pub changed: bool,
    /// 本次是否走了 daemon。
    pub from_daemon: bool,
}

/// 查询某目录层的状态。优先走 daemon，失败降级直连。
///
/// 这是 `svnui q` 与 yazi 插件共用的入口。
pub fn query(svn: &Svn, dir: &Path, since: Option<u64>) -> Result<Layer> {
    // ⚠️ macOS 陷阱：`/tmp`、`/var`、`/etc` 都是 `/private/xxx` 的**软链接**。
    //    yazi 给的 cwd 是软链形式（`/tmp/...`），而 svn 探测出的 root 是
    //    canonicalize 过的（`/private/tmp/...`）。两者 strip_prefix 会失败，
    //    静默退化成"查根层" —— 在子目录里就查错层了。
    //    所以这里先 canonicalize 再往下传。
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());

    match try_daemon(&svn.root, &dir, since) {
        Ok(Some(layer)) => Ok(layer),
        Ok(None) => direct(svn, &dir),
        Err(_) => direct(svn, &dir),
    }
}

/// 尝试连接 daemon。返回 `Ok(None)` 表示 daemon 不可用，应降级。
fn try_daemon(root: &Path, dir: &Path, since: Option<u64>) -> Result<Option<Layer>> {
    cfg_if_unix(root, dir, since)
}

/// unix 实现：UDS + 换行分隔的 JSON。
#[cfg(unix)]
fn cfg_if_unix(root: &Path, dir: &Path, since: Option<u64>) -> Result<Option<Layer>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let sock = socket_path(root);
    if !sock.exists() {
        return Ok(None);
    }

    // socket 文件存在但不属于当前用户 → 可能是别人在 /tmp 里预置的冒名 socket。
    // 连上去会拿到伪造的状态，进而诱导误提交/误回滚。宁可降级直连 svn。
    if !crate::daemon::protocol::socket_is_ours(&sock) {
        return Ok(None);
    }

    let mut stream = match UnixStream::connect(&sock) {
        Ok(s) => s,
        // socket 文件在但连不上 = 上次 daemon 异常退出留下的残骸
        Err(_) => return Ok(None),
    };
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    let q = Query { dir: dir.to_string_lossy().to_string(), since };
    let mut payload = serde_json::to_string(&q).map_err(|e| Error::Parse(e.to_string()))?;
    payload.push('\n');
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;

    let mut line = String::new();
    let mut reader = BufReader::new(&stream);
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }

    let reply: Reply = serde_json::from_str(line.trim()).map_err(|e| Error::Parse(e.to_string()))?;
    Ok(Some(Layer {
        root: reply.root.unwrap_or_else(|| root.to_string_lossy().to_string()),
        v: reply.v,
        branch: reply.branch,
        map: reply.map,
        ready: reply.ready,
        changed: reply.changed,
        from_daemon: true,
    }))
}

/// 非 unix 平台：daemon 不支持，一律降级。
#[cfg(not(unix))]
fn cfg_if_unix(_root: &Path, _dir: &Path, _since: Option<u64>) -> Result<Option<Layer>> {
    Ok(None)
}

/// 降级路径：直接跑 `svn status` 并组装成目录层。
///
/// 与 daemon 路径返回**同样结构**的 `Layer`，只有 `v` 恒为 0
/// （客户端据此知道没法做增量判断）。
fn direct(svn: &Svn, dir: &Path) -> Result<Layer> {
    let entries = svn.status(&Default::default())?;

    let mut snap = Snapshot::new(svn.root.to_string_lossy().to_string());
    snap.ready = true;
    for e in entries {
        snap.insert(e);
    }
    snap.build_bubbles();

    // ⚠️ 必须先绑定到具名变量再取 &str。
    // `normalize_dir` 返回借用自入参的 &str，若入参是 `.unwrap_or_default()`
    // 产生的临时 String，它会在这一行结束时被 drop —— E0716，编译不过。
    let rel = dir
        .strip_prefix(&svn.root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let rel_dir = crate::domain::normalize_dir(&rel);
    let layer = snap.layer(rel_dir);

    let mut map = HashMap::new();
    let dir_abs = PathBuf::from(rel_dir);
    for (name, mark) in layer.files {
        let abs = svn.root.join(&dir_abs).join(&name);
        map.insert(abs.to_string_lossy().to_string(), xy_of(mark.sign));
    }
    for (name, mark) in layer.dirs {
        let abs = svn.root.join(&dir_abs).join(&name);
        map.insert(abs.to_string_lossy().to_string(), xy_of(mark.sign));
    }

    Ok(Layer {
        root: svn.root.to_string_lossy().to_string(),
        v: 0,
        branch: svn.info().ok().and_then(|i| i.branch_name()),
        map,
        ready: true,
        changed: true,
        from_daemon: false,
    })
}

/// 单个符号 → porcelain 两字符。yazi 侧按两字符解析。
fn xy_of(sign: char) -> String {
    if sign == ' ' || sign == '\0' {
        "__".to_string()
    } else if sign == 'T' {
        // 树冲突是第 7 列， porcelain 里放在第一位
        "T_".to_string()
    } else {
        format!("{sign}_")
    }
}
