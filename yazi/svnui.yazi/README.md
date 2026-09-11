# svnui.yazi —— SVN 状态可视化与操作集成

把 `svnui`（Rust 写的 SVN 封装）接进 yazi：列表里一眼看出新增/修改/删除/冲突，
`<Tab>` 全屏看 diff，弹框完成提交/日志/冲突处理。

依赖：`svnui` 必须在 `PATH` 里（`cargo install --path .`）。

---

## macOS 必读：PATH 陷阱

brew 安装的 yazi 启动时，**PATH 里通常没有 `~/.cargo/bin`**，
而 yazi 的 `Command` 继承自己的环境 —— 于是 `Command("svnui")` 直接失败，
表现为"按 `vs` 没反应 + 一条 spawn 错误"。

两种解法，任选其一：

```bash
# 方案 A：装到 yazi 一定能找到的地方（推荐）
sudo cp target/release/svnui /usr/local/bin/
```

```lua
-- 方案 B：显式指定绝对路径
require("svnui"):setup { svnui_path = "/Users/you/.cargo/bin/svnui" }
```

验证：`which svnui` 有输出，且在 yazi 里按 `vs` 能弹出状态框。

## 文件结构

```
svnui.yazi/
├── main.lua      # 全部实现（单文件）
├── theme.toml
├── keymap.toml
└── README.md
```

### 改代码流程

```bash
$EDITOR yazi/svnui.yazi/main.lua   # 直接改
cargo build --release             # include_str! 是编译期展开，必须重编
svnui install-yazi --force
```

### 为什么是单文件

查了 yazi 官方插件仓库，**全部是单文件 main.lua**：

| 插件 | 文件 |
|---|---|
| git.yazi | main.lua（+ types.lua 仅给 LSP，运行时不 require）|
| diff.yazi | main.lua |
| full-border.yazi | main.lua |
| smart-filter.yazi | main.lua |

插件内部 `require("svnui.state")` 依赖 yazi loader 的未公开行为
（`Loader::normalize_id` 按第一个 `.` 切片成 插件名 + 入口名）。
**能跑不等于该用** —— 它是实现细节，跨版本不稳。

`main.lua` 内用 `-- ====...` 分隔条分区，按标题搜索即可跳转：
`JSON 解码器` / `state` / `util` / `diff` / `commit` / `log` / `rescue` / `入口`。

## 安装

```bash
# 1. 拷插件目录本身（不是它的父目录）
cp -r svnui.yazi ~/.config/yazi/plugins/

# 2. 在你的 ~/.config/yazi/init.lua 里启用
echo 'require("svnui"):setup {}' >> ~/.config/yazi/init.lua

# 3. 键位（见下）
```

> ⚠️ `require("svnui")` 加载的是 **`main.lua`**（yazi 的插件入口约定），
> 所以 `setup` 定义在 main.lua 里。这里没有 init.lua，也不需要。

## init.lua 配置

```lua
require("svnui"):setup {
  order = 1500,        -- Linemode 显示顺序，越大越靠右
  show_branch = true,  -- 状态栏右侧显示分支名 + 冲突计数
}
```

## 键位（建议：释放 `v` 作 version 前缀）

| 键 | 动作 | 说明 |
|---|---|---|
| `vs` | status | **刷新**状态 + 重绘 + 通知「已刷新：N 项变更」 |
| `vR` | refresh | 强制全量刷新（清掉 daemon 版本戳，忽略增量） |
| `vd` | diff | diff 弹框 |
| `vc` | commit | 提交（勾选面板） |
| `vl` | log | 日志 |
| `vb` | blame | blame |
| `va` | add | `svn add` |
| `vu` | update | `svn update` |
| `vr` | revert | `svn revert`（高危，二次确认） |
| `vk` | conflicts | 冲突处理 |
| `vx` | cleanup | 清理菜单 |
| `vh` | doctor | 工作副本体检 |

### `vs` 为什么是"刷新"而不是"打开状态弹框"

状态信息的最佳载体是**文件列表本身的行尾标记**和**状态栏**，
不是一次性的弹框。所以 `vs` 做三件事：

1. 刷缓存（spawn `svnui`）
2. `ui.render()` 让 yazi 重绘（linemode 回调只在重绘时被调用）
3. 用短通知回报结果

⚠️ 第 2 步不能省。只更新 Lua 侧的 map 而不重绘，界面上完全看不出变化，
表现就是"按了没反应"。

`vR` 与 `vs` 的差异是实打实的：`vR` 会把 `state.stamp` 清零，
强制 daemon 回全量（不用增量），状态可疑时用。

### 提交流程（两阶段，用 yazi 原生选中）

```
第一次 vc → 自动选中所有待提交文件（文件列表里高亮）
            通知「已选中 N 项。用 Space/v/ESC 调整后，再按 vc 提交」

            ← 这里能用 yazi 全部原生操作：
              Space 逐个切 / v 进 visual mode 批量选
              Ctrl+A 全选 / Ctrl+R 反选 / ESC 清空
              搜索结果里 v 全选

第二次 vc → 输入提交信息 → 只提交当前选中的文件 → 清空选中

vq         → 取消提交模式，清空选中
```

**为什么不用弹框做勾选**：

yazi **没有 `ui.Popup` 组件**，`ya.confirm` 只有 `[Y]es` / `(N)o` 两个按钮，
做不出能 ↑↓/Space 操作的面板。插件也拿不到弹框内的按键事件，自绘不可行。

但 yazi 自己有一套成熟的选中机制（visual mode、鼠标、搜索批量选），
**比自绘弹框强得多**。所以把"勾选提交范围"外包给它。

规则：

- `?`（未版本化）/ `I` / `X` **不会**被自动选中，也不会被提交
- 你已经有选中时，第一次 `vc` 会**尊重它**，直接进第二阶段（不会覆盖）
- 提交时按当前选中过滤，选了未版本化文件会被跳过并提示
- **commit 一定显式传路径**（`-- p1 p2`），不依赖"提交全部"
- 提交信息为空 → 取消，但**保留选中**（不用重新选一遍）
- 提交成功后自动清空选中 + 刷新 + 重绘

状态栏在提交模式下会显示：

```
 SVN 提交模式：Space/v 调整，vc 确认，vq 取消
```

## 预览器 / 探测器

```toml
# yazi.toml
[plugin]
prepend_spotters   = [ { url = "*", run = "svnui" } ]
prepend_previewers = [ { url = "*", run = "svnui" } ]
```

- **spotter**：`<Tab>` 全屏看 diff，可 `j/k` 滚动、`h/l` 切文件。着色用 `ui.Span`，一定生效。
- **previewer**：hover 即见 diff。**先查缓存做短路** —— 无状态的文件直接回落默认预览器，
  否则在源码目录里移动光标会一卡一顿。

## 状态符号与配色

| 符号 | 含义 | 颜色 |
|---|---|---|
| `A` | 新增 | 绿 |
| `M` | 修改 | 黄 |
| `_M` | 仅属性改动 | 黄（列表显示 `M`） |
| `D` `!` | 删除 / 缺失 | 红 |
| `C` | 文本或属性冲突 | 红 + 粗 |
| `T` | 树冲突 | 红 + 粗 |
| `R` `S` | 替换 / 切换 | 品红 |
| `X` `K` | 外部定义 / 锁令牌 | 蓝 |
| `?` `I` | 未版本化 / 忽略 | 深灰 |
| `~` | 类型阻碍 | 亮红 |

子目录变更会**向上冒泡**：父目录显示子树里最严重的那个标记。
优先级 `T/C(100) > !(90) > ~(80) > R(70) > D(60) > A(50) > M(40) > ?(20) > I(10) > X(5)`。

与 Rust 侧 `StatusKind::priority` 保持一致 —— 改任一侧都要同步另一侧。

## 要求

- **yazi ≥ 25.2.7**（`@since` 注解会拦截更老的版本）
- 已在 **yazi 26.9.1** 上核对过 API

## 官方原语说明

yazi **没有 `ui.Popup` 组件**（社区里的 Popup 都是插件自己用 `ui.Layout` 拼的）。
所以这里全部用官方原语：

| 用途 | 原语 |
|---|---|
| 展示长文本 / 确认 | `ya.confirm { pos, title, content }` —— 多行可滚动，自带 `k/j` 绑定 |
| 单行输入 | `ya.input { pos, title, value }` → `(value, event)`，`event==1` 为确认 |
| 按键菜单 | `ya.which { cands }` → **1-based** 索引 |
| 结果回执 | `ya.notify { title, content, level }` |

> ⚠️ `ya.confirm` 的字段是 **`content`**，不是 `body`。且它是纯文本，
> 不保证解析 ANSI —— 所以 diff 弹框不塞转义序列，改用 `+/-/@` 前缀区分。

## 排错

### 「提交能用，但列表里一个标记都没有、状态栏也没有 SVN」

**几乎一定是 init.lua 里缺了 setup。**

染色（Linemode）和状态栏（Status）**只在 `setup()` 里注册**，
而 `vc` / `vd` 这些动作走 `entry()`，不需要 setup。
所以会出现「功能都能用，但界面上一个标记都没有」的诡异现象。

确认：

```bash
grep svnui ~/.config/yazi/init.lua
# 必须有：require("svnui"):setup {}
```

没有就加（注意是 `init.lua` 不是 `keymap.toml`），然后**彻底退出 yazi 重启**。

插件会在每次动作时检测这点并弹提示；按 `vD` 也能看到 `setup 已调用 : false` 和该加什么。

### 「明明 init.lua 里有 setup，还是弹未初始化」

老版本的 bug，已修。原因是判定标记存在模块级变量里，
而 async 上下文每次都是**全新的 Lua 实例**（见下节），读到的永远是初始值。

现在这个标记存在 **plugin state**（Rust 侧持有，贯穿 yazi 生命周期），
跨上下文可靠。如果还在弹，说明你的 `main.lua` 是旧的 ——
检查文件里有没有 `get_field("setup_done")`：

```bash
grep -c 'get_field("setup_done")' ~/.config/yazi/plugins/svnui.yazi/main.lua   # 应 >0
```

### 「va/vc 报 W155010: '/wc/1' is not found」

路径变成了 `1` `2` `3` —— 这是文件**索引号**，不是路径。

原因是 `cx.active.selected` 的 `__pairs` **文档和实现顺序不一致**：

| 来源 | 返回值 |
|---|---|
| 官方文档（Context → tab::Selected） | `(integer, Url)` |
| yazi 26.9 实际 | `(Url, integer)` |

按文档写 `for _, u in pairs(sel)`，拿到的 `u` 就是 index。

修法不是猜顺序，而是**两个返回值都检查、按类型挑**（`selected_paths`）：

- index 一定是 `number` → 排除
- 路径一定是以 `/` 开头的 `string` → 只要它

`va` / `vu` / `vr` / `vc` 全部走这个 helper。已验证两种顺序都能正确取到路径。

### 为什么必须用 `ya.sync` 存状态

yazi 官方文档（Plugins → Async context）：

> When a plugin executes asynchronously, Yazi creates an isolated async
> context: **Created per invocation, destroyed after completion. Each
> execution has its own Lua state.**

也就是说：

| 执行位置 | 上下文 | Lua state |
|---|---|---|
| `init.lua` 里的 `setup()` | sync | **持久**，整个 yazi 生命周期 |
| `Linemode` / `Status` 回调 | sync | **持久**（同一份） |
| 按键触发的 `entry()` | async | **每次新建，跑完销毁** |

所以模块级 `local state = {}` 在 async 里是空壳 ——
按键刷新的数据写进去，调用结束就没了，染色的那个 `state` 完全不知情。

**唯一可靠的跨上下文存储是 plugin state**，即 `ya.sync(fn)` 回调的第一个参数。

### 其它检查

```bash
ls ~/.config/yazi/plugins/svnui.yazi/     # 应只有 4 个文件，无嵌套
which svnui                                # macOS 需要 /usr/local/bin 下
grep -c diagnose ~/.config/yazi/plugins/svnui.yazi/main.lua   # 应 >0，确认是新版
```

## 已知限制

- **换目录不会自动刷新**。yazi 没有稳定的 on-cd 钩子，
  进新目录后按一下 `vs`。
- **`ya.confirm` 超长内容会截断**（`MAX_CONFIRM_LINES = 500`）。
  完整 diff 用 `vd` 之外的 peek / spot。
- **预览区一次最多渲染 2000 行**（`MAX_PEEK_LINES`），
  超了截断并提示。每个 `ui.Line` 都是一次分配 + 布局计算，
  几万行会让 hover 明显卡顿。
- **`?` 文件的预览回落默认预览器**（显示文件内容 + 语法高亮），
  不在预览区重复画"未版本化"提示 —— 这个信息由行尾的 `?` 标记承担。
- **提交面板是 which 菜单不是复选框**（yazi 无 Popup 组件），
  一次最多列 9 个候选供切换，更多需要先 `va` 或分批提交。
- **diff 无缓存**：每次 hover 都 spawn 一次 `svn diff`。
  `svn diff` 在百毫秒级，通常无感；超大仓库可考虑开 daemon。

