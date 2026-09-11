use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// `svn info` 的关键字段。yazi 状态栏显示分支、根 URL 用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    /// 工作副本根（绝对路径）。
    pub root: Utf8PathBuf,
    pub url: String,
    /// `^/trunk` 这种相对 URL。老版本 svn 可能没有。
    pub relative_url: Option<String>,
    pub revision: u64,
    pub repository_root: Option<String>,
    pub uuid: Option<String>,
}

impl RepoInfo {
    /// 从 URL 里猜分支名：trunk / branches\<x\> / tags\<x\>。
    ///
    /// 猜不出来返回 `None` —— 别硬凑一个错的显示在状态栏。
    pub fn branch_name(&self) -> Option<String> {
        let path = self.relative_url.as_deref().unwrap_or(&self.url);
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        for (i, s) in segs.iter().enumerate() {
            match *s {
                "trunk" => return Some("trunk".to_string()),
                "branches" | "tags" => {
                    if let Some(next) = segs.get(i + 1) {
                        return Some((*next).to_string());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// 是否位于 trunk（很多团队的提交策略依赖这个判断）。
    pub fn is_trunk(&self) -> bool {
        self.branch_name().as_deref() == Some("trunk")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(rel: &str, url: &str) -> RepoInfo {
        RepoInfo {
            root: Utf8PathBuf::from("/repo"),
            url: url.into(),
            relative_url: Some(rel.into()),
            revision: 100,
            repository_root: Some("svn://x/repo".into()),
            uuid: None,
        }
    }

    #[test]
    fn detect_trunk_and_branches() {
        assert_eq!(info("^/trunk", "svn://x/repo/trunk").branch_name().as_deref(), Some("trunk"));
        assert_eq!(
            info("^/branches/feature-x", "svn://x/repo/branches/feature-x").branch_name().as_deref(),
            Some("feature-x")
        );
        assert_eq!(info("^/tags/v1.0", "svn://x/repo/tags/v1.0").branch_name().as_deref(), Some("v1.0"));
    }

    #[test]
    fn falls_back_to_url_when_no_relative() {
        let mut i = info("^/trunk", "svn://x/repo/branches/y");
        i.relative_url = None;
        assert_eq!(i.branch_name().as_deref(), Some("y"));
    }

    #[test]
    fn unknown_layout_gives_none() {
        assert_eq!(info("^/", "svn://x/repo").branch_name(), None);
    }
}
