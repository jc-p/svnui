# 故障排查

## 染色不生效

**症状**：按 `vd` 能弹 diff，但列表里看不到 `M`/`A`/`D` 标记。

**原因**：`main.lua` 只在按键触发时加载，**linemode 必须在 `init.lua` 里注册**。

```bash
# 检查
grep svnui ~/.config/yazi/init.lua
# 没有就加
echo 'require("svnui"):setup {}' >> ~/.config/yazi/init.lua
```

或者重跑 `svnui install-yazi --patch-init`。

---

## 插件报错 / 状态栏空白

**症状**：yazi 启动时报 Lua 错误。

**排查顺序**：

1. 插件目录层级对不对 —— 必须是
   `~/.config/yazi/plugins/svnui.yazi/init.lua`，
   **不是** `plugins/svnui.yazi/svnui.yazi/init.lua`（多拷了一层是常见错误）。
2. `svnui` 在 PATH 里吗 —— `which svnui`
3. 版本够吗 —— 插件顶部的 `--- @since 25.2.13` 会阻止在更老版本上加载

```bash
svnui install-yazi --check    # 看会装到哪
svnui probe                   # 验证 svnui 自身
```

---

## 大仓库卡顿

**症状**：滚列表一卡一顿，或进目录要等好几秒。

**按顺序排查**：

```bash
# 1. 基线：svn status 本身多慢？
time svn status > /dev/null

# 2. 缓存命中后多快？（应快一个数量级）
time svnui status --porcelain    # 首次，写缓存
time svnui status --porcelain    # 二次，命中

# 3. 如果二次还是慢 → 树遍历成瓶颈，上 daemon
svnui daemon start
time svnui q --dir .             # 应 <10ms
```

**如果 daemon 也慢**：说明仓库文件数极大（几十万+），
`notify` 的事件风暴会让增量刷新反而变慢。这时：

```bash
svnui daemon stop
svnui status --porcelain --no-cache   # 退回直连
```

并把 `state.lua` 的 `M.ttl` 调大（默认 5 秒），减少刷新频率。

---

## 状态显示不对

**症状**：列表显示 `M`，但 diff 是空的。

**这是已知的最阴的一类 bug**，两种可能：

1. **daemon 的增量刷新没删干净**。
   `svn status <path>` 对"已恢复正常"的路径返回空，只 insert 不 remove 就会残留。
   `Shared::apply()` 已处理并有测试守着 —— 如果仍复现，跑：
   ```bash
   svnui daemon rescan     # 强制全量重扫
   ```

2. **缓存指纹失配**。跑 `svnui cache --clear` 再试。

**验证一致性**（这是 M7 的完成判据）：
```bash
svnui status --porcelain --no-cache > /tmp/a
svnui status --porcelain              > /tmp/b
diff /tmp/a /tmp/b    # 必须为空
```

---

## 树冲突（T）

SVN 独有的一类麻烦：本地删除/移动 vs 远端修改。
比文本冲突难处理，**`theirs-full` 可能丢整个目录的本地改动**。

```bash
svnui conflicts              # 看清单，区分文本/属性/树
svnui doctor                 # 体检
```

yazi 里按 `vk`，选策略时会**对 `theirs-full` 二次确认**。

---

## 工作副本被锁

**症状**：报 `E155004` 或 "Working copy locked"。

```bash
svnui cleanup               # 仅解锁，不删任何文件
svnui doctor                # 确认
```

通常是上次操作被中断（Ctrl-C、崩溃）导致。

---

## 提交被拒绝

**症状**：`svnui commit` 在非交互环境报"需要显式 --yes"。

这是**设计如此**：中危及以上操作在脚本/CI 里必须显式确认。

```bash
svnui commit -m "msg" --yes
```

yazi 侧已自带 `--yes`（确认在 `ya.confirm` 里做过一次了）。

---

## daemon 起不来

```bash
svnui daemon status         # 看状态 + socket 路径
```

常见原因：

- **socket 残骸**：上次异常退出留下文件。`is_running` 会真连一次，
  所以残骸不会误判为"运行中"，但 `start` 会自动清理后重 bind。
- **非 unix 平台**：UDS 不存在，daemon 不支持，自动降级为直连 svn。
- **inotify 句柄耗尽**：`watch()` 失败。不影响服务，只是退化成靠 TTL/手动 rescan。

```bash
# 强制重来
svnui daemon stop
rm -f /tmp/svnui-*.sock /tmp/svnui-*.pid    # 或 $XDG_RUNTIME_DIR 下
svnui daemon start
```

---

## svn 未安装 / 找不到

```bash
svnui probe
```

提示里会给出安装命令。或用 `SVNR_SVN=/path/to/svn` 指定。

---

## 解析异常

**症状**：状态里出现不该有的路径（"幽灵路径"）。

`svn status` 在树冲突时会**多打一行说明**（第 7 列是 `>`），
还有 `Summary of conflicts:` 统计块。这些行如果没被跳过就会产生假路径。

`porcelain.rs` 有 20+ 条断言守着这个契约。若复现：

```bash
svn status | cat -A | head -40    # 看真实列位
```

确认列宽后调 `parse_line` 里的 `split_at(7)`。
换 svn 版本后建议跑 `just gen-fixtures` 重新校准样本。
