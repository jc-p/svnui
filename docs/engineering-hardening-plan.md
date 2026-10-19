# svnui 工程加固任务计划

- 日期：2026-10-19。
- 基线：`c755755`，0.4.1，分支 `codex/tui-refresh-usability`；制定计划时工作区干净。
- 当前阶段：T1–T8 完成，分模块审查与本机集成验证已收口；未提交或发布。
- 发布前复核（2026-10-19）：用户授权将工程加固与后续 LLM 配置优化一并发布 0.4.2，并沿用 Homebrew tap 同步安排。下文“未提交/发布”描述的是工程阶段的历史验证边界。
- 目标：限制异常输入与连续操作的资源开销，使终端恢复、测试和发布路径可验证、可恢复。

## 范围与行为锚点

本轮计划覆盖命令执行、预览调度、终端恢复、CI、发布脚本，以及测量后的局部性能优化。保留现有 Ratatui、标准库线程与模块分层，默认不新增依赖、不修改锁文件、不引入通用任务框架。

必须保留：

- 当前 SVN 操作及 CLI 输出契约；危险操作仍按既有范围确认，不自动重试写操作。
- `Tab` 切换预览、`Esc` 返回、全屏、缩窗退回文件树及中文滚动行为。
- 图片支持、2 MiB 文件限制、压缩包禁用策略和 `docs` / `tui` feature 边界。
- 刷新保留选择、过滤、折叠和有效勾选；临时测试不得使用用户工作副本。
- 已发布安装包、tag 与 Homebrew 内容；失败恢复不删除既有版本或自动重写历史。

不在本轮顺带实现：新增 SVN 功能、修改快捷键含义、UI 框架迁移、全仓格式化、全量历史 lint 清理、自动升级用户安装版本。

## 已确认的源码依据

- `src/svn/command.rs`：普通输出完整读入内存；流式单行读完后才截断；stdin 写入完成后才进入超时等待。
- `src/svn/client.rs`：status 使用文本解析，log/info/list 等使用 XML；`run_with_stdin` 同时服务提交和密码 stdin 登录。
- `src/tui/app.rs`：预览每次选择都启动线程，generation 只拒绝旧结果；编辑器返回后仍同步扫描。
- `src/tui/mod.rs`：终端守卫创建前已改变终端状态，初始化中途失败需要额外清理。
- `src/tui/panels/preview.rs`：绘制时克隆全部行；事件循环定期绘制，实际性能收益尚未测量。
- `scripts/dist.sh`：删除整个 dist、发布判断前写 tap、其他 tap 拉取未受 no-publish 保护、已有附件覆盖上传。
- `scripts/release.sh`：透传 no-publish 前先执行版本提交、tag 与 push。
- `.github/workflows/release.yml`：手动输入 tag 未用于源码 checkout；仓库当前没有日常 CI workflow。
- `justfile` 的 test-all 未显式执行 ignored 测试；`tests/integration.rs` 的命令退出码和 fixture 失败处理可能造成测试假绿。

以上记录的是实施前的静态检查依据；当次验证结果见文末验收记录。

## 并行与文件归属

第一波安排三名执行者并行，主代理负责契约、集成和验证调度：

1. 运行层负责人：T1 → T2，同一负责人串行修改 command/client/error 与命令测试。
2. 交付负责人：T3 → T4，同一负责人持有 CI、justfile、发布脚本和测试 fixture。
3. 终端负责人：T5，仅修改终端入口及其局部测试。

第二波执行 T6，依赖 T1/T2 契约稳定和 T5 合入；第三波由主代理按顺序执行 T7、T8，避免多人修改 App。

协作规则：

- 所有执行者都不独占代码库，不回退他人改动；需要修改他人文件时先交主代理集成。
- 第一波 `src/tui/app.rs` 只由主代理修改必要的错误展示适配；运行层负责人提交准确的适配要求。
- `src/tui/mod.rs` 第一波归 T5；T6 的新模块声明等 T5 完成后再加入。
- T3 的 fixture/helper 修改归交付负责人；命令测试使用独立的 `tests/command_runtime.rs`。
- 构建和测试由主代理排队执行，避免并行 Cargo 锁竞争；源码研究和互不冲突的编辑可并行。
- 子代理不提交、推送、发布，不修改真实 tap，不安装依赖或启动服务；主代理统一交付。
- 不自动创建新 worktree；当前 checkout 如出现非本任务改动，先保留并重新确认隔离边界。

## T1：命令生命周期

- 状态：完成并验证；负责人：运行层；依赖：无。
- 文件：`src/svn/command.rs`、新增 `tests/command_runtime.rs`；必要时仅增加该测试使用的 helper。
- 输入/输出：命令、cwd、stdin、执行期限 → 原始结果或明确的超时、IO、取消错误。
- 实现：统一普通、流式、stdin 路径的执行期限；并发 stdin 写入；失败路径统一回收直接子进程及读写线程，不再静默吞读写失败。
- 接口约束：保留现有公开调用形式，优先增加内部受控执行入口；如需调整公共结构体字面量，先列全调用点并由主代理分配适配，禁止隐式扩范围。
- 验收：stdout/stderr 同时超管道容量不死锁；子进程不读大 stdin 时在期限内退出；BrokenPipe 保留真实失败原因；超时后验证 PID 已回收。
- 安全：不输出 stdin，包括模拟密码；测试进程只读写临时目录。
- 最小验证：`cargo test --offline --locked --test command_runtime`（拟新增入口）。
- 边界：后代进程继承管道必须加入专门用例；若需进程组管理，只操作本次创建的组。不能用回收直接子进程推导整棵进程树已回收，不承诺未验证平台的行为。

## T2：命令输出完整性与限额

- 状态：完成并验证；负责人：运行层；依赖：T1。
- 文件：`src/svn/command.rs`、`src/svn/client.rs`、必要的 `src/domain/error.rs`、`tests/command_runtime.rs`；App 错误展示由主代理适配。
- 输入/输出：明确的完整输出或可截断展示策略 → 完整结果、带独立截断标记的结果，或超限错误。
- 实现：在读取阶段限制缓冲；流式超长行继续排空但只保留限额内容；stderr 保留有界诊断。限额值分别针对查询、预览和诊断，写入代码常量并测试精确边界，不按一个预览阈值裁切所有命令。
- 验收：文本 status 和 XML 查询超限必须失败，不能应用半份状态；预览 diff 可截断并明确提示；UTF-8 边界安全；正常 CLI diff 保持原语义；密码不进入错误或诊断。
- 最小验证：命令测试及 `cargo test --offline --locked --test parse_porcelain --test parse_xml`。
- 交接门：冻结预览专用调用、请求取消语义和截断元数据，再启动 T6。取消仅用于只读预览；写操作超时提示核验结果，不自动重试。

## T3：真实测试入口与日常 CI

- 状态：完成并验证；负责人：交付；依赖：可与 T1/T2 并行，最终结果在其合入后复核。
- 文件：新 `.github/workflows/ci.yml`、`justfile`、`tests/integration.rs`，确有共用需求时修改 `tests/common/mod.rs`。
- 输入/输出：源码和测试环境 → 明确执行了哪些测试、成功或失败，不以静默跳过表示集成通过。
- 实现：常规测试与 ignored 测试分入口；test-all 显式覆盖两者；CI 预检 svn/svnadmin，fixture 初始化及 SVN 非零退出必须报失败。CI 明确发现工具缺失或准备失败，不能返回成功。
- feature 验证：默认功能、`--no-default-features`、`--no-default-features --features tui`。
- 验收：故意使 fixture checkout/commit 失败时测试红灯；正常临时仓库能完成提交与冲突生命周期；日常 CI 与发布任务使用一致测试入口。
- 最小验证：`cargo test --offline --locked --test integration -- --ignored`，测试默认 skip 与 CI 必须执行的分支；feature 检查按本地门禁执行。
- 边界：只读记录历史 fmt/clippy 基线；不直接开启未经验证的全仓严格检查，不混入历史清理。远端 CI 是否真实通过须在后续授权推送后验证。

## T4：可恢复的本地打包与发布

- 状态：完成并验证；负责人：交付；依赖：T3 的验证入口确定，脚本实现可与运行层并行。
- 文件：`scripts/dist.sh`、`scripts/release.sh`、`scripts/bump.sh`、`.github/workflows/release.yml`、`PUBLISH.md`、新增 `tests/release_scripts.py`。
- 输入/输出：版本、commit、目标架构、发布模式 → 版本独立产物、manifest、SHA256SUMS，及明确的发布阶段结果。
- 本地打包：使用 staging 和 `dist/releases/<version>`；构建成功、版本验证通过后才提升目录；保留既有版本。纯打包模式不得提交、建 tag、push/pull、调用 gh/brew 或修改 tap；release 入口须在任何版本操作前识别该模式。
- 离线模式：`--locked --offline`，目标架构缺失时明确失败，不自动安装 target；需要安装时另走显式准备步骤。
- 一致性：核对 Cargo、lock、二进制版本、源码 commit 和 tag 目标；手动 workflow 的源码与模板 checkout 绑定经过验证的输入 tag；用户输入通过环境变量传入 shell。已有 Release/附件默认拒绝覆盖，每个 tag 的发布互斥。
- 发布顺序：构建并验证 → 明确目标提交的 Release → 回下载并校验 → 更新 tap。单架构本地包不能以占位校验和更新双架构 formula。
- 恢复：版本提交后构建失败保留现场；Release 失败不写 tap；Release 成功而 tap 失败可仅重试 tap。拒绝通过删除既有版本、覆盖附件或自动 reset 恢复。
- 验收：临时 Git/tap 仓库与 PATH 命令替身记录外部调用，覆盖无发布模式零外部副作用、旧产物保留、构建失败、tag 不匹配、重复发布、上传失败及 tap 失败。测试不得调用真实 gh 发布。
- 最小验证：`bash -n scripts/dist.sh scripts/release.sh scripts/bump.sh`；`python3 -m unittest discover -s tests -p 'release_scripts.py'`（拟新增，使用标准库）；YAML 用现有可用解析工具检查，未提供工具时记录未验证，不为验证自动装依赖。
- 边界：本任务只修改 svnui 仓库；真实 GitHub 发布与外部 Homebrew 仓库更新不属于脚本测试。

## T5：终端初始化与恢复

- 状态：完成并验证；负责人：终端；依赖：无，与运行层和交付并行。
- 文件：`src/tui/mod.rs` 及其局部测试；不修改 App、命令层或预览模块。
- 输入/输出：进入、失败、编辑器让渡与返回、退出 → 正确恢复 raw mode、alternate screen、鼠标、粘贴和光标状态。
- 实现：在首次改变终端状态前建立清理保障；集中记录实际开启状态，失败路径恢复；保持现有编辑器和系统打开行为。
- 验收：各初始化阶段注入失败后清理顺序正确；PTY 验证普通退出和编辑器返回；检查 panic 恢复。真实终端视觉未验证时单独列出。
- 最小验证：`cargo test --offline --locked --lib tui::terminal_tests::`（拟新增测试模块）；PTY 子进程仅使用临时目录和测试工具，限制输出与等待时间。
- 边界：SIGKILL 无法触发 Rust 清理，不承诺恢复；不能通过测试替身推导所有终端原生图片行为正常。

## T6：预览有界调度

- 状态：完成并验证；负责人：第二波预览执行者；依赖：T1/T2 接口冻结、T5 完成。
- 文件：新 `src/tui/preview_worker.rs`、`src/tui/app.rs` 的预览字段/入口/tick/测试、`src/tui/mod.rs` 模块声明；如需展示截断，仅调整 `src/tui/panels/preview.rs` 对应部分。
- 输入/输出：选中快照、generation、图片能力 → 带同一 generation 的预览结果及独立截断状态。
- 实现：最多一个运行任务、一个可覆盖的最新待办；新选择替换旧待办；目录、空选中、退出使旧请求失效。尽早取消过期只读 SVN 请求，解码阶段遵守现有限额并在前后检查是否过期。
- 验收：A→B→C 连续选择最多一个在途、一个待办，B 可被覆盖；旧 A 不能覆盖 C；选择目录或退出清待办；退出不无限等待 worker；已有焦点、图片、缩窗回归保持通过。
- 最小验证：`cargo test --offline --locked --lib preview_worker::`（拟新增入口）；`cargo test --offline --locked --lib preview_ -- --ignored`。
- 边界：不承诺强制中断库内部图片解码；不扩成日志、冲突和所有后台任务共用的调度框架；现有 busy 写操作互斥继续有效。

## T7：测量后优化渲染与刷新

- 状态：完成并验证；负责人：主代理；依赖：T6。
- 文件：`src/tui/mod.rs`、`src/tui/panels/preview.rs`、`src/tui/app.rs` 的刷新与绘制段；测量结论记录在本文件。
- 先测：固定同一机器、终端尺寸与临时工作副本，分别记录空闲 30 秒的 CPU/终端输出量、5000 行预览切换延迟、1000/10000 文件状态刷新耗时；每个场景至少三次，区分冷/热。
- 再改：优先移除每帧全文克隆；编辑器返回扫描转后台，保持立即可见的加载反馈；仅在测量支持时增加 dirty 重绘，保证 spinner、刷新计时、resize 和后台完成仍按需更新。
- 验收：同条件记录修改前后结果，不预先承诺百分比；键盘/图片/长行与自动刷新回归通过。没有收益或出现交互回归时撤去该局部优化。
- 最小验证：受影响的 TUI 单测、真实预览回归及上述有界测量；不得在用户大工作副本默认执行压力测试。

## T8：App 局部职责拆分

- 状态：完成并验证；负责人：主代理；依赖：T6/T7 稳定，串行执行。
- 文件：`src/tui/app.rs` 与必要的同模块子文件；预览 worker 已由 T6 独立，不重复抽象。
- 实现：逐个提取预览接入、文件操作或历史记录中可自洽的行为；保持事件和状态入口，不为减少行数扩大可见性或添加转发层。
- 验收：每次提取只包含一个职责；检查按键、确认范围、busy、后台结果路由、刷新保存状态等锚点，配套现有及必要新增回归。无明确维护收益时停止继续拆分。
- 最小验证：每个职责对应的 focused 测试；全部整合后再执行最终验证集。

## 集成验收与交付边界

- 每个 slice 完成：针对性差异审查、指定文件 whitespace 检查、对应测试、风险与未验证平台记录。
- 全部整合后才安排一次全量验证：默认测试、ignored SVN 测试、feature 组合、脚本故障测试和 PTY；全量检查执行前按 AGENTS 明确确认范围，日志写临时文件并限量读取。
- 全仓 fmt/clippy 只在确认基线与范围后执行；若存在历史问题，报告并保持任务边界，不静默跳过新增问题。
- 新依赖、锁文件、其他仓库、真实安装或网络写入若变得必要，先报告理由再按范围门禁确认。
- 提交、推送、版本号、GitHub Release、Homebrew 更新在实现验收后按用户当次授权办理；本计划不自动触发这些动作，也不提前指定下一版本。
- 回滚按独立提交或局部补丁处理；不使用 reset --hard、强推或删除既有 tag 来清理现场。

## 本轮计划检查结论

- 可执行：第一波三条线写入范围互斥；预览及 App 后续工作有明确依赖。
- 已补齐：文本/XML 完整性、密码 stdin、发布入口 no-publish、测试 fixture 假绿、工作流输入 tag 与 checkout 对齐。
- 实施结果：T1–T6 由边界明确的子任务完成，T7/T8 由主代理串行集成，并交叉审查。
- 用户已授权完整分模块审查、一次离线全量测试、真实 SVN 回归及三种 feature 检查；本轮不提交、推送、发布或写真实 tap。

## 实施与验证记录（2026-10-19）

- T1/T2：Unix 非阻塞 IO 统一期限，覆盖 stdin、双管道、进程等待；RAII 回收直接子进程。仅可取消预览隔离进程组，普通 CLI 保留 Ctrl+C 行为。完整输出 64 MiB 超限报错；预览 stdout 2 MiB、诊断 256 KiB、单行进度 64 KiB，有界读取和独立截断元数据。
- 密码与路径：stdin 登录从构造 argv 时排除密码；保留旧版 SVN 兼容回退。只读预览沿用普通 diff 的 symlink 目标语义，增加真实外指链接回归。
- T3/T4：日常 CI 显式运行 ignored SVN 测试，fixture 准备失败报错；构建锁定依赖。打包使用版本目录、staging、manifest 与校验和；no-publish 在任何版本写操作前分流；拒绝覆盖发布，回下载校验后才操作显式 tap，支持 tap-only 重试及首次创建 formula。
- T5：终端状态先登记再修改，失败继续恢复其余模式，编辑器让渡/恢复贯通。panic hook 只在会话线程 panic 时恢复模式；后台捕获的 panic 不改变前台终端。
- T6：一个 worker、一个可覆盖待办、一个结果槽；generation 拒绝旧结果，目录/空选中/退出取消，loader panic 变 Notice 后继续服务。退出不等待不可中断的图片解码。
- T7：缓存文本显示宽度，每帧仅克隆可见行，保持原 Paragraph 的中文与横向裁切；编辑器返回改后台扫描，busy 时保留补扫请求，失败结束加载提示。不新增 dirty 重绘状态机。
- T8：仅提取 `src/tui/app/history.rs` 的历史加载职责，状态、按键和结果路由保留在 App。方法只暴露给父模块；与原块去空白/局部可见性后比较一致。

验证证据：

- 离线普通测试：211 通过；21 项 ignored 测试由下一入口显式执行。
- 真实 SVN：21 通过（17 个 App 场景、3 个集成场景、1 个 symlink 场景），只使用临时仓库。命令运行层的 15 个用例包括 stdin 阻塞、双管道大输出、取消、输出上限和继承管道边界。
- fixture 故障注入：替身 `--version` 成功、checkout 退出 9，指定集成测试正确红灯，未误报通过。
- feature：默认、无默认、仅 tui 的 `cargo check --offline --locked --all-targets` 均通过；无默认组合保留 cli/mod.rs 两处既有 unused_mut 警告，不在本轮清理。
- 脚本：15 项标准库/PATH 替身回归通过；bash 语法、ShellCheck、workflow YAML/嵌入 Python 语法已验证。无真实 GitHub 或 Homebrew 写入。
- 全量之后补充的编辑返回扫描失败处理，单独运行对应真实 SVN 回归；其余全量结果不冒充该补丁后的再次全量执行。

发布前合并验证（包含后续 LLM 配置优化）：

- `cargo test --offline --locked`：221 通过；22 项 ignored 由独立入口执行。
- `cargo test --offline --locked -- --ignored`：22 通过，仅操作临时 SVN 仓库。
- 默认、无默认、仅 tui 的 `cargo check --offline --locked --all-targets` 均通过；无默认组合仍有两处既有 unused_mut 警告。
- `python3 -B -m unittest discover -s tests -p release_scripts.py`：15 通过。
- LLM 协议与错误诊断使用本机临时模拟服务验证；真实云端账号及本地 Ollama 未进行连通测试。

边界与未验证项：

- 非 Unix 的线程兼容实现没有本机运行证据；继承句柄可能仍使辅助线程等待。不承诺 Windows 同等取消能力。
- 普通命令只回收直接子进程；取消预览清理自己创建的进程组，不涵盖主动脱离组的后代。回调应快速返回，不能中断库内部图片解码。
- 远端 CI、实际双架构发布、真实 tap 更新尚未执行；本机原生终端图片协议视觉仍需人工验证。
- 不增加依赖、未修改 Cargo.toml/Cargo.lock、版本号、用户安装或真实工作副本。

终端与失败路径补充证据：

- `/tmp/svnui-hardening-pty-smoke.log`：真实 PTY 下 `E` 启动临时编辑器、写临时文件、返回后新预览可见、恢复 raw mode，`q` 正常退出且 termios 属性与启动前完全一致。
- PTY harness 会响应终端光标位置查询；首轮缺少此协议应答导致测试超时，补齐 harness 后通过，未据此改动产品。
- `/tmp/svnui-hardening-editor-final.log`：新增编辑返回后台扫描、busy 补扫及缺失 cwd 的失败收口回归通过。仅移走 `.svn` 时 SVN 可能警告后仍以 0 退出，故故障注入改为临时移走整个测试 WC；测试后恢复。
- 性能验证未在产品中插桩 draw 次数，以实际 PTY 输出字节观测终端流量；不将该字节量等同绘制调用次数。事件循环的既有轮询频率保留，本轮不扩展重绘状态机。

### 本机性能对照

同一机器、release 构建、临时 SVN 仓库、xterm-256color / halfblocks；每项三次，以下取中位数。TUI 初始 140×30，采样前 resize 为 141×30；原 JSON 的静态 terminal 标签仍为 140×30，实际采样尺寸以脚本 resize 为准。文件系统缓存没有清空，status 表示应用无缓存扫描，不能解释成严格冷缓存。

| 场景 | 原本地 0.4.1 二进制 | 本轮最终构建 |
| --- | ---: | ---: |
| 空闲 CPU（单核百分比） | 3.667 % | 0.933 % |
| 5000 行预览切换 | 132.145 ms | 125.113 ms |
| 1000 文件无缓存 status | 81.849 ms | 75.670 ms |
| 10000 文件无缓存 status | 131.930 ms | 124.924 ms |
| 空闲 30 秒 PTY 输出 | 6574.000 bytes | 6774.000 bytes |

- 空闲 CPU 中位数降低约 75%；状态查询和切换延迟的小幅变化只代表本机三次观测，不作为稳定吞吐承诺。终端输出量未降低，本轮保留轮询重绘。
- 首轮局部优化测得切换中位数约 247 ms，暴露原事件循环先 draw 后 tick 的额外等待；将后台结果接收移到绘制前后，最终约 125 ms。修正后 6 项终端测试、实际 PTY 编辑器往返和三种 feature 检查再次通过。
- 原始数据：`/tmp/svnui-hardening-before.json`、`/tmp/svnui-hardening-after.json`（首轮实验）、`/tmp/svnui-hardening-after-final.json`（最终）。可复测脚本保留于 `/tmp/svnui-hardening-bench.py`，均属于本轮临时证据，不是仓库运行时依赖。
- 全量与回归日志：`/tmp/svnui-hardening-all.log`、`/tmp/svnui-hardening-real-svn.log`、`/tmp/svnui-hardening-release-script-final.log`。最终 feature 日志为 `/tmp/svnui-hardening-feature-{default,cli,tui}-final.log`。
