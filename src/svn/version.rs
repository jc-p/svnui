use std::fmt;

/// svn 版本号。用于能力探测 —— 不同版本选项差异很大，低版本要能降级而不是直接报错。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SvnVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SvnVersion {
    /// 解析 `svn --version --quiet` 的输出，形如 `1.14.2` 或 `1.14.2 (r1899510)`。
    pub fn parse(text: &str) -> Option<Self> {
        let first = text.lines().next()?.trim();
        let num_part = first.split_whitespace().next()?;
        let mut it = num_part.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        let patch = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        Some(Self { major, minor, patch })
    }

    /// `svn cleanup --vacuum-pristines`（1.10+）。清理 .svn/pristine 里的垃圾。
    pub fn supports_vacuum_pristines(&self) -> bool {
        (self.major, self.minor) >= (1, 10)
    }

    /// `svn info --show-item`（1.9+）。
    pub fn supports_show_item(&self) -> bool {
        (self.major, self.minor) >= (1, 9)
    }

    /// `--password-from-stdin`（svn 1.12+）。
    ///
    /// 有了它就能避免把密码暴露在 `ps` 输出里。
    pub fn supports_password_from_stdin(&self) -> bool {
        (self.major, self.minor) >= (1, 12)
    }

    /// `svn patch`（1.7+）。
    pub fn supports_patch(&self) -> bool {
        (self.major, self.minor) >= (1, 7)
    }

    /// 工作副本格式 1.8+（`.svn` 单一 wc.db，现代 svn 都是）。
    pub fn is_modern(&self) -> bool {
        (self.major, self.minor) >= (1, 7)
    }
}

impl fmt::Display for SvnVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_and_suffixed() {
        assert_eq!(SvnVersion::parse("1.14.2\n"), Some(SvnVersion { major: 1, minor: 14, patch: 2 }));
        assert_eq!(
            SvnVersion::parse("1.14.2 (r1899510)\n"),
            Some(SvnVersion { major: 1, minor: 14, patch: 2 })
        );
    }

    #[test]
    fn parse_missing_patch_defaults_zero() {
        assert_eq!(SvnVersion::parse("1.9\n"), Some(SvnVersion { major: 1, minor: 9, patch: 0 }));
    }

    #[test]
    fn parse_garbage_returns_none() {
        assert_eq!(SvnVersion::parse(""), None);
        assert_eq!(SvnVersion::parse("not a version"), None);
    }

    #[test]
    fn feature_gates() {
        let v114 = SvnVersion { major: 1, minor: 14, patch: 0 };
        let v19 = SvnVersion { major: 1, minor: 9, patch: 0 };
        let v18 = SvnVersion { major: 1, minor: 8, patch: 0 };
        // --vacuum-pristines 是 1.10+
        assert!(v114.supports_vacuum_pristines());
        assert!(!v19.supports_vacuum_pristines());
        assert!(!v18.supports_vacuum_pristines());
        // --show-item 是 1.9+
        assert!(v114.supports_show_item());
        assert!(v19.supports_show_item());
        assert!(!v18.supports_show_item());
        assert!(v18.is_modern());
    }

    #[test]
    fn display_is_dotted() {
        assert_eq!(SvnVersion { major: 1, minor: 14, patch: 2 }.to_string(), "1.14.2");
    }
}
