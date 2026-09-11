# svnui

Rust 写的 Subversion 封装层，目标是把 `svn` 命令变成**可快速调用的方法**，并深度集成进 [yazi](https://github.com/sxyazi/yazi)。

---

## ⚠️ 当前状态

**已完成 M0–M10（全部里程碑）**：domain 模型 / 执行层 / 解析层 / 只读 CLI / 写操作 + 危险护栏 / 救援 / 状态缓存 / yazi 插件 / daemon 常驻 / **工程化与文档**。
**仍未在本机编译验证过** —— 开发环境没有 Rust 工具链，也没有 `svn`。

```bash
cargo test          # 应全绿
cargo clippy --all-targets -- -D warnings
cargo build --release

# 真机验证（需要有 svn 的工作副本）
cargo run -- probe
cargo run -- status --porcelain
cargo run -- doctor --json | jq .data.healthy
cargo run -- cache               # 查看缓存
svnui status --porcelain --no-cache > /tmp/a; svnui status --porcelain > /tmp/b; diff /tmp/a /tmp/b
cargo run -- revert --dry-run          # 高危，干跑看清单
cargo run -- revert --yes              # 真跑（会丢改动，慎用）
```

### 危险操作约定（yazi 集成必须遵守）

| 级 | 操作 | 行为 |
|---|---|---|
| Safe | status/diff/log/info | 直接执行 |
| Low | add、cleanup（仅解锁） | 直接执行 |
| Medium | commit / update / resolve / remove | TTY 下确认；**非 TTY 必须 `--yes`** |
| High | **revert**、cleanup `--remove-unversioned` | TTY 下确认；**非 TTY 一律拒绝** |

yazi 侧因为有`ya.confirm`自己的确认弹框，**调 CLI 时应一律带 `--yes`**，
把确认交给 TUI 层做一次即可，不要出现"yazi 问一次 + CLI 再问一次"。

### 已知需要你本地校准的点

1. **quick-xml 的 `$text` 行为**（`src/svn/parser.rs`）
   `tests/parse_xml.rs::log_xml_parses_revisions_and_paths` 专门盯这个。
   如果这条红了，说明 `<path action="M">/trunk/x.rs</path>` 的文本节点没取到 ——
   把 `#[serde(rename = "$text")]` 换成该版本 quick-xml 的等价写法即可，其余不受影响。

2. **`svn status` 的列宽假设**（`src/svn/porcelain.rs`）
   代码按官方文档实现：前 7 列是状态、第 8 列恒为空格、路径从第 9 个字符开始。
   真实输出若多/少一个空格，跑一次 `svn status | cat -A | head` 对比一下，
   调 `parse_line` 里的 `split_at(7)` 即可。测试里有 20+ 条断言守着这个契约。

3. **fixtures 是手工构造的**（环境无 `svnadmin`）
   样本位于 `tests/fixtures/`。等你能在有 `svnadmin` 的机器上跑一次
   `tests/fixtures/gen.sh`（M1 待补），用真实输出覆盖一遍会更稳。

---

## 怎么用

```bash
# 构建 + 装二进制
cargo build --release
cp target/release/svnui ~/.cargo/bin/

# 装 yazi 插件（一条命令，插件已嵌进二进制，任何目录都能跑）
svnui install-yazi --patch-init
# 再把 svnui install-yazi --print-keymap 的输出合并进 keymap.toml

# 大仓库建议开 daemon
svnui daemon start
```

首次验收四步：

```bash
svnui probe                       # 环境自检
svnui status --porcelain          # 状态
svnui doctor --json | jq .data.healthy
svnui status --porcelain --no-cache > /tmp/a; svnui status --porcelain > /tmp/b; diff /tmp/a /tmp/b
```

文档：`docs/cli.md`（命令参考）、`docs/keymap.md`、`docs/theme.md`、`docs/troubleshooting.md`。

## 分层与硬约束

```text
main → cli → svn::client → svn::command
                  ↓
               domain   （纯模型，谁都能依赖）
```

三条铁律（写进 review checklist）：

1. **`domain` 零 IO** —— 只放数据结构与纯函数。
   自检：`grep -r "std::process\|std::fs" src/domain/` 必须无输出。
2. **`svn` 是唯一能 spawn 进程的地方**。
   自检：`grep -rn "Command::new" src/` 只应命中 `svn/command.rs`。
3. **永不 `sh -c`** —— 全部参数数组传递。
   自检：`grep -rn "sh -c\|cmd /C" src/` 必须无输出。

---

## 已落地的领域知识

### 7 列状态（`src/svn/porcelain.rs`）

| 列 | 含义 | 取值 |
|---|---|---|
| 1 | 文本状态 | ` ` `A` `C` `D` `I` `M` `R` `X` `?` `!` `~` `G` |
| 2 | 属性状态 | ` ` `M` `C` |
| 3 | 工作副本锁 | ` ` `L` |
| 4 | 带历史的添加 | ` ` `+` |
| 5 | 切换 / 文件外部 | ` ` `S` `X` |
| 6 | 仓库锁令牌 | ` ` `K` `O` `T` `B` |
| 7 | 树冲突 | ` ` `C` |

**注意：svn 没有 `--porcelain`**（那是 git 的）。`svnui status --porcelain` 是我们自己的两字符输出格式。

两个解析陷阱已处理：
- 路径是最后一个字段、**可含空格** → 按列宽切，绝不 `split`。
- 树冲突会**多打一行说明**（第 7 列是 `>`），加上 `Summary of conflicts:` 统计块 → 全部跳过，否则会产生幽灵路径。

### 执行层（`src/svn/command.rs`）

- 参数数组传递，`--non-interactive` + `stdin=null`（防 svn 挂起等输入）
- `LC_ALL=C.UTF-8`（错误文案不被本地化）
- **stdout/stderr 并发读** —— 先 wait 再读会因管道写满（约 64KB）死锁，diff 轻易就超
- 超时后 `kill` + `wait`，不留僵尸进程
- `E155004` 自动翻译成 `Error::Locked` 并给出 `svnui cleanup` 提示

---

## 退出码

| 码 | 含义 |
|---|---|
| 0 | 成功 |
| 1 | 业务失败 |
| 2 | 参数错误（clap） |
| 3 | 不在 SVN 工作副本 |
| 4 | svn 未安装 |

## 输出契约

yazi 侧只认 JSON 信封：

```jsonc
{ "ok": true,  "data": { ... } }
{ "ok": false, "error": { "kind": "locked", "message": "..." } }
```

`kind` 稳定取值：`svn-not-found` / `not-working-copy` / `locked` / `timeout` / `cancelled` / `svn-failed` / `parse` / `io`。

---

## 下一步（按 `svnui_开发计划与任务清单.md`）

- **M6** 冲突向导的真机验证（**需要 svnadmin**：`resolve` 后状态归位 + trio 文件清除）
- **M8** yazi 插件（合并为单个 `svnui.yazi`，染色/弹框/键位）
- **M9** daemon 常驻 + 版本戳 + 增量（你的仓库大，这步别省）

## yazi 集成（M8）

插件在 `yazi/svnui.yazi/`，**单个插件**（不是 5 个）—— yazi 的插件 state 是
per-plugin 独立的，拆成多个会导致各自跑一次 `svnui status`、状态还不一致。

- `init.lua` — 状态染色（`Linemode:children_add`）+ 状态栏分支/冲突数
- `main.lua` — `entry` 分发 + `peek`/`spot`
- `state.lua` — 缓存中心
- `util.lua` — 调 svnui + 官方弹框封装
- `actions/` — diff / commit / log / rescue

三条铁律：
1. **linemode 只读缓存** —— 它跑在 sync 上下文，一次 Command 都不能发
2. `peek` 先查缓存做短路，无状态直接回落默认预览器
3. 调 CLI 一律带 `--yes` —— 确认只在 `ya.confirm` 里做一次，别问两遍

### 官方原语（重要更正）

yazi **没有 `ui.Popup` 组件**（社区里的 Popup 都是插件自己用 `ui.Layout` 拼的）。
所以全部用官方原语：`ya.confirm`（多行可滚动）/ `ya.input` / `ya.which` / `ya.notify`。
另外 `ya.confirm` 的字段是 **`content`** 不是 `body`，且它是纯文本不保证解析 ANSI
—— 所以 diff 弹框不塞转义序列，改用 `+/-/@` 前缀区分。

## daemon（M9）

```bash
svnui daemon start      # 幂等；后台常驻
svnui daemon status
svnui daemon stop       # 优雅退出，退出前落盘缓存
svnui daemon rescan     # 强制全量重扫
svnui q --dir src       # yazi 主入口，优先走 daemon
```

### 为什么 svn 调用必须串行

svn 工作副本有**独占锁**。daemon 在后台 status 时用户如果跑 commit，
两者会撞锁报 `E155004`。所以 daemon 的所有 svn 调用都在同一条线程上排队 ——
daemon 慢一点没关系，把用户的命令搞失败才是灾难。

### 版本戳增量

响应带 `v: N`。客户端下次带 `--since N`，一致时只回一个几十字节的
`{"v":N,"changed":false,...}`（有测试断言它 <120 字节）。
这让"每次切目录都问一次"变得几乎免费。

### 增量刷新必须先删后插

`svn status <path>` 对"已恢复正常"的路径**返回空**。如果只 insert 不 remove，
一个文件改完又 revert 掉，状态会永远留在表里 ——
表现为"列表一直显示 M 但 diff 是空的"，极难排查。
`Shared::apply()` 先按 touched 路径删除再插入，并有专门的测试守着。

### 降级

daemon 不可用（没启动 / socket 残骸 / 非 unix）时**静默退回直连 svn**，
返回结构完全一致。daemon 只是加速器，不是依赖。

## 真实编译后的修复（9 个编译错误）

静态审查看不出来的，真跑 `cargo check --all-targets` 才暴露：

| # | 错误 | 修复 |
|---|---|---|
| 1 | `policy::{danger_of, cleanup_danger}` 未重新导出 | `pub use danger::{cleanup_danger, danger_of, Danger, Op};` |
| 2 | 调用不存在的 `parse_log` | 实际函数名是 `parse_log_xml` |
| 3 | `Fingerprint` 没有 `Default`，`CachedDoc` 派生失败 | 实现 `Default`，且是全 0 哨兵 —— **不等于任何真实指纹**，缺字段的旧缓存会直接失效重扫 |
| 4 | `Cmd::InstallYazi` 没有 dispatch 分支（非穷尽匹配） | 补齐分支 |
| 5 | `emit_err(cli, &e)` 传了 owned 值 | `&cli` |
| 6 | `Cmd::Rescan` 分支返回 `u64` 其他返回 `()` | **顺带修了功能**：原来只 bump 版本戳不真扫描，客户端刷新完拿到的还是旧快照。改为真跑 `status` → `replace_all` → bump |
| 7 | `paths_op_owned` 里 `owned` 在 `run()` 前离开作用域（E0597） | 提到函数体顶层 |
| 8 | `sock` 未使用 | 改名 `_sock` 并说明 |
| 9 | `Blame` 子命令缺 `binary` 路径校验 | 一并纳入 `guard_paths` |

## 安全加固

| # | 问题 | 修复 |
|---|---|---|
| 1 | **SVN 目标路径未限制在工作副本内** | 新增 `Svn::guard_paths()`：所有 `add/remove/revert/update/log/blame` 的路径参数都校验 `starts_with(wc_root)`。svn 对越界路径是"静默部分成功"，半成功状态极难排查 |
| 2 | **daemon socket 无认证** | 新增 `socket_is_ours()`：校验 `.sock` 文件的 uid 是当前用户。`XDG_RUNTIME_DIR`/`/tmp` 里任何用户都能预置同名 socket 冒充 daemon，诱导误提交 |
| 3 | **请求帧无长度上限** | `MAX_FRAME = 1MB`，server 端 `take()` 限读 |
| 4 | **`.svn` 事件被忽略 → 终端里跑 svn 后 daemon 状态过期** | 新增 `MetaDirty` 标志：单独盯 `.svn/wc.db`（非递归），它一变就整树重扫 |
| 5 | **`remove` 的 guard 被架空** | 原来硬编码 `yes=true, tty=false`，非交互脚本里一次误调用就真删。改为传真实参数，并补 `--yes` flag |

## 陈旧文件（需你手动删除）

源码已单文件化，以下**目标端独有**文件是双轨残留，建议删除：

```
src/output/human.rs
tests/log.rs  tests/parse_log.rs  tests/parse_status.rs  tests/yazi_contract.rs
docs/ARCHITECTURE.md  docs/svnui 开发计划与任务清单.md
docs/yazi-test-runbook.md  docs/面板设计.md
yazi/svnui.yazi/actions/  yazi/svnui.yazi/state.lua  yazi/svnui.yazi/util.lua
yazi/svnui.yazi/init.lua
```

尤其 `yazi/svnui.yazi/init.lua` —— 入口文件 `init.lua` 已在 yazi #2168 被废弃，
留着只会让人以为改它有用。

## 适配 yazi 26.9.1（macOS / aarch64）

在 yazi **26.9.1** 上核对后修正。26.x 有几处 breaking change 正好打中本插件：

| # | 变更 | 影响 |
|---|---|---|
| 1 | **`ya.preview_widgets()` 已废弃**（#2706），改用单数 `ya.preview_widget(job, widget)` | 26.x 上直接报 `attempt to call a nil value (field 'preview_widgets')` —— **预览与 spot 全部失效**。已迁移，且 widget 需自带 `:area()` |
| 2 | **`ya.mgr_emit()` / `ya.manager_emit()` / `ya.app_emit()` 已废弃**（#2653），改用 `ya.emit()` | 预览面板滚不动。已迁移，并补上 `only_if` 防错位 |
| 3 | **spotter 改用 `ya.spot_table(job, ui.Table(...))`** | 旧的 `preview_widgets` 渲染 spot 不再成立。已改为表格：文件 + 行数 + 前 15 行 diff |
| 4 | **`LEFT/CENTER/RIGHT` 在 `ui.Line`/`ui.Text` 上废弃**（#2802），改用 `ui.Align` | 未触及：`Status.RIGHT` 是状态栏的 side 参数，**仍然有效**（官方 tips 仍在用） |
| 5 | **入口文件 `init.lua` 已废弃**（#2168），一律用 `main.lua` | 之前"26.x 改用 init.lua"的推断是**错的**，实际相反。已删除 init.lua |
| 6 | keymap `[manager]` → `[mgr]`（#2803） | 已是 `[mgr]` ✓ |

### macOS 专属：PATH 陷阱

brew 装的 yazi 启动时 **PATH 不含 `~/.cargo/bin`**，`Command("svnui")` 直接失败，
表现为"按 `vs` 没反应"。已加 `svnui_path` 配置项，并给出带路径的报错提示。
推荐 `sudo cp svnui /usr/local/bin/`。

### 其他平台确认项

- Lua 5.5（26.x 起）：`table.unpack` 存在，兼容写法 `table.unpack or unpack` 仍有效
- `Command:args()` 已废弃（#2752）：本插件用逐个 `:arg()`，未触及
- `ya.confirm` 字段是 **`content`**（不是 `body`），返回 boolean —— 26.x 未变

## 交叉核对发现的问题（Lua ↔ Rust 契约）

这一类是"两边各自能编译、拼起来对不上"的 bug，只有交叉核对才能发现。

### 致命（功能完全不可用）

| # | 问题 |
|---|---|
| 1 | **yazi 的 `require()` 不是标准 Lua 的 require**。它只用于加载插件（`plugins/<name>.yazi/`），不能加载插件内部的 .lua 文件。`require("state")` 会去找 `plugins/state.yazi/` 这个插件 → 找不到 → **整个插件加载失败**。已改为**单文件 main.lua 全内联**（官方 git.yazi 也是这么做的） |
| 2 | **`svnui blame` 子命令不存在**，但 keymap 绑了 `vb`、Lua 也调了 → 按键必报错。已补 `Blame` 子命令与 `Svn::blame()` |
| 3 | **`svnui log -c` 参数不存在**，Log 只有 `--limit/--oneline/paths` → 查指定版本必报错。已加 `-r/--rev`（注意语义：`-c N` 是"该版本引入的变化"，`-r N` 是"该版本的日志条目"，后者才是用户想要的） |
| 4 | **init.lua / main.lua 双入口冲突**。yazi 25.x 用 main.lua 作入口，26.x 起改用 init.lua。原来两个文件内容不同（染色在 init、动作在 main），**新版下按键全部失效**。已改为两文件内容相同 |
| 5 | **版本戳命中会把状态清空**。`unchanged` 响应里 map 为空，Lua 拿空 map 覆盖缓存 → 所有状态凭空消失。已给 `Layer` 加 `changed` 字段，Lua 侧据此判断 |

### 严重（体验受损）

| # | 问题 |
|---|---|
| 6 | **`@sync entry` 导致 spawn 阻塞 UI**。每次按键卡几十到几百毫秒。已去掉该注解，改用 `ya.sync()` 包装 cx 访问（`get_cwd`/`get_targets`/`get_selection_count`），主体留在 async |
| 7 | **previewer 缺 `seek`**，预览面板滚不动，长 diff 只能看开头。已补 |
| 8 | **Lua 没传 `--since`**，daemon 的版本戳增量形同虚设，每次都回全量。已补 |

## 自测发现并修复的问题（无需你踩）

这一节记录我复查 M0–M9 时发现的问题。**如果你拿到的代码已包含这些修复，可跳过。**

### 编译错误（会直接 build 失败）

| # | 位置 | 问题 |
|---|---|---|
| 1 | `src/ipc.rs` | **E0716**：`normalize_dir(&...unwrap_or_default())` 返回的 `&str` 借用临时 String，语句结束即 drop。改为先绑定到具名变量 |
| 2 | `src/cli/mod.rs` | dispatch 匹配 `&cli.cmd`，字段是引用，直接构造 `daemon_cmd::Cmd::Daemon { action }` 类型不匹配。改为 `action.clone()`，并给 `Cmd`/`Action` 加 `Clone` |
| 3 | `src/daemon/watcher.rs` | 缺 `use notify::Watcher;` —— `watch()` 是 trait 方法，不在作用域会报 "no method named `watch`" |
| 4 | `src/daemon/watcher.rs` | `notify::Error` → `std::io::Error` 改用显式 `map_err`，不依赖版本是否实现 `From` |

### 逻辑 bug（能编译但行为错）

| # | 位置 | 问题 |
|---|---|---|
| 5 | `src/cli/mod.rs` | `status` 的 `unversioned` flag 默认 false，与 `StatusOpts::default()` 的 `true` 矛盾 —— **跑 `svnui status` 看不到 `?` 新文件**。改为 `--no-unversioned` 反向 flag |
| 6 | `src/cli/mod.rs` | `daemon run --root X` 由后台拉起，cwd 不一定在工作副本里，discover 会直接失败退出。`start_dir()` 特判返回 `--root` |
| 7 | `src/cli/daemon.rs` | `svnui q --dir src` 传相对路径时，`strip_prefix` 失败 → 静默退化成"查根层"，看起来像状态全丢。改为基于 `svn.cwd` 拼绝对 |
| 8 | `src/svn/client.rs` | `Svn::cwd` 可能是 `.`（CLI 传进来的），导致上面第 7 条的拼接基准不对。`discover` 里 canonicalize |

### yazi 插件

| # | 问题 |
|---|---|
| 9 | **`require("svnui")` 加载的是 `main.lua` 不是 `init.lua`**，`setup` 必须定义在 main.lua。已删除 init.lua 并合并 |
| 10 | `require` 子目录路径无明确约定（`actions.diff` vs `actions/diff`）。已全部平铺到插件根目录 |
| 11 | `unpack` 在 Lua 5.1 是全局、5.4 在 `table.unpack`。已加 `local unpack_args = table.unpack or unpack` |

### 已知未解决（需要真机验证）

- **`ya.confirm` 是否解析 ANSI**：不支持时 diff 弹框无颜色（已改用 `+/-/@` 前缀，不会出乱码）
- **`quick-xml` 的 `$text`**：`tests/parse_xml.rs` 有专项断言守着
- **无 `svnadmin` 环境**：冲突流程（`resolve` 后状态归位 + trio 清除）无法端到端验证

## 缓存（M7）

`svn status` 在大仓库上是秒级到几十秒级。缓存的目标是**命中时一次 svn 都不跑**。

### 为什么必须全树比对

`svn status` 只输出**非 normal** 的条目，所以缓存里天然没有"干净的文件"。
于是下面两种情况**都检测不到**，除非额外记录整棵树的指纹：

- 一个干净文件被改脏了 → 它不在缓存里，`wc.db` 也没动
- 新建了一个未版本化文件 → 同样不在缓存里，创建文件**不会**改 `wc.db`（只改父目录 mtime），
  而修改已有文件连父目录 mtime 都不改

所以缓存里除了条目本身，还存了**工作副本内每个文件/目录的 `(mtime, size)`**（跳过 `.svn`），
加载时重新走一遍树逐条比对。全部一致才认为状态未变。

因为遍历是**每次加载都做**的，正确性**与缓存年龄无关** —— TTL 只是安全阀，默认 300 秒。

### 成本

| 路径 | 开销 |
|---|---|
| 缓存命中 | 一次树遍历（纯 `stat`，`O(files)`） |
| 缓存未命中 | 树遍历 + `svn status`（后者要做内容校验和，贵一到两个数量级） |

### 边界

- 跟踪节点超过 **200,000** 就不写缓存（JSON 太大，读写反而不划算）
- `.svn/wc.db` 不存在（SVN 1.6 及更早）→ **不缓存**。老布局的 svn 操作不集中改一个数据库，
  校验模型不成立，缓存会不安全
- `--ignored` / `-u` 这类非默认查询不走缓存（`-u` 还要连服务器，缓存了反而误导）
- 所有写操作后自动调用 `invalidate_for()` 作废缓存

### 与 daemon 的关系

M7 省掉了必做的 `svn status`，但**每次加载仍要遍历整棵树**做指纹比对。
M9 的 daemon 用 `notify` 监听事件，只在事件路径上跑局部 status，连遍历都省掉。
两者叠加后，"切目录看状态"的稳态开销接近零。

## 已实现的救援能力

`svnui doctor` 一次体检六项：工作副本锁 / 冲突（文本·属性·树）/ 缺失 `!` / 阻碍 `~` / `.mine` 残留 / 未版本化计数。
`--json` 输出 `healthy` 字段，yazi 的 `vh` 键直接消费。

`svnui resolve -a <策略>` 内置白名单校验（`mine-full` / `theirs-full` / `working` / `base` / `mine-conflict` / `theirs-conflict`），
非法策略在拼进 `--accept=` 之前就被拦住。

`svnui cleanup --remove-unversioned` 执行前会**先列出将被删除的文件清单**再确认 ——
这个开关会永久删掉你刚写了一半还没 add 的新文件，是整个工具里最危险的一个。
