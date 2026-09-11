//! 错误 → 人类提示。**每个错误都要给出"下一步该干什么"**，
//! 而不是复述错误本身 —— 那对用户没有增量信息。

use crate::domain::Error;

pub fn human_hint(e: &Error) -> String {
    match e {
        Error::SvnNotFound => {
            "找不到 svn 命令。\n  macOS:  brew install subversion\n  Ubuntu: sudo apt install subversion\n  或设置 SVNR_SVN=/path/to/svn".to_string()
        }
        Error::NotWorkingCopy(p) => {
            format!("{} 不在 SVN 工作副本内。\n  跑 `svnui probe` 看看找到了什么。", p.display())
        }
        Error::Locked(p) => format!(
            "工作副本被锁：{}\n  多半是上次操作被中断。先跑：svnui cleanup\n  （仅解锁，不会删你的文件）",
            p.display()
        ),
        Error::Timeout(s) => format!(
            "svn 超时（{s}s）。\n  仓库很大？试试 --timeout 120。\n  也可能是在等凭据：先手动跑一次 svn info 完成认证。"
        ),
        Error::SvnFailed { code, stderr } => {
            let mut s = format!("svn 失败（退出码 {code}）\n{stderr}");
            // E155004 有时没被 is_lock_error 抓到（文案随版本变化），这里兜个底。
            if stderr.contains("E155004") {
                s.push_str("\n  这是工作副本锁，先跑：svnui cleanup");
            }
            s
        }
        Error::Parse(msg) => format!("{msg}"),
        _ => format!("{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn every_error_gets_actionable_hint() {
        // 提示里必须含"该跑什么命令"，否则等于没提示。
        let cases = vec![
            (Error::SvnNotFound, "install"),
            (Error::NotWorkingCopy(PathBuf::from("/x")), "probe"),
            (Error::Locked(PathBuf::from("/x")), "cleanup"),
            (Error::Timeout(30), "--timeout"),
            (Error::SvnFailed { code: 1, stderr: "svn: E155004 lock".into() }, "cleanup"),
        ];
        for (e, kw) in cases {
            let h = human_hint(&e);
            assert!(h.contains(kw), "{e:?} 的提示里应含 `{kw}`，实际：{h}");
        }
    }

    #[test]
    fn locked_hint_reassures_no_data_loss() {
        let h = human_hint(&Error::Locked(PathBuf::from("/x")));
        assert!(h.contains("不会删"), "锁的提示必须安抚用户（cleanup 只解锁不删文件）");
    }
}
