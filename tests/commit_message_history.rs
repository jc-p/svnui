//! 只在临时 file:// 仓库验证本人提交说明，不访问真实账号或远端仓库。

use std::{path::Path, process::Command, time::Duration};
use svnui::svn::Svn;

fn run(command: &str, args: &[&str], cwd: &Path) {
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn author_matching_is_exact_and_pagination_reaches_older_personal_messages() {
    let temp = tempfile::tempdir().unwrap();
    run("svnadmin", &["create", "repo"], temp.path());
    let url = format!("file://{}/repo", temp.path().display());
    run(
        "svn",
        &["checkout", "--non-interactive", &url, "wc"],
        temp.path(),
    );
    let wc = temp.path().join("wc");
    for (index, (author, message)) in [
        ("alice", "SELF_OLDER\n自己以前写的多行说明"),
        ("bob", "alice 提到的说明，实际作者是 bob"),
        ("Alice", "大小写不同的另一位作者"),
        ("alice", "SELF_NEWER"),
        ("bob", "最新提交提到 alice，也不能当作本人记录"),
        ("domain\\alice", "带域名前缀的作者"),
        ("name*?[]", "带通配符字符的作者"),
        ("张三", "中文作者"),
    ]
    .iter()
    .enumerate()
    {
        std::fs::write(wc.join("sample.txt"), format!("revision {index}\n")).unwrap();
        if index == 0 {
            run("svn", &["add", "sample.txt"], &wc);
        }
        run(
            "svn",
            &[
                "commit",
                "--non-interactive",
                "--username",
                author,
                "-m",
                message,
            ],
            &wc,
        );
    }
    let svn = Svn::discover(&wc, Duration::from_secs(5), false).unwrap();
    let mut before = None;
    let mut entries = Vec::new();
    for _ in 0..10 {
        let page = svn.log_by_author("alice", before, 1).unwrap();
        assert!(page.scanned <= 1);
        assert!(page.entries.iter().all(|entry| entry.author == "alice"));
        entries.extend(page.entries);
        before = page.next_before;
        if before.is_none() {
            break;
        }
    }
    assert!(before.is_none(), "游标必须继续推进到末尾");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.revision)
            .collect::<Vec<_>>(),
        vec![4, 1]
    );
    assert_eq!(entries[1].msg, "SELF_OLDER\n自己以前写的多行说明");
    assert!(svn.log_by_author(" ", None, 100).is_err());
    for author in ["domain\\alice", "name*?[]", "张三"] {
        let page = svn.log_by_author(author, None, 100).unwrap();
        assert_eq!(
            page.entries.len(),
            1,
            "{author} 必须按字面用户名查到本人记录"
        );
        assert_eq!(page.entries[0].author, author);
    }
}
