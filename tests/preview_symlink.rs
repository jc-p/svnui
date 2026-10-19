//! 只读预览须沿用 SVN 节点语义，不能把受管 symlink 跟随到工作副本外再拒绝。
#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use svnui::svn::command::CancelToken;
use svnui::svn::Svn;

fn checked(tool: &str, args: &[&str], cwd: &Path) {
    let output = Command::new(tool)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("cannot launch {tool}: {error}"));
    assert!(
        output.status.success(),
        "{tool} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "需要 svn + svnadmin；只创建临时仓库和外指 symlink"]
fn versioned_symlink_pointing_outside_working_copy_has_same_diff_in_preview() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let repo = root.join("repo");
    let wc = root.join("wc");
    checked("svnadmin", &["create", repo.to_str().unwrap()], root);
    let url = format!("file://{}", repo.display());
    checked(
        "svn",
        &["checkout", "--non-interactive", &url, wc.to_str().unwrap()],
        root,
    );
    let first_target = root.join("outside-one.txt");
    let second_target = root.join("outside-two.txt");
    std::fs::write(&first_target, "first outside content\n").unwrap();
    std::fs::write(&second_target, "second outside content\n").unwrap();
    let link = wc.join("tracked-link");
    symlink(&first_target, &link).unwrap();
    checked("svn", &["add", "--non-interactive", "tracked-link"], &wc);
    checked(
        "svn",
        &["commit", "--non-interactive", "-m", "add tracked symlink"],
        &wc,
    );
    std::fs::remove_file(&link).unwrap();
    symlink(&second_target, &link).unwrap();

    let svn = Svn::discover(&wc, Duration::from_secs(5), false).unwrap();
    let paths = [link];
    let complete = svn.diff(&paths, None, false).unwrap();
    assert!(complete.stdout.contains("tracked-link"));
    assert!(complete.stdout.contains("outside-one.txt"));
    assert!(complete.stdout.contains("outside-two.txt"));
    let preview = svn.diff_preview(&paths, &CancelToken::default()).unwrap();
    assert_eq!(preview.output.stdout, complete.stdout);
    assert!(!preview.stdout_truncated);
    // 回归测试不能意外改写符号链接的外部目标。
    assert_eq!(
        std::fs::read_to_string(&first_target).unwrap(),
        "first outside content\n"
    );
    assert_eq!(
        std::fs::read_to_string(&second_target).unwrap(),
        "second outside content\n"
    );
}
