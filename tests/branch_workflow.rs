//! 真正的分支工作流验收；所有写操作仅发生在本测试拥有的临时 file:// 仓库。

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use svnui::svn::workflow::{WorkflowOperation, WorkflowRequest};
use svnui::svn::Svn;

struct Fixture {
    temp: tempfile::TempDir,
    wc: PathBuf,
    url: String,
    svn: PathBuf,
    config: PathBuf,
}

fn successful(output: Output) -> String {
    assert!(
        output.status.success(),
        "exit={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let wc = temp.path().join("wc");
        let svn = svnui::svn::svn_exe().expect("需要 svn").clone();
        let svnadmin = std::env::var_os("SVNR_TEST_SVNADMIN").unwrap_or_else(|| "svnadmin".into());
        successful(
            Command::new(svnadmin)
                .arg("create")
                .arg(&repo)
                .output()
                .expect("需要 svnadmin"),
        );
        let url = format!("file://{}", repo.display());
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "").unwrap();
        let fixture = Self {
            temp,
            wc,
            url,
            svn,
            config,
        };
        fixture.native_at(
            fixture.temp.path(),
            &[
                "mkdir",
                &fixture.url("trunk"),
                &fixture.url("branches"),
                &fixture.url("tags"),
                "-m",
                "layout",
            ],
        );
        fixture.native_at(
            fixture.temp.path(),
            &[
                "checkout",
                &fixture.url("trunk"),
                fixture.wc.to_str().unwrap(),
            ],
        );
        std::fs::write(fixture.wc.join("a.txt"), "initial\n").unwrap();
        fixture.native(&["add", "a.txt"]);
        fixture.native(&["commit", "-m", "initial"]);
        fixture.native(&["update"]);
        fixture
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.url)
    }

    fn native_at(&self, cwd: &Path, args: &[&str]) -> String {
        successful(
            Command::new(&self.svn)
                .args(args)
                .arg("--non-interactive")
                .env("LC_ALL", "C.UTF-8")
                .current_dir(cwd)
                .output()
                .unwrap(),
        )
    }

    fn native(&self, args: &[&str]) -> String {
        self.native_at(&self.wc, args)
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_svnui"))
            .args(args)
            .current_dir(&self.wc)
            .env("SVNUI_CONFIG", &self.config)
            .env("SVNR_SVN", &self.svn)
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn youngest(&self) -> String {
        self.native(&["info", "--show-item", "revision", &self.url])
    }

    fn handle(&self) -> Svn {
        Svn::discover(&self.wc, Duration::from_secs(5), false).unwrap()
    }

    fn branch_change(&self) -> (String, String) {
        self.native(&[
            "copy",
            &self.url("trunk"),
            &self.url("branches/topic"),
            "-m",
            "branch",
        ]);
        let start = self.youngest().trim().to_owned();
        let branch_wc = self.temp.path().join("branch-wc");
        self.native_at(
            self.temp.path(),
            &[
                "checkout",
                &self.url("branches/topic"),
                branch_wc.to_str().unwrap(),
            ],
        );
        std::fs::write(branch_wc.join("a.txt"), "branch content\n").unwrap();
        std::fs::write(branch_wc.join("feature.txt"), "feature\n").unwrap();
        self.native_at(&branch_wc, &["add", "feature.txt"]);
        self.native_at(&branch_wc, &["commit", "-m", "feature"]);
        let end = self.youngest().trim().to_owned();
        self.native(&["update"]);
        (start, end)
    }
}

#[test]
fn workflow_arguments_are_checked_before_svn_discovery() {
    use clap::Parser;
    use svnui::cli::{Cli, Cmd};
    assert!(matches!(
        Cli::try_parse_from(["svnui", "branch", "^/branches/topic", "-m", "create"])
            .unwrap()
            .cmd,
        Some(Cmd::Branch { .. })
    ));
    assert!(Cli::try_parse_from(["svnui", "tag", "^/tags/v1"]).is_err());
    assert!(
        Cli::try_parse_from(["svnui", "merge-preview", "^/branches/topic", "--yes"]).is_err(),
        "预演不能通过 --yes 升级为写操作"
    );
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn branch_and_tag_copy_repository_history_without_local_edits() {
    let fixture = Fixture::new();
    std::fs::write(fixture.wc.join("a.txt"), "uncommitted\n").unwrap();
    let output = successful(fixture.cli(&[
        "--json",
        "branch",
        "^/branches/topic@user",
        "-m",
        "branch",
        "--yes",
    ]));
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["plan"]["revision"], 2);
    assert_eq!(
        fixture.native(&[
            "cat",
            &format!("{}@HEAD", fixture.url("branches/topic@user/a.txt"))
        ]),
        "initial\n"
    );
    let log = fixture.native(&[
        "log",
        "--xml",
        "-v",
        "--stop-on-copy",
        &format!("{}@HEAD", fixture.url("branches/topic@user")),
    ]);
    assert!(log.contains("copyfrom-path=\"/trunk\""));
    assert!(log.contains("copyfrom-rev=\"2\""));
    successful(fixture.cli(&["tag", "^/tags/v1", "-m", "release", "--yes"]));
    assert_eq!(
        fixture.native(&["cat", &fixture.url("tags/v1/a.txt")]),
        "initial\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.wc.join("a.txt")).unwrap(),
        "uncommitted\n"
    );
    assert_eq!(
        fixture.native(&["info", "--show-item", "url"]).trim(),
        fixture.url("trunk")
    );
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn copy_requires_confirmation_and_dry_run_does_not_commit() {
    let fixture = Fixture::new();
    let youngest = fixture.youngest();
    let denied = fixture.cli(&["branch", "^/branches/topic", "-m", "branch"]);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("--yes"));
    successful(fixture.cli(&[
        "branch",
        "^/branches/topic",
        "-m",
        "branch",
        "--dry-run",
        "--yes",
    ]));
    assert_eq!(fixture.youngest(), youngest);
    assert!(fixture
        .native(&["list", &fixture.url("branches")])
        .is_empty());
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn duplicate_encoded_destination_is_rejected_instead_of_nesting() {
    let fixture = Fixture::new();
    successful(fixture.cli(&[
        "branch",
        "^/branches/%E6%B5%8B%E8%AF%95",
        "-m",
        "branch",
        "--yes",
    ]));
    let youngest = fixture.youngest();
    let duplicate = fixture.cli(&["branch", "^/branches/测试", "-m", "duplicate", "--yes"]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("目标已存在"));
    assert_eq!(fixture.youngest(), youngest);
    assert_eq!(
        fixture.native(&["list", &fixture.url("branches/测试")]),
        "a.txt\n"
    );
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn switch_changes_url_and_contents_only_after_explicit_confirmation() {
    let fixture = Fixture::new();
    fixture.branch_change();
    successful(fixture.cli(&["switch", "^/branches/topic", "--dry-run"]));
    assert!(!fixture
        .cli(&["switch", "^/branches/topic"])
        .status
        .success());
    assert_eq!(
        fixture.native(&["info", "--show-item", "url"]).trim(),
        fixture.url("trunk")
    );
    successful(fixture.cli(&["switch", "^/branches/topic", "--yes"]));
    assert_eq!(
        fixture.native(&["info", "--show-item", "url"]).trim(),
        fixture.url("branches/topic")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.wc.join("a.txt")).unwrap(),
        "branch content\n"
    );
    assert!(fixture.wc.join("feature.txt").exists());
    assert!(fixture.native(&["status"]).is_empty());
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn switch_refuses_content_properties_and_unversioned_files_even_with_yes() {
    let fixture = Fixture::new();
    fixture.branch_change();
    std::fs::write(fixture.wc.join("a.txt"), "keep me\n").unwrap();
    assert!(!fixture
        .cli(&["switch", "^/branches/topic", "--yes"])
        .status
        .success());
    assert_eq!(
        std::fs::read_to_string(fixture.wc.join("a.txt")).unwrap(),
        "keep me\n"
    );
    std::fs::write(fixture.wc.join("a.txt"), "initial\n").unwrap();
    fixture.native(&["propset", "workflow:test", "local", "a.txt"]);
    assert!(!fixture
        .cli(&["switch", "^/branches/topic", "--yes"])
        .status
        .success());
    fixture.native(&["propdel", "workflow:test", "a.txt"]);
    std::fs::write(fixture.wc.join("untracked.txt"), "preserve\n").unwrap();
    assert!(!fixture
        .cli(&["switch", "^/branches/topic", "--yes"])
        .status
        .success());
    assert!(fixture.wc.join("untracked.txt").exists());
    assert_eq!(
        fixture.native(&["info", "--show-item", "url"]).trim(),
        fixture.url("trunk")
    );
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn merge_preview_reports_changes_without_touching_files_or_mergeinfo() {
    let fixture = Fixture::new();
    let (start, end) = fixture.branch_change();
    let revision = fixture.youngest();
    let before_status = fixture.native(&["status", "--xml"]);
    let before_props = fixture.native(&["proplist", "--xml", "-v", "."]);
    for args in [
        vec!["merge-preview", "^/branches/topic"],
        vec![
            "merge-preview",
            "^/branches/topic",
            "-r",
            &format!("{start}:{end}"),
        ],
    ] {
        let output = successful(fixture.cli(&args));
        assert!(output.contains("feature.txt"), "{output}");
        assert!(output.contains("未应用改动"));
        assert_eq!(fixture.native(&["status", "--xml"]), before_status);
        assert_eq!(
            fixture.native(&["proplist", "--xml", "-v", "."]),
            before_props
        );
        assert_eq!(
            std::fs::read_to_string(fixture.wc.join("a.txt")).unwrap(),
            "initial\n"
        );
        assert!(!fixture.wc.join("feature.txt").exists());
        assert_eq!(fixture.youngest(), revision);
    }
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn merge_preview_reports_conflicts_as_json_without_leaving_conflict_files() {
    let fixture = Fixture::new();
    fixture.branch_change();
    std::fs::write(fixture.wc.join("a.txt"), "trunk content\n").unwrap();
    fixture.native(&["commit", "-m", "conflicting trunk change"]);
    fixture.native(&["update"]);
    let before_status = fixture.native(&["status", "--xml"]);
    let before_props = fixture.native(&["proplist", "--xml", "-v", "."]);
    let output = successful(fixture.cli(&["--json", "merge-preview", "^/branches/topic"]));
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["dry_run"], true);
    let transcript = json["data"]["output"].as_str().unwrap();
    assert!(
        transcript
            .lines()
            .any(|line| line.starts_with('C') && line.contains("a.txt")),
        "{transcript}"
    );
    assert_eq!(fixture.native(&["status", "--xml"]), before_status);
    assert_eq!(
        fixture.native(&["proplist", "--xml", "-v", "."]),
        before_props
    );
    assert_eq!(
        std::fs::read_to_string(fixture.wc.join("a.txt")).unwrap(),
        "trunk content\n"
    );
    let mut names = std::fs::read_dir(&fixture.wc)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, [".svn", "a.txt"]);
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn confirmed_plan_rechecks_local_changes_and_pins_head() {
    let fixture = Fixture::new();
    fixture.branch_change();
    let svn = fixture.handle();
    let plan = svn
        .prepare_workflow(&WorkflowRequest {
            operation: WorkflowOperation::Switch,
            url: "^/branches/topic".into(),
            revision: None,
            message: String::new(),
        })
        .unwrap();
    std::fs::write(fixture.wc.join("a.txt"), "edited after confirmation\n").unwrap();
    assert!(plan.execute(&svn).is_err());
    let plan = svn
        .prepare_workflow(&WorkflowRequest {
            operation: WorkflowOperation::Tag,
            url: "^/tags/pinned".into(),
            revision: Some("HEAD".into()),
            message: "pin HEAD".into(),
        })
        .unwrap();
    fixture.native(&["commit", "-m", "new HEAD"]);
    plan.execute(&svn).unwrap();
    assert_eq!(
        fixture.native(&["cat", &fixture.url("tags/pinned/a.txt")]),
        "initial\n"
    );

    let duplicate = svn
        .prepare_workflow(&WorkflowRequest {
            operation: WorkflowOperation::Tag,
            url: "^/tags/raced".into(),
            revision: None,
            message: "prepared earlier".into(),
        })
        .unwrap();
    fixture.native(&[
        "copy",
        &fixture.url("trunk"),
        &fixture.url("tags/raced"),
        "-m",
        "created meanwhile",
    ]);
    let youngest = fixture.youngest();
    assert!(duplicate.execute(&svn).is_err());
    assert_eq!(fixture.youngest(), youngest);
    assert_eq!(
        fixture.native(&["list", &fixture.url("tags/raced")]),
        "a.txt\n"
    );
}

#[test]
#[ignore = "需要 svn + svnadmin；仅操作临时本地仓库"]
fn invalid_urls_and_revisions_cannot_create_repository_changes() {
    let fixture = Fixture::new();
    let youngest = fixture.youngest();
    for destination in [
        "--force",
        "^/../escape",
        "^/branches/%2e%2e/escape",
        "^/branches/a%2fb",
        "^/branches/a%00b",
        "^/branches/a?query",
        "https://example.invalid/foreign",
    ] {
        assert!(
            !fixture
                .cli(&["branch", destination, "-m", "invalid", "--yes"])
                .status
                .success(),
            "{destination}"
        );
    }
    for revision in ["-1", "1:2", "HEAD --force", "18446744073709551616"] {
        assert!(!fixture
            .cli(&[
                "tag",
                "^/tags/invalid",
                "-m",
                "invalid",
                &format!("--revision={revision}"),
                "--yes"
            ])
            .status
            .success());
    }
    for range in ["1", "HEAD:2", "1:2:3", "-1:2"] {
        assert!(!fixture
            .cli(&["merge-preview", "^/trunk", &format!("--revision={range}")])
            .status
            .success());
    }
    assert_eq!(fixture.youngest(), youngest);
    assert!(fixture.native(&["status"]).is_empty());
}
