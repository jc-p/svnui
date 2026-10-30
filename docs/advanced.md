# 命令行、构建与开发

[返回 README](../README.md) · [命令参考](cli.md) · [发布流程](../PUBLISH.md)

## 命令行

### 分支工作流

TUI 主界面按 `B` 打开分支面板：`F1` 创建分支、`F2` 创建标签、`F3` 切换、`F4` 合并预演。
`Tab` 切字段，`Ctrl+U` 清空当前字段，`Ctrl+S` 准备操作。创建和切换须在确认框按 `Y`；`Enter` / `N` / `Esc` 取消。

```bash
# 目标父目录必须已存在；^/ 表示当前 SVN 仓库根。
svnui branch '^/branches/feature-a' -m '创建 feature-a' --dry-run
svnui branch '^/branches/feature-a' -m '创建 feature-a' --yes
svnui tag '^/tags/v1.0' -r HEAD -m '发布 v1.0' --yes
svnui switch '^/branches/feature-a' --dry-run
svnui switch '^/branches/feature-a' --yes
svnui merge-preview '^/trunk'
svnui merge-preview '^/trunk' -r 12:18
```

创建分支和标签会**立即提交仓库副本**，不会自动切换，也不包含本地未提交改动。
默认复制当前工作副本根的 `BASE` 版本，可用 `-r` 指定整数版本、`HEAD` 或 `BASE`；标签与分支采用同样的 SVN copy 机制，不提供不可变标签保证。
准备阶段会固定实际版本号；重复目标、跨仓库 URL 和越级路径会被拒绝。

切换与合并预演要求整个工作副本干净，包括无未纳管项和已切换子树；不处理 externals。
`merge-preview` 始终运行 SVN 原生 `--dry-run`，不会应用改动或提交；`N:M` 表示两个端点之间的差异。
创建/切换的 `--dry-run` 只读取信息并打印计划，不执行 copy/switch；非交互执行写操作必须显式给 `--yes`。

### 通用参数

`svnui` 同时是可以直接脚本调用的 CLI。全局参数：

```
--cwd PATH       工作目录
--json           JSON 信封输出 {"ok":..}
--porcelain      紧凑两字符格式，如 "AM src/main.rs"
--timeout N      超时秒数（默认 30）
--trust-cert     放行 SSL 证书校验失败
--no-cache       禁用状态缓存
--cache-ttl N    缓存秒数（默认 300）
```

子命令：

| 命令 | 说明 |
|---|---|
| `probe` | 环境自检 |
| `status` | 工作副本状态 |
| `log` | 提交历史（`-l` 条数、`-r` 版本/区间、`--oneline`） |
| `diff` | 差异（`-c REV` 看某次提交、`--stat`、`--page`） |
| `info` | 仓库信息 |
| `blame` | 逐行追溯 |
| `add` | 加入版本控制 |
| `remove` | 移除（`--keep-local` 保留本地文件） |
| `commit` | 提交（`-m` 信息、`--dry-run`） |
| `update` | 更新（默认 `--accept postpone`，冲突留给人工） |
| `resolve` | 解决冲突（`-a` 策略白名单校验） |
| `revert` | **回滚本地改动，不可撤销**（`--dry-run` 先看清单） |
| `cleanup` | 清理（`--remove-unversioned` 等危险开关） |
| `purge-mine` | 清理已解决冲突残留的 `.mine` / `.rOLD` / `.rNEW`（仍在冲突中的不动） |
| `doctor` | 体检：锁 / 冲突 / 缺失 / 阻碍 / `.mine` 残留 |
| `conflicts` | 列出所有冲突 |
| `checkout` | 检出（唯一不需要先有工作副本的命令） |
| `login` / `auth` | 凭据验证 / 列出已缓存凭据 |
| `preview` | 独立预览本地文件或浏览 SVN URL，无需进入工作副本 |
| `projects` | 已知工作副本清单：列出 / `--add` 登记 / `--remove` 移除（打开过的仓库会自动记住） |
| `tui` | 打开交互式 TUI（不带子命令时的默认形态也是它） |
| `llm` | LLM 配置示例与自检（`--preset` / `--test`） |
| `config` | 配置文件状态与初始化（`--init`） |
| `cache` | 查看状态缓存 / `--clear` 清除 |
| `daemon` | 常驻进程（`start` / `status` / `stop` / `rescan`） |

> `q` 和 `install-yazi` 是早期 yazi 集成阶段的遗留命令，代码还在，
> 但本项目已经转向独立 TUI，日常用不到。

### 危险操作分级

| 级别 | 操作 | 行为 |
|---|---|---|
| Safe | `status` `diff` `log` `info` | 直接执行 |
| Low | `add`、`cleanup`（仅解锁） | 直接执行 |
| Medium | `commit` `update` `resolve` `remove` `purge-mine` | TTY 下确认；非 TTY **必须 `--yes`** |
| High | **`revert`**、`cleanup --remove-unversioned` | TTY 下确认；非 TTY **一律拒绝** |

`revert` 会丢弃已纳管内容的本地改动；`cleanup --remove-unversioned` 会删除未纳管文件，
是本工具里最危险的两个，务必先 `--dry-run`。

---

## 缓存
`svn status` 在大仓库上是秒级到几十秒级。缓存的目标是**命中时一次 svn 都不跑**。

`svn status` 只输出非 normal 的条目，所以缓存里天然没有"干净的文件"。
于是"干净文件被改脏"和"新建未版本化文件"这两种情况**都检测不到**——
修改已有文件连父目录 mtime 都不改，创建文件也只改父目录。

所以缓存额外存了工作副本内每个文件/目录的 `(mtime, size)`（跳过 `.svn`），
加载时重新走一遍树逐条比对，全一致才认为状态未变。
因为遍历每次都做，正确性与缓存年龄无关，TTL 只是安全阀。

边界：

- 超过 200,000 个节点不写缓存
- `.svn/wc.db` 不存在（SVN 1.6 及更早）→ 不缓存
- `--ignored` / `-u` 这类非默认查询不走缓存
- 所有写操作后自动作废相关缓存

---

## 构建开关

源码构建需要 Rust 1.88 或更新版本。

```toml
[features]
default = ["tui", "docs"]
tui     = ["dep:ratatui", "dep:ratatui-themekit", "dep:crossterm", "dep:tui-textarea", "dep:ratatui-image", "dep:image"]
docs    = ["dep:csv", "dep:calamine", "dep:docx-lite"]
```

| 场景 | 命令 |
|---|---|
| 完整 | `cargo build --release` |
| 只要 CLI（省掉 crossterm 一堆依赖） | `cargo build --release --no-default-features` |
| 不要办公文档预览 | `cargo build --release --no-default-features --features tui` |

`docs` feature 提供 CSV/TSV 表格排版和 Excel/Word 内容提取。关掉后 CSV/TSV 按普通文本预览，Excel/Word 提示未启用文档预览。图片支持随 `tui` feature 编译，无需安装额外插件或系统图片工具。

> ratatui 0.30 默认配 crossterm 0.29，不同主版本各自维护事件队列和 raw mode，
> 所以 `crossterm` 钉在 0.29、`tui-textarea` 用维护分支 `tui-textarea-2` 0.13。
> 升级时这三个要一起动。

---

## 开发

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

分层与硬约束：

```text
main → cli → svn::client → svn::command
                  ↓
               domain   （纯模型，谁都能依赖）
```

1. **`domain` 零 IO** — 只放数据结构与纯函数
2. **`svn` 是唯一能 spawn 进程的地方** — 别处不许 `Command::new`
3. **永不 `sh -c`** — 全部参数数组传递，防注入

文档：[命令参考](cli.md)、[TUI 快捷键](tui-keymap.md)、[主题配置](theme.md)、[Q&A](qa.md)、[专项故障排查](troubleshooting.md)。
发布流程见 [PUBLISH.md](../PUBLISH.md)。
