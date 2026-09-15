# 发布 svnui 到 Homebrew

> 本文档已合并原 `PUBLISH_MANUAL.md`。两份文档讲同一件事，是之前反复出错的根源
> （一份说 tag 带 `v`，另一份不带，改的时候只改了一边）。**只保留这一份。**

---

## 结论：走自建 Tap，别投 homebrew-core

`homebrew-core` 对新 formula 有硬性知名度门槛（看 star / fork / watch），
不接受公司内部或个人自用工具。投上去基本是等几天然后被拒。

**自建 Tap** 体验几乎一样，代价只是用户多一条 `brew tap`：

```bash
brew tap jc-p/svnui
brew trust jc-p/svnui      # 第三方 tap 需要显式信任
brew install svnui
brew upgrade svnui         # 后续升级就这一条
```

> **命名规则容易踩**：Tap 仓库名**必须**叫 `homebrew-svnui`（带前缀），
> 但 `brew tap` 命令里**不带**前缀——Homebrew 会自动去找 `homebrew-<名字>`。

---

## 术语对照

| 仓库 | 地址 | 作用 |
|---|---|---|
| 主仓库 | `jc-p/svnui` | 源码、Release、二进制产物 |
| Tap 仓库 | `jc-p/homebrew-svnui` | 只有一个 `Formula/svnui.rb` |

两者是不同的 git 仓库，改完 formula **必须 push 到 Tap 仓库**才算生效。

---

## 一次性准备

### 1. 建 Tap 仓库

GitHub 新建空仓库 `homebrew-svnui`，然后：

```bash
git clone https://github.com/jc-p/homebrew-svnui.git
cd homebrew-svnui
mkdir -p Formula
# 先放一份占位的 formula（sha256 随便填），保证文件存在且语法合法
git add . && git commit -m "init tap" && git push
```

### 2. 解除 tag 创建限制（**必做，否则发布必失败**）

主仓库 → **Settings → Rules → Rulesets**。

如果有规则作用范围覆盖了 tag，会报：

```
Cannot create ref due to creations being restricted.
```

二选一：

- **Delete ruleset**（自己的仓库，最省事）
- 或 **Bypass list** → 把 `jc-p` 加进去

> ⚠️ **Rulesets 不自动豁免 owner**。你是仓库 owner 也照样被拦——
> 这跟老版 Branch Protection 不一样。必须显式加 bypass 或删规则。

### 3. 确认用的是 rustup 的 cargo

```bash
which -a cargo
```

只应有 `~/.cargo/bin/cargo`。如果 `/opt/homebrew/bin/cargo` 排在前面，
`rustup target add` 装的 target 它用不到，交叉编译会报 `can't find crate for core`。

```bash
brew uninstall rust       # 或用下面这条只调 PATH
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
```

### 4. 生成 PAT（跨仓库推送用）

默认的 `GITHUB_TOKEN` 只能操作当前仓库，推不了 Tap 仓库。

- Settings → Developer settings → Personal access tokens → **Fine-grained tokens**
- Repository access：只选 `homebrew-svnui`
- Permissions → Contents：**Read and write**

主仓库 → Settings → Secrets and variables → Actions：

| 类型 | 名字 | 值 |
|---|---|---|
| Secret | `HOMEBREW_TAP_TOKEN` | 上面的 PAT |
| Variable | `HOMEBREW_TAP_REPO` | `jc-p/homebrew-svnui` |

### 5. 把发布脚本放进项目

```
scripts/dist.sh                    本地打包
scripts/bump.sh                    改版本号 + 打 tag
packaging/homebrew/svnui.rb.tpl    formula 模板（带 @@VERSION@@ 占位符）
.github/workflows/release.yml      CI（可选，走通手动流程后再加）
```

```bash
chmod +x scripts/*.sh
echo "/dist" >> .gitignore        # 构建产物别进仓库
```

---

## 日常发布（手动，6 步）

### 第 1 步：改版本号

```bash
cd ~/learn/Lazy/svnui
./scripts/bump.sh 0.1.2
git push origin main
```

> **版本号用过就不能再用。** GitHub 的 immutable release 会永久占用 tag 名，
> 即使删掉 release 也不释放。打错了只能升版本号，不能重来。

### 第 2 步：本地打包

```bash
./scripts/dist.sh
```

产出：

```
dist/svnui-0.1.2-aarch64-apple-darwin.tar.gz
dist/svnui-0.1.2-x86_64-apple-darwin.tar.gz
dist/svnui.rb          ← 填好真实 sha256 的成品 formula
```

> 交叉编译 x86_64 不需要 Rosetta 就能编，但编出来的二进制**在本机跑不了**
> （`Bad CPU type`），所以脚本跳过它的冒烟测试——那不是编译失败。

### 第 3 步：创建 Release 并上传

```bash
gh release create 0.1.2 --generate-notes dist/svnui-0.1.2-*.tar.gz
```

**⚠️ tag 名必须是不带 `v` 的 `0.1.2`。**

这是本文档最需要记住的一条。`gh release create` 用什么 tag，
`releases/download/<tag>/` 就是什么。而模板里如果写死 `v` 前缀，
URL 就会变成 `download/v0.1.2/` → **404**。这个坑已经踩过两次。

核对一下：

```bash
gh release view 0.1.2 --json tagName,isDraft,assets \
  --jq '{tagName, isDraft, assets:[.assets[].name]}'
```

必须是 `"tagName": "0.1.2"` 且 `"isDraft": false`。

> 如果是 `isDraft: true`，匿名下载一律 404（Draft 的 assets 不对外公开）。
> 发布它：`gh release edit 0.1.2 --draft=false`

### 第 4 步：更新 Tap 仓库

```bash
cd ~/path/to/homebrew-svnui
cp ~/learn/Lazy/svnui/dist/svnui.rb Formula/
git add Formula/svnui.rb
git commit -m "svnui 0.1.2"
git push                 # ← 忘了这步等于没改
```

**顺序不能反**：必须先有 Release 和 assets，再推 formula。
反过来的空档里用户 `brew upgrade` 会拿到指向 404 的 url。

### 第 5 步：验证

```bash
brew update --force
brew upgrade svnui

svnui --version                    # 应输出 0.1.2
brew audit --formula jc-p/svnui/svnui
brew test svnui
```

### 第 6 步：通知用户

```bash
brew upgrade svnui
```

---

## 上 CI（可选）

手动流程跑通之后，放上 `.github/workflows/release.yml`，
之后推 tag 就自动完成第 2~4 步：

```bash
git push origin 0.1.2
```

```
build (macos-14 arm64 + macos-13 x86_64)
   └─ 编译 → 冒烟测试 --version → 打 tar.gz
release
   └─ 创建 Release，上传两个 tar.gz
update-tap
   └─ 下载产物 → 算 sha256 → 渲染 formula → 推到 homebrew-svnui
```

约 3-5 分钟。workflow 用 `github.repository_owner` 自动取 owner，不用改。

⚠️ **上 CI 前先确认没有第二个会响应 tag 的 workflow**——两个都跑会互相覆盖 assets。

---

## 坑（按实际踩过的顺序）

### 1. tag 带不带 `v` —— 全链路必须统一

统一用**不带 `v`** 的 `0.1.2`。需要对齐四处：

| 位置 | 内容 |
|---|---|
| `Cargo.toml` | `version = "0.1.2"` |
| git tag / Release | `0.1.2`（无 v） |
| `svnui.rb.tpl` | `releases/download/@@VERSION@@/...` |
| `scripts/bump.sh` | 打的 tag 无 v |

`bump.sh` 和模板如果还带着 `v`，改掉：

```bash
sed -i '' 's|download/v@@VERSION@@|download/@@VERSION@@|g' packaging/homebrew/svnui.rb.tpl
```

**症状**：`brew install` 报 `curl: (56) ... 404`。
**排查**：`gh release view <版本> --json tagName` 看实际 tag 是什么。

### 2. Draft Release 导致 404

Release 建了但没发布，assets 不对外公开，匿名下载 404。

```bash
gh release edit <版本> --draft=false
```

### 3. `brew audit` 报 version 冗余

```
* Stable: `version 0.1.1` is redundant with version scanned from URL
* line 5, col 3: `version` (line 5) should be put before `license` (line 4)
```

url 里已有版本号，brew 能自动解析。删掉显式的 `version` 那行，两个问题一起消失：

```bash
sed -i '' '/^  version "/d' Formula/svnui.rb
```

模板里也删，否则下次渲染又带回去。
`test do` 里用 `assert_match version.to_s, ...`（引用而非硬编码），删了也没影响。

### 4. 第三方 tap 需要显式信任

```
Refusing to load formula ... from untrusted tap jc-p/svnui
```

```bash
brew trust jc-p/svnui
```

这是设计如此：tap 里的 `.rb` 是 Ruby 代码，会在用户机器上执行。
建议写进 Tap 仓库 README，让同事别卡在这：

```bash
brew tap jc-p/svnui && brew trust jc-p/svnui && brew install svnui
```

### 5. immutable release 占住版本号

```
tag_name was used by an immutable release
```

删 release 也没用，tag 名永久占用。**升版本号，别想着重来。**

### 6. `brew test` 要求版本严格对齐

```
Error: Testing requires the latest version of jc-p/svnui/svnui
```

按 `push formula → brew update → brew upgrade → brew test` 的顺序来。

### 7. Gatekeeper（用户侧最常见）

从网上下载二进制带 quarantine，首次运行弹「来自身份不明的开发者」：

```bash
sudo xattr -d com.apple.quarantine $(which svnui)
```

彻底解决要 Apple Developer 账号（$99/年）做签名 + 公证，内部工具不值当。
写进 Tap 仓库 README 即可。

> 本地 `cargo build` 出来的没这问题，所以自己可能从来没遇到过——但同事会。

### 8. tar 包必须平铺

tar 里直接是 `svnui` 文件，不套子目录。
套了 `svnui-0.1.2-aarch64-apple-darwin/svnui` 的话，
`bin.install "svnui"` 就得改成带版本号的路径，每次发版都得跟着改。

### 9. sha256 别手改

脚本从实际文件算好直接写进 formula。手动复制少一位，
用户装的时候才报 checksum mismatch，排查很费劲。

重传同名文件要加 `--clobber`：

```bash
gh release upload 0.1.2 dist/svnui-0.1.2-*.tar.gz --clobber
```

### 10. `.gitignore` 漏掉 `dist`

不然 `git add .` 会把几 MB 的 tar.gz 提交进仓库，后面很难清理。

---

## 可选：同时发布到 crates.io

```bash
cargo login
cargo publish
```

之后 `cargo install svnui` 也能装（从源码编译，慢但不需要 tap）。
发布前确认 `Cargo.toml` 有 `repository` / `homepage` / `keywords`。

---

## 文件对照表

| 文件 | 放到 |
|---|---|
| `LICENSE` | 主仓库根目录 |
| `scripts/dist.sh` `scripts/bump.sh` | 主仓库 `scripts/`（记得 `chmod +x`） |
| `packaging/homebrew/svnui.rb.tpl` | 主仓库 `packaging/homebrew/` |
| `.github/workflows/release.yml` | 主仓库 `.github/workflows/` |
| `Formula/svnui.rb` | **Tap 仓库**的 `Formula/` |

`svnui.rb.tpl` 是模板（带 `@@VERSION@@` 占位符，给脚本/CI 用），
`Formula/svnui.rb` 是填好值的成品（给 Tap 仓库用）。别放混。

> `.github` 是隐藏目录，macOS Finder 默认不显示。
> 看不到不代表没给——按 `Cmd + Shift + .` 可切换显示，或在终端用 `ls -a`。
