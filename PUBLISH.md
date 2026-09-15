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
cp ~/Downloads/svnui-release-v4/github-workflows/release.yml .github/workflows/
cp -r ~/Downloads/svnui-release-v4/packaging/homebrew/svnui.rb.tpl packaging/homebrew/
cp ~/Downloads/svnui-release-v4/scripts/*.sh scripts/
chmod +x scripts/*.sh
git add . && git commit -m "ci: add homebrew release workflow" && git push
```

> 包里刻意用 `github-workflows/` 而不是 `.github/` —— 点开头的目录在
> Finder 里看不见，放项目里时记得改回 `.github/workflows/`。

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

## 二、日常发布

### 本地一条命令（推荐）

```bash
cd ~/learn/Lazy/svnui
./scripts/release.sh 0.2.0
```

等价于下面三步，`release.sh` 把它们串起来了：

```bash
./scripts/bump.sh 0.2.0     # 改 Cargo.toml + commit + 打 tag（不带 v）
git push origin main        # 先推提交，否则 tag 会指到上一个 commit
./scripts/dist.sh           # 剩下的全自动
```

> 想分步执行也行（`--no-publish` 先只打包看结果，确认后去掉参数再跑一次）。

`dist.sh` 一次跑完：

```
编译 arm64 + x86_64 → 冒烟测试 → 打 tar.gz → 算 sha256
   ├─ 就地更新 tap 仓库的 Formula/svnui.rb
   ├─ gh release create（已存在则改用 upload --clobber 覆盖）
   ├─ tap 仓库 commit + push
   ├─ 同步本机其余 tap 克隆（见下）
   └─ brew update --force
```

常用参数：

| 参数 | 用途 |
|---|---|
| `--host-only` | 只编本机架构，交叉编译出问题时先出包 |
| `--no-publish` | 只打包，不碰任何远端 |
| `--no-push` | 发 Release，但 tap 只 commit 不 push |
| `--tap-dir <路径>` | 手动指定 homebrew-svnui 仓库位置 |

tap 目录会自动探测以下几个位置，**并且会全部同步**（不是只改第一份）：

```
~/Library/Taps/<owner>/homebrew-svnui          ← 你自己 clone 的那份
/opt/homebrew/Library/Taps/<owner>/homebrew-svnui   ← brew 实际读的那份
/usr/local/Homebrew/Library/Taps/<owner>/homebrew-svnui
$(brew --repository)/Library/Taps/<owner>/homebrew-svnui
```

探测不到才需要 `--tap-dir`。

owner 从 git remote 解析（`TAP_OWNER` 可覆盖）。万一解析不出来（SSH 别名、
自建 git 服务器、remote 改过名），脚本会在上面几个前缀目录下直接 glob
`*/homebrew-svnui` 兜底，不会静默跳过。

### 为什么要有「同步本地 tap 克隆」这一步

`brew tap` 是把 GitHub 上的 tap 仓库 **clone 到本地**。你 push 更新后，
本地这份**不会自动跟着变** —— brew 只在 `brew update` 时才 pull。

所以发完版立刻 `brew upgrade svnui`，拿到的可能还是旧 formula
（旧 url + 旧 sha256），表现为 404 或 checksum mismatch。
你以为没发成功，其实只是本地那份没刷新。

`dist.sh` 因此会在 push 之后额外做两件事：

1. 对其余每一份本地克隆执行 `git pull --ff-only`
2. 跑一次 `brew update --force`（清掉 brew 的 formula 解析缓存）

`--ff-only` 是刻意的：只允许快进。如果那份克隆有本地未提交改动或提交分叉，
脚本**不会**硬拉 —— 它打印警告让你自己处理，绝不静默覆盖你的改动。

### 关于「就地更新」

`dist.sh` **只改** Formula 里的三样：url 的版本号、两个 sha256、test 的版本断言。
注释、`depends_on`、块结构这些手工调整全部原样保留 —— 不会把你调好的
formula 覆盖回模板的样子。

需要一份全新的就渲染 `dist/svnui.rb`（从 `packaging/homebrew/svnui.rb.tpl`
整体生成），自己 `cp` 过去覆盖。

### 或者走 GitHub Actions

```bash
./scripts/bump.sh 0.2.0
git push origin main && git push origin 0.2.0
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

想重跑某次发布：Actions 页面 → Release → Run workflow → 填 tag（比如 `0.2.0`）。

> 两条路都走也不冲突：dist.sh 发现 release 已存在会改用 upload 覆盖，
> Actions 的 update-tap 也会重算 sha256。但别同时跑，容易互相覆盖产物。

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
