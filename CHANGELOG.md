# 更新日志

本文件从 **0.2.5 起正式维护**。更早的提交信息不规范（大量 `初始化`、`fix`），
历史条目只能事后整理，可能与 `git log` 对不上 —— 以 `git tag` 为准。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

---

## [Unreleased]

## [0.3.0]

### Added
- 远端仓库浏览面板（`V`）：不检出即可浏览 SVN 仓库目录，每项带 r号/作者/日期
- 面板内 `o` 检出选中目录，URL 自动填入检出框；`Y`/`Shift+Y` 复制路径带可见反馈
- 检出进度：spinner + 滑动色块 + 计时 + 真实条目数（流式读取 svn 输出）
- 统一 loading 组件（`panels/loading.rs`）：顶栏单行 / 整屏居中块两种形态
- 不带子命令时默认进入 TUI；非交互环境改为提示而非卡住
- README「SVN 认证配置」章节：证书、凭证缓存、错误码对照表
- CHANGELOG.md 从本版本起正式维护，附提交规范与 `scripts/changelog.sh`

### Fixed
- 检出输入框内容不可见：字段仅 2 行，被上下边框占满，内容区高度为 0
- 检出面板无法粘贴：`handle_paste` 只认提交框，其余模式静默忽略
- 输入框无可见光标，四框全空时无法判断焦点
- TUI 键盘无响应：鼠标捕获默认开启 + 主循环每帧只读一个事件，键盘被鼠标事件饿死
- 按键 `kind` 只认 `Press`，支持 ModifyOtherKeys 的终端按键被静默丢弃
- `R` 键重复绑定，重新扫描成为死分支（已移至 `F5`）
- 仓库面板起点用仓库根，按子树授权的仓库报 forbidden（改用工作副本 URL）
- `E175013`（无权访问）被归入「连不上服务器」，提示方向错误
- CLI `log`/`diff`/`blame` 空结果时零输出且退出码 0
- `diff --page` 双重静默：空内容时 pager 不启动且返回空串
- 破坏性操作护栏返回空串的死分支，改为明确提示「未执行」
- 编译修复：`Mode::Repo` 变体与 `App.repo` 字段缺失；`repo.rs` 借用冲突
- 检出面板 URL 双重编码（中文路径 `%E5%8D%87` 被二次编码成 `%25`）

### Changed
- 横向滚动精简为两套键：`←/→` 慢移、`< / >` 翻页（3/4 屏宽）；移除 `Shift+←/→`
- 鼠标默认不捕获（`M` 键开启），恢复终端文本选择复制
- 冲突解决面板 `V` → `X`；检出键收敛为 `o`
- 选中文件时底栏提示「仅对目录有效」，不再静默无反应
- 检出输入框文字改白色、聚焦加粗；面板加外层边框
- CLI `checkout` 增加 stderr 进度提示，不再静默等待

### Performance
- 目录缓存上限 32 层 FIFO 淘汰
- 流式 stdout 累积上限 2MB；单行读取上限 64KB
- `svn list --xml` 解析前 8MB 闸门
- 排序改 `sort_by_cached_key`，避免每次比较现算 lowercase
---

## 历史版本

> 以下为事后整理，细节以 `git log` 为准。
> 补齐方式：`scripts/changelog.sh v0.2.0 v0.2.4`

### [0.2.4]

- 仓库浏览面板的初步实现与后续修复

### [0.2.2]

- 中间版本，条目待补

### [0.2.1]

- 未发布成功：编译期 `include_str!` 找不到 yazi 插件文件（`yazi/` 从未进过版本库）

### [0.2.0]

- 发布链路打通：双架构编译 → Release → 自动更新 tap 的 formula

### [0.1.1]

- 修 formula 里 tag 多写 `v` 前缀导致的 404（发布是 `0.1.1`，url 写成 `v0.1.1`）

### [0.1.0]

- 首个对外发布

---

## 提交信息规范

从现在起用 [Conventional Commits](https://www.conventionalcommits.org/zh-hans/)：

```
<type>(<scope>): <简述>

<正文，可选：为什么改，不是改了什么>
```

常用 type：

| type | 用于 | CHANGELOG 归到 |
|---|---|---|
| `feat` | 新功能 | Added |
| `fix` | 修 bug | Fixed |
| `perf` | 性能 | Performance |
| `refactor` | 重构，行为不变 | Changed |
| `docs` | 文档 | Documentation |
| `test` | 测试 | — |
| `build` / `ci` | 构建 / CI | — |
| `chore` | 杂项（发版、依赖） | — |

scope 可选：`tui` / `cli` / `svn` / `repo` / `release`。

要点：

- 简述用**祈使句**、不加句号，一行 72 字符内
- 破坏性变更在 type 后加 `!`，或正文里写 `BREAKING CHANGE:`
- 一个提交只做一件事 —— 混在一起就没法归进 CHANGELOG

发版前生成条目：

```bash
scripts/changelog.sh v0.2.4 HEAD     # 打印按 type 分好组的条目，粘进上面
```
