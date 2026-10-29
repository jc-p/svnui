//! 独立预览的命令契约：参数解析、工作目录和 SVN 探测隔离。
#![cfg(feature = "tui")]

use std::process::{Command, Output};

fn preview(args: &[&str]) -> Output {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    std::fs::write(&config, "").unwrap();
    Command::new(env!("CARGO_BIN_EXE_svnui"))
        .current_dir(temp.path())
        .env("SVNUI_CONFIG", config)
        .env("SVNR_SVN", temp.path().join("no-svn-installed"))
        .env("PATH", temp.path())
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn preview_help_and_required_path_are_available_outside_working_copy() {
    let help = preview(&["preview", "--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("<PATH>"));
    let missing = preview(&["preview"]);
    assert_eq!(missing.status.code(), Some(2));
}

#[test]
fn missing_preview_file_reports_io_without_discovering_svn() {
    let output = preview(&["--json", "preview", "missing.txt"]);
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["ok"], false);
    assert_eq!(error["error"]["kind"], "io");
    assert!(!output.stdout.contains(&0x1b), "读取失败不得切换终端模式");
}

#[test]
fn svn_urls_request_svn_instead_of_being_read_as_local_files() {
    for url in [
        "https://example.invalid/repo/A8%e5%a4%96/",
        "http://example.invalid/repo/",
        "svn://example.invalid/repo/",
        "svn+ssh://example.invalid/repo/",
        "file:///missing-repository/",
    ] {
        let output = preview(&["--json", "preview", url]);
        assert_eq!(output.status.code(), Some(4), "{url}");
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["kind"], "svn-not-found", "{url}");
    }
}
