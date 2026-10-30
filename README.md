# svnui

Subversion 的 TUI 客户端。用 Rust + [ratatui](https://github.com/ratatui/ratatui) 写的独立终端程序，
不依赖 yazi，也不需要额外配置就能用。

[下载最新版本](https://github.com/jc-p/svnui/releases/latest) · [安装](#下载与安装) · [快速上手](#快速上手) · [Q&A](docs/qa.md) · [更新日志](CHANGELOG.md)

```
┌─ 文件树 ─────────────┬─ 预览 / diff ────────────────────────┐
│ [x] M src/main.rs    │ Index: src/main.rs                   │
│ [ ] A src/new.rs     │ @@ -1,5 +1,6 @@                      │
│     ? notes.txt      │  fn main() {                         │
│                      │ -    println!("hi");                 │
│                      │ +    println!("hello");              │
└──────────────────────┴──────────────────────────────────────┘
 j/k 移动 | Space 勾选 | C 提交 | L 日志 | ? 帮助
```

---

## 下载与安装

当前正式版 **[0.7.2](https://github.com/jc-p/svnui/releases/tag/0.7.2)**，提供 macOS 11+ 的双架构安装包。
下载二进制无需 Rust；使用 SVN 功能需要本机有 `svn`，可先运行 `svn --version --quiet` 检查。
Linux 等其他系统请[从源码构建](#从源码构建)。

### 一条命令安装或升级（推荐）

```bash
curl -fsSL https://raw.githubusercontent.com/jc-p/svnui/main/install.sh | bash
```

脚本识别当前终端架构，下载官方最新正式 Release，校验 SHA256 和二进制版本，
再安装到 `~/.local/bin/svnui`。不需要 sudo，不修改 shell 配置；下载、校验或新程序启动失败时保留旧安装。

首次安装后，将用户目录加入当前终端的 PATH 并核对版本：

```bash
export PATH="$HOME/.local/bin:$PATH"
hash -r
svnui --version
```

需要永久生效时，将 `export PATH="$HOME/.local/bin:$PATH"` 加入 `~/.zshrc`（Bash 用户用 `~/.bashrc`）。
指定版本或安装目录：

```bash
curl -fsSL https://raw.githubusercontent.com/jc-p/svnui/main/install.sh | bash -s -- --version 0.7.2 --bin-dir "$HOME/.local/bin"
```

不想执行在线脚本，可以使用下面的官方安装包，按 [Q&A 的手动安装步骤](docs/qa.md#manual-install) 校验并安装。
GitHub 网页或脚本域名连接失败时，Q&A 也提供已验证的 GitHub CLI 下载方式。

### 直接下载安装包

| Mac 类型 | `uname -m` | 0.7.2 安装包 |
|---|---|---|
| Apple Silicon（M 系列） | `arm64` | [下载 ARM64](https://github.com/jc-p/svnui/releases/download/0.7.2/svnui-0.7.2-aarch64-apple-darwin.tar.gz) |
| Intel，或运行在 Rosetta 下的终端 | `x86_64` | [下载 x86_64](https://github.com/jc-p/svnui/releases/download/0.7.2/svnui-0.7.2-x86_64-apple-darwin.tar.gz) |

[SHA256SUMS](https://github.com/jc-p/svnui/releases/download/0.7.2/SHA256SUMS) ·
[源码提交与产物清单](https://github.com/jc-p/svnui/releases/download/0.7.2/manifest.json) ·
[查看最新 Release](https://github.com/jc-p/svnui/releases/latest)

选择 `svnui-版本-架构.tar.gz`，Release 页面里的 **Source code** 是源码，不是可直接运行的程序。
不确定架构时先执行 `uname -m`，不要凭文件名猜。

### Homebrew

```bash
brew tap jc-p/svnui
brew install jc-p/svnui/svnui
```

若 Homebrew 提示第三方 tap 未受信任，再按提示执行 `brew trust --tap jc-p/svnui`。
升级前先核对 formula 提供的版本：

```bash
brew update
brew info jc-p/svnui/svnui
brew upgrade jc-p/svnui/svnui
```

GitHub Release 与 Homebrew tap 分别发布；如果 formula 尚未提供需要的版本，使用上面的安装脚本或直接下载。
不混用安装来源时更容易升级；已有多个 `svnui` 时见 [Q&A：升级后仍运行旧版](docs/qa.md#old-version)。

### 从源码构建

需要 Rust 1.88+、Git，以及使用 SVN 功能时所需的 `svn`。

```bash
git clone https://github.com/jc-p/svnui.git
cd svnui
cargo build --release --locked
mkdir -p "$HOME/.local/bin"
./install-local.sh --no-build --bin-dir "$HOME/.local/bin"
```

这会构建 `main` 的当前源码；复现特定发布版本时，在构建前先 `git checkout 0.7.2`。
首次构建可能下载依赖；使用 `--locked` 保持依赖版本一致。

### 本地源码更新

```bash
mkdir -p "$HOME/.local/bin"
./install-local.sh --bin-dir "$HOME/.local/bin"
```

`install-local.sh` 离线构建当前源码，校验安装结果与 PATH；它与下载预编译包的 `install.sh` 用途不同。
依赖未缓存时先执行 `cargo fetch --locked`。省略 `--bin-dir` 时安装到 `/usr/local/bin`，可能需要 sudo。
更新完成后退出旧界面，重新运行 `svnui tui`。

---

## 快速上手

```bash
cd 你的工作副本
svnui tui
```

也可直接运行 `svnui`：交互式终端中默认进入 TUI；不在工作副本中时进入项目选择或检出界面。
首次连接服务器前，先按 [Q&A：认证与证书](docs/qa.md#authentication) 配置 SVN 凭证。

首次建议先跑自检：

```bash
svnui probe           # svn 路径 / 版本 / 工作副本根 / 能力开关
```

独立预览本地文件，无需先进入项目或 SVN 工作副本，也无需安装 svn：

```bash
svnui preview /path/to/file.txt
svnui preview ./image.png
svnui --cwd /path/to/project preview docs/report.xlsx
```

也可直接打开远端 SVN 目录，无需先检出项目（需要已安装 svn 并配置认证）：

```bash
svnui preview "https://svn.example.com/repo/trunk/project/"
svnui --trust-cert preview "https://svn.example.com/repo/trunk/project/"
```

远端目录复用仓库浏览器：`Enter` 进入子目录，`←` / 退格返回，`q` / `Esc` 退出。
浏览边界为传入 URL；已编码的中文 URL 可直接传入，不会二次编码。

支持文本、图片和办公文档，沿用 2 MiB 上限；压缩包、大文件和不支持的二进制
显示提示。`j/k`、方向键和 `d/u` 滚动，`q` / `Esc` / `Ctrl+C` 退出。

### 外观与提示

在已有 `~/.config/svnui/config.toml` 的 `[ui]` 段内设置 `theme = "catppuccin"`，重启后生效。
支持 Nord、Tokyo Night、终端配色和无色模式，完整选项见 [主题配置](docs/theme.md)。

`●` / `○` 表示面板焦点，`◆` / `◇` 区分当前工作副本和其他项目，无需专用图标字体。
普通通知显示在右上角；`F4` 查看详情，工作流内用 `Shift+F4`，`y` 复制全文，`Esc` 关闭。

---

## TUI 键位

### 文件树（主界面）

| 键 | 作用 |
|---|---|
| `j` / `k` 或 `↑` `↓` | 移动 |
| `Enter` | 目录展开/折叠；文件进全屏预览 |
| `Space` | 文件勾选/取消；目录批选后代文件改动，包含折叠项，遵守搜索范围；半选目录再次按下补齐 |
| `Ctrl+A` | 全选当前搜索范围内的文件改动，包含折叠项；重复按不扩大到搜索之外 |
| `Ctrl+R` | 清空勾选 |
| `i` | **反选**当前搜索范围内的文件改动，包含折叠项 |
| `M` | 鼠标开关：开启可拖拽滚动条；**关闭才能正常选中文本复制**（macOS 也可按住 Option 拖选） |
| `T` | 切换「改动 / 全部文件」，保留当前路径（仍存在时）、筛选和有效勾选 |
| `S` | 切换排序 |
| `/` | 搜索文件 |
| `R` / `F5` | 立即重新扫描；保留光标、折叠、过滤和有效勾选 |
| `Esc` | 清空已确认的文件过滤；无过滤时退出 |

面板和弹窗统一使用终端原生圆角边框（`╭ ╮ ╰ ╯`），显示效果取决于终端字体。

默认只显示改动树，连续的单子目录合并成一行；`T` 可查看按需加载的完整工程树。
目录用 `[ ]`、`[-]`、`[x]` 表示未选、部分选中、全部选中，并显示「已选/可选」文件数。
目录复选框只批选具体文件，不把目录作为递归提交目标；未纳管项仍需先添加。
树底部显示勾选总数与隐藏勾选数；折叠、搜索不丢失已有勾选，提交清单包含全部有效勾选。

窗口足够宽时左右显示文件树和预览；窄窗口自动改为上下排列。高度不足时优先显示文件树，选中文件按 `Enter` 查看全屏预览。底栏快捷键会随宽度换行，更多操作可按 `?` 查看。

主界面自动合并外部文件变化后刷新；提交、确认、搜索和后台操作期间暂缓，回到空闲主界面后补扫。顶栏显示刷新方式、距上次成功扫描的时间和状态数量，底栏随选中项提示可用操作。监听启动失败时会提示原因，仍可按 `R` / `F5` 手动刷新。

只有**真正能提交**的文件（`M A D R C T !`）才显示复选框。
干净文件、未版本化、忽略项、目录留空——`svn commit` 本来就会跳过它们，
给它们画框只会让人以为勾上了就会提交。

### 操作

| 键 | 作用 |
|---|---|
| `C` | 提交（勾选范围） |
| `a` | 添加当前未纳管文件；**目录**会先列出后代清单供裁剪再添加 |
| `A` | 列出全部未纳管目标（含过滤外项目），确认后批量添加 |
| `d` | `svn delete` 连本地文件一起删（**高危**，不可撤销） |
| `D` | `svn delete` 只移出版本控制、本地文件留着（中危）；两种都能在确认框里再切开关 |
| `r` | 丢弃本地改动：有勾选时处理勾选项（含隐藏项），否则处理当前文件或目录；先列清单，仅 `y/Y` 执行，`Enter` / `N` / `Esc` / `q` 取消 |
| `u` | `svn update` 到最新版本 |
| `U` | 更新到指定版本：填版本号执行 `svn update -r <版本>`（也支持 `1200:1234`） |
| `V` | 远端仓库浏览（无需本地工作副本） |
| `B` | 分支工作流：建分支 / 打标签 / 切换分支 / 合并预演 |
| `X` | 冲突面板 |
| `E` | 用外部编辑器打开（`$VISUAL` / `$EDITOR` / `vim`） |
| `H` | 工作副本体检 |
| `L` | 提交历史 |
| `Ctrl+B` | `svn blame`：这一行最后是谁改的、在哪一版改的 |
| `Ctrl+K` | `svn cleanup`：解锁、清中断操作；删文件的开关默认关闭，打开即升高危 |
| `Ctrl+L` | `svn lock` / `unlock`（一个开关切换，加锁会挡住别人提交） |
| `I` | `svn info`：这个工作副本到底连着哪个仓库 / 分支 / UUID |
| `Ctrl+P` | 项目切换：一台终端里换工作副本，不用再另开终端 `cd` |

撤销清单会重新查询本地 SVN 状态，逐项说明后果。目录按 SVN 规则递归撤销，执行时目录内的新版本改动也可能被包含；普通 `svn add` 新增项取消添加后保留本地文件，未纳管文件不受影响。复制、移动、替换项会单独提示可能删除新增内容。

### 冲突面板（`X`）

| 键 | 作用 |
|---|---|
| `j` / `k` | 切换冲突文件 |
| `1` / `2` / `3` 或 `Tab` | 切出对比的那一版：我的 / 服务器的 / 磁盘现状 |
| `Enter` | **保留现状**（存盘即标记为已解决，最保守） |
| `m` | 用我的版本 |
| `t` / `r` | 用服务器的版本（`r` 在普通冲突下与 `t` 同义） |
| `Esc` / `q` | 返回 |

> 树冲突**不能** resolve 到 `mine-full` / `theirs-full`（svn 会报 E155027），
> 所以树冲突下 `r` 走的是 revert + update，`m` 表达成「坚持磁盘现状」。

### 项目切换（`Ctrl+P`）

列出这台机器上已知的工作副本，选中直接切过去 —— 换项目不用再"另开终端、`cd`、再敲一次"。

| 来源 | 怎么来的 | 说明 |
|---|---|---|
| 登记 | 配置 `[[projects]]`，或面板里按 `a` / 命令行 `svnui projects --add` | 稳定，可起别名 |
| 打开过 | TUI 或 `svnui status` 扫过的仓库自动记住 | 零配置 |

当前所在的副本一定在列表里并标「当前」——它可能既没登记也没留缓存，但"我现在在哪儿"是这个面板必须能回答的。

| 键 | 作用 |
|---|---|
| `↑↓` / `j` `k` | 选择 |
| `Enter` | 切过去 |
| `a` | 把当前副本登记进清单 |
| `d` | 从清单移除（当前正在用的那个不让删） |
| `/` | 过滤（匹配名字或路径） |
| `Esc` | 返回 |

列表最后一条是「＋ 检出新工作副本」。面板里的所有信息都来自配置和缓存文件，**不跑 `svn status`** ——
它存在的理由是省时间，一旦要等就退化成"另一个要等的东西"。

不在工作副本里启动 `svnui` 时：有已知项目就先进这个面板，一个都没有才进检出界面。

### 提交框

| 键 | 作用 |
|---|---|
| `Enter` | **换行**（不是提交） |
| `Ctrl+S` / `F2` | 提交 |
| `Esc` | 退出，内容存草稿 |

退出提交框不会丢内容——再按 `C` 会恢复上次的草稿。
提交失败时草稿和勾选都会保留，按 `C` 继续即可。

### 确认框

危险操作都会先弹确认框：**先说清做什么、影响哪些文件、代价是什么**，再让你选。

| 键 | 作用 |
|---|---|
| `y` / `Y` | 执行 |
| `Enter` | 执行；但**丢弃本地改动**和**分支操作**例外 —— 这两类 Enter 是取消，必须明确按 `Y` |
| `N` / `Esc` / `q` | 取消 |
| `Tab` | 在开关、输入框、清单之间循环焦点 |
| `Space` | 清单可裁剪时剔除/加回光标项，否则切换聚焦的开关 |
| `j` / `k` | 清单可裁剪时上下移动光标 |
| `Ctrl+A` / `Ctrl+R` | 清单全选 / 全部剔除 |

标题里的**危险等级**（Safe / Low / Medium / High）由 `policy` 统一判定，边框颜色跟着等级走：切换开关会实时改写等级——比如删除时打开「保留本地文件」就从高危降到中危，边框立刻变色。

**批量**操作（新增、撤销丢弃、加锁）给出的是**文件级精确清单，可以在执行前裁剪**：`j/k` 移动、`Space` 剔除、`Ctrl+A` 全选、`Ctrl+R` 清空，标题上实时显示「选中的 / 总数」。
批量新增（`A`）列出**全部**未纳管目标——**包含当前过滤之外的**——并按过滤条件预选：不会因为你在搜索框里打了字就悄悄漏掉一半目录，也不会自作主张全勾上。

### 预览区

| 键 | 作用 |
|---|---|
| `Tab` / `Shift+Tab` | 在文件树与预览之间切换焦点，高亮边框标识当前面板 |
| `↑` / `↓` 或 `k` / `j` | 预览激活后逐行滚动，文件树选中项保持不变 |
| `PageUp` / `PageDown` 或 `Ctrl+u` / `Ctrl+d` | 预览激活后按可见高度翻页 |
| `Home` / `End` 或 `g` / `G` | 预览激活后跳到顶部 / 底部 |
| `←` / `→` | 横向慢移；预览激活后也可用 `h` / `l` |
| `<` / `>` | 横向翻页 |
| `0` | 回到最左 |
| `Esc` | 预览激活时返回文件树，保留过滤与滚动位置 |
| `J` / `K` | 文件树焦点下直接滚动预览（兼容原快捷键） |
| `o` | 用系统默认应用打开当前文件（图片全屏内也可用） |

预览激活后只处理阅读按键；添加、撤销和提交等文件操作需先返回文件树。窗口过小时预览隐藏，焦点自动返回文件树，仍可按 `Enter` 全屏查看文件。

支持文本、代码、CSV/TSV、Excel、Word（`.docx`）以及 PNG、JPEG、WebP、GIF、BMP、ICO 图片。GIF 显示静态首帧，SVG 按源码预览。图片按比例适应窗口，文件树或预览焦点下按 `Enter` 都可全屏查看；终端支持时使用图片协议，否则使用字符色块预览。

Ghostty/Kitty 使用 Kitty 图片协议，iTerm2/WezTerm/Rio 使用 iTerm2 图片协议；其他终端及 tmux/screen 使用兼容色块。根据终端标识选择协议，不读取键盘输入进行能力探测。

压缩包及超过 **2 MiB** 的文件默认关闭内容预览，只显示原因和大小；图片、办公文档也遵守此上限。图片额外限制解码内存为 64 MiB、单边尺寸为 8192 像素。文本最多显示 5000 行。

### 日志界面

| 键 | 作用 |
|---|---|
| `j` / `k` | 选择版本 |
| `Enter` | 查看该版本改动 |
| `d` | 把当前版本设为**对比基准** |
| `D` | 与基准做**区间对比**（两个版本之间改了什么） |
| `r` | 回退到选中的版本（高危，会先列清单） |
| `R` | 刷新：重新拉取提交记录 |
| `/` | 搜索（Enter 确认走**服务端**搜索，能搜到已加载范围之外） |
| `N` | 加载更早一页（默认 100 条，可用 `log.page_size` 改） |
| `Esc` | 返回 |

右栏分上下两块：上面是完整提交信息，下面是改动文件列表。

---

## 命令行

### 分支工作流

TUI 主界面按 `B` 打开分支面板：`F1` 创建分支、`F2` 创建标签、`F3` 切换、`F4` 合并预演。
`Tab` 切字段，`Ctrl+U` 清空当前字段，`Ctrl+S` 准备操作。创建和切换须在确认框按 `Y`；`Enter` / `N` / `Esc` 取消。

```bash
# 目标父目录必须已存在；^/ 表示当前 SVN 仓库根。
svnui branch '^/branches/feature-a' -m '创建 feature-a' --dry-run
svnui branch '^/branches/feature-a' -m '创建 feature-a' --yes
svnui tag '^/tags/v1.0' -r HEAD -m '发布 v1.0' --yes
svnui switch '^/branches/feature-a' --dry-run
svnui switch '^/branches/feature-a' --yes
svnui merge-preview '^/trunk'
svnui merge-preview '^/trunk' -r 12:18
```

创建分支和标签会**立即提交仓库副本**，不会自动切换，也不包含本地未提交改动。
默认复制当前工作副本根的 `BASE` 版本，可用 `-r` 指定整数版本、`HEAD` 或 `BASE`；标签与分支采用同样的 SVN copy 机制，不提供不可变标签保证。
准备阶段会固定实际版本号；重复目标、跨仓库 URL 和越级路径会被拒绝。

切换与合并预演要求整个工作副本干净，包括无未纳管项和已切换子树；不处理 externals。
`merge-preview` 始终运行 SVN 原生 `--dry-run`，不会应用改动或提交；`N:M` 表示两个端点之间的差异。
创建/切换的 `--dry-run` 只读取信息并打印计划，不执行 copy/switch；非交互执行写操作必须显式给 `--yes`。

### 通用参数

`svnui` 同时是可以直接脚本调用的 CLI。全局参数：

```
--cwd PATH       工作目录
--json           JSON 信封输出 {"ok":..}
--porcelain      紧凑两字符格式，如 "AM src/main.rs"
--timeout N      超时秒数（默认 30）
--trust-cert     放行 SSL 证书校验失败
--no-cache       禁用状态缓存
--cache-ttl N    缓存秒数（默认 300）
```

子命令：

| 命令 | 说明 |
|---|---|
| `probe` | 环境自检 |
| `status` | 工作副本状态 |
| `log` | 提交历史（`-l` 条数、`-r` 版本/区间、`--oneline`） |
| `diff` | 差异（`-c REV` 看某次提交、`--stat`、`--page`） |
| `info` | 仓库信息 |
| `blame` | 逐行追溯 |
| `add` | 加入版本控制 |
| `remove` | 移除（`--keep-local` 保留本地文件） |
| `commit` | 提交（`-m` 信息、`--dry-run`） |
| `update` | 更新（默认 `--accept postpone`，冲突留给人工） |
| `resolve` | 解决冲突（`-a` 策略白名单校验） |
| `revert` | **回滚本地改动，不可撤销**（`--dry-run` 先看清单） |
| `cleanup` | 清理（`--remove-unversioned` 等危险开关） |
| `purge-mine` | 清理已解决冲突残留的 `.mine` / `.rOLD` / `.rNEW`（仍在冲突中的不动） |
| `doctor` | 体检：锁 / 冲突 / 缺失 / 阻碍 / `.mine` 残留 |
| `conflicts` | 列出所有冲突 |
| `checkout` | 检出（唯一不需要先有工作副本的命令） |
| `login` / `auth` | 凭据验证 / 列出已缓存凭据 |
| `preview` | 独立预览本地文件或浏览 SVN URL，无需进入工作副本 |
| `projects` | 已知工作副本清单：列出 / `--add` 登记 / `--remove` 移除（打开过的仓库会自动记住） |
| `tui` | 打开交互式 TUI（不带子命令时的默认形态也是它） |
| `llm` | LLM 配置示例与自检（`--preset` / `--test`） |
| `config` | 配置文件状态与初始化（`--init`） |
| `cache` | 查看状态缓存 / `--clear` 清除 |
| `daemon` | 常驻进程（`start` / `status` / `stop` / `rescan`） |

> `q` 和 `install-yazi` 是早期 yazi 集成阶段的遗留命令，代码还在，
> 但本项目已经转向独立 TUI，日常用不到。

### 危险操作分级

| 级别 | 操作 | 行为 |
|---|---|---|
| Safe | `status` `diff` `log` `info` | 直接执行 |
| Low | `add`、`cleanup`（仅解锁） | 直接执行 |
| Medium | `commit` `update` `resolve` `remove` `purge-mine` | TTY 下确认；非 TTY **必须 `--yes`** |
| High | **`revert`**、`cleanup --remove-unversioned` | TTY 下确认；非 TTY **一律拒绝** |

`revert` 会丢弃已纳管内容的本地改动；`cleanup --remove-unversioned` 会删除未纳管文件，
是本工具里最危险的两个，务必先 `--dry-run`。

---

## AI 生成提交信息

提交面板按 `Ctrl+G` 生成可编辑候选，再按一次轮换候选。生成不会自动提交，已有编辑内容不会被覆盖；
未启用 AI 或请求失败时使用本地规则，并显示当前来源或回退原因。

### 复用本机 Codex（推荐）

Codex 通道从 **0.7.2** 起提供，安装上面的正式版即可使用，无需从源码编译。
需要安装 Codex CLI，并在启动 svnui 的同一终端中确认程序和登录可用：

```bash
codex --version
codex login status
svnui llm --preset codex
```

最后一条只输出配置示例，不联网、不写配置。将示例合并到已有配置的 `[llm]` 段，避免重复段：

```toml
[llm]
enabled = true
provider = "codex"
codex_bin = "codex"
timeout_secs = 90
send_diff = false
```

无需再填写模型、服务商地址或 API Key：由 Codex CLI 自己读取本机配置与登录，尊重 `CODEX_HOME`。
`codex_bin` 也可填可执行文件的绝对路径。切换为 Codex 后，原有 `base_url`、`model`、`api_key_env`
不再参与请求；桌面端某次聊天临时选择的模型不保证等于 CLI 配置，需以 CLI 实际生效配置为准。

```bash
svnui llm        # 查看通道和生效策略，不联网
svnui llm --test # 只发送最小 OK 请求，验证 Codex 实际连通性
```

生成在独立临时目录运行，只传入本次勾选的文件清单，启用 `send_diff` 后才附带差异正文。
任务禁用工程说明、技能说明、MCP、插件、hooks 和内置工具，不执行自动提交；超时返回本地规则。
Codex CLI 仍可能附加其全局用户说明，这部分也会进入模型上下文；这里不复制或改写你的全局规则。
Unix 上超时会清理任务所在进程组，临时输出在成功和失败后都会移除。Codex 首次启动和继承的推理设置
可能增加耗时，可按需调整 `timeout_secs`。本机 CLI 版本兼容性与常见问题见 [Q&A](docs/qa.md#ai-commit)。

### 已有 HTTP 模型配置

已有的 DeepSeek、智谱、Ollama 通道继续兼容；暂不扩展更多模型适配。
`svnui llm --preset zhipu` / `svnui llm --preset ollama` 可查看原有示例。
仅这些 HTTP 通道使用 `base_url`、`model` 和 `api_key_env`；显式地址与模型优先于预设。
云端 Key 默认由 `SVNUI_LLM_KEY` 环境变量提供；Ollama 由用户自行准备本地服务和模型，不发送 Authorization。

所有通道默认只发送勾选文件的路径与状态；配置 `send_diff = true` 后才发送 diff 正文，
受 `max_diff_bytes` 的 UTF-8 字节预算限制。`--test` 不发送工作副本内容，实际请求会使用对应模型服务的额度。
生成失败保留本地规则候选，不会自动切换服务商。配置路径支持 `SVNUI_CONFIG`、`XDG_CONFIG_HOME`，
默认 `~/.config/svnui/config.toml`。

## 缓存

`svn status` 在大仓库上是秒级到几十秒级。缓存的目标是**命中时一次 svn 都不跑**。

`svn status` 只输出非 normal 的条目，所以缓存里天然没有"干净的文件"。
于是"干净文件被改脏"和"新建未版本化文件"这两种情况**都检测不到**——
修改已有文件连父目录 mtime 都不改，创建文件也只改父目录。

所以缓存额外存了工作副本内每个文件/目录的 `(mtime, size)`（跳过 `.svn`），
加载时重新走一遍树逐条比对，全一致才认为状态未变。
因为遍历每次都做，正确性与缓存年龄无关，TTL 只是安全阀。

边界：

- 超过 200,000 个节点不写缓存
- `.svn/wc.db` 不存在（SVN 1.6 及更早）→ 不缓存
- `--ignored` / `-u` 这类非默认查询不走缓存
- 所有写操作后自动作废相关缓存

---

## 构建开关

源码构建需要 Rust 1.88 或更新版本。

```toml
[features]
default = ["tui", "docs"]
tui     = ["dep:ratatui", "dep:ratatui-themekit", "dep:crossterm", "dep:tui-textarea", "dep:ratatui-image", "dep:image"]
docs    = ["dep:csv", "dep:calamine", "dep:docx-lite"]
```

| 场景 | 命令 |
|---|---|
| 完整 | `cargo build --release` |
| 只要 CLI（省掉 crossterm 一堆依赖） | `cargo build --release --no-default-features` |
| 不要办公文档预览 | `cargo build --release --no-default-features --features tui` |

`docs` feature 提供 CSV/TSV 表格排版和 Excel/Word 内容提取。关掉后 CSV/TSV 按普通文本预览，Excel/Word 提示未启用文档预览。图片支持随 `tui` feature 编译，无需安装额外插件或系统图片工具。

> ratatui 0.30 默认配 crossterm 0.29，不同主版本各自维护事件队列和 raw mode，
> 所以 `crossterm` 钉在 0.29、`tui-textarea` 用维护分支 `tui-textarea-2` 0.13。
> 升级时这三个要一起动。

---

## 开发

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

分层与硬约束：

```text
main → cli → svn::client → svn::command
                  ↓
               domain   （纯模型，谁都能依赖）
```

1. **`domain` 零 IO** — 只放数据结构与纯函数
2. **`svn` 是唯一能 spawn 进程的地方** — 别处不许 `Command::new`
3. **永不 `sh -c`** — 全部参数数组传递，防注入

文档：[命令参考](docs/cli.md)、[快捷键](docs/keymap.md)、[主题配置](docs/theme.md)、[Q&A](docs/qa.md)、[专项故障排查](docs/troubleshooting.md)。
发布流程见 [PUBLISH.md](PUBLISH.md)。

---

## Q&A

安装、认证、主题、文件树、撤销和发布中遇到的问题统一记录在 **[Q&A](docs/qa.md)**，按“现象 → 原因 → 处理 → 验证”维护。

- [手动下载安装、选架构、Gatekeeper](docs/qa.md#manual-install)
- [安装后找不到命令，或升级后仍运行旧版](docs/qa.md#old-version)
- [SVN 登录、证书和子路径权限](docs/qa.md#authentication)
- [Codex 生成提交信息、登录、模型继承与超时](docs/qa.md#ai-commit)
- [主题无颜色、中文 URL、图标与提示详情](docs/qa.md#appearance)
- [目录批选、隐藏勾选、撤销 E155010](docs/qa.md#selection)
- [发布遗留锁、离线依赖、Rust 工具链、Homebrew 版本不同步](docs/qa.md#publishing)
- [Yazi、缓存、daemon 和工作副本排查](docs/qa.md#other-troubleshooting)

---

## License

MIT
