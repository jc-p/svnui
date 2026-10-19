//! LLM 提交信息生成 —— **兼容 OpenAI `/v1/chat/completions`**。
//!
//! ## 为什么不绑死某一家
//!
//! 只认 `/v1/chat/completions` 这一套协议，换算服务商就改两个字符串
//! （`base_url` + `model`），不用改代码。DeepSeek / 通义 / 智谱 /
//! 本地 Ollama / 公司内网网关都能这么接。
//!
//! ## 发出去的是什么
//!
//! **默认只发"改了哪些文件、各自什么状态"，不发 diff 正文。**
//! 公司代码出网这件事不能默认做 —— `send_diff` 默认 false 就是这条线。
//! 没有 diff 时模型只能从路径和状态推测，会粗一些，但这是有意的取舍。
//!
//! ## 失败是常态
//!
//! 网络不通、key 没配、超时、返回格式怪 —— 都返回 `Err(原因)`，
//! 由调用方回退到本地规则。**这里不 panic、不吞错误**，
//! 因为"生成失败"应该表现为"退回到规则候选"，而不是弹个红框打断流程。

use crate::config::LlmConfig;

/// 一次改动：`(相对路径, 状态符号)`。
///
/// 用元组而不是引入 `commit_gen::Change`：那个类型在 `tui` feature 下，
/// 而本模块是无条件编译的（CLI 也可能用）。依赖方向不能倒过来。
pub type Item = (String, char);

/// 生成提交信息。成功返回 1~N 条候选（第一条最可能是想要的）。
///
/// 失败返回**人话原因**，调用方直接拿去显示或据以回退。
pub fn suggest(items: &[Item], diff: Option<&str>, cfg: &LlmConfig) -> Result<Vec<String>, String> {
    if !cfg.enabled {
        return Err("未启用（llm.enabled = false）".to_string());
    }
    if items.is_empty() {
        return Err("没有勾选项，无从生成".to_string());
    }

    let prompt = build_prompt(items, diff, cfg);
    let url = format!("{}/chat/completions", cfg.endpoint().trim_end_matches('/'));

    let body = serde_json::json!({
        "model": cfg.model_name(),
        "messages": [
            {"role": "user", "content": prompt}
        ],
        // 提交信息要稳，不要创意：温度低一点，重复调用结果更一致
        "temperature": 0.3,
        "max_tokens": 300,
    });

    let resp = send_request(cfg, &url, body)?;

    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("响应不是 JSON：{}", e))?;

    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| "模型响应缺少 choices[0].message.content，请检查接口兼容性和模型权限".to_string())?;

    let out = parse_candidates(content);
    if out.is_empty() {
        return Err("模型返回了空内容".to_string());
    }
    Ok(out)
}

/// 连通性自检：确认 base_url / model / API Key 三样都对。
///
/// 存在的理由是"失败太安静了" —— 配置写错时调用方只回退本地规则，
/// 用户看到的是"生成的还是老样子"，分不清是模型没接上还是本来就该这样。
/// 所以给一个能直接问的命令：`svnui llm --test`。
pub fn ping(cfg: &LlmConfig) -> Result<String, String> {
    if !cfg.enabled {
        return Err("llm.enabled = false：配置里没打开，当前走的是本地规则".to_string());
    }

    let url = format!("{}/chat/completions", cfg.endpoint().trim_end_matches('/'));
    let body = serde_json::json!({
        "model": cfg.model_name(),
        "messages": [{"role": "user", "content": "只回复 OK 两个字"}],
        "max_tokens": 16,
    });

    let t0 = std::time::Instant::now();
    let resp = send_request(cfg, &url, body)?;
    let ms = t0.elapsed().as_millis();

    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("响应不是 JSON（base_url 是不是指错了？）：{}", e))?;

    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| "模型响应缺少 choices[0].message.content，请检查接口兼容性和模型权限".to_string())?;

    Ok(format!(
        "连通正常\n  模型: {}\n  地址: {}\n  耗时: {} ms\n  回复: {}",
        cfg.model_name(),
        url,
        ms,
        content.trim()
    ))
}

/// 两条调用路径共享认证与错误诊断，避免自检通过但生成采用另一套配置。
fn send_request(
    cfg: &LlmConfig,
    url: &str,
    mut body: serde_json::Value,
) -> Result<ureq::Response, String> {
    if cfg.endpoint().trim_end_matches('/').ends_with("/chat/completions") {
        return Err("base_url 不要带 /chat/completions；工具会自动补齐该路径".into());
    }
    let mut req = ureq::post(url)
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(cfg.timeout_secs.max(1)));
    if cfg.requires_key() {
        let key = std::env::var(&cfg.api_key_env)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "API Key 未设置或为空：{}；设置后重新启动 svnui",
                    cfg.api_key_env
                )
            })?;
        req = req.set("Authorization", &format!("Bearer {}", key.trim()));
    }
    // Flash 默认思考可能耗尽短提交信息的输出预算，仅对已核实的模型关闭。
    if cfg.model_name() == "glm-4.7-flash" {
        body["thinking"] = serde_json::json!({"type": "disabled"});
    }
    req.send_json(body).map_err(|err| match err {
        ureq::Error::Status(401 | 403, _) => "认证失败或模型无权限，请检查 API Key 和账号权限".into(),
        ureq::Error::Status(429, _) => "请求限流或额度不足，请稍后重试并检查服务商额度；不会自动切换付费模型".into(),
        ureq::Error::Status(404, _) => "接口或模型不存在，请检查 base_url 和 model；本地模型可用 ollama list 查看".into(),
        ureq::Error::Status(code, _) => format!("模型服务返回 HTTP {code}，请检查服务状态"),
        ureq::Error::Transport(_) if !cfg.requires_key() => "无法连接 Ollama 或请求超时，请确认本地服务已启动、模型已下载，并检查地址和 timeout_secs".into(),
        ureq::Error::Transport(_) => "模型请求连接失败或超时，请检查网络、接口地址和 timeout_secs".into(),
    })
}

/// 当前 LLM 配置的文字说明（不联网，只说清"会怎么走"）。
pub fn describe(cfg: &LlmConfig) -> String {
    let key_state = if !cfg.requires_key() {
        "无需 Key（不发送 Authorization）".to_string()
    } else {
        match std::env::var(&cfg.api_key_env) {
            Ok(v) if !v.trim().is_empty() => "已设置".to_string(),
            _ => "未设置 → Ctrl+G 回退本地规则".to_string(),
        }
    };

    format!(
        "LLM 状态\n  预设: {}\n  启用: {}\n  地址: {}\n  模型: {}\n  Key 环境变量: {}（{}）\n  发送 diff: {}（{}）\n  超时: {} 秒\n\n{}",
        cfg.provider.name(),
        cfg.enabled,
        cfg.endpoint(),
        cfg.model_name(),
        cfg.api_key_env,
        key_state,
        cfg.send_diff,
        if cfg.send_diff {
            "调用方提供时可携带正文；当前 TUI 仅发送路径和状态"
        } else {
            "只发文件路径和状态"
        },
        cfg.timeout_secs,
        if cfg.enabled {
            "用 svnui llm --test 实发一个请求验证连通性"
        } else {
            "在 ~/.config/svnui/config.toml 里把 llm.enabled 改成 true 启用"
        }
    )
}

/// 构造提示词。
///
/// 明确要求"只输出提交信息本身、每行一条" —— 不然模型很爱先来一段
/// "好的，这是为你生成的提交信息："，那段话混进候选里很烦。
fn build_prompt(items: &[Item], diff: Option<&str>, cfg: &LlmConfig) -> String {
    let zh = !cfg.lang.eq_ignore_ascii_case("en");

    let mut p = String::new();
    if zh {
        p.push_str("你是提交信息生成助手。根据下面这次改动，给出 1~2 条提交信息。\n\n");
        p.push_str("要求：\n");
        p.push_str("- 使用 conventional commit 格式：type(scope): 简述\n");
        p.push_str("- type 用 feat / fix / refactor / docs / chore / test 之一\n");
        p.push_str("- 简述用中文，不超过 50 字，只描述改了什么，不要编造动机\n");
        p.push_str("- 只输出提交信息本身，每行一条，不要编号、不要解释、不要代码块\n");
    } else {
        p.push_str("You are a commit message assistant. Given the changes, output 1~2 messages.\n\n");
        p.push_str("Rules:\n");
        p.push_str("- conventional commit format: type(scope): summary\n");
        p.push_str("- type is one of feat / fix / refactor / docs / chore / test\n");
        p.push_str("- summary under 72 chars, describe what changed only\n");
        p.push_str("- output messages only, one per line, no numbering, no code fences\n");
    }

    if !cfg.extra_prompt.trim().is_empty() {
        p.push_str(&format!("- {}\n", cfg.extra_prompt.trim()));
    }

    p.push_str("\n改动文件（状态符号：M 修改 / A 新增 / D 删除 / R 替换）：\n");
    for (rel, sign) in items.iter().take(50) {
        p.push_str(&format!("{} {}\n", sign, rel));
    }
    if items.len() > 50 {
        p.push_str(&format!("… 以及另外 {} 个文件\n", items.len() - 50));
    }

    if let Some(d) = diff {
        if cfg.send_diff && !d.trim().is_empty() {
            let mut body = d.to_string();
            if body.len() > cfg.max_diff_bytes {
                // 按字符截而不是字节：中文路径按字节截会切出半个字
                let take: String = body.chars().take(cfg.max_diff_bytes).collect();
                body = format!("{}\n…（diff 过长已截断）", take);
            }
            p.push_str("\ndiff：\n");
            p.push_str(&body);
            p.push('\n');
        }
    }

    p
}

/// 把模型输出拆成候选。
///
/// 模型爱加的东西都清掉：markdown 代码围栏、`1.` 编号、`- ` 列表符、
/// 首尾引号、空行。剩下的非空行各算一条候选。
fn parse_candidates(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;

    for line in raw.lines() {
        let t = line.trim();

        if t.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            // 围栏内是正文，但仍然要清掉可能的编号
        }

        let mut s = t;
        // 去掉列表符号
        s = s.trim_start_matches("- ").trim_start_matches("* ");
        // 去掉 "1." / "1)" 这类编号
        if let Some(rest) = strip_leading_num(s) {
            s = rest;
        }
        // 去掉包裹的引号
        s = s.trim_matches('"').trim_matches('`');

        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        // 明显是模型的客套话就跳过
        if is_preamble(s) {
            continue;
        }
        if !out.contains(&s.to_string()) {
            out.push(s.to_string());
        }
    }
    out
}

/// 去掉开头的 `1.` / `2)` / `3、` 这类编号，返回剩余部分。
///
/// 按 `char` 而不是字节走：分隔符里有中文顿号，
/// 用字节字面量写这个分隔符编译不过（非 ASCII），
/// 而且按字节截中文会切出半个字。
fn strip_leading_num(s: &str) -> Option<&str> {
    let mut end = 0usize;
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_digit() {
            end = i + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let rest = &s[end..];
    let sep = rest.chars().next()?;
    if matches!(sep, '.' | ')' | '、' | ':' | ' ') {
        Some(rest[sep.len_utf8()..].trim_start())
    } else {
        None
    }
}

/// 模型的开场白/结束语。这些不是提交信息，混进来会让人以为生成错了。
fn is_preamble(s: &str) -> bool {
    let low = s.to_lowercase();
    let marks = [
        "以下是", "这是", "好的", "当然", "希望", "如果需要", "请告诉我",
        "here is", "here are", "sure", "certainly", "i hope", "let me know",
    ];
    marks.iter().any(|m| low.contains(m)) && !low.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LlmConfig {
        LlmConfig {
            enabled: true,
            ..LlmConfig::default()
        }
    }

    #[test]
    fn 未启用直接返回错误() {
        let c = LlmConfig::default();
        assert!(suggest(&[("a.rs".into(), 'M')], None, &c).is_err());
    }

    #[test]
    fn 没有key报错并且说清楚变量名() {
        // 确保环境变量确实没设（测试环境不该有）
        std::env::remove_var("SVNUI_LLM_KEY_TEST_ONLY");
        let mut c = cfg();
        c.api_key_env = "SVNUI_LLM_KEY_TEST_ONLY".to_string();
        let e = suggest(&[("a.rs".into(), 'M')], None, &c).unwrap_err();
        assert!(e.contains("SVNUI_LLM_KEY_TEST_ONLY"), "错误信息要指明变量名：{}", e);
    }

    #[test]
    fn 清理代码围栏和编号() {
        let raw = "好的，这是为你生成的：\n\n```\n1. feat(tui): 新增配置面板\n2. fix(svn): 修证书提示\n```\n希望有用";
        let v = parse_candidates(raw);
        assert_eq!(v.len(), 2, "{:?}", v);
        assert_eq!(v[0], "feat(tui): 新增配置面板");
        assert_eq!(v[1], "fix(svn): 修证书提示");
    }

    #[test]
    fn 空输入返回空() {
        assert!(parse_candidates("").is_empty());
        assert!(parse_candidates("```\n\n```").is_empty());
    }

    #[test]
    fn 提示词默认不含diff() {
        let c = cfg();
        let p = build_prompt(&[("a.rs".into(), 'M')], Some("secret code"), &c);
        assert!(!p.contains("secret code"), "send_diff=false 时 diff 绝不能进提示词");
    }

    #[test]
    fn 开启后才带diff() {
        let mut c = cfg();
        c.send_diff = true;
        let p = build_prompt(&[("a.rs".into(), 'M')], Some("+ let x = 1;"), &c);
        assert!(p.contains("let x = 1"));
    }

    #[test]
    fn 超长diff被截断() {
        let mut c = cfg();
        c.send_diff = true;
        c.max_diff_bytes = 10;
        let p = build_prompt(&[("a".into(), 'M')], Some("0123456789ABCDEFGH"), &c);
        assert!(p.contains("截断"));
        assert!(!p.contains("ABCDEFGH"));
    }

    #[test]
    fn 文件太多时省略() {
        let items: Vec<Item> = (0..60).map(|i| (format!("f{}.rs", i), 'M')).collect();
        let p = build_prompt(&items, None, &cfg());
        assert!(p.contains("另外 10 个文件"));
    }
}
