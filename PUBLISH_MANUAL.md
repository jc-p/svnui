# 手动发布 svnui 到 Homebrew

先手动走通一遍，确认链路没问题，再决定要不要上 CI 自动发布。

整体就 **6 步**，日常更新时只需要后 4 步。

---

## 第 0 步：建 Tap 仓库（只做一次）

在 GitHub 新建空仓库，名字**必须**叫 `homebrew-svnui`（带 `homebrew-` 前缀）。

> 命名规则容易踩：仓库名带前缀，但 `brew tap` 命令里不带 ——
> Homebrew 会自动去找 `homebrew-<名字>`。

```bash
git clone https://github.com/jc-p/homebrew-svnui.git
cd homebrew-svnui
mkdir -p Formula
cp /path/to/svnui-release/Formula/svnui.rb Formula/
git add . && git commit -m "init tap" && git push
```

首次的 formula 内容不重要（sha256 是占位的），CI/脚本后续会覆盖。
但文件必须存在且语法合法，否则 `brew tap` 会报错。

---

## 第 1 步：补齐项目缺失项（只做一次）

```bash
cd ~/learn/Lazy/svnui
cp /path/to/svnui-release/LICENSE .
```

`Cargo.toml` 的 `[package]` 段补上（缺这几项 `brew audit` 会有提示）：

```toml
repository = "https://github.com/jc-p/svnui"
homepage   = "https://github.com/jc-p/svnui"
readme     = "README.md"
keywords   = ["svn", "subversion", "tui"]
categories = ["command-line-utilities"]
```

把脚本放进项目：

```bash
mkdir -p scripts packaging/homebrew
cp /path/to/svnui-release/scripts/dist.sh      scripts/
cp /path/to/svnui-release/scripts/bump.sh      scripts/
cp /path/to/svnui-release/packaging/homebrew/svnui.rb.tpl packaging/homebrew/
chmod +x scripts/*.sh
git add . && git commit -m "chore: add release tooling"
```

---

## 第 2 步：本地打包

```bash
cd ~/learn/Lazy/svnui
./scripts/dist.sh
```

会自动完成：

```
编 aarch64-apple-darwin  →  冒烟测试 --version  →  打 tar.gz
编 x86_64-apple-darwin   →  (跳过冒烟，跨架构)   →  打 tar.gz
算两个 sha256
从模板渲染 dist/svnui.rb
```

产出：

```
dist/svnui-0.1.0-aarch64-apple-darwin.tar.gz
dist/svnui-0.1.0-x86_64-apple-darwin.tar.gz
dist/svnui.rb          ← 填好真实 sha256 的成品
```

**关于交叉编译**：Apple Silicon 上编 x86_64 是纯交叉编译，不需要 Rosetta 就能编。
但编出来的二进制**在本机跑不了**（`Bad CPU type`），所以脚本会自动跳过它的冒烟测试 ——
那不是编译失败。真要验证，找台 Intel 机器，或者在 CI 上跑。

---

## 第 3 步：上传 GitHub Release

### 情况 A：Release 还不存在

打开 <https://github.com/jc-p/svnui/releases/new>：

- Tag：`v0.1.0`（注意带 `v`，跟 `Cargo.toml` 里的 `0.1.0` 不同）
- Title：`v0.1.0`
- 把 `dist/` 下**两个** `.tar.gz` 拖进去上传
- Publish release

⚠️ **tag 名和版本号别搞混**：`Cargo.toml` 写 `0.1.0`，git tag 写 `v0.1.0`。

### 情况 B：v0.1.0 已经存在

你的仓库里 v0.1.0 **已经存在且挂了 3 个 assets**，所以走这条：

先看看现在挂的是什么：

```bash
# 列出已有 assets（需要 gh，没装就用浏览器看）
gh release view v0.1.0 --repo jc-p/svnui
```

然后二选一：

**删掉重来**（推荐，干净）：

```bash
gh release delete v0.1.0 --repo jc-p/svnui --yes
# 然后按情况 A 重新建
```

**或者沿用现有文件名**：如果已有 assets 叫 `svnui-0.1.0-aarch64-apple-darwin.tar.gz` 这类名字，
那就不用删，直接把你的两个包传上去**覆盖同名文件**；名字不一样的话，
改 `packaging/homebrew/svnui.rb.tpl` 里的 url 模板对齐现有命名，重跑 `dist.sh`。

> 3 个 assets 通常意味着已有别的发布工具在跑（cargo-dist 之类常多一个 `.sha256` 文件），
> 或者你手动传过。建议先看清再动手 —— 名字对不上的话，
> formula 里的 url 会 404，用户 `brew install` 时才炸。

---

## 第 4 步：更新 Tap 仓库

```bash
cd ~/path/to/homebrew-svnui
cp ~/learn/Lazy/svnui/dist/svnui.rb Formula/
git add Formula/svnui.rb
git commit -m "svnui 0.1.0"
git push
```

**发布顺序有讲究**：必须先有 Release 和 assets，再推 formula。
反过来的话，用户在这个空档 `brew upgrade` 会拿到一个指向 404 的 url。

---

## 第 5 步：验证

```bash
# 换台干净的机器，或者先 uninstall 再走一遍
brew uninstall svnui 2>/dev/null
brew untap jc-p/svnui 2>/dev/null

brew tap jc-p/svnui
brew install svnui
svnui --version          # 应输出 0.1.0

brew test svnui
brew audit --formula jc-p/svnui/svnui
```

`brew audit` 可能提示 `version` 冗余（url 里已有版本号）—— 是**警告不是错误**，
显式写 version 是为了让模板替换可控。别加 `--strict`，那个会拦。

---

## 日常更新（第 2~5 步）

```bash
cd ~/learn/Lazy/svnui

# 1. 改版本号 + 提交 + 打 tag（脚本会校验格式和干净的工作区）
./scripts/bump.sh 0.2.0
git push origin main && git push origin v0.2.0

# 2. 打包
./scripts/dist.sh

# 3. 传 Release（浏览器拖，或 gh）
gh release create v0.2.0 --generate-notes dist/*.tar.gz

# 4. 更新 tap
cd ~/path/to/homebrew-svnui
cp ~/learn/Lazy/svnui/dist/svnui.rb Formula/
git commit -am "svnui 0.2.0" && git push
```

用户侧：

```bash
brew upgrade svnui
```

---

## 坑

### 1. Gatekeeper（最常见）

从网上下的二进制带 quarantine 属性，首次运行会弹「无法打开，因为来自身份不明的开发者」。

你本地 `cargo build` 出来的**没有这个问题**，所以自己可能从来没遇到 —— 但通过 brew 装的同事会。

```bash
sudo xattr -d com.apple.quarantine $(which svnui)
```

或者：系统设置 → 隐私与安全性 → 点「仍要打开」。

彻底解决要 Apple Developer 账号（$99/年）做签名 + 公证。内部工具不值当，
在 tap 仓库的 README 里写一句 `xattr -d` 就行。

### 2. tar 包必须平铺

脚本里是平铺的（tar 里直接是 `svnui`，无子目录）。

如果套了子目录 `svnui-0.1.0-aarch64-apple-darwin/svnui`，formula 里的
`bin.install "svnui"` 就得改成带版本号的路径，每次发版都要跟着改。

### 3. sha256 必须跟实际文件对得上

这是最容易出错的地方 —— 手动复制粘贴时少一位、或者传完文件后又改了本地包，
都会导致 `brew install` 报 checksum mismatch。

脚本从实际文件算 sha256 并直接写进 formula，**别手改**。

### 4. tag 与 Release 的先后

`gh release create` 会自动建 tag。如果你手动在 GitHub 上建 Release，
记得选「Create new tag」而不是选一个已存在的。

---

## 走通之后：上 CI

手动流程确认没问题，就把 `.github/workflows/release.yml` 放进项目。
之后 `git push origin v0.2.0` 会自动跑完第 2~4 步。

workflow 用的是 `github.repository_owner`，会取到 `jc-p`，不用改。

需要配两个东西（在 svnui 仓库 → Settings → Secrets and variables → Actions）：

| 类型 | 名字 | 值 |
|---|---|---|
| Secret | `HOMEBREW_TAP_TOKEN` | PAT（对 homebrew-svnui 有 Contents 写权限） |
| Variable | `HOMEBREW_TAP_REPO` | `jc-p/homebrew-svnui` |

默认的 `GITHUB_TOKEN` 跨不了仓库，所以必须 PAT。
没配这两个的话 CI 会跳过 tap 更新并打 warning，其他步骤照常。

⚠️ 上 CI 之前**先删掉或改掉现有那个会响应 tag 的 workflow** —— 两个都跑会互相覆盖 assets。
