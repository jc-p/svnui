# svnui

Subversion 的 TUI 客户端。用 Rust + [ratatui](https://github.com/ratatui/ratatui) 写的独立终端程序，
不依赖 yazi，也不需要额外配置就能用。

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

<!-- BEGIN svnui:auth -->
## SVN 认证配置

svnui 调用 svn 时**强制加 `--non-interactive`** —— TUI 里没法回答
"是否接受证书 / 请输入用户名"这类交互式提问，不加就会挂死。
所以凭证和证书都必须**提前处理好**，不能指望运行时现问。

### 凭证存在哪

```
~/.subversion/auth/svn.simple/     # 按 realm（服务器 + 路径）分文件存放
```

第一次连接某个服务器时，先交互式认证一次把凭证存下来：

```bash
svn info <仓库 URL>          # 按提示输入用户名密码，之后就不问了
# 或
svnui login                  # 走 svnui 自己的登录流程
```

存下之后 svnui 的所有命令都用缓存，不再索要用户名密码。
**检出面板里的"用户名 / 密码"可以留空**，留空就不传 `--username/--password`，
svn 自己按 realm 挑缓存里的那条。

缓存不对或过期：删掉 `svn.simple/` 下对应的文件，或重跑 `svnui login`。

### 自建 / 内网服务器：自签名证书

症状：

```
svnui update
SSL 证书校验失败（svn 退出码 1）
```

原因就是开头那句 —— 命令行 `svn` 遇到证书会弹 `(p)` 让你永久接受，
svnui 不能弹，于是直接失败。

解法（二选一）：

```bash
svnui update --trust-cert                  # 单次
export SVNUI_TRUST_CERT=1                  # 长期，写进 ~/.zshrc
```

对应的 svn 参数是
`--trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other`。

> 改完 `~/.zshrc` 要 `exec zsh` 或重开窗口才生效。
> 用 `env | grep SVNUI` 确认，没输出就是没生效。
>
> 放行证书等于放弃防中间人，只对自己确认过的内网服务器这么干。

### 错误码速查

svn 的报错有个坑：**它把"结果"放在前面、"原因"放在后面**。
同时出现两个码时，第一个往往是结果，照它排查会跑偏。

| 错误码 | 真实含义 | 怎么办 |
|---|---|---|
| `E170013` | 连不上（多半是**结果**） | 往下看有没有 `E175013` |
| `E175013` | 该路径**无权访问** | 见下面"按子树授权" |
| `E230001` | 证书问题 | `--trust-cert` |
| `E170001` | 没有可用用户名 | 先做一次交互式认证缓存凭证 |
| `E215004` | 凭证不对 / 过期 | 删缓存重登 |
| `E155007` | 不是工作副本 | `cd` 进工作副本再跑 |
| `E000111` | 连接被拒 | 查 `~/.subversion/servers` 里的 `http-proxy` |

### 按子树授权：能进子目录，但看不了仓库根

很多 SVN 是按路径授权的。你能访问 `/trunk/A8_Patch/...`，
不代表能 `svn list` 仓库根 `/JR/KAMP` —— 后者会报 `E175013 forbidden`。

仓库浏览面板（`V`）**从当前工作副本的 URL 起始**，而不是仓库根，
就是为了避开这个。往上退到无权的一层会自动返回上一层并提示。

### 诊断命令

```bash
# 仓库根（注意是 repos-root-url，不是 repository-root）
svn info --show-item repos-root-url

# 复现 svnui 的调用方式：非交互 + 放行证书
svn list --xml --non-interactive \
  --trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other \
  "<URL>"
```

### 环境变量

| 变量 | 作用 |
|---|---|
| `SVNUI_TRUST_CERT=1` | 放行自签名 / 内网证书 |
| `SVN_CONFIG_DIR` | 指定 svn 配置目录（不搜 `~/.subversion`） |
<!-- END svnui:auth -->
## 安装

### Homebrew（推荐）

```bash
brew tap jc-p/svnui
brew trust jc-p/svnui      # 第三方 tap 需要显式信任，见「注意」
brew install svnui
```

升级：

```bash
brew upgrade svnui
```

### 从源码

```bash
git clone https://github.com/jc-p/svnui.git
cd svnui
cargo build --release
sudo cp target/release/svnui /usr/local/bin/
```

### 本地源码更新

修改源码后，在项目根目录运行一键更新脚本：

```bash
./install-local.sh
```

脚本离线编译 Release，安装到 `/usr/local/bin/svnui`，需要时提示输入 sudo
密码，并核对安装后的二进制和 PATH。编译或安装失败会立即停止。
依赖需已缓存在本机；已编译好时可加 `--no-build` 直接安装。
更新完成后退出旧界面，重新运行 `svnui tui`。

---

## 快速上手

```bash
cd 你的工作副本
svnui tui
```

> 子命令是必填的，直接敲 `svnui` 会报 clap 的参数错误。
> 嫌长可以加 alias：`alias s='svnui tui'`。

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

### 内网自签名证书（公司 SVN 常见）

如果报 `E230001: Server SSL certificate verification failed`：

```bash
export SVNR_TRUST_CERT=1          # 写进 ~/.zshrc 永久生效
# 或单次：svnui --trust-cert login -u 你的账号
```

等价于 svn 的
`--trust-server-cert-failures=unknown-ca,cn-mismatch,expired,not-yet-valid,other`。

> ⚠️ 这等于放弃对中间人攻击的防护，只在确认是**自己的内网服务器**时启用。

---

## TUI 键位

### 文件树（主界面）

| 键 | 作用 |
|---|---|
| `j` / `k` 或 `↑` `↓` | 移动 |
| `Enter` | 目录展开/折叠；文件进全屏预览 |
| `Space` | **勾选/取消**（提交及撤销范围） |
| `Ctrl+A` | 分级全选：第一次勾**看得见的**可提交项，再按一次把隐藏项也带进来（会告诉你多带了多少） |
| `Ctrl+R` | 清空勾选 |
| `i` | **反选**当前可见项（「除了这几项都要」时不用先全选再挨个取消） |
| `M` | 鼠标开关：开启可拖拽滚动条；**关闭才能正常选中文本复制**（macOS 也可按住 Option 拖选） |
| `T` | 切换「仅变更 / 全部文件」 |
| `S` | 切换排序 |
| `/` | 搜索文件 |
| `R` / `F5` | 立即重新扫描；保留光标、折叠、过滤和有效勾选 |
| `Esc` | 清空已确认的文件过滤；无过滤时退出 |

面板和弹窗统一使用终端原生圆角边框（`╭ ╮ ╰ ╯`），显示效果取决于终端字体。

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

提交面板按 `Ctrl+G` 生成可编辑候选，再按一次轮换候选。生成不会自动提交，已有编辑内容不会被覆盖。默认未启用 AI 时使用本地规则。

先查看配置示例（不联网、不写文件，也不要求 SVN 工作副本）：

```bash
svnui llm --preset zhipu
svnui llm --preset ollama
```

将所需示例合并到 `~/.config/svnui/config.toml` 的 `[llm]` 段；也支持 `SVNUI_CONFIG` 或 `XDG_CONFIG_HOME`。不要重复添加同名段。最小配置：

```toml
[llm]
enabled = true
provider = "zhipu" # 或 "ollama"
base_url = ""
model = ""
send_diff = false
```

- `zhipu`：预设 `glm-4.7-flash` 与智谱 OpenAI 兼容地址。官网目前将其列为[免费模型](https://docs.bigmodel.cn/cn/guide/models/free/glm-4.7-flash)，调用仍需申请 API Key；可用性、限流和费用以服务商当前政策为准。将 Key 放入 `SVNUI_LLM_KEY` 环境变量，不要写入配置或提交到仓库。
- `ollama`：预设 `http://localhost:11434/v1`、`qwen2.5-coder:3b`，不需要 Key，也不发送 Authorization。需自行准备 Ollama 服务并下载模型（例如 `ollama pull qwen2.5-coder:3b`）；冷启动较慢时建议 `timeout_secs = 60`。工具不自动安装或启动服务。协议见 [Ollama 文档](https://docs.ollama.com/openai)。
- `deepseek`：兼容既有默认选择，需要 Key。其他兼容服务继续通过 `base_url`、`model` 覆盖，不必新增 SDK。

**显式地址和模型优先于预设。** 切换 `provider` 时，要清空旧的 `base_url` / `model`，或改成你要的覆盖值。模型覆盖示例：`model = "你的本地模型名称"`。

```bash
svnui llm        # 查看实际生效的预设、地址、模型和 Key 状态
svnui llm --test # 发送一个最小请求；失败返回非零退出码
```

云端测试需要先在当前终端设置 Key；模型服务未启动、Key 缺失、认证失败、模型不存在、限流或超时都会给出提示。不会自动切换付费模型。测试不会发送工作副本内容；提交面板生成时默认只发送勾选文件的路径与状态。若 `[llm] send_diff = true`，才会读取并发送 diff 正文（受 `max_diff_bytes` 截断，默认 20000 字节）；设为 `true` 前请确认合规允许代码出网。生成失败保留本地规则候选，并在面板中显示回退原因。

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
tui     = ["dep:ratatui", "dep:crossterm", "dep:tui-textarea", "dep:ratatui-image", "dep:image"]
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

文档：`docs/cli.md`（命令参考）、`docs/keymap.md`、`docs/theme.md`、`docs/troubleshooting.md`。
发布流程见 `PUBLISH.md`。

---

## 注意：macOS 上的两个坑

**1. Gatekeeper**

通过 brew 安装的二进制带 quarantine 属性，首次运行会弹「无法打开，因为来自身份不明的开发者」：

```bash
sudo xattr -d com.apple.quarantine $(which svnui)
```

或者：系统设置 → 隐私与安全性 → 点「仍要打开」。
本地 `cargo build` 出来的没有这个问题，所以自己编译可能从来没遇到过。

**2. Homebrew 的 rust 装不了交叉编译 target**

如果机器上同时有 `/opt/homebrew/bin/cargo` 和 `~/.cargo/bin/cargo`（rustup），
`rustup target add x86_64-apple-darwin` 装进的是 rustup 那套，
而 `cargo build` 可能用的是 Homebrew 那套，于是报
`can't find crate for core` + 一句误导性的 `the target may not be installed`。

确认并修复：

```bash
which -a cargo                                    # ~/.cargo/bin 应该排第一
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
```

---

## License

MIT
