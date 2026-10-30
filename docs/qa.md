# Q&A

适用于 svnui 0.7.1。先运行 `svnui --version`、`command -v svnui`，确认正在使用哪个程序。
本文集中记录实际遇到的问题；发布与源码步骤另见 [PUBLISH.md](../PUBLISH.md)，配色选项见 [主题配置](theme.md)。

[下载与安装](#manual-install) · [认证与证书](#authentication) · [外观与输入](#appearance) ·
[文件树与撤销](#selection) · [构建与发布](#publishing) · [其他排查](#other-troubleshooting)

<a id="manual-install"></a>
## 下载与安装

### Q：应该下载哪个文件？Source code 能直接运行吗？

官方二进制目前提供 macOS 11+ 两种架构。执行 `uname -m`：`arm64` 选择
`aarch64-apple-darwin`，`x86_64` 选择 `x86_64-apple-darwin`。Rosetta 终端可能报告 `x86_64`，
此时选择 Intel 包也可以在该运行环境中使用。

下载 [最新 Release](https://github.com/jc-p/svnui/releases/latest) 中的 `svnui-版本-架构.tar.gz`。
**Source code** 是源码，需要 Rust 编译。其他系统没有对应预编译包时，从源码构建，不要尝试运行 macOS 包。
无法判断架构时停在 `uname -m` 的检查结果，不要关闭系统保护来强行运行不匹配的程序。

### Q：不用在线安装脚本，怎样手动安装？

以 Apple Silicon 的 0.7.1 为例：将 [安装包](https://github.com/jc-p/svnui/releases/download/0.7.1/svnui-0.7.1-aarch64-apple-darwin.tar.gz)
和 [SHA256SUMS](https://github.com/jc-p/svnui/releases/download/0.7.1/SHA256SUMS) 下载到 `~/Downloads`。

先校验实际下载的那一个包，预期显示文件名与 `OK`：

```bash
cd "$HOME/Downloads"
awk '$2 == "svnui-0.7.1-aarch64-apple-darwin.tar.gz"' SHA256SUMS | shasum -a 256 -c -
```

只在校验通过后安装；Intel 用户将文件名中的 `aarch64` 换成 `x86_64`：

```bash
mkdir -p "$HOME/.local/bin"
tar -xzf svnui-0.7.1-aarch64-apple-darwin.tar.gz -C "$HOME/.local/bin" svnui
chmod 755 "$HOME/.local/bin/svnui"
"$HOME/.local/bin/svnui" --version
```

预期版本为 `svnui 0.7.1`。校验失败或下载内容不是 tar.gz 时，停止安装并重新下载官方附件。
不要把 `shasum -c SHA256SUMS` 对“未下载的另一架构包”的缺失提示误认为当前包损坏。

### Q：GitHub 下载失败，或者下载回来的是 HTML？

使用 README 中的官方 Release 附件直链。`curl` 需要 `-L` 跟随跳转、`-f` 在 HTTP 错误时失败；
本项目安装脚本已包含这两项，并检查 SHA256、压缩包内容和二进制版本。
本次实测遇到过 `github.com:443` 超时、`raw.githubusercontent.com` 连接重置，但 GitHub API 仍可用。
这不是“Release 不存在”；源码脚本域名和 Release 附件域名也可能分别被网络策略阻断。

如果已安装且能够使用 GitHub CLI，可通过它下载官方附件，再按上面的手动步骤校验安装。
以下 Apple Silicon 命令已验证；Intel 用户将 `aarch64` 替换为 `x86_64`：

```bash
mkdir -p "$HOME/Downloads/svnui-0.7.1"
gh release download 0.7.1 --repo jc-p/svnui \
  --pattern 'svnui-0.7.1-aarch64-apple-darwin.tar.gz' \
  --pattern SHA256SUMS --dir "$HOME/Downloads/svnui-0.7.1"
cd "$HOME/Downloads/svnui-0.7.1"
```

此时手动校验命令从当前目录执行，无需再切回 `~/Downloads`。
GitHub CLI 也无法访问时，先处理本机网络；不会自动改走未知镜像，也不能跳过校验安装。

### Q：macOS 提示“无法打开”或开发者无法验证？

浏览器下载的程序可能带有 quarantine 属性。先核对官方来源和 SHA256，
再通过“系统设置 → 隐私与安全性 → 仍要打开”处理。

检查具体程序是否带有该属性：

```bash
xattr "$HOME/.local/bin/svnui"
```

确认来源后，如确需通过命令处理，只移除这个文件的 quarantine 属性：

```bash
xattr -d com.apple.quarantine "$HOME/.local/bin/svnui"
"$HOME/.local/bin/svnui" --version
```

如果属性本就不存在，不需要执行移除命令。不要全局关闭 Gatekeeper，也不要对整个目录递归移除属性。

<a id="old-version"></a>
### Q：安装成功，为什么提示 command not found，或者还是旧版本？

安装到 `~/.local/bin` 不会自动修改 shell 配置。本机也可能同时存在 Homebrew、
`/usr/local/bin` 和用户目录里的多个程序，版本号相同也不代表二进制内容相同。

```bash
type -a svnui
"$HOME/.local/bin/svnui" --version
export PATH="$HOME/.local/bin:$PATH"
hash -r
svnui --version
```

预期 `command -v svnui` 指向用户目录。需要永久生效时将该 `export` 加入 `~/.zshrc` 或 `~/.bashrc`。
退出已经打开的旧 TUI 后再启动，新安装不会替换内存中仍在运行的旧进程。

### Q：安装到 /usr/local/bin 需要 sudo，怎么办？

下载脚本默认安装到可写的 `~/.local/bin`。源码更新也可以指定同一目录：

```bash
mkdir -p "$HOME/.local/bin"
./install-local.sh --bin-dir "$HOME/.local/bin"
```

`install-local.sh` 构建当前源码；`install.sh` 下载已发布的二进制。无需覆盖由 root 管理的旧文件。

### Q：为什么运行 SVN 功能时提示没有 svn？

svnui 是界面和命令封装，SVN 操作仍调用本机 `svn`。检查并安装：

```bash
svn --version --quiet
brew install subversion
svnui probe
```

已有自定义 SVN 时，在配置的 `[svn]` 段设置 `svn_path`，也可用 `SVNR_SVN=/绝对路径/svn`。
独立预览本地文件 `svnui preview ./README.md` 不需要 SVN；网络仓库预览仍需要。

### Q：必须输入 svnui tui 吗？不在工作副本里能启动吗？

交互式终端直接运行 `svnui` 默认进入 TUI；没有当前工作副本时进入已知项目列表，
没有已知项目时进入检出界面。自动化脚本应显式使用 CLI 子命令，TUI 需要真实终端。
旧 README 中“子命令必填”的说明已经过时。

<a id="authentication"></a>
## 认证、证书与仓库权限

### Q：命令行 svn 能登录，TUI 为什么还提示认证失败？

svnui 的 SVN 调用带 `--non-interactive`，不能在界面里回答用户名、密码或证书确认。
先在普通终端完成认证，再进入 TUI。URL 要使用自己有权限的项目路径：

```bash
svn info "https://svn.example.com/repo/trunk/project/" --username yourname
```

按 SVN 提示输入密码并选择是否缓存；成功时应显示该路径的信息。不要把真实密码写进命令或截图。
能否保存取决于本机 SVN 的缓存策略与凭据存储；若下次仍要求密码，先检查 SVN 配置及系统钥匙串是否可用。

已有可用缓存时，可验证 svnui 的非交互调用：

```bash
svnui login "https://svn.example.com/repo/trunk/project/" -u yourname
```

`svnui login` 不会弹出密码输入框。省略 URL 时仅适用于当前工作副本；未检出时要传完整 URL。
检出面板的用户名、密码留空时，SVN 会尝试使用已有缓存，不代表匿名访问。

### Q：凭据存在哪里？密码改了怎么恢复？

SVN 的认证元数据通常在 `~/.subversion/auth/`，例如 `svn.simple/`；密码也可能交给系统钥匙串。
缓存按服务器声明的认证域（realm）匹配，同一认证域可能服务多个仓库。
先用上面的 `svn info ... --username ...` 重新认证，再验证 svnui；不要打印或分享缓存文件内容。

不要为单个仓库的问题直接运行 `svnui login --logout`：它会清理所有仓库的本地认证缓存。
如果怀疑缓存损坏，应先确认受影响的认证域，再针对该条记录处理。

### Q：自建或内网服务器报 SSL 证书校验失败怎么办？

先确认访问的是正确服务器；优先修复服务器证书或配置受信任的内部 CA。
原生 SVN 可交互确认的证书问题，TUI 不一定能处理，部分 `other` 类型失败也不能永久接受。
确认服务器可信且确需放行时，可单次执行：

```bash
svnui --trust-cert update
# 或启动 TUI
svnui --trust-cert tui
```

也可在当前 shell 设置 `export SVNUI_TRUST_CERT=1`，或在配置已有 `[svn]` 段中设置 `trust_cert = true`。
旧名 `SVNR_TRUST_CERT` 仍兼容。写入 shell 配置后，需重开终端才会影响新进程。
只检查开关是否开启可运行 `printenv SVNUI_TRUST_CERT`，不要输出所有环境变量。

该开关会放行 `unknown-ca,cn-mismatch,expired,not-yet-valid,other` 等证书失败，
范围比单纯信任内部 CA 更广；只在确认过的服务器上使用，不要把它当成通用网络修复。

### Q：E170013、E175013 等错误应该先看哪个？

SVN 的错误常把外层连接失败放前面、具体原因放后面。保留完整错误链，按最具体的一条判断：

| 错误码 | 排查方向 | 下一步 |
|---|---|---|
| `E170013` | 无法访问仓库，可能包裹其他错误 | 继续看后面的证书、权限或网络错误 |
| `E175013` | 路径访问被禁止 | 核对 URL、账号及子树授权 |
| `E230001` | 证书验证失败 | 检查证书和 CA，确认可信后再考虑 `--trust-cert` |
| `E170001` / `E215004` | 认证失败或非交互调用缺少可用凭据 | 用原生 SVN 重新认证并验证缓存 |
| `E155007` | 当前路径不是工作副本 | 进入正确副本，或在支持 URL 的命令中传完整地址 |
| `E000111` | 连接被拒绝 | 核对服务、端口及 SVN 代理配置，不先删凭据 |

无法判断时，通过 `F4`（工作流内 `Shift+F4`）复制完整诊断，反馈前遮住账号、内网地址等敏感信息。

### Q：能访问子目录，为什么仓库根报 forbidden？

SVN 可以按子树授权。能访问 `/repo/trunk/project/` 不代表有权浏览 `/repo/`。
用有权限的完整 URL 登录、检出和预览，不要为了排查而强行访问仓库根。
仓库浏览器从当前工作副本的 URL 开始；上退到无权访问的目录时会提示并保留可用位置。

在工作副本内查看服务器声明的根 URL：

```bash
svn info --show-item repos-root-url
```

注意参数是 `repos-root-url`，不是 `repository-root`。核对当前路径的非交互权限可运行：

```bash
svn list --xml --non-interactive "https://svn.example.com/repo/trunk/project/"
```

如果这里也失败，先处理底层 SVN 的认证、证书或授权；更换主题和字体不会改变访问权限。

<a id="appearance"></a>
## 外观、输入与项目切换

### Q：配置了主题，为什么界面还是没有颜色？

`NO_COLOR` 环境变量优先于 `[ui].theme`。只对这次启动去掉它：

```bash
env -u NO_COLOR svnui tui
```

在配置文件已有的 `[ui]` 段设置 `theme = "catppuccin"` 后重启；不要重复添加同名 TOML 段。
旧配置不写主题时默认 Catppuccin，未知名称会回退并提示；`terminal` 跟随终端配色。
无色模式通过形状、粗体、下划线与反显保留状态区别。

### Q：激活标题的竖条、项目标记和图标怎么区分？

0.7.1 使用 `●` / `○` 区分激活和未激活面板，用 `◆` / `◇` 标记当前工作副本和其他项目。
列表选中高亮表示光标位置，与“当前项目”是两个状态。标记使用普通 Unicode 字符，不依赖 Nerd Font。
如果仍看到旧竖条，先按[旧版本排查](#old-version)核对路径并退出旧进程。

### Q：一行成功提示为什么曾经占满屏幕？长错误去哪里看？

旧详情面板按整个窗口布局，短消息也占满高度。0.7.1 改为右上角通知和按内容定高的详情。
成功/信息保留 8 秒、警告 15 秒，错误自动打开并保留原始诊断。
`F4` 查看详情；工作流内 F4 用于合并预演，通知详情改用 `Shift+F4`。
详情中 `↑↓`、`PgUp/PgDn` 滚动，`Home/End` 首尾，`y` 复制全文，`Esc` 关闭。

### Q：从网页复制的中文 SVN 地址，标题为什么曾显示 %E5%8D…？

这是 URL 百分号编码，不是主题或字体问题。0.7.1 仅在显示标题时解码一次，
实际 SVN 请求仍使用原 URL。先升级并确认运行的是新程序；命令中用引号包住 URL，
不要自行重复编码、反复解码，或把 `+` 当作空格替换。

### Q：Ctrl+S 没反应，粘贴说明又像触发了 Esc？

部分终端的 IXON 流控会吞掉 `Ctrl+S`。提交说明准备好后可使用提交面板的 `C` 或 `F2`。
svnui 已启用 bracketed paste，防止粘贴转义序列被拆成 Esc。
先升级、退出旧 TUI 后再试；不要把带终端控制符的内容当普通命令粘贴到 shell 中执行。

### Q：为什么鼠标不能选中文字复制，或者预览突然不能操作？

鼠标捕获启用后，终端的原生文字选择会被 TUI 接管。按 `M` 关闭捕获后再选取，
或在支持的 macOS 终端中按住 Option 选择。`Tab` 切换文件树与预览焦点，`Esc` 返回文件树。
弹层显示时图片预览会暂缓，避免终端图片协议盖住文字；关闭弹层后再检查。

### Q：切换项目失败，或项目列表没有更新？

0.7.0 的补丁已让切换失败保留当前项目与面板，错误不会直接退出 TUI；登记、移除后会刷新清单。
当前副本始终列出并标记“当前”，不能从面板移除正在使用的副本。
升级后在 `Ctrl+P` 内检查，删除了实际目录的项目会标出不可用，不能靠重新高亮恢复目录。

<a id="selection"></a>
## 文件树、勾选与撤销

### Q：目录显示 [ ]、[-]、[x] 是什么？折叠和搜索会丢失勾选吗？

它们分别表示目录内文件改动未选、部分选中、全部选中。Space 在半选目录上会补齐当前搜索范围内的文件改动。
目录只是批选分组，不会因此作为递归提交目标；未纳管项需要先添加。
全选、反选遵守搜索范围并包含折叠项，已有的搜索范围外勾选会保留；
底部“隐藏”数量用于提示这些项目，提交清单仍包含全部有效勾选。

### Q：完整工程树为什么曾漏掉干净的兄弟文件？

“有改动子节点”不代表“已读取整个目录”。0.7.1 单独记录目录加载状态，
`T` 切换到全部文件后按需读取实际目录，保留筛选、勾选和可恢复的当前路径。
想检查整个工程时，先清空搜索再按 `T`；变更树压缩单子目录只改变显示，不改变实际操作路径。

### Q：大写 A 的清单包含搜索外文件，为什么没有全部加入 SVN？

确认清单列出全部扫描到的未纳管项，但存在搜索条件时只预选命中项。
需要把清单内的其他项也加入时，在确认框中显式按 `Ctrl+A`，核对数量后按 `Y`。
确认框打开后新出现的文件不自动带入，取消确认不写入 SVN。

### Q：目录撤销曾报 E155010，而且界面状态没有刷新？

旧执行路径递归撤销父目录后，再处理已解除纳管的新增子项，导致 `E155010` 和部分执行后的错误状态。
0.7.1 的 TUI 按确认清单浅撤销，先处理深层子项，再处理父目录，保留未选后代。
升级后重新扫描并核对实际文件；失败后不要假设“所有目标都没执行”，也不要直接重复破坏性操作。
CLI 显式目录撤销仍保留原来的递归语义。

<a id="publishing"></a>
## 源码构建与发布维护

### Q：离线构建报依赖未缓存，或者安装脚本看似没更新程序？

源码更新脚本使用 `--locked --offline`，首次构建前需显式准备依赖：

```bash
cargo fetch --locked
./install-local.sh --bin-dir "$HOME/.local/bin"
```

构建失败会停止安装。`--no-build` 只安装已有的 Release 构建，不能代替重新编译。
脚本会比较二进制内容、核对 PATH；不能仅因版本号相同就认定程序已更新。

### Q：已安装 target，为什么仍报 can't find crate for core？

可能同时使用了 Homebrew 的 Cargo 和 Rustup 的工具链，target 装在另一套 Rust 中。
先检查 `type -a cargo rustc`、`rustup show active-toolchain`，不要反复给同一错误环境安装 target。

要使用 stable 工具链构建 Intel 包，显式保证安装和构建使用同一套：

```bash
rustup target add --toolchain stable x86_64-apple-darwin
rustup run stable cargo build --release --locked --target x86_64-apple-darwin
```

如果 stable 尚未安装，先由维护者准备工具链。只想使用 svnui 时直接下载预编译包，不必配置交叉编译。

### Q：发布提示 dist/.release-0.7.1.lock 已有同版本任务？

该目录是发布互斥锁，正常退出会清理，强制终止可能留下空锁。
先检查是否仍有同版本 `release.sh`、`dist.sh` 或相应构建任务在运行；无法确认就停止，不能抢删锁。
**仅在确认没有运行任务后**，对确认过的遗留空锁执行：

```bash
rmdir dist/.release-0.7.1.lock
```

`rmdir` 对非空目录会拒绝删除。不要递归清空 `dist`，旧产物、manifest 和 SHA256SUMS 是恢复依据。
后续失败保留提交与已有 tag；恢复步骤见 [发布文档](../PUBLISH.md#失败恢复)。

### Q：同版本产物或 Release 已存在，能覆盖重发吗？

不能。相同版本目录只在提交、架构集合和校验和完全一致时复用；已发布附件拒绝覆盖。
二进制有变化时发新版本。构建失败保留现场，Release 成功而 tap 失败时仅重试 tap，
不要同时从本地和 Actions 发布同一 tag，也不要自动 reset 提交或删除 tag。

### Q：GitHub 已发布新版本，为什么 brew upgrade 还是旧版？

Release 和 `jc-p/homebrew-svnui` 的 formula 是两条发布链路。省略 `--tap-dir` 的本地发布只上传并校验 Release，
不会自动更新另一个仓库；Actions 未配置 tap 所需变量/凭据时也会跳过。

```bash
brew update
brew info jc-p/svnui/svnui
```

如果 formula 的版本仍旧，用户先使用 README 的下载脚本或官方附件；
维护者按 [PUBLISH.md](../PUBLISH.md) 显式同步 tap。反复重装旧 formula 不会得到尚未同步的版本。

### Q：macOS 系统 Bash 下，中文提示为什么会报 unbound variable？

系统 Bash 3 在变量名紧贴某些 UTF-8 标点时可能把标点字节算入变量名。
脚本输出中使用 `${REPORTED}` 等花括号明确边界，不写 `$REPORTED，`。
下载脚本的回归同时覆盖中文输出、空格目录和失败时保留旧程序。

<a id="other-troubleshooting"></a>
## 其他故障排查

以下问题的详细命令保留在 [故障排查](troubleshooting.md) 中，避免维护两份不同答案：

- [Yazi 列表不染色](troubleshooting.md#染色不生效)：需要在 `init.lua` 注册，按键触发才加载不足以启动 linemode。
- [Yazi 插件报错或状态栏空白](troubleshooting.md#yazi-plugin)：检查目录层级、PATH 和 Yazi 最低版本。
- [大仓库卡顿](troubleshooting.md#大仓库卡顿)：先对比原生 `svn status`，再检查缓存和 daemon，不把所有延迟归给 TUI。
- [状态显示不对](troubleshooting.md#状态显示不对)：区分属性改动、daemon 刷新残留与缓存指纹，用无缓存查询核对。
- [树冲突](troubleshooting.md#tree-conflict)：区分树与文本冲突，不能用可能丢目录改动的策略直接覆盖。
- [工作副本被锁](troubleshooting.md#工作副本被锁)：检查 SVN 锁和中断操作，先 cleanup、doctor；这与发布目录锁不同。
- [提交被拒绝](troubleshooting.md#提交被拒绝)：非交互执行中危及以上 CLI 操作需要显式 `--yes`。
- [daemon 起不来](troubleshooting.md#daemon-起不来)：检查状态、平台和监听资源，不把残留 socket 当成正在运行。
- [SVN 未安装或找不到](troubleshooting.md#svn-path)：检查探测结果和指定的 SVN 路径。
- [状态解析出现幽灵路径](troubleshooting.md#解析异常)：树冲突说明行和 summary 不是普通状态条目。
