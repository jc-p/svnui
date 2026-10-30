# svnui 打包与发布

发布产物按版本保存在 `dist/releases/<版本>/`，包括两个 macOS 架构的 tar 包、`SHA256SUMS` 和记录源码提交的 `manifest.json`。既有版本不会被清空，已发布 Release 和附件不会被覆盖。

## 准备

使用 Rustup 管理的 Cargo，提前安装两个目标架构和所需依赖。安装、联网准备与发布分开执行：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo fetch --locked
```

打包前提交当前改动，确保 `Cargo.toml` 与 `Cargo.lock` 的 svnui 版本一致。发布需要已登录的 GitHub CLI，以及仓库 push / Release 写权限。更新 Homebrew 还需要显式指定 `jc-p/homebrew-svnui` 的干净本地 checkout，并具有该仓库的推送权限。

## 先验证本地包

当前版本可以先纯打包：

```bash
./scripts/dist.sh --no-publish --offline
# 或仅验证本机架构：
./scripts/dist.sh --no-publish --offline --host-only
```

`--no-publish` 不提交、不创建 tag、不 push/pull、不调用 gh/brew、不查找或修改 tap。`--offline` 对 Cargo 使用 `--locked --offline`，缺失目标架构时明确失败，不自动安装。两者可独立组合：未指定 `--offline` 时 Cargo 可能下载锁定依赖。

`release.sh <当前版本> --no-publish --offline` 同样是纯打包；指定其他版本会拒绝，不会先修改版本再发布。同版本目录只在 commit、版本、目标集合和校验和全部一致时复用。若有意重新构建同一版本，先手动保留旧目录并移走；脚本不会自动删除旧产物。

正式发布需要双架构。`--host-only` 产物只供本地验证，不能带占位校验和更新双架构 formula。

## 测试入口

```bash
# 常规测试（ignored 测试默认不执行）
just test
# 显式执行真实 SVN 回归；需要 svn 与 svnadmin，准备失败必须报错
just test-svn
# 两者都运行
just test-all
# 临时仓库与命令替身，不构建或发布真实安装包
python3 -B -m unittest discover -s tests -p release_scripts.py
```

等价的 Cargo 命令为 `cargo test --offline --locked` 和 `cargo test --offline --locked -- --ignored`。日常 CI 另外检查无默认功能、仅 TUI 两种 feature 边界。没有顺带开启全仓严格 fmt/clippy，避免把历史问题混进发布加固。

## 本地发布

先更新并提交更新日志等源码改动，再执行。以下以新版本 `0.7.4` 为例，替换成实际待发布版本；已发布版本（例如 `0.7.3`）不要重复发布：

```bash
./scripts/release.sh 0.7.4 --offline --tap-dir /绝对路径/homebrew-svnui
```

流程依次为：本地版本提交与 tag → 推源码提交 → 双架构构建和二进制版本检查 → 提升 staging 为版本目录 → 指定准确源码提交创建 Release → 回下载并校验所有附件 → 更新显式 tap 的 formula、提交和推送。

也可以分步执行：

```bash
./scripts/bump.sh 0.7.4
# bump 仅修改 package/lock 的 svnui 版本，提交并创建本地 tag；不构建、不联网。
git push origin HEAD
./scripts/dist.sh --offline --tap-dir /绝对路径/homebrew-svnui
```

发布前本地与远端已有 tag 都必须指向当前源码提交；创建新 Release 显式使用该提交。仅推源码提交，**不要同时推 tag 触发另一条 Actions 发布路径**。

省略 `--tap-dir` 时仅发布并验证 Release；不自动查找或同步其他本机 tap。`--no-push` 让 tap 只提交不推送；`--brew-update` 只有成功推送 tap 后才调用 brew。用户安装版本不会由脚本自动升级。

## 失败恢复

- 构建或版本检查失败：既有版本目录不变，本次 staging 清理；版本提交及 tag 保留。按错误信息修复后重跑，不自动 reset。
- 同版本产物存在：先核对 manifest 及 SHA256SUMS，再复用；不一致则停止，不覆盖。
- GitHub API 查询失败：停止，不能把权限/网络失败当作“版本不存在”。
- Release 已存在：拒绝创建或上传，不能用 `--clobber` 覆盖；要改二进制应使用新版本。
- Release 创建失败：不修改 tap。
- Release 成功但回下载校验失败：保留已发布附件，停止 tap 更新，人工核验问题。
- Release 成功但 tap 失败：保留本地产物和 Release，仅重试 tap：

```bash
./scripts/dist.sh --tap-only --tap-dir /同一绝对路径/homebrew-svnui
```

恢复入口使用原来的干净源码 checkout 和 `dist/releases/<版本>`；重新下载附件比对本地 manifest 和校验和后才操作 tap，不重新构建或上传。tap push 失败而本地已提交时，再次执行会复用该提交并重试 push。脚本不自动 pull、合并或重写 tap 历史；发生分叉由维护者处理。

同版本任务使用 `dist/.release-<版本>.lock` 互斥；普通退出会清理。若进程被强制终止，先确认没有运行中的任务，再手动移除遗留锁与 `.stage-*` 目录。

## GitHub Actions 发布

另一条独立路径是推送新 tag，或手动输入已存在的未发布 tag。所有源码和 formula 模板 checkout 绑定该 tag，并核对 Cargo/lock 版本及源码提交。两个 macOS runner 原生构建、运行普通和 ignored 回归；发布使用与本地相同的 `dist.sh` 验证流程。

不要与本地发布同时运行。每 tag 的 Actions 通过 concurrency 互斥，脚本拒绝覆盖已有版本。跨本地与 Actions 的同时竞态仍可能有一方报错停止，不能依赖它实现跨机器互斥。

配置仓库 Variable `HOMEBREW_TAP_REPO=jc-p/homebrew-svnui` 和 Secret `HOMEBREW_TAP_TOKEN`（仅该 tap 的 Contents 写权限）。未配置时明确 warning，Release 保留，tap 跳过。凭据不写入 clone URL。

如果只有 tap 失败，可手动运行 workflow，输入已发布 tag 并勾选 `tap_only`。这会跳过构建及上传，下载原始附件、核对 manifest/source，然后重试 tap。工作流配置的静态检查与替身测试不能代替远端实际执行；推送获授权后需核验 Actions 状态。

安装或升级由用户单独执行：

```bash
brew tap jc-p/svnui
brew upgrade jc-p/svnui/svnui
svnui --version
```

## 发布后的下载与文档核验

- 确认 Release 的双架构附件、`SHA256SUMS`、`manifest.json` 都可下载，且版本与 tag 对应。
- 更新 [README](README.md) 的当前正式版、双架构直链和指定版本示例，以及 [Q&A](docs/qa.md) 中的手动安装示例和适用版本。
- 下载脚本 `install.sh` 默认跟随最新正式 Release，无需随版本修改默认值。先跑安装脚本回归，再安装到临时目录验证；不要覆盖现有用户程序作为测试。
- README 的在线安装命令依赖 `main/install.sh`，推送文档后核对公开脚本可访问。Release 说明应链接 README 下载区及 Q&A。
- 验证 Homebrew formula 是否同步；未同步时明确告知使用 Release 或下载脚本，不能把 Release 成功当成 tap 成功。
- 将本次实际遇到的问题、原因、恢复步骤与适用版本补入 Q&A；不覆盖旧版本二进制来修正文档。

```bash
bash -n install.sh
python3 -B -m unittest discover -s tests -p install_script.py
```
