use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static SVN_EXE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 找 svn 可执行文件。
///
/// 优先级：环境变量 `SVNR_SVN` → `PATH` 查找。结果缓存（进程内只找一次）。
/// 返回 `None` 时，上层应提示安装 Subversion 命令行工具。
pub fn svn_exe() -> Option<&'static PathBuf> {
    SVN_EXE.get_or_init(|| {
        if let Ok(p) = env::var("SVNR_SVN") {
            let p = PathBuf::from(p);
            if p.is_file() {
                return Some(p);
            }
        }
        which("svn")
    })
    .as_ref()
}

/// 强制忽略缓存重新查找（测试或 `SVNR_SVN` 变更时用）。
pub fn svn_exe_forced() -> Option<PathBuf> {
    which("svn")
}

fn which(name: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };

    for dir in env::split_paths(&paths) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let cand = dir.join(&exe_name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// 从 `start` 向上找 `.svn`，定位工作副本根。
///
/// 走到文件系统根还没有就返回 `None` —— 此时上层应报 `NotWorkingCopy`。
pub fn wc_root(start: &Path) -> Option<PathBuf> {
    // 先规范化，避免 `a/../b` 这类路径导致重复遍历。
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());

    let mut cur: Option<&Path> = Some(start.as_path());
    while let Some(p) = cur {
        if p.join(".svn").exists() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

/// 给定路径相对于工作副本根的相对路径。供 Snapshot 的 key 使用。
pub fn relative_to(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).map(|p| p.to_path_buf()).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wc_root_finds_ancestor() {
        let tmp = std::env::temp_dir().join(format!("svnui-root-{}", std::process::id()));
        let deep = tmp.join("a/b/c");
        let _ = std::fs::create_dir_all(&deep);
        let _ = std::fs::create_dir_all(tmp.join(".svn"));

        let found = wc_root(&deep);
        let expected = tmp.canonicalize().ok();
        assert!(found.is_some(), "应找到 .svn");
        if let (Some(f), Some(e)) = (found, expected) {
            assert_eq!(f.canonicalize().ok(), Some(e));
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn which_finds_binary_on_path() {
        // sh 在任何 unix 上都存在，用它验证 PATH 扫描逻辑本身是对的。
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }

    #[test]
    fn relative_to_strips_root() {
        let r = Path::new("/repo");
        let p = Path::new("/repo/src/main.rs");
        assert_eq!(relative_to(r, p), PathBuf::from("src/main.rs"));
    }
}
