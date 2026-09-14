# 发布 svnui 到 Homebrew（含自动升级）

## 先说结论：走自建 Tap，别投 homebrew-core

Homebrew 官方仓库（`homebrew-core`）对新 formula 有硬性门槛 —— 要求项目"有足够知名度"（看 GitHub 的 star / fork / watch 数），并且不接受公司内部或个人自用工具。个人 TUI 工具投上去基本是等几天然后被拒。

**自建 Tap** 是正解，而且体验几乎一样好：

```bash
brew tap jc-p/svnui
brew install svnui
brew upgrade svnui      # 后续升级就这一条
```

代价只是用户多了第一条 `brew tap`。

> Tap 仓库的命名规则容易踩：仓库名**必须**叫 `homebrew-svnui`（带前缀），
> 但 `brew tap` 命令里**不带**前缀 —— Homebrew 会自动去找 `homebrew-<名字>`。

---

## 一、一次性准备

### 1. 补齐项目里的缺失项

```bash
cd ~/learn/Lazy/svnui
```

把包里的 `LICENSE` 放到项目根目录（`Cargo.toml` 声明了 `license = "MIT"`，但缺 LICENSE 文件，这在发布时是硬伤）。

给 `Cargo.toml` 的 `[package]` 段补几个字段（发布到 crates.io 需要，对 brew 也有用）：

```toml
repository = "https://github.com/jc-p/svnui"
homepage   = "https://github.com/jc-p/svnui"
readme     = "README.md"
keywords   = ["svn", "subversion", "tui"]
categories = ["command-line-utilities"]
```

### 2. 建 GitHub 仓库并推上去

```bash
git init
git add .
git commit -m "init"
git branch -M main
git remote add origin https://github.com/jc-p/svnui.git
git push -u origin main
```

### 3. 建 Tap 仓库

在 GitHub 上新建一个**空仓库**，名字必须是 `homebrew-svnui`。

本地把它准备好：

```bash
git clone https://github.com/jc-p/homebrew-svnui.git
cd homebrew-svnui
mkdir -p Formula
cp ~/Downloads/svnui-release/Formula/svnui.rb Formula/
# 把 @@VERSION@@ / @@SHA_ARM@@ / @@SHA_X86@@ / @@OWNER@@ 先填成占位值
git add . && git commit -m "init tap" && git push
```

> 第一次的 formula 内容不重要，CI 会在第一次 release 时整体覆盖它。
> 但要保证文件存在且语法合法，否则 `brew tap` 会报错。

### 4. 生成 PAT（跨仓库推送用）

默认的 `GITHUB_TOKEN` **只能操作当前仓库**，推不了 tap 仓库，所以要用 PAT。

- GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained tokens**
- Repository access：只选 `homebrew-svnui`
- Permissions → Contents：**Read and write**
- 生成后复制（只显示一次）

### 5. 配置 Secrets / Variables

在 `svnui` 仓库 → Settings → Secrets and variables → Actions：

| 类型 | 名字 | 值 |
|---|---|---|
| Secret | `HOMEBREW_TAP_TOKEN` | 上面生成的 PAT |
| Variable | `HOMEBREW_TAP_REPO` | `jc-p/homebrew-svnui` |

没配这两个的话 CI 会跳过 tap 更新并打 warning，其他步骤照常 —— 可以先跑通 release，再回来补。

### 6. 放进 workflow 和模板

```bash
cd ~/learn/Lazy/svnui
mkdir -p .github/workflows packaging/homebrew scripts
cp ~/Downloads/svnui-release/.github/workflows/release.yml .github/workflows/
cp ~/Downloads/svnui-release/packaging/homebrew/svnui.rb.tpl packaging/homebrew/
cp ~/Downloads/svnui-release/scripts/bump.sh scripts/
chmod +x scripts/bump.sh
git add . && git commit -m "ci: add homebrew release workflow" && git push
```

### 7. 验证 formula 语法（本地）

```bash
brew tap jc-p/svnui
brew install svnui
svnui --version
brew test svnui
brew audit --formula svnui
```

`brew audit` 可能会提示 `version` 是冗余的（因为 url 里已含版本号）。这是**警告不是错误** —— 显式写 version 是为了让 CI 替换模板时更可控，忽略即可。用 `brew audit --strict` 才会拦截，日常别加这个参数。

---

## 二、日常发布（一条命令）

```bash
cd ~/learn/Lazy/svnui
./scripts/bump.sh 0.2.0
git push origin main && git push origin v0.2.0
```

push tag 之后 Actions 自动跑完三步：

```
build (macos-14 arm64 + macos-13 x86_64)
   └─ 编译 → 冒烟测试 --version → 打 tar.gz
release
   └─ 创建 GitHub Release，上传两个 tar.gz，自动生成更新日志
update-tap
   └─ 下载 release 产物 → 算 sha256 → 渲染 formula → 推到 homebrew-svnui
```

全程约 3-5 分钟。推完用户那边 `brew upgrade svnui` 就能拿到新版。

想重跑某次发布：Actions 页面 → Release → Run workflow → 填 tag（比如 `v0.2.0`）。

---

## 三、用户侧的安装与升级

```bash
# 首次
brew tap jc-p/svnui
brew install svnui

# 升级（brew 会先自动 update tap 仓库）
brew upgrade svnui

# 看看有没有新版
brew outdated svnui
```

---

## 四、可能会遇到的坑

### 1. Gatekeeper 拦截（最常见）

从网上下载的二进制带 quarantine 属性，首次运行 macOS 会弹「无法打开，因为来自身份不明的开发者」。

用户侧解决：

```bash
sudo xattr -d com.apple.quarantine $(which svnui)
```

或者：系统设置 → 隐私与安全性 → 点「仍要打开」。

彻底消除需要 Apple Developer 账号（$99/年）做签名 + 公证。**对内部工具不值得**，在 tap 仓库 README 里写一句 `xattr -d` 就行。

> 顺带一提：本地 `cargo build` 出来的二进制没有这个问题，所以你自己可能从来没遇到过，
> 但通过 brew 装的同事会 —— 值得在 README 里提前说明。

### 2. `MACOSX_DEPLOYMENT_TARGET`

workflow 里设成了 `11.0`。不设的话 GitHub 的 macos-13 runner 编出的二进制会要求 macOS 13+，老机器装了跑不起来。设了之后覆盖 Big Sur 及以上。

如果要支持更老的系统（10.15 Catalina），改成 `10.15`，但要注意依赖库是否还支持。

### 3. 版本号别带 `v`

`Cargo.toml` 里写 `0.2.0`，git tag 写 `v0.2.0`。`bump.sh` 已经处理了这个转换（两种写法都接受）。

### 4. tar 包结构

工作流里是**平铺**的（tar 里直接是 `svnui` 文件，不带子目录）。

如果改成套子目录 `svnui-0.2.0-aarch64-apple-darwin/svnui`，那 formula 里的 `bin.install "svnui"` 就得改成带版本号的路径 —— CI 每次都要跟着改。平铺能让它永远是 `bin.install "svnui"`。

---

## 五、包里文件放哪

| 文件 | 放到 |
|---|---|
| `LICENSE` | 项目根目录 |
| `.github/workflows/release.yml` | 项目 `.github/workflows/` |
| `packaging/homebrew/svnui.rb.tpl` | 项目 `packaging/homebrew/` |
| `scripts/bump.sh` | 项目 `scripts/`（记得 `chmod +x`） |
| `Formula/svnui.rb` | **tap 仓库**（`homebrew-svnui`）的 `Formula/` |

`svnui.rb.tpl` 是模板（带 `@@VERSION@@` 这类占位符，给 CI 用），
`Formula/svnui.rb` 是填好值的成品（给 tap 仓库首次初始化用）。两者别放混了。

---

## 六、可选：同时发布到 crates.io

Homebrew 之外多一条安装途径，Rust 用户会更习惯：

```bash
cargo login          # 拿 https://crates.io/me 的 token
cargo publish
```

之后用户 `cargo install svnui` 也能装（会从源码编译，慢一些但不需要 tap）。

注意：发布前确认 `Cargo.toml` 的 `repository` / `homepage` / `keywords` 都填了，否则 crates.io 会警告。
