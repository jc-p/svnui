# svnui CLI 参考

## 全局参数

| 参数 | 默认 | 说明 |
|---|---|---|
| `--cwd PATH` | 当前目录 | 工作目录 |
| `--json` | off | JSON 信封输出 |
| `--porcelain` | off | 两字符紧凑格式（status/log 生效） |
| `--timeout N` | 30 | 超时秒数 |
| `--no-cache` | off | 禁用状态缓存，每次真跑 svn |
| `--cache-ttl N` | 300 | 缓存有效秒数（严格校验已保证正确性，TTL 只是安全阀） |
| `-v, --verbose` | off | 详细日志到 stderr |

## 退出码

| 码 | 含义 |
|---|---|
| 0 | 成功 |
| 1 | 业务失败 |
| 2 | 参数错误（clap） |
| 3 | 不在 SVN 工作副本 |
| 4 | svn 未安装 |

## JSON 信封

```jsonc
{ "ok": true,  "data": { ... } }
{ "ok": false, "error": { "kind": "locked", "message": "..." } }
```

`kind` 稳定取值：`svn-not-found` `not-working-copy` `locked` `timeout` `cancelled` `svn-failed` `parse` `io`。

---

## 只读

### `svnui probe`
环境自检：svn 路径 / 版本 / 工作副本根 / 分支 / revision / 能力开关。

### `svnui status`
```bash
svnui status --porcelain          # "AM src/main.rs"
svnui status --changed            # 只显示有变更的
svnui status --conflicts          # 只显示冲突
svnui status --ignored            # 含被忽略的
svnui status -u                   # 连服务器查更新（慢，默认关）
```

### `svnui log`
```bash
svnui log --oneline -l 20         # r1234  alice  2026-09-09  修复xxx
svnui log -l 5                    # 完整格式（含正文）
svnui log -c 1234                 # 某次提交
```

### `svnui info`
仓库信息：root / url / branch / revision / uuid。

### `svnui diff`
```bash
svnui diff                        # 短 diff 直接打印，长 diff 自动分页
svnui diff --stat                 # 只显示统计
svnui diff -c 1234                # 某次提交的差异
svnui diff --page                 # 强制分页
```
分页器降级链：`delta` → `bat` → `less -R` → 直接打印，`SVNR_PAGER` 可覆盖。

### `svnui conflicts`
列出所有冲突，区分 **文本冲突 / 属性冲突 / 树冲突**。

### `svnui doctor`
六项体检：锁 / 冲突 / 缺失 / 阻碍 / `.mine` 残留 / 未版本化计数。
`--json` 有 `healthy` 字段。

---

## 写操作

**危险分级与确认行为**：

| 级 | 命令 | TTY | 非 TTY |
|---|---|---|---|
| Low | `add`、cleanup（仅解锁） | 直接执行 | 直接执行 |
| Medium | `commit` `update` `resolve` `remove` | 确认 | **必须 `--yes`** |
| High | `revert`、cleanup `--remove-*` | 确认 | **一律拒绝** |

> yazi 侧因为有 `ya.confirm` 自己的确认弹框，调 CLI 时**一律带 `--yes`**，
> 把确认交给 TUI 层做一次，不要让用户被问两遍。

```bash
svnui add PATH...
svnui remove [--keep-local] PATH...      # 默认同时删本地文件（svn 原生语义）
svnui revert [--dry-run] [--yes] PATH... # 高危
svnui commit -m "msg" [--dry-run] [--yes] [PATH...]
svnui update [-r REV] [--yes]
svnui resolve -a STRATEGY [--dry-run] [--yes] [PATH...]
```

`resolve` 的策略白名单：
`mine-full` / `theirs-full` / `working` / `base` / `mine-conflict` / `theirs-conflict`

> ⚠️ `svn revert` **没有** `--dry-run`。`--dry-run` 是我们自己实现的：
> 先打印"将要回滚"的清单，不调 svn。

---

## 救援

```bash
svnui cleanup                                  # 仅解锁，不删任何文件
svnui cleanup --vacuum-pristines               # 清 .svn/pristine（svn 1.10+）
svnui cleanup --remove-unversioned --dry-run   # 先看清单！
svnui cleanup --remove-unversioned             # ⚠️ 永久删除未版本化文件
```

`--remove-unversioned` 会删掉你刚写了一半还没 add 的新文件，
执行前**强制列出将被删除的清单**并二次确认。

---

## daemon（M9）

```bash
svnui daemon start     # 幂等
svnui daemon status
svnui daemon stop      # 优雅退出，落盘缓存
svnui daemon rescan    # 强制全量重扫
svnui q --dir src      # yazi 主入口，优先走 daemon
```

`svnui q --since N` 带版本戳，一致时只回几十字节的"无变化"。

---

## 缓存

```bash
svnui cache            # 查看：路径/大小/年龄/条目数/跟踪节点数
svnui cache --clear    # 删除
svnui cache --json
```

---

## yazi 插件

```bash
svnui install-yazi                 # 装插件
svnui install-yazi --check         # 只看会装到哪，不写盘
svnui install-yazi --force         # 覆盖
svnui install-yazi --patch-init    # 自动追加 require("svnui"):setup {}
svnui install-yazi --print-keymap  # 打印键位片段
```
