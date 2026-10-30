//! LLM 配置与协议回归；仅连接一次性 loopback 替身，不访问真实模型。
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::time::Duration;

use svnui::config::{Config, LlmConfig, LlmProvider};

fn serve(status: u16, reply: &str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let reply = reply.to_string();
    let thread = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "模型请求未到达替身");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 2048];
            let count = stream.read(&mut buf).unwrap();
            assert!(count > 0, "请求尚未完整即关闭");
            bytes.extend_from_slice(&buf[..count]);
            assert!(bytes.len() < 65536, "替身只接受小请求");
            if let Some(split) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..split]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .expect("JSON 请求应携带 Content-Length");
                if bytes.len() >= split + 4 + length {
                    break;
                }
            }
        }
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (endpoint, thread)
}

fn cli(config: &str, args: &[&str], key: Option<&str>) -> Output {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, config).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_svnui"));
    command
        .env("SVNUI_CONFIG", &path)
        .env_remove("SVNUI_LLM_TEST_KEY")
        .args(args);
    if let Some(key) = key {
        command.env("SVNUI_LLM_TEST_KEY", key);
    }
    let output = command.output().unwrap();
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        config,
        "自检或示例不得修改配置"
    );
    output
}

#[test]
fn presets_resolve_defaults_and_legacy_overrides_remain_effective() {
    for provider in [
        LlmProvider::Codex,
        LlmProvider::Zhipu,
        LlmProvider::Ollama,
        LlmProvider::Deepseek,
    ] {
        let example = LlmConfig::preset_example(provider);
        let cfg: Config = toml::from_str(&example).unwrap();
        assert_eq!(cfg.llm.provider, provider);
        assert!(cfg.llm.enabled);
        assert!(!cfg.llm.send_diff);
        assert!(!cfg.llm.endpoint().is_empty());
        assert!(!cfg.llm.model_name().is_empty());
    }
    let legacy: Config =
        toml::from_str("[llm]\nbase_url='https://gateway.example/v1'\nmodel='custom-model'\n")
            .unwrap();
    assert_eq!(legacy.llm.endpoint(), "https://gateway.example/v1");
    assert_eq!(legacy.llm.model_name(), "custom-model");
    assert!(legacy.llm.requires_key());
    let override_cfg: Config =
        toml::from_str("[llm]\nprovider='ollama'\nmodel='another-local-model'\n").unwrap();
    assert_eq!(override_cfg.llm.model_name(), "another-local-model");
    assert!(!override_cfg.llm.requires_key());
}

#[test]
fn codex_preset_inherits_auth_and_ignores_legacy_http_overrides() {
    let example = LlmConfig::preset_example(LlmProvider::Codex);
    let config: Config = toml::from_str(&example).unwrap();
    assert_eq!(config.llm.codex_bin, "codex");
    assert_eq!(config.llm.timeout_secs, 90);
    assert!(!config.llm.requires_key());
    let mut cfg = config.llm;
    cfg.base_url = "https://legacy.example/v1".into();
    cfg.model = "legacy-model".into();
    assert!(!cfg.endpoint().contains("legacy"));
    assert!(!cfg.model_name().contains("legacy"));
    let report = svnui::llm::describe(&cfg);
    assert!(report.contains("CODEX_HOME"));
    assert!(!report.contains("legacy"));
    let output = cli("", &["llm", "--preset", "codex"], None);
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("provider = \"codex\""));
}

#[test]
fn local_generation_needs_no_key_and_preserves_chat_protocol() {
    let reply = r#"{"choices":[{"message":{"content":"fix(config): 完善配置体验"}}]}"#;
    let (url, request) = serve(200, reply);
    let cfg = LlmConfig {
        enabled: true,
        provider: LlmProvider::Ollama,
        base_url: url,
        api_key_env: "SVNUI_LLM_TEST_KEY_UNSET".into(),
        ..LlmConfig::default()
    };
    let result =
        svnui::llm::suggest(&[("example.rs".into(), 'M')], Some("private diff"), &cfg).unwrap();
    assert_eq!(result, vec!["fix(config): 完善配置体验"]);
    let sent = request.join().unwrap();
    assert!(sent.starts_with("POST /v1/chat/completions "));
    assert!(!sent.to_ascii_lowercase().contains("authorization:"));
    assert!(sent.contains("qwen2.5-coder:3b"));
    assert!(!sent.contains("private diff"));
}

#[test]
fn cloud_preflight_identifies_missing_key_without_requesting_network() {
    let output = cli(
        "[llm]\nenabled=true\nprovider='zhipu'\napi_key_env='SVNUI_LLM_TEST_KEY'\n",
        &["llm", "--test"],
        None,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("SVNUI_LLM_TEST_KEY"));
}

#[test]
fn cloud_ping_uses_bearer_and_disables_flash_thinking() {
    let (url, request) = serve(200, r#"{"choices":[{"message":{"content":"OK"}}]}"#);
    let config = format!("[llm]\nenabled=true\nprovider='zhipu'\nbase_url={url:?}\napi_key_env='SVNUI_LLM_TEST_KEY'\n");
    let output = cli(&config, &["llm", "--test"], Some("dummy-test-credential"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sent = request.join().unwrap();
    let (_, body) = sent.split_once("\r\n\r\n").unwrap();
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    assert!(sent.contains("Bearer dummy-test-credential"));
    assert_eq!(body["model"], "glm-4.7-flash");
    assert_eq!(body["thinking"]["type"], "disabled");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("dummy-test-credential"));
}

#[test]
fn failed_ping_returns_nonzero_with_actionable_errors_and_no_response_echo() {
    for (status, expected) in [(401, "认证失败"), (429, "限流"), (404, "模型不存在")] {
        let (url, request) = serve(
            status,
            r#"{"error":{"message":"sensitive-provider-detail"}}"#,
        );
        let config = format!("[llm]\nenabled=true\nprovider='ollama'\nbase_url={url:?}\n");
        let output = cli(&config, &["llm", "--test"], None);
        request.join().unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("sensitive-provider-detail"));
    }
}

#[test]
fn invalid_config_is_reported_and_presets_can_still_be_printed() {
    let invalid = "[llm]\nprovider='typo'\nextra_prompt='sensitive-test-content'\n";
    let output = cli(invalid, &["llm"], None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("配置文件"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("sensitive-test-content"));
    let output = cli(invalid, &["llm", "--preset", "ollama"], None);
    assert!(output.status.success());
    let example = String::from_utf8(output.stdout).unwrap();
    let cfg: Config = toml::from_str(&example).unwrap();
    assert_eq!(cfg.llm.provider, LlmProvider::Ollama);
    assert_eq!(cfg.llm.timeout_secs, 60);
    assert!(!cli("", &["llm", "--preset", "ollama", "--test"], None)
        .status
        .success());
    assert!(!cli("", &["llm", "--preset", "unknown"], None)
        .status
        .success());
}

#[test]
fn presets_need_no_svn_and_never_create_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("absent/config.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_svnui"))
        .env("SVNUI_CONFIG", &path)
        .env("SVNR_SVN", temp.path().join("missing-svn"))
        .current_dir(temp.path())
        .args(["llm", "--preset", "ollama"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!path.exists(), "查看示例不能创建配置");
}

#[test]
fn local_connection_failure_has_ollama_instructions_and_never_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    drop(listener);
    let config =
        format!("[llm]\nenabled=true\nprovider='ollama'\ntimeout_secs=1\nbase_url={endpoint:?}\n");
    let output = cli(&config, &["llm", "--test"], None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Ollama"));
}

#[test]
fn full_completion_url_is_rejected_before_network() {
    let config = "[llm]\nenabled=true\nprovider='ollama'\nbase_url='http://127.0.0.1:1/v1/chat/completions'\n";
    let output = cli(config, &["llm", "--test"], None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("base_url 不要带 /chat/completions"));
}

#[test]
fn malformed_success_does_not_echo_provider_error_body() {
    let (url, request) = serve(200, r#"{"error":{"message":"sensitive-provider-detail"}}"#);
    let config = format!("[llm]\nenabled=true\nprovider='ollama'\nbase_url={url:?}\n");
    let output = cli(&config, &["llm", "--test"], None);
    request.join().unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("接口兼容性"));
    assert!(!error.contains("sensitive-provider-detail"));
}
