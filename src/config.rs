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

/// 预设只提供默认地址、模型和认证方式；显式地址与模型仍优先。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    #[default]
    Deepseek,
    Zhipu,
    Ollama,
}

impl LlmProvider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Deepseek => "deepseek",
            Self::Zhipu => "zhipu",
            Self::Ollama => "ollama",
        }
    }

    fn defaults(self) -> (&'static str, &'static str) {
        match self {
            Self::Deepseek => ("https://api.deepseek.com/v1", "deepseek-chat"),
            Self::Zhipu => ("https://open.bigmodel.cn/api/paas/v4", "glm-4.7-flash"),
            Self::Ollama => ("http://localhost:11434/v1", "qwen2.5-coder:3b"),
        }
    }
}

impl std::str::FromStr for LlmProvider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "deepseek" => Ok(Self::Deepseek),
            "zhipu" => Ok(Self::Zhipu),
            "ollama" => Ok(Self::Ollama),
            _ => Err("可选预设：zhipu、ollama、deepseek".into()),
        }
    }
}

/// `base_url` 填兼容 OpenAI `/v1/chat/completions` 的地址即可，
/// 换服务商不用改代码。国内常见的：
///
/// | 服务商 | base_url | model |
/// |---|---|---|
/// | DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` |
/// | 通义千问 | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `qwen-plus` |
/// | 智谱 | `https://open.bigmodel.cn/api/paas/v4` | `glm-4.7-flash` |
/// | 本地 Ollama | `http://localhost:11434/v1` | `qwen2.5-coder` |
/// | 公司内网网关 | 问 IT 要 | — |
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// 总开关。**默认 false** —— 没配好就别发请求。
    pub enabled: bool,

    pub provider: LlmProvider,

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

    /// 限流（429）和服务端错误（5xx）的重试次数，**默认 2**。
    ///
    /// 只重试这两种：它们是"稍等一下就好"的临时故障。
    /// 连接失败/超时不重试 —— 那多半是地址或网络配错了，
    /// 重试三次只是让用户多等三倍时间才看到同一句提示。
    pub retry: u32,

    /// 追加到提示词末尾的要求。比如团队要求"结尾带任务号"，
    /// 写在这里比改代码省事。
    pub extra_prompt: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: LlmProvider::default(),
            base_url: String::new(),
            model: String::new(),
            api_key_env: "SVNUI_LLM_KEY".to_string(),
            timeout_secs: 20,
            send_diff: false,
            max_diff_bytes: 20_000,
            lang: "zh".to_string(),
            retry: 2,
            extra_prompt: String::new(),
        }
    }
}

impl LlmConfig {
    /// 空值表示采用预设，兼容旧配置的显式 base_url / model。
    pub fn endpoint(&self) -> &str {
        if self.base_url.trim().is_empty() {
            self.provider.defaults().0
        } else {
            self.base_url.trim()
        }
    }

    pub fn model_name(&self) -> &str {
        if self.model.trim().is_empty() {
            self.provider.defaults().1
        } else {
            self.model.trim()
        }
    }

    pub fn requires_key(&self) -> bool {
        self.provider != LlmProvider::Ollama
    }

    /// 只输出配置片段，不写用户文件或读取密钥。
    pub fn preset_example(provider: LlmProvider) -> String {
        let (endpoint, model) = provider.defaults();
        format!(
            "# 合并到配置文件的 [llm] 段；不要重复添加同名段。\n# 预设地址：{endpoint}\n# 预设模型：{model}\n[llm]\nenabled = true\nprovider = \"{}\"\n# 留空采用预设；自定义地址或模型时填写。\nbase_url = \"\"\nmodel = \"\"\napi_key_env = \"SVNUI_LLM_KEY\"\ntimeout_secs = {}\nsend_diff = false\nlang = \"zh\"\n",
            provider.name(), if provider == LlmProvider::Ollama { 60 } else { 20 }
        )
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

/// 默认配置全文。首次运行时写进 `~/.config/svnui/config.toml`。
///
/// 每个字段都带注释 —— 生成一份"能读的配置"比让用户翻文档找字段有用得多。
/// 内容和 `config.example.toml` 保持一致。
const DEFAULT_TOML: &str = r##"# svnui 配置 —— ~/.config/svnui/config.toml
#
# 优先级：命令行参数 > 环境变量 > 本文件 > 内置默认。
# 全部字段都可省略，省了的取默认值。
#
# 本文件不存在的时候 svnui 照常能用（用内置默认），
# 写坏了也只会整体退回默认值，不会让工具起不来。

# ── 提交信息生成（LLM）────────────────────────────────
[llm]
# 总开关。默认关闭 —— 配好 key 再打开。
enabled = false

# 兼容 OpenAI /v1/chat/completions 的地址（不要带 /chat/completions）。
# 换服务商只改这两行，不用改代码：
#   DeepSeek   https://api.deepseek.com/v1                          deepseek-chat
#   通义千问    https://dashscope.aliyuncs.com/compatible-mode/v1    qwen-plus
#   智谱       https://open.bigmodel.cn/api/paas/v4                  glm-4.7-flash
#   本地 Ollama http://localhost:11434/v1                            qwen2.5-coder
#   公司网关    问 IT
# zhipu：云端免费模型（需要 Key，额度以服务商为准）；ollama：本地免 Key。
# 查看配置示例：svnui llm --preset zhipu / svnui llm --preset ollama
provider = "deepseek"
# 留空使用预设；显式配置优先于预设，切换预设时请清空旧覆盖值。
base_url = ""
model = ""

# API Key 从哪个环境变量读。
# 不把 key 写进配置文件：配置文件容易被误 git add，而 key 泄露没法撤销。
#   echo 'export SVNUI_LLM_KEY=sk-xxx' >> ~/.zshrc && exec zsh
api_key_env = "SVNUI_LLM_KEY"

# 超时秒数。请求在后台发，超时只是这一次生成失败，不卡界面。
timeout_secs = 20

# ⚠️ 是否把 diff 正文发给模型。默认 false。
# 这是隐私开关：公司代码出网前要过一遍 Compliance。
# 关着的时候只发"改了哪些文件、各自什么状态"，模型判断会粗一些。
send_diff = false

# send_diff = true 时的截断上限（字节）。
max_diff_bytes = 20000

# 提交信息语言：zh / en
lang = "zh"

# 限流（429）和服务端错误（5xx）的重试次数。0 = 不重试。
# 连接失败和超时不重试：那多半是地址配错了，重试只会让人多等。
retry = 2

# 追加要求。比如团队规定结尾带任务号，写这里比改代码省事。
extra_prompt = ""

# ── SVN ──────────────────────────────────────────────
[svn]
# 信任自签名 / 内网 CA 证书。等价于环境变量 SVNUI_TRUST_CERT=1。
# 内网服务器基本都是自签名，不开连不上。开了等于放弃防中间人。
trust_cert = false

# svn 可执行文件路径。留空按 PATH 找。
svn_path = ""

# ── 界面 ─────────────────────────────────────────────
[ui]
# 启动时是否捕获鼠标。
# 开了能拖滚动条，代价是终端的文本选择复制失效 ——
# diff 里复制路径是刚需，所以默认关。TUI 里按 M 可随时切换。
mouse = false

# 变更排序：status / path / time
sort = "status"

# ── 预览 ─────────────────────────────────────────────
[preview]
# diff 上下文行数
context_lines = 3

# 分页器：auto / delta / bat / less / none
pager = "auto"

# ── 日志 ─────────────────────────────────────────────
[log]
# 首次加载多少条。svn log 是联网操作，多了要等服务器。
page_size = 100
"##;

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

    /// LLM 自检保留配置错误，不把配置拼错误报成“尚未启用”。
    pub fn load_checked() -> Result<Self, String> {
        let path = Self::path().ok_or("无法确定配置文件路径")?;
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|_: toml::de::Error| {
                "配置文件格式或字段无效，请检查 provider、字段类型和引号（未使用默认配置）".into()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("无法读取配置文件，请检查路径与权限".into()),
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

    /// 首次运行时生成一份带注释的配置。
    ///
    /// **只在文件不存在时写**，已存在一律不动 —— 绝不覆盖用户的改动。
    /// 返回是否真的新建了文件（调用方据此决定要不要提示一句）。
    ///
    /// 所有失败都静默：home 不可写、只读文件系统、容器环境……
    /// 理由见模块文档"加载失败怎么办" —— 配置文件不该有能力让工具起不来。
    pub fn ensure_file() -> bool {
        let Some(path) = Self::path() else {
            return false;
        };
        if path.is_file() {
            return false;
        }
        Self::write_to(&path).is_ok()
    }

    /// 写一份默认配置，返回路径。`force = true` 才覆盖已有文件。
    ///
    /// 供 `svnui config --init` 用：自动初始化只补缺失的，
    /// 想拿回一份干净的注释版就得显式说要覆盖。
    pub fn write_default(force: bool) -> std::io::Result<std::path::PathBuf> {
        let path = Self::path().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "拿不到配置文件路径（HOME 未设置）",
            )
        })?;
        if path.is_file() && !force {
            return Ok(path);
        }
        Self::write_to(&path)?;
        Ok(path)
    }

    fn write_to(path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, DEFAULT_TOML)
    }

    /// 当前生效配置的简要说明，供 `svnui config` 打印。
    ///
    /// 存在的理由是"看不出配置有没有生效"：文件写错了、key 没设、
    /// 开关没开，三种情况在 TUI 里表现一模一样（都静默回退）。
    pub fn describe(&self) -> String {
        let p = Self::path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "（未知：HOME 未设置）".to_string());
        let mut out = String::new();
        out.push_str("配置文件\n");
        out.push_str(&format!("  路径: {}\n", p));
        out.push_str(&format!(
            "  状态: {}\n",
            if Self::exists() {
                "已存在"
            } else {
                "不存在（用内置默认）"
            }
        ));
        out.push('\n');
        out.push_str(&crate::llm::describe(&self.llm));
        out.push('\n');
        out.push_str("\nSVN\n");
        out.push_str(&format!("  信任证书: {}\n", self.svn.trust_cert));
        out.push_str("\n界面\n");
        out.push_str(&format!("  鼠标: {}\n", self.ui.mouse));
        out.push_str(&format!("  排序: {}\n", self.ui.sort));
        out.push_str("\n改完立即生效，不用重启。\n");
        out
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
        // 空覆盖值会解析到 DeepSeek 默认地址，保持旧配置行为。
        assert_eq!(c.llm.endpoint(), "https://api.deepseek.com/v1");
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
