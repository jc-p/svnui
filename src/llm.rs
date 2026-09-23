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

    let key = std::env::var(&cfg.api_key_env)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "没读到 API Key：环境变量 {} 未设置或为空（写进 ~/.zshrc 后要 exec zsh）",
                cfg.api_key_env
            )
        })?;

    let prompt = build_prompt(items, diff, cfg);
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        // 提交信息要稳，不要创意：温度低一点，重复调用结果更一致
        "temperature": 0.3,
        "max_tokens": 300,
    });

    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", key))
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(cfg.timeout_secs.max(1)))
        .send_json(body)
        .map_err(|e| format!("请求失败：{}", e))?;

    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("响应不是 JSON：{}", e))?;

    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            // 常见的是 key 错了被网关挡回来，body 里带 error.message
            let hint = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("（响应里没有 error 字段）");
            format!("响应格式不符预期：{}", hint)
        })?;

    let out = parse_candidates(content);
    if out.is_empty() {
        return Err("模型返回了空内容".to_string());
    }
    Ok(out)
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
