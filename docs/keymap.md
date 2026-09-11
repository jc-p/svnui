# yazi 键位配置

## 为什么用 `v` 前缀

`v` 是 **version** 的语义，与 `g`（git）在同类插件里的用法一致，
且所有 SVN 操作都以 `v` 开头，肌肉记忆只需记一个字母。

代价是 `v` 默认被 yazi 的 **visual mode** 占用，需要先挪走：

```toml
{ on = "<C-v>", run = "visual_mode",         desc = "Visual mode (selection)" },
{ on = "<C-V>", run = "visual_mode --unset", desc = "Visual mode (unset)" },
```

## 完整键位

合并进 `keymap.toml` 的 `[mgr]` 段（用 `prepend_keymap` 保证不被覆盖）：

```toml
[mgr]
prepend_keymap = [
  { on = "<C-v>", run = "visual_mode",         desc = "Visual mode (selection)" },
  { on = "<C-V>", run = "visual_mode --unset", desc = "Visual mode (unset)" },

  { on = [ "v", "s" ], run = "plugin svnui -- status",    desc = "SVN 状态" },
  { on = [ "v", "d" ], run = "plugin svnui -- diff",      desc = "SVN diff" },
  { on = [ "v", "c" ], run = "plugin svnui -- commit",    desc = "SVN 提交" },
  { on = [ "v", "l" ], run = "plugin svnui -- log",       desc = "SVN 日志" },
  { on = [ "v", "b" ], run = "plugin svnui -- blame",     desc = "SVN blame" },
  { on = [ "v", "a" ], run = "plugin svnui -- add",       desc = "svn add" },
  { on = [ "v", "u" ], run = "plugin svnui -- update",    desc = "svn update" },
  { on = [ "v", "r" ], run = "plugin svnui -- revert",    desc = "svn revert" },
  { on = [ "v", "k" ], run = "plugin svnui -- conflicts", desc = "冲突处理" },
  { on = [ "v", "x" ], run = "plugin svnui -- cleanup",   desc = "清理菜单" },
  { on = [ "v", "h" ], run = "plugin svnui -- doctor",    desc = "工作副本体检" },
  { on = [ "v", "R" ], run = "plugin svnui -- refresh",   desc = "强制刷新状态" },
]
```

## 可选：直绑加速键

这几个键 yazi 默认未占用，可跳过 `v` 前缀直接触发：

```toml
  { on = "b", run = "plugin svnui -- blame",  desc = "SVN blame" },
  { on = "u", run = "plugin svnui -- update", desc = "svn update" },
  { on = "C", run = "plugin svnui -- commit", desc = "SVN 提交" },
```

## 预览器 / 探测器

```toml
# yazi.toml
[plugin]
prepend_spotters   = [ { url = "*", run = "svnui" } ]
prepend_previewers = [ { url = "*", run = "svnui" } ]
```

| 入口 | 行为 |
|---|---|
| **spotter** `<Tab>` | 全屏看 diff，`j/k` 滚动、`h/l` 切文件。**着色一定生效**（用 `ui.Span`） |
| **previewer** | hover 即见 diff。先查缓存短路，无状态直接回落默认预览器 |

## 已被 yazi 占用的键（别去抢）

`s`(fd) `d`(trash) `c`(copy) `l`(enter) `a`(create) `r`(rename) `v/V`(visual)
`m` `f` `g` `w` `z` `h`(leave) `j/k` `p` `y` `x` `o` `O` `n` `N` `.` `,` `~` `-` `+`

## 关于 `noop`

如果你的 yazi 版本不支持 `noop`，删掉相关行即可 ——
本项目没有依赖 `noop` 的键位（前缀键 `v` 是靠多键序列实现的，不需要 `noop`）。
