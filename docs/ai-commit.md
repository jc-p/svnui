# AI 生成提交信息

[返回 README](../README.md) · [Q&A](qa.md#ai-commit)

提交面板按 `Ctrl+G` 生成可编辑候选，再按一次轮换候选。生成不会自动提交，已有编辑内容不会被覆盖；
未启用 AI 或请求失败时使用本地规则，并显示当前来源或回退原因。

## 复用本机 Codex（推荐）

Codex 通道从 **0.7.2** 起提供，安装上面的正式版即可使用，无需从源码编译。
需要安装 Codex CLI，并在启动 svnui 的同一终端中确认程序和登录可用：

```bash
codex --version
codex login status
svnui llm --preset codex
```

最后一条只输出配置示例，不联网、不写配置。将示例合并到已有配置的 `[llm]` 段，避免重复段：

```toml
[llm]
enabled = true
provider = "codex"
codex_bin = "codex"
timeout_secs = 90
send_diff = false
```

无需再填写模型、服务商地址或 API Key：由 Codex CLI 自己读取本机配置与登录，尊重 `CODEX_HOME`。
`codex_bin` 也可填可执行文件的绝对路径。切换为 Codex 后，原有 `base_url`、`model`、`api_key_env`
不再参与请求；桌面端某次聊天临时选择的模型不保证等于 CLI 配置，需以 CLI 实际生效配置为准。

```bash
svnui llm        # 查看通道和生效策略，不联网
svnui llm --test # 只发送最小 OK 请求，验证 Codex 实际连通性
```

生成在独立临时目录运行，只传入本次勾选的文件清单，启用 `send_diff` 后才附带差异正文。
任务禁用工程说明、技能说明、MCP、插件、hooks 和内置工具，不执行自动提交；超时返回本地规则。
Codex CLI 仍可能附加其全局用户说明，这部分也会进入模型上下文；这里不复制或改写你的全局规则。
Unix 上超时会清理任务所在进程组，临时输出在成功和失败后都会移除。Codex 首次启动和继承的推理设置
可能增加耗时，可按需调整 `timeout_secs`。本机 CLI 版本兼容性与常见问题见 [Q&A](qa.md#ai-commit)。

## 已有 HTTP 模型配置

已有的 DeepSeek、智谱、Ollama 通道继续兼容；暂不扩展更多模型适配。
`svnui llm --preset zhipu` / `svnui llm --preset ollama` 可查看原有示例。
仅这些 HTTP 通道使用 `base_url`、`model` 和 `api_key_env`；显式地址与模型优先于预设。
云端 Key 默认由 `SVNUI_LLM_KEY` 环境变量提供；Ollama 由用户自行准备本地服务和模型，不发送 Authorization。

所有通道默认只发送勾选文件的路径与状态；配置 `send_diff = true` 后才发送 diff 正文，
受 `max_diff_bytes` 的 UTF-8 字节预算限制。`--test` 不发送工作副本内容，实际请求会使用对应模型服务的额度。
生成失败保留本地规则候选，不会自动切换服务商。配置路径支持 `SVNUI_CONFIG`、`XDG_CONFIG_HOME`，
默认 `~/.config/svnui/config.toml`。
