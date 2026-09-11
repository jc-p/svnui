# 配色

## 状态符号与语义

| 符号 | 含义 | 默认色 |
|---|---|---|
| `A` | 新增（已 add，待提交） | 绿 |
| `M` | 内容修改 | 黄 |
| `_M` | **仅属性改动**（列表显示 `M`） | 黄 |
| `D` | 已删除（svn rm） | 红 |
| `!` | 缺失（被非 svn 命令删了 / 目录不完整） | 红 |
| `C` | 文本或属性冲突 | 红 + 粗 |
| `T` | **树冲突**（最麻烦的一类） | 红 + 粗 |
| `R` | 替换（删除后原地新增） | 品红 |
| `S` | 切换到别的分支 | 品红 |
| `X` | 外部定义（svn:externals） | 蓝 |
| `K` | 持有锁令牌 | 蓝 |
| `?` | 未版本化 | 深灰 |
| `I` | 被忽略 | 深灰 |
| `~` | 类型阻碍（文件↔目录 被替换） | 亮红 |

## theme.toml

把 `yazi/svnui.yazi/theme.toml` 的内容合并进你的 `~/.config/yazi/theme.toml`：

```toml
[svn]
added        = { fg = "green" }
modified     = { fg = "yellow" }
deleted      = { fg = "red" }
conflict     = { fg = "red", bold = true }
tree_conflict = { fg = "red", bold = true }
replaced     = { fg = "magenta" }
switched     = { fg = "magenta" }
external     = { fg = "blue" }
locked       = { fg = "blue" }
unversioned  = { fg = "darkgray" }
ignored      = { fg = "darkgray" }
obstructed   = { fg = "lightred" }
branch       = { fg = "cyan" }
```

> ⚠️ `init.lua` 里**硬编码了一份同样的配色**（不依赖 theme.toml）。
> 这是刻意的：颜色错了只是难看，但配色段缺失导致 Lua 报错才是灾难。
> theme.toml 这一份的作用是**让你能覆盖它**。改任一侧都要同步另一侧。

## 冒泡规则

子目录的变更会**向上冒泡**到父目录，显示子树里最严重的那个标记。

优先级（与 Rust 侧 `StatusKind::priority` 一一对应）：

```
T/C(100) > !(90) > ~(80) > R(70) > D(60) > A(50) > M(40) > G(35) > ?(20) > I(10) > X(5)
```

**为什么 Added > Modified**：新增文件还没进版本库，丢了就真没了，比修改更需要提醒。

如果你更习惯 `M` 优先（修改更常见、更需要 review），改 `src/domain/status.rs`
的 `priority()` 与 `yazi/svnui.yazi/init.lua` 的 `PRIORITY` 表即可 ——
**两边必须同时改**，否则列表显示和内部排序会不一致。

## 状态栏

右侧显示：分支名（青色）+ 冲突计数（红色粗体，如 ` !3`）。

无信息时返回空串 —— 状态栏每帧都渲染，返回空串比返回空格更省。

在 `setup { show_branch = false }` 可关闭。
