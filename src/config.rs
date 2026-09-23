//! 配置：`~/.config/svnui/config.toml`
//!
//! ## 为什么要有配置文件
//!
//! 之前这些开关全散在环境变量里（`SVNUI_TRUST_CERT`、`SVNR_PAGER`、
//! `SVNR_SVN`…），换台机器就得重新记一遍。集中到一个文件里，初始化一次就够。
//!
//! ## 优先级
//!
//! **命令行参数 > 环境变量 > 配置文件 > 内置默认。**
//!
//! 环境变量排在配置文件前面，是为了"临时改一次"不用动文件 ——
//! 比如这次要信任证书，下次不需要，那用环境变量正合适。
//!
//! ## 文件位置
//!
//! `$SVNUI_CONFIG` → `$XDG_CONFIG_HOME/svnui/config.toml` → `~/.config/svnui/config.toml`
//!
//! `$SVNUI_CONFIG` 留着是为了测试和"临时换一份配置"的场景。
//!
//! ## 加载失败怎么办
//!
//! **一律降级为默认值，不报错、不退出。** 配置文件是"锦上添花"，
//! 缺了它 svnui 应该照常能用 —— 因为配错一个字段就让整个工具起不来，
//! 比没有配置文件糟糕得多。

use serde::Deserialize;
use std::path::PathBuf;

// ── LLM（提交信息生成）────────────────────────────────────

/// `base_url` 填兼容 OpenAI `/v1/chat/completions` 的地址即可，
/// 换服务商不用改代码。国内常见的：
///
/// | 服务商 | base_url | model |
/// |---|---|---|
/// | DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` |
/// | 通义千问 | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `qwen-plus` |
/// | 智谱 | `https://open.bigmodel.cn/api/paas/v4` | `glm-4-flash` |
/// | 本地 Ollama | `http://localhost:11434/v1` | `qwen2.5-coder` |
/// | 公司内网网关 | 问 IT 要 | — |
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// 总开关。**默认 false** —— 没配好就别发请求。
    pub enabled: bool,

    /// 兼容 OpenAI 的接口地址（不带 `/chat/completions`，代码会拼）。
    pub base_url: String,

    /// 模型名。
    pub model: String,

    /// **从哪个环境变量读 API Key。**
    ///
    /// 不把 key 写进配置文件：配置文件容易被误 `git add`，
    /// 而 key 泄露是没法撤销的。环境变量写进 `~/.zshrc` 就好。
    pub api_key_env: String,

    /// 超时秒数。TUI 里请求是后台发的，超时只是这一次生成失败，不卡界面。
    pub timeout_secs: u64,

    /// **是否把 diff 正文一起发给模型。默认 false。**
    ///
    /// 这是隐私开关：公司代码发给外部模型之前要过一遍 Compliance。
    /// 关着的时候只发"改了哪些文件、各自是什么状态"，模型的判断
    /// 会粗一些，但不会有代码出网。
    pub send_diff: bool,

    /// `send_diff = true` 时的上限（字节）。超了就截断 ——
    /// 大改动全发过去既慢又费 token，截断的部分对生成标题影响不大。
    pub max_diff_bytes: usize,

    /// 提交信息语言：`zh` / `en`。
    pub lang: String,

    /// 追加到提示词末尾的要求。比如团队要求"结尾带任务号"，
    /// 写在这里比改代码省事。
    pub extra_prompt: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            api_key_env: "SVNUI_LLM_KEY".to_string(),
            timeout_secs: 20,
            send_diff: false,
            max_diff_bytes: 20_000,
            lang: "zh".to_string(),
            extra_prompt: String::new(),
        }
    }
}

// ── SVN ──────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SvnConfig {
    /// 信任自签名 / 内网 CA 证书。等价于环境变量 `SVNUI_TRUST_CERT=1`。
    ///
    /// 内网服务器基本都是自签名，不开这个连不上。开了等于放弃防中间人，
    /// 只在确认是自家服务器时开。
    pub trust_cert: bool,

    /// svn 可执行文件路径。留空则按 `PATH` 找。
    pub svn_path: String,
}

impl Default for SvnConfig {
    fn default() -> Self {
        Self {
            trust_cert: false,
            svn_path: String::new(),
        }
    }
}

// ── 界面 ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// 启动时是否捕获鼠标。
    ///
    /// 开了能拖滚动条，代价是**终端的文本选择复制失效** ——
    /// diff 里复制路径是刚需，所以默认关。TUI 里按 `M` 可随时切换。
    pub mouse: bool,

    /// 变更排序：`status`（按状态分组）/ `path`（按路径）/ `time`。
    pub sort: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            mouse: false,
            sort: "status".to_string(),
        }
    }
}

// ── 预览 / 日志 ──────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PreviewConfig {
    /// diff 上下文行数。
    pub context_lines: usize,

    /// 分页器：`auto` / `delta` / `bat` / `less` / `none`。
    pub pager: String,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            context_lines: 3,
            pager: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// 首次加载多少条日志。`svn log` 是联网操作，多了要等服务器。
    pub page_size: usize,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self { page_size: 100 }
    }
}

// ── 总配置 ───────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub llm: LlmConfig,
    pub svn: SvnConfig,
    pub ui: UiConfig,
    pub preview: PreviewConfig,
    pub log: LogConfig,
}

impl Config {
    /// 加载配置。**任何失败都返回默认值**，不传播错误。
    ///
    /// 理由见模块文档"加载失败怎么办"：配置文件不该有能力让工具起不来。
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match toml::from_str::<Config>(&text) {
            Ok(c) => c,
            Err(_) => Self::default(),
        }
    }

    /// 配置文件路径。找不到 home 时返回 `None`。
    pub fn path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("SVNUI_CONFIG") {
            return Some(PathBuf::from(p));
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".config"))
            })?;
        Some(base.join("svnui").join("config.toml"))
    }

    /// 配置文件是否存在。用于给用户提示"你现在用的是默认值"。
    pub fn exists() -> bool {
        Self::path().map(|p| p.is_file()).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 默认值可用() {
        let c = Config::default();
        assert!(!c.llm.enabled, "LLM 默认必须关闭");
        assert!(!c.llm.send_diff, "diff 默认不能出网");
        assert_eq!(c.ui.mouse, false, "鼠标默认关闭，否则复制失效");
        assert_eq!(c.log.page_size, 100);
    }

    #[test]
    fn 空toml解析成默认值() {
        let c: Config = toml::from_str("").unwrap();
        assert!(!c.llm.enabled);
        assert_eq!(c.preview.context_lines, 3);
    }

    #[test]
    fn 只写一段时其余取默认() {
        let c: Config = toml::from_str("[llm]\nenabled = true\nmodel = \"glm-4\"\n").unwrap();
        assert!(c.llm.enabled);
        assert_eq!(c.llm.model, "glm-4");
        // 未写的字段仍应是默认，不是空串
        assert_eq!(c.llm.base_url, LlmConfig::default().base_url);
        assert_eq!(c.llm.api_key_env, "SVNUI_LLM_KEY");
    }

    #[test]
    fn 未知字段不报错() {
        // 以后加了新字段、用户还是旧配置时，不能因为多了字段就整体失效
        let c: Config = toml::from_str("[llm]\nfoo_bar = 1\n").unwrap();
        assert!(!c.llm.enabled);
    }

    #[test]
    fn 坏toml降级为默认() {
        // load() 走不到这里（不解析坏文件），但解析函数本身要能容错
        assert!(toml::from_str::<Config>("[[[坏").is_err());
    }
}
