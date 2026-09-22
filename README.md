# svnui

Subversion 的 TUI 客户端。用 Rust + [ratatui](https://github.com/ratatui/ratatui) 写的独立终端程序，
不依赖 yazi，也不需要额外配置就能用。

```
┌─ 文件树 ─────────────┬─ 预览 / diff ────────────────────────┐
│ [x] M src/main.rs    │ Index: src/main.rs                   │
│ [ ] A src/new.rs     │ @@ -1,5 +1,6 @@                      │
│     ? notes.txt      │  fn main() {                         │
│                      │ -    println!("hi");                 │
│                      │ +    println!("hello");              │
└──────────────────────┴──────────────────────────────────────┘
 j/k 移动 | Space 勾选 | C 提交 | L 日志 | ? 帮助
```

---

<!-- BEGIN svnui:auth -->
## SVN 认证配置

svnui 调用 svn 时**强制加 `--non-interactive`** —— TUI 里没法回答
"是否接受证书 / 请输入用户名"这类交互式提问，不加就会挂死。
所以凭证和证书都必须**提前处理好**，不能指望运行时现问。

### 凭证存在哪

```
~/.subversion/auth/svn.simple/     # 按 realm（服务器 + 路径）分文件存放
```

第一次连接某个服务器时，先交互式认证一次把凭证存下来：

```bash
svn info <仓库 URL>          # 按提示输入用户名密码，之后就不问了
# 或
svnui login                  # 走 svnui 自己的登录流程
```

存下之后 svnui 的所有命令都用缓存，不再索要用户名密码。
**检出面板里的"用户名 / 密码"可以留空**，留空就不传 `--username/--password`，
svn 自己按 realm 挑缓存里的那条。

缓存不对或过期：删掉 `svn.simple/` 下对应的文件，或重跑 `svnui login`。

### 自建 / 内网服务器：自签名证书

症状：

```
svnui update
SSL 证书校验失败（svn 退出码 1）
```

原因就是开头那句 —— 命令行 `svn` 遇到证书会弹 `(p)` 让你永久接受，
svnui 不能弹，于是直接失败。

解法（二选一）：

```bash
svnui update --trust-cert                  # 单次
export SVNUI_TRUST_CERT=1                  # 长期，写进 ~/.zshrc
```

对应的 svn 参数是
`--trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other`。

> 改完 `~/.zshrc` 要 `exec zsh` 或重开窗口才生效。
> 用 `env | grep SVNUI` 确认，没输出就是没生效。
>
> 放行证书等于放弃防中间人，只对自己确认过的内网服务器这么干。

### 错误码速查

svn 的报错有个坑：**它把"结果"放在前面、"原因"放在后面**。
同时出现两个码时，第一个往往是结果，照它排查会跑偏。

| 错误码 | 真实含义 | 怎么办 |
|---|---|---|
| `E170013` | 连不上（多半是**结果**） | 往下看有没有 `E175013` |
| `E175013` | 该路径**无权访问** | 见下面"按子树授权" |
| `E230001` | 证书问题 | `--trust-cert` |
| `E170001` | 没有可用用户名 | 先做一次交互式认证缓存凭证 |
| `E215004` | 凭证不对 / 过期 | 删缓存重登 |
| `E155007` | 不是工作副本 | `cd` 进工作副本再跑 |
| `E000111` | 连接被拒 | 查 `~/.subversion/servers` 里的 `http-proxy` |

### 按子树授权：能进子目录，但看不了仓库根

很多 SVN 是按路径授权的。你能访问 `/trunk/A8_Patch/...`，
不代表能 `svn list` 仓库根 `/JR/KAMP` —— 后者会报 `E175013 forbidden`。

仓库浏览面板（`V`）**从当前工作副本的 URL 起始**，而不是仓库根，
就是为了避开这个。往上退到无权的一层会自动返回上一层并提示。

### 诊断命令

```bash
# 仓库根（注意是 repos-root-url，不是 repository-root）
svn info --show-item repos-root-url

# 复现 svnui 的调用方式：非交互 + 放行证书
svn list --xml --non-interactive \
  --trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other \
  "<URL>"
```

### 环境变量

| 变量 | 作用 |
|---|---|
| `SVNUI_TRUST_CERT=1` | 放行自签名 / 内网证书 |
| `SVN_CONFIG_DIR` | 指定 svn 配置目录（不搜 `~/.subversion`） |
<!-- END svnui:auth -->
## 安装

### Homebrew（推荐）

```bash
brew tap jc-p/svnui
brew trust jc-p/svnui      # 第三方 tap 需要显式信任，见「注意」
brew install svnui
```

升级：

```bash
brew upgrade svnui
```

### 从源码

```bash
git clone https://github.com/jc-p/svnui.git
cd svnui
cargo build --release
sudo cp target/release/svnui /usr/local/bin/
```

---

## 快速上手

```bash
cd 你的工作副本
svnui tui
```

> 子命令是必填的，直接敲 `svnui` 会报 clap 的参数错误。
> 嫌长可以加 alias：`alias s='svnui tui'`。

首次建议先跑自检：

```bash
svnui probe           # svn 路径 / 版本 / 工作副本根 / 能力开关
```

### 内网自签名证书（公司 SVN 常见）

如果报 `E230001: Server SSL certificate verification failed`：

```bash
export SVNR_TRUST_CERT=1          # 写进 ~/.zshrc 永久生效
# 或单次：svnui --trust-cert login -u 你的账号
```

等价于 svn 的
`--trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other`。

> ⚠️ 这等于放弃对中间人攻击的防护，只在确认是**自己的内网服务器**时启用。

---

## TUI 键位

### 文件树（主界面）

| 键 | 作用 |
|---|---|
| `j` / `k` 或 `↑` `↓` | 移动 |
| `Enter` | 目录展开/折叠；文件进全屏预览 |
| `Space` | **勾选/取消**（提交范围） |
| `Ctrl+A` / `Ctrl+R` | 全选可提交项 / 清空勾选 |
| `T` | 切换「仅变更 / 全部文件」 |
| `S` | 切换排序 |
| `/` | 搜索文件 |
| `R` | 刷新 |

只有**真正能提交**的文件（`M A D R C T !`）才显示复选框。
干净文件、未版本化、忽略项、目录留空——`svn commit` 本来就会跳过它们，
给它们画框只会让人以为勾上了就会提交。

### 操作

| 键 | 作用 |
|---|---|
| `C` | 提交（勾选范围） |
| `A` | `svn add` |
| `D` | `svn delete`（带确认） |
| `r` | `svn revert`（带确认，**不可撤销**） |
| `U` | `svn update` |
| `V` | 冲突面板 |
| `E` | 用外部编辑器打开（`$VISUAL` / `$EDITOR` / `vim`） |
| `H` | 工作副本体检 |
| `L` | 提交历史 |

### 提交框

| 键 | 作用 |
|---|---|
| `Enter` | **换行**（不是提交） |
| `Ctrl+S` / `F2` | 提交 |
| `Esc` | 退出，内容存草稿 |

退出提交框不会丢内容——再按 `C` 会恢复上次的草稿。
提交失败时草稿和勾选都会保留，按 `C` 继续即可。

### 预览区

| 键 | 作用 |
|---|---|
| `J` / `K` | 上下滚一行 |
| `Ctrl+d` / `Ctrl+u` | 翻页 |
| `Shift+←` `Shift+→` | 横向滚动 |
| `<` / `>` | 横向滚动 |

### 日志界面

| 键 | 作用 |
|---|---|
| `j` / `k` | 选择版本 |
| `Enter` | 查看该版本改动 |
| `/` | 搜索（Enter 确认走**服务端**搜索，能搜到已加载范围之外） |
| `N` | 加载更早一页（默认 100 条） |
| `Esc` | 返回 |

右栏分上下两块：上面是完整提交信息，下面是改动文件列表。

---

## 命令行

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
| `doctor` | 体检：锁 / 冲突 / 缺失 / 阻碍 / `.mine` 残留 |
| `conflicts` | 列出所有冲突 |
| `checkout` | 检出（唯一不需要先有工作副本的命令） |
| `login` / `auth` | 凭据验证 / 列出已缓存凭据 |
| `cache` | 查看状态缓存 / `--clear` 清除 |
| `daemon` | 常驻进程（`start` / `status` / `stop` / `rescan`） |

> `q` 和 `install-yazi` 是早期 yazi 集成阶段的遗留命令，代码还在，
> 但本项目已经转向独立 TUI，日常用不到。

### 危险操作分级

| 级别 | 操作 | 行为 |
|---|---|---|
| Safe | `status` `diff` `log` `info` | 直接执行 |
| Low | `add`、`cleanup`（仅解锁） | 直接执行 |
| Medium | `commit` `update` `resolve` `remove` | TTY 下确认；非 TTY **必须 `--yes`** |
| High | **`revert`**、`cleanup --remove-unversioned` | TTY 下确认；非 TTY **一律拒绝** |

`revert` 和 `cleanup --remove-unversioned` 会永久删掉你写了一半还没 add 的内容，
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

```toml
[features]
default = ["tui", "docs"]
tui     = ["dep:ratatui", "dep:crossterm", "dep:tui-textarea"]
docs    = ["dep:csv", "dep:calamine", "dep:docx-lite"]
```

| 场景 | 命令 |
|---|---|
| 完整 | `cargo build --release` |
| 只要 CLI（省掉 crossterm 一堆依赖） | `cargo build --release --no-default-features` |
| 不要办公文档预览 | `cargo build --release --no-default-features --features tui` |

`docs` feature 提供 CSV / Excel / Word 预览。关掉后这类文件会提示"请开启 docs feature"
而不是显示二进制乱码。

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

文档：`docs/cli.md`（命令参考）、`docs/keymap.md`、`docs/theme.md`、`docs/troubleshooting.md`。
发布流程见 `PUBLISH.md`。

---

## 注意：macOS 上的两个坑

**1. Gatekeeper**

通过 brew 安装的二进制带 quarantine 属性，首次运行会弹「无法打开，因为来自身份不明的开发者」：

```bash
sudo xattr -d com.apple.quarantine $(which svnui)
```

或者：系统设置 → 隐私与安全性 → 点「仍要打开」。
本地 `cargo build` 出来的没有这个问题，所以自己编译可能从来没遇到过。

**2. Homebrew 的 rust 装不了交叉编译 target**

如果机器上同时有 `/opt/homebrew/bin/cargo` 和 `~/.cargo/bin/cargo`（rustup），
`rustup target add x86_64-apple-darwin` 装进的是 rustup 那套，
而 `cargo build` 可能用的是 Homebrew 那套，于是报
`can't find crate for core` + 一句误导性的 `the target may not be installed`。

确认并修复：

```bash
which -a cargo                                    # ~/.cargo/bin 应该排第一
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
```

---

## License

MIT
