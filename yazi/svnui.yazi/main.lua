--- @since 25.2.13

-- svnui.yazi —— SVN 状态集成（yazi 26.x）
--
-- 单文件实现。yazi 官方插件（git / diff / full-border / smart-filter）
-- 全部是单文件 main.lua，这是生态事实标准。
-- 插件内部 `require("svnui.state")` 依赖 loader 未公开行为，跨版本不稳，不用。
--
-- 文件内分区（用下面的 === 分隔条导航）：
--   JSON 解码器 → state → util → diff → commit → log → rescue → 入口


-- ==========================================================================
-- JSON 解码器（yazi 没有 json 全局，自带）
-- ==========================================================================

--- @since 25.2.13

--
-- ⚠️ 为什么不用 `json.decode`：yazi **没有 `json` 全局**。
--    内置的 `json` 是 `.json` 文件的**预览器**，不是解析库。
--    写 `pcall(json.decode, out)` 时，参数在 pcall 执行前求值 ——
--    `json` 为 nil 直接抛错，pcall 完全拦不住，
--    表现就是弹框 "attempt to index a nil value (field 'json')"。
--
-- 只支持 svnui 自己输出的子集：object / array / string / number / true / false / null。
-- 字符串用 find 批量跳过普通字符，避免长路径上逐字节 sub。
--
-- 失败时 **抛错**，调用方必须用 pcall 包住。

local json_val -- 前向声明（下面的函数会闭包捕获它）

local function json_skip_ws(s, i)
  return s:find("[^ \t\r\n]", i) or (#s + 1)
end

local function json_str(s, i)
  -- 前置条件：s:sub(i, i) == '"'
  i = i + 1
  local buf, n = {}, 0
  while true do
    local j = s:find('["\\]', i)
    if not j then
      error("unterminated string")
    end
    if j > i then
      n = n + 1
      buf[n] = s:sub(i, j - 1)
    end
    if s:sub(j, j) == '"' then
      return table.concat(buf), j + 1
    end
    -- 转义
    local e = s:sub(j + 1, j + 1)
    if e == "n" then
      n = n + 1; buf[n] = "\n"; i = j + 2
    elseif e == "t" then
      n = n + 1; buf[n] = "\t"; i = j + 2
    elseif e == "r" then
      n = n + 1; buf[n] = "\r"; i = j + 2
    elseif e == "b" then
      n = n + 1; buf[n] = "\b"; i = j + 2
    elseif e == "f" then
      n = n + 1; buf[n] = "\f"; i = j + 2
    elseif e == "u" then
      local cp = tonumber(s:sub(j + 2, j + 5), 16)
      if not cp then error("bad \\u escape") end
      n = n + 1
      buf[n] = (utf8 and utf8.char) and utf8.char(cp) or string.char(cp % 256)
      i = j + 6
    else
      n = n + 1; buf[n] = e; i = j + 2
    end
  end
end

local function json_obj(s, i)
  local out = {}
  i = json_skip_ws(s, i + 1)
  if s:sub(i, i) == "}" then
    return out, i + 1
  end
  while true do
    i = json_skip_ws(s, i)
    local k
    k, i = json_str(s, i)
    i = json_skip_ws(s, i)
    i = i + 1 -- 跳过 ':'
    local v
    v, i = json_val(s, i)
    out[k] = v
    i = json_skip_ws(s, i)
    local c = s:sub(i, i)
    if c == "," then
      i = i + 1
    elseif c == "}" then
      return out, i + 1
    else
      error("expected , or }")
    end
  end
end

local function json_arr(s, i)
  local out = {}
  i = json_skip_ws(s, i + 1)
  if s:sub(i, i) == "]" then
    return out, i + 1
  end
  while true do
    local v
    v, i = json_val(s, i)
    out[#out + 1] = v
    i = json_skip_ws(s, i)
    local c = s:sub(i, i)
    if c == "," then
      i = i + 1
    elseif c == "]" then
      return out, i + 1
    else
      error("expected , or ]")
    end
  end
end

json_val = function(s, i)
  i = json_skip_ws(s, i)
  local c = s:sub(i, i)
  if c == "{" then
    return json_obj(s, i)
  elseif c == "[" then
    return json_arr(s, i)
  elseif c == '"' then
    return json_str(s, i)
  elseif c == "t" then
    return true, i + 4
  elseif c == "f" then
    return false, i + 5
  elseif c == "n" then
    return nil, i + 4 -- null
  end
  -- 数字（走到这里必定是 '-' 或数字）
  local a, b = s:find("%-?%d+%.?%d*[eE]?[-+]?%d*", i)
  if not a then
    error("unexpected char: " .. tostring(c))
  end
  return tonumber(s:sub(a, b)), b + 1
end

--- 解析 JSON 文本。失败抛错 —— 调用方必须用 pcall 包住。
local function json_decode(s)
  local v, i = json_val(s, 1)
  -- 尾部校验：不验证的话 `not json` 会被当成 `null` 静默解析成功
  -- （首字母 n 走了 null 分支），坏输入就悄悄变成 nil 而不是报错。
  if json_skip_ws(s, i) <= #s then
    error("trailing garbage at " .. i)
  end
  return v
end

local json = {
  decode = json_decode,
}


-- ==========================================================================
-- state —— 状态缓存中心（唯一数据源）
-- ==========================================================================

--- @since 25.2.13

--
-- 所有模块共享这一份缓存。刻意做成**唯一**的数据源：
-- yazi 的插件 state 是 per-plugin 独立的，所以必须把缓存放在一个
-- 被所有模块 require 的公共模块里，而不是各自存一份。
--
-- ⚠️ 铁律：这里的函数会被 Linemode 染色调用，而 Linemode 跑在
--    **sync 上下文**，每渲染一行跑一次 —— 因此本模块
--    **绝不能 spawn 进程、不能挂起**。只读内存。

-- ============================================================ state

-- 状态缓存中心。
--
-- ⚠️ 这个文件被 init.lua（sync 上下文）和 main.lua（async 上下文）同时 require。
--    所以**所有跨上下文的数据都必须是 Sendable 类型**：nil / boolean / number /
--    string / Url / 只含这些类型的 table。
--
-- 具体说：map 的 value 是**字符串**（如 "AM"），不是 Rust userdata。
-- 不要试图在这里存 Url —— 跨线程传递会转移所有权，存下来就是悬垂引用。

local state = {}

--- path(绝对路径字符串) → porcelain 两字符标记。
--- 例：`/repo/src/main.rs` → `"M "` / `"_M"` / `"A+"`
---
--- ⚠️ 这个表**只用于查表**（sign_for / porcelain_for）。
---    它包含 /private 双索引，同一文件可能有两个 key。
---    要枚举请用 `state.paths` —— 直接遍历 map 会重复计数。
state.map = {}

--- 真实路径列表（去重后，每个文件一条）。枚举一律走这里。
---
--- 为什么不直接 pairs(state.map)：map 里有 macOS 软链的两种写法
--- （`/var/x` 和 `/private/var/x`），遍历会把同一个文件数两遍。
state.paths = {}

--- 上一次刷新的工作副本根。用于检测"换仓库了要清缓存"。
state.root = nil

--- 版本戳。由 Rust daemon 提供；这里只做增量判断用。
state.stamp = 0

--- 上次刷新时间（os.time）。
state.updated_at = 0

--- 是否处于 SVN 工作副本内。false 时所有查询直接返回 nil，不做无谓尝试。
state.in_wc = false

--- 分支名（来自 `svnui info`）。
state.branch = nil

--- 冲突数量。状态栏展示。
state.conflict_count = 0
--- 提交模式：true 表示"范围已标记，等待确认提交"。
--- 见 commit.run 的两阶段说明。
state.commit_mode = false
--- setup() 是否被调用过。
---
--- ⚠️ 染色（Linemode）和状态栏（Status）**只在 setup 里注册**。
---    而 vc / vd 这些动作走 entry，不需要 setup ——
---    所以会出现"提交能用，但列表里一个标记都没有"的诡异现象。
---    用这个标记在 entry 时给出明确提示，而不是让人干瞪眼。
state.setup_done = false

--- TTL（秒）。超过就认为需要刷新。
state.ttl = 5

-- ---------------------------------------------------------- 状态同步层
--
-- ⚠️⚠️ 整个插件最关键的一段，务必理解。
--
-- yazi 官方文档（docs/plugins/overview "Async context"）：
--   "When a plugin executes asynchronously, Yazi creates an isolated
--    async context: Created per invocation, destroyed after completion.
--    Each execution has its own Lua state."
--
-- 也就是说：**每次按 vc / vd / vs，都是一个全新的 Lua 实例，跑完就销毁**。
-- 模块级 local 变量（比如上面的 `state`）**不会跨调用保留**。
--
-- 于是出过这个 bug：
--   · 用户在 entry（async）里刷新，数据写进了自己实例的 `state`
--   · 调用结束，实例销毁，数据一起没了
--   · 染色（Linemode）和状态栏跑在 sync 上下文（持久实例），
--     它读的是**另一个** `state`，永远是初始空值
--   → 表现："提交能用，但列表里一个标记都没有"
--
-- 唯一可靠的跨上下文存储是 **plugin state** —— 也就是 `ya.sync(fn)` 回调的
-- 第一个参数，由 Rust 侧持有，贯穿整个 yazi 生命周期。
--
-- 所以：所有需要跨调用/跨上下文的数据，一律走下面的 ya.sync 函数。

--- 把一批状态写进 plugin state（唯一真相）。
--- 本地 `state` 只是当前上下文的镜像，方便读；这里负责落盘。
local apply_state = ya.sync(function(st, d)
  for k, v in pairs(d) do
    st[k] = v
  end
end)

--- 从 plugin state 拉回镜像（async 侧开局调用一次）。
local pull_state = ya.sync(function(st)
  return {
    setup_done = st.setup_done,
    in_wc = st.in_wc,
    map = st.map,
    paths = st.paths,
    root = st.root,
    stamp = st.stamp,
    updated_at = st.updated_at,
    branch = st.branch,
    conflict_count = st.conflict_count,
    commit_mode = st.commit_mode,
  }
end)

--- 只更新单个字段（commit_mode 这种高频小改动）。
local set_field = ya.sync(function(st, k, v)
  st[k] = v
end)

local get_field = ya.sync(function(st, k)
  return st[k]
end)

--- 查表拿 sign（染色用）。
---
--- ⚠️ 必须走 ya.sync：linemode 跑在 sync 上下文，
---    它读的模块级 `state` 和 async 侧那个**不是同一份**。
local get_sign = ya.sync(function(st, path)
  if not st.in_wc or not path then
    return nil
  end
  local xy = st.map and st.map[path]
  if not xy or #xy < 2 then
    return nil
  end
  local x = xy:sub(1, 1)
  local y = xy:sub(2, 2)
  if x ~= "_" and x ~= " " then
    return x
  end
  if y ~= "_" and y ~= " " then
    return y
  end
  return nil
end)

--- 查表拿 porcelain 两字符（预览 / 提交候选判断用）。
local get_xy = ya.sync(function(st, path)
  if not st.in_wc or not path then
    return nil
  end
  return st.map and st.map[path]
end)

--- 状态栏文案。同样必须走 ya.sync。
local get_status_line = ya.sync(function(st)
  -- 提交模式优先 —— 用户必须知道自己在"半路"上
  if st.commit_mode then
    return " SVN 提交模式：Space/v 调整，vc 确认，vq 取消 "
  end
  if not st.in_wc then
    return ""
  end

  local s = "SVN"
  if st.branch then
    s = s .. " " .. st.branch
  end
  if st.conflict_count and st.conflict_count > 0 then
    s = s .. string.format(" !%d", st.conflict_count)
  end
  if not st.updated_at or st.updated_at == 0 then
    s = s .. " ⟳"
  end
  return " " .. s .. " "
end)

--- 变更路径列表（提交候选）。
local get_paths = ya.sync(function(st)
  return st.paths or {}, st.map or {}, st.in_wc == true
end)

--- 用 Rust 侧扫描的结果替换缓存。
---
--- @param root string|nil 工作副本根，nil 表示不在工作副本内
--- @param entries table<string,string> path → porcelain
--- @param stamp integer
--- @param branch string|nil
--- 给路径建"双索引"。
---
--- ⚠️ macOS 陷阱：`/var`、`/tmp`、`/etc` 都是 `/private/xxx` 的**软链接**。
---    svn 内部会 canonicalize，所以它返回的 wc root 是 `/private/var/folders/...`；
---    而 yazi 的 `file.url` 来自它自己的 cwd，是 `/var/folders/...`。
---    两者字符串不相等 → 查表 miss → **染色和状态全部消失**，
---    但命令行直接跑 `svnui status` 却有输出，极难定位。
---
--- 解法：把两种写法都塞进 map（只多一次哈希写入，查询仍是 O(1)）。
--- 非 macOS 上 `/private` 不存在，第二次写入是空操作，无害。
local function link_variants(p)
  if p:sub(1, 9) == "/private/" then
    return p:sub(9) -- 去掉 /private
  elseif p:sub(1, 1) == "/" then
    return "/private" .. p
  end
  return nil
end

function state.replace(root, entries, stamp, branch)
  local src = entries or {}
  local map = {}
  local paths = {}
  for k, v in pairs(src) do
    map[k] = v
    paths[#paths + 1] = k
    local alt = link_variants(k)
    if alt then
      map[alt] = v
    end
  end
  table.sort(paths)
  state.map = map
  state.paths = paths
  -- ⚠️ 必须落进 plugin state，否则 async 实例销毁后数据就没了，
  --    而 sync 侧的染色读的正是 plugin state（见上面同步层说明）。
  apply_state({
    in_wc = true, map = map, paths = paths,
    root = state.root, stamp = stamp, branch = branch,
    conflict_count = state.conflict_count,
    updated_at = os.time(),
    commit_mode = state.commit_mode,
  })
  state.root = root
  state.stamp = stamp or 0
  state.branch = branch
  state.updated_at = os.time()
  state.in_wc = root ~= nil

  local n = 0
  for _, xy in pairs(src) do
    if xy:sub(1, 1) == "C" or xy:sub(1, 1) == "T" or xy:sub(2, 2) == "C" then
      n = n + 1
    end
  end
  state.conflict_count = n
end

--- 清空。退出工作副本或刷新失败时调用。
function state.clear()
  state.map = {}
  state.paths = {}
  state.conflict_count = 0
  state.updated_at = os.time()
  apply_state({
    in_wc = state.in_wc, map = {}, paths = {},
    conflict_count = 0, updated_at = os.time(),
  })
end

--- 是否需要刷新。
---
--- 用 TTL 而不是精确失效：linemode 无法知道文件系统何时变了，
--- 而 TTL 过期只是"多跑一次 svnui"，代价可控。
--- @return boolean
function state.stale()
  if not state.in_wc then
    return true
  end
  return (os.time() - state.updated_at) >= state.ttl
end

--- 取某个绝对路径的展示符号。
---
--- porcelain 是两字符 `XY`：X=文本状态，Y=属性状态。
--- 展示时优先取 X；X 为空则取 Y（对应"仅属性改动"的 `_M`）。
---
--- @param path string 绝对路径
--- @return string|nil
function state.sign_for(path)
  if not state.in_wc or not path then
    return nil
  end
  local xy = state.map[path]
  if not xy or #xy < 2 then
    return nil
  end
  local x = xy:sub(1, 1)
  local y = xy:sub(2, 2)
  if x ~= "_" and x ~= " " then
    return x
  end
  if y ~= "_" and y ~= " " then
    return y
  end
  return nil
end

--- 取完整 porcelain（调试 / diff 弹框标题用）。
--- @return string|nil
function state.porcelain_for(path)
  if not state.in_wc then
    return nil
  end
  return state.map[path]
end

--- 所有有变更的绝对路径（提交/回滚的目标集合）。
--- @return string[]
function state.changed_paths()
  local out = {}
  for _, path in ipairs(state.paths) do
    local xy = state.map[path]
    if xy then
      local x = xy:sub(1, 1)
      -- 忽略 ignored / external / normal
      if x ~= "I" and x ~= "X" and x ~= "_" and x ~= " " then
        out[#out + 1] = path
      end
    end
  end
  return out
end

--- 所有冲突路径。
--- @return string[]
function state.conflict_paths()
  local out = {}
  for _, path in ipairs(state.paths) do
    local xy = state.map[path]
    if xy then
      local x = xy:sub(1, 1)
      if x == "C" or x == "T" or xy:sub(2, 2) == "C" then
        out[#out + 1] = path
      end
    end
  end
  return out
end

--- 状态栏右侧内容。
---
--- 只在有信息可展示时才返回内容，否则返回空串 ——
--- 状态栏每帧都渲染，返回空串比返回一个空格更省。
--- @return any
--- 状态栏右侧文本：分支名 + 冲突计数。
---
--- ⚠️ 刻意返回**纯字符串**，不用 ui.Line：
---    yazi 26.9 上 Status:children_add 回调返回 ui.Line 时，
---    span 拼装会渲染错乱（状态栏出现乱字符/重叠）。
---    返回 string 一定安全。颜色等确认稳定后再加。
---
--- @return string
function state.status_line()
  if not state.in_wc then
    -- 提交模式下即使状态丢了也要提示，否则用户以为键没生效
    return state.commit_mode and " SVN 提交模式：vc 确认，vq 取消 " or ""
  end

  -- 提交模式优先显示 —— 用户必须知道自己在"半路"上
  if state.commit_mode then
    return " SVN 提交模式：Space/v 调整，vc 确认，vq 取消 "
  end

  local s = "SVN"
  if state.branch then
    s = s .. " " .. state.branch
  end
  if state.conflict_count > 0 then
    s = s .. string.format(" !%d", state.conflict_count)
  end
  if state.updated_at == 0 then
    -- 还没刷过：提示一下，避免"怎么什么都没有"
    s = s .. " ⟳"
  end
  return " " .. s .. " "
end


-- ==========================================================================
-- util —— 调 svnui CLI + 官方弹框
-- ==========================================================================

--- @since 25.2.13

--
-- ⚠️ 本模块假定自己运行在 **async 上下文**（entry / peek）。
--    从 sync 上下文（Linemode 染色）调用会阻塞 UI —— 不要那么做。


--- svnui 可执行文件路径。由 `setup { svnui_path = ... }` 覆盖。
---
--- ⚠️ macOS 陷阱：brew 安装的 yazi 启动时 **PATH 里通常没有 ~/.cargo/bin**，
---    而 yazi 的 Command 继承自己的环境 —— 于是 `Command("svnui")` 直接失败，
---    表现为"按键没反应 + 一条 spawn 错误"。
---    解决：`svnui_path` 显式指定绝对路径，或者把 svnui 装进 /usr/local/bin。
local CFG = { bin = "svnui" }

-- ---------------------------------------------------------------- 重绘
--- 触发 UI 重绘。
---
--- ⚠️ 两个坑：
---   1. 官方 types.yazi 里这个 API 叫 **`ui.render()`**，不是 `ya.render()`
---   2. 它标注 **Sync context only** —— 而 entry/peek 跑在 **async** 上下文，
---      直接 `ui.render()` 拿不到（nil），必须用 ya.sync 包一层
---
--- 刷完状态必须调它：linemode 的回调只在 yazi 重绘时被调用，
--- 只更新 Lua 侧 map 不重绘，界面上看不到任何变化（"按了没反应"）。
local do_render = ya.sync(function()
  if ui and type(ui.render) == "function" then
    ui.render()
  elseif ya and type(ya.render) == "function" then
    -- 少数版本挂在 ya 下，兜一下
    ya.render()
  end
end)

-- ============================================================ util

-- 调用 svnui CLI 的封装 + 官方弹框封装。
--
-- 这里的所有函数都假定自己运行在 **async 上下文**（main.lua 的 entry / peek）。
-- 从 sync 上下文（init.lua 的 Linemode）调用会阻塞 UI —— 不要那么做。

local util = {}

-- ---------------------------------------------------------------- cx 访问
--
-- ⚠️ 这三个字段由 **main.lua** 注入（`util.cwd = get_cwd` 之类）。
--
-- 为什么不在本模块直接用 ya.sync 定义：ya.sync 注册的回调是**按模块 ID**
-- 存的（`Runtime.blocks: HashMap<模块ID, ...>`），执行时按当前模块定位。
-- 在被 require 的子模块里注册、却从入口调用，会跑到入口的槽位里找 ——
-- 轻则 nil，重则 index 撞上别的 block，**执行到错误的函数**。
--
-- 所以统一在入口文件 main.lua 顶层定义，再注入进来。
util.cwd = nil
util.targets = nil
util.selection_count = nil

--- setup 时由 main.lua 调用，设置 svnui 可执行文件路径。
--- 必须在 `local util` 之后定义 —— Lua 的 local 要先声明才能捕获。
function util.set_bin(path)
  if path and path ~= "" then
    CFG.bin = path
  end
end

-- Lua 5.1 用全局 `unpack`，5.4 挪到了 `table.unpack`。
-- yazi 在不同平台上可能链接 LuaJIT(5.1) 或 Lua 5.4，这里做兼容。
local unpack_args = table.unpack or unpack

-- ---------------------------------------------------------------- 进程调用

--- 跑一次 svnui 并返回 stdout。
---
--- 用 `--cwd` 全局参数而不是给 Command 设工作目录：
--- 前者由 Rust 侧统一处理路径解析，后者在不同 yazi 版本上 API 不完全一致。
---
--- @param cwd string 工作目录
--- @param args string[] 子命令及参数
--- @return string|nil stdout，失败返回 nil
--- @return string|nil 错误信息
function util.svnui(cwd, args)
  local cmd = Command(CFG.bin)
  if not cmd then
    return nil, "Command 不可用（yazi 版本过低？）"
  end

  cmd = cmd:arg("--cwd"):arg(cwd)
  for _, a in ipairs(args) do
    cmd = cmd:arg(a)
  end

  local child, spawn_err = cmd:stdout(Command.PIPED):stderr(Command.PIPED):spawn()
  if not child then
    -- 最常见的就是 PATH 里没有 svnui（macOS + brew 装 yazi 时尤其常见）
    return nil, ("无法启动 svnui（路径 %q）: %s\n" .. "  提示：把 svnui 放进 /usr/local/bin，" ..
      "或在 setup 里指定绝对路径 require('svnui'):setup { svnui_path = '/full/path/svnui' }"):format(
      CFG.bin, tostring(spawn_err))
  end

  local output, wait_err = child:wait_with_output()
  if not output then
    return nil, "svnui 执行失败: " .. tostring(wait_err)
  end

  if not output.status.success then
    return nil, (output.stderr or ""):gsub("%s+$", "")
  end
  return output.stdout, nil
end

--- 跑 svnui 并解析 JSON 信封。
---
--- @return table|nil data 字段，失败返回 nil
--- @return string|nil 错误信息
function util.svnui_json(cwd, args)
  local out, err = util.svnui(cwd, { "--json", unpack_args(args) })
  if not out then
    return nil, err
  end

  -- pcall 包住：坏 JSON 应该弹提示，而不是让整个动作崩掉
  local ok, decoded = pcall(json.decode, out)
  if not ok or type(decoded) ~= "table" then
    return nil, "svnui 返回的不是合法 JSON"
  end
  if decoded.ok == false then
    local e = decoded.error or {}
    return nil, tostring(e.message or e.kind or "未知错误")
  end
  return decoded.data, nil
end

--- 刷新状态缓存。
---
--- 这是**唯一**会真正 spawn svnui 的地方（除 diff/log 等按需调用）。
--- 所有 action 在执行前都应该先调一次，避免基于过期状态做决策。
---
--- @param cwd string
--- @return boolean 是否成功
--- @return string|nil 错误信息
function util.refresh(cwd)

  -- 优先走 daemon（M9）：带版本戳，无变化时只回几十字节。
  --
  -- ⚠️ 必须传 --since，否则每次都回完整层，版本戳增量形同虚设。
  -- ⚠️ changed == false 时 map 是空的 —— 那表示"没变化"，
  --    绝不能拿空 map 去替换缓存，否则所有状态会凭空消失。
  local data, err = util.svnui_json(cwd, { "q", "--dir", cwd, "--since", tostring(state.stamp or 0) })
  if data then
    if data.changed ~= false then
      state.replace(data.root, data.map or {}, data.v or 0, data.branch)
    else
      -- 无变化：只更新版本戳，保留现有 map
      state.stamp = data.v or state.stamp
      state.updated_at = os.time()
    end
    return true, nil
  end

  -- daemon 不可用 → 退回全量 status
  local entries, err2 = util.svnui_json(cwd, { "status" })
  if not entries then
    -- 不在工作副本：清掉缓存，让染色消失，而不是留着上一次的残留标记
    if err2 and err2:find("not%-working%-copy") then
      state.root = nil
      state.in_wc = false
      state.clear()
      apply_state({ in_wc = false, root = nil, map = {}, paths = {} })
      return false, nil
    end
    return false, err2 or err
  end

  local map = {}
  for _, e in ipairs(entries or {}) do
    if e.path then
      -- svnui status --json 给的是相对路径，这里转成绝对路径便于 O(1) 查表
      local abs = e.path:sub(1, 1) == "/" and e.path or (cwd .. "/" .. e.path)
      map[abs] = e.xy or "  "
    end
  end

  local root = cwd
  local info = util.svnui_json(cwd, { "info" })
  local branch = info and info.branch or nil
  if info and info.root then
    root = info.root
  end

  state.replace(root, map, 0, branch)
  return true, nil
end

-- ---------------------------------------------------------------- 弹框封装
--
-- 全部使用 yazi **官方提供的**交互原语。yazi 没有 ui.Popup 组件
-- （社区里的"Popup"都是插件自己用 ui.Layout 拼的），所以：
--   · 确认/展示 → ya.confirm（多行可滚动，[confirm] 上下文自带 k/j 绑定）
--   · 单行输入   → ya.input
--   · 按键菜单   → ya.which
--   · 结果回执   → ya.notify

--- 展示一段文本。用 ya.confirm 当"可滚动弹框"。
---
--- ⚠️ content 字段是 `content` 不是 `body` —— 这是官方 API 的实际命名。
---
--- @param title string
--- @param content string 可含换行
--- @param w integer|nil 宽度
--- @param h integer|nil 高度
function util.popup(title, content, w, h)
  ya.confirm({
    pos = { "center", w = w or 80, h = h or 24 },
    title = ui.Line(title):fg("yellow"):bold(),
    -- ⚠️ 字段名是 **body**，不是 content。
    --    `ya.confirm{ pos, title, body }` 是官方 types.yazi 里写死的签名；
    --    写成 content 会被静默忽略 → 弹框出来但里面空白。
    body = ui.Text(tostring(content or "")),
  })
end

--- 确认框。返回用户是否确认。
--- @param title string
--- @param content string
--- @return boolean
function util.confirm(title, content)
  local answer = ya.confirm({
    pos = { "center", w = 70, h = 20 },
    title = ui.Line(title):fg("red"):bold(),
    body = ui.Text(tostring(content or "")),
  })
  return answer == true
end

--- 单行输入。
---
--- ya.input 返回 `(value, event)`，event 为 1 表示确认、2 表示取消。
---
--- @param title string
--- @param value string|nil 默认值
--- @return string|nil 用户输入，取消返回 nil
function util.input(title, value)
  local v, event = ya.input({
    pos = { "center", w = 70 },
    title = ui.Line(title):fg("cyan"):bold(),
    value = value or "",
  })
  if event ~= 1 then
    return nil
  end
  return v
end

--- 按键菜单。
---
--- ya.which 返回 **1-based 索引**（不是 0-based）。
---
--- @param cands table[] 形如 { on = "1", desc = "..." }
--- @return integer|nil 选中索引（1-based），取消返回 nil
function util.which(cands)
  local idx = ya.which({ cands = cands })
  if not idx then
    return nil
  end
  return idx
end

--- 通知回执。
--- @param title string
--- @param content string
--- @param level string|nil "info"(默认) / "warn" / "error"
function util.notify(title, content, level)
  ya.notify({
    title = title,
    content = tostring(content or ""),
    timeout = 3,
    level = level or "info",
  })
end


-- ==========================================================================
-- diff —— 弹框 / peek / spot
-- ==========================================================================

--- @since 25.2.13

-- svnui.yazi / diff
--
-- diff 的三种呈现：
--   · `diff.popup(path)`  —— ya.confirm 弹框（vd）
--   · `diff.peek(job)`    —— 预览面板，可滚动
--   · `diff.spot(job)`    —— <Tab> 的 ya.spot_table 表格（不可滚动，只放摘要）
--
-- ⚠️ 三者都只在**有状态**时才渲染 diff，干净文件一律回落，
--    否则每 hover 一个文件就 spawn 一次进程，滚列表会一卡一卡。

-- ============================================================ diff

-- diff 动作 + 预览器接入。
--
-- 两条展示路径：
--   1. `spot`（<Tab>）—— yazi 原生的全屏只读视图，可 j/k 滚动、h/l 切文件。
--      用 ui.Span 给 +/- 行着色，**一定能上色**。这是 diff 的主入口。
--   2. `vd` —— 用 ya.confirm 当可滚动弹框。
--      ⚠️ content 是纯文本，是否解析 ANSI 取决于 yazi 版本，不保证上色，
--      所以这里不主动塞 ANSI 转义，改用 +/-/@ 前缀区分（见 colorize）。


local diff = {}

local MAX_CONFIRM_LINES = 500

--- 预览区一次最多渲染多少行。
---
--- ⚠️ 每个 ui.Line 都是一次 Lua 对象分配 + 一次布局计算。
---    几万行的 diff 会让 hover 变成肉眼可见的卡顿。
---    超了就截断并提示，而不是硬渲。
local MAX_PEEK_LINES = 2000

--- 给 diff 行选颜色。
--- @param line string
--- @return any ui.Span
local function span_for(line)
  local head = line:sub(1, 1)
  if head == "+" then
    return ui.Span(line):fg("green")
  elseif head == "-" then
    return ui.Span(line):fg("red")
  elseif head == "@" then
    return ui.Span(line):fg("blue")
  elseif line:find("^Index:") or line:find("^=====") then
    return ui.Span(line):fg("yellow")
  end
  return ui.Span(line)
end

--- 取某个文件的 diff 原文。
--- @param path string 绝对路径
--- @return string|nil
--- 这类状态的文件 svn diff 会直接报错，得先拦掉。
---
--- 最典型的是未版本化文件（`?`）：`svn diff d.txt` 会报
---   `svn: E155010: The node '...' was not found.`
--- 因为 svn 在 wc.db 里根本没有这个节点的 BASE ——
--- 它不是"改了什么"，而是"从来没进过版本库"。
local NO_DIFF_STATE = {
  ["?"] = "未版本化文件：svn 里没有 BASE，无法 diff。先按 va 执行 svn add。",
  I = "已被 svn:ignore 忽略，不参与版本控制。",
  X = "外部定义（svn:externals），差异在它自己的仓库里。",
  ["~"] = "类型被阻碍（obstructed），先解决类型冲突。",
  ["!"] = "文件缺失（missing）。用 svn revert 恢复，或 svn rm 删除记录。",
}

--- 返回不能 diff 的原因；可以 diff 返回 nil。
function diff.blocked_reason(path)
  local xy = state.porcelain_for(path)
  if not xy then
    return nil
  end
  return NO_DIFF_STATE[xy:sub(1, 1)]
end

function diff.raw(path)
  -- 先查缓存状态，避免对不能 diff 的文件白跑一次 svn（还会报错）
  local blocked = diff.blocked_reason(path)
  if blocked then
    return nil, blocked
  end

  local out, err = util.svnui(util.cwd(), { "diff", "--", path })
  if not out then
    -- svn 对未纳入版本控制的路径报 E155010。缓存还没刷时上面的检查会漏，
    -- 这里兜住，给一句人话而不是原始退出码。
    if err and err:find("E155010") then
      return nil, "svn 认为这个文件不在版本控制内（E155010）。按 vR 刷新后再试。"
    end
    return nil, err
  end
  if out:gsub("%s", "") == "" then
    return nil, nil
  end
  return out
end

--- `vd`：弹框看 diff。
---
--- 超过 MAX_CONFIRM_LINES 行会截断 —— confirm 装不下几千行，
--- 这时提示用户改用 spot 或 pager。
function diff.popup(path)
  local out, err = diff.raw(path)
  if err then
    -- blocked_reason 是"预期内的不能 diff"，用 warn 而不是 error
    local lv = diff.blocked_reason(path) and "warn" or "error"
    util.notify("svnui", err, lv)
    return
  end
  if not out then
    util.notify("svnui", "无本地改动", "info")
    return
  end

  local lines = {}
  for l in out:gmatch("[^\r\n]*") do
    lines[#lines + 1] = l
  end

  local body = lines
  if #lines > MAX_CONFIRM_LINES then
    body = {}
    for i = 1, MAX_CONFIRM_LINES do
      body[i] = lines[i]
    end
    body[#body + 1] = ""
    body[#body + 1] = string.format("... 已截断（共 %d 行），按 <Tab> 用 spot 查看完整内容", #lines)
  end

  util.popup("SVN diff — " .. (path:match("([^/]+)$") or path), table.concat(body, "\n"), 100, 30)
end

--- `spot` 回调：全屏渲染 diff。
---
--- 供 yazi.toml 注册：
---   [plugin]
---   prepend_spotters = [ { url = "*", run = "svnui" } ]
---
--- @param job table
function diff.spot(job)
  local path = tostring(job.file.url)

  -- 26.x 的 spotter 用 ya.spot_table + ui.Table/ui.Row 渲染，
  -- 它是一张**不可滚动**的表格 —— 所以只放摘要与前若干行 diff，
  -- 完整 diff 请走 peek（可滚动）或 vd 弹框。
  local rows = {
    ui.Row({ "SVN diff" }):style(ui.Style():fg("yellow")),
    ui.Row({ " Path:", path }),
  }

  -- 与 peek 同理：不能 diff 的状态在这里就说明原因，别去碰 svn
  local blocked = diff.blocked_reason(path)
  if blocked then
    rows[#rows + 1] = ui.Row({ " Status:", state.porcelain_for(path) or "-" })
    rows[#rows + 1] = ui.Row({ " 说明:", blocked })
    return ya.spot_table(job, ui.Table(rows):area(ui.Pos { "center", w = 90, h = 20 }))
  end

  local out = diff.raw(path)
  if not out then
    local xy = state.porcelain_for(path)
    rows[#rows + 1] = ui.Row({ " Status:", xy and ("状态 " .. xy) or "无本地改动" })
    return ya.spot_table(job, ui.Table(rows):area(ui.Pos { "center", w = 90, h = 20 }))
  end

  local lines = {}
  for l in out:gmatch("[^\r\n]*") do
    if l ~= "" then
      lines[#lines + 1] = l
    end
  end

  rows[#rows + 1] = ui.Row({ " Lines:", tostring(#lines) })

  local SPOT_MAX = 15
  for i = 1, math.min(#lines, SPOT_MAX) do
    local l = lines[i]
    local head = l:sub(1, 1)
    local style = head == "+" and ui.Style():fg("green")
      or head == "-" and ui.Style():fg("red")
      or head == "@" and ui.Style():fg("blue")
      or nil
    local row = ui.Row({ " " .. l })
    rows[#rows + 1] = style and row:style(style) or row
  end
  if #lines > SPOT_MAX then
    rows[#rows + 1] = ui.Row({ (" ... 还有 %d 行，用 peek 或 vd 查看"):format(#lines - SPOT_MAX) })
  end

  ya.spot_table(
    job,
    ui.Table(rows)
      :area(ui.Pos { "center", w = 100, h = math.min(#rows + 2, 30) })
      :row(1)
      :col(1)
      :col_style(th.spot and th.spot.tbl_col or ui.Style())
      :widths({ ui.Constraint.Length(12), ui.Constraint.Fill(1) })
  )
end

--- `peek` 回调：预览面板 hover 即见 diff。
---
--- ⚠️ 与 linemode 不同，peek 是 async 的，可以 spawn 进程。
--- 但必须**先查缓存做短路**：无改动的文件直接回落到 yazi 默认预览器，
--- 否则在源码目录里上下移动光标会一卡一顿。
---
--- @param job table
--- 在预览区渲染一段提示。
---
--- ⚠️ 为什么不用"返回 nil 让 yazi 自己处理"：
---    这些状态（缺失 / 冲突 / diff 失败）**没有内容可展示**，
---    yazi 默认预览器要么报错要么空白。宁可自己画一屏说明。
local function notice(job, title, lines, color)
  local out = { ui.Line(ui.Span(title):fg(color or "yellow"):bold()) }
  for _, l in ipairs(lines or {}) do
    out[#out + 1] = ui.Line(ui.Span(l):fg("darkgray"))
  end
  ya.preview_widget(job, ui.Text(out):area(job.area))
end

--- 短路径（去掉工作副本根前缀，预览区宽度有限）。
local function rel(path)
  if state.root and path:sub(1, #state.root) == state.root then
    return path:sub(#state.root + 2)
  end
  return path
end

function diff.peek(job)
  local path = tostring(job.file.url)
  -- peek 跑在 async，必须走 plugin state 查表
  local xy = get_xy(path)

  -- 无状态（干净文件 / 不在工作副本）→ 完全不接管，用 yazi 默认预览器。
  -- 代码高亮、大文件截断这些默认行为比我们自绘的好得多。
  if not xy then
    return
  end

  local x = xy:sub(1, 1)
  local y = xy:sub(2, 2)

  -- ? / I / X：文件**有内容但没进版本控制**。
  -- 这里刻意回落默认预览器 —— 用户想看的是文件内容本身。
  -- "未版本化"这个信息由行尾的 `?` 标记承担（linemode），
  -- 不必在预览区重复占位。
  if x == "?" or x == "I" or x == "X" then
    return
  end

  -- D / !：文件已删除或缺失，读不到内容
  if x == "D" or x == "!" then
    return notice(job, "⚠ 文件缺失", {
      "",
      "状态: " .. xy,
      rel(path),
      "",
      "该文件在工作副本中不存在（已删除或被外部移除）。",
      "svn revert 可恢复；svn rm 可确认删除。",
    }, "red")
  end

  -- C / T：冲突
  if x == "C" or x == "T" or y == "C" then
    return notice(job, "⚠ 冲突未解决", {
      "",
      "状态: " .. xy .. (x == "T" and "（树冲突）" or "（内容冲突）"),
      rel(path),
      "",
      "按 vk 打开冲突处理菜单。",
    }, "red")
  end

  local out, err = diff.raw(path)
  if err then
    -- ⚠️ 明确画出错误，不留白。空白预览会让人以为"文件是空的"。
    return notice(job, "⚠ 无法获取 diff", { "", rel(path), "", err }, "red")
  end

  if not out then
    -- 有状态但 diff 为空：常见于刚 add 还没内容的新文件
    if x == "A" then
      return notice(job, "＋ 新增文件", {
        "",
        "状态: " .. xy,
        rel(path),
        "",
        "已纳入版本控制，尚无基线可比较（提交后才会产生 diff）。",
      }, "green")
    end
    return notice(job, "无本地改动", { "", rel(path) }, "darkgray")
  end

  local raw = {}
  for l in out:gmatch("[^\r\n]*") do
    raw[#raw + 1] = l
  end

  local lines = {}
  local n = math.min(#raw, MAX_PEEK_LINES)
  for i = 1, n do
    lines[#lines + 1] = ui.Line({ span_for(raw[i]) })
  end
  if #raw > n then
    lines[#lines + 1] = ui.Line(
      ui.Span(("... 已截断（共 %d 行，上限 %d）。完整内容按 vd 弹框查看"):format(#raw, n))
        :fg("darkgray")
    )
  end
  ya.preview_widget(job, ui.Text(lines):area(job.area))
end

function diff.spot(job)
  local path = tostring(job.file.url)

  -- 26.x 的 spotter 用 ya.spot_table + ui.Table/ui.Row 渲染，
  -- 它是一张**不可滚动**的表格 —— 所以只放摘要与前若干行 diff，
  -- 完整 diff 请走 peek（可滚动）或 vd 弹框。
  local rows = {
    ui.Row({ "SVN diff" }):style(ui.Style():fg("yellow")),
    ui.Row({ " Path:", path }),
  }

  -- 与 peek 同理：不能 diff 的状态在这里就说明原因，别去碰 svn
  local blocked = diff.blocked_reason(path)
  if blocked then
    rows[#rows + 1] = ui.Row({ " Status:", state.porcelain_for(path) or "-" })
    rows[#rows + 1] = ui.Row({ " 说明:", blocked })
    return ya.spot_table(job, ui.Table(rows):area(ui.Pos { "center", w = 90, h = 20 }))
  end

  local out = diff.raw(path)
  if not out then
    local xy = state.porcelain_for(path)
    rows[#rows + 1] = ui.Row({ " Status:", xy and ("状态 " .. xy) or "无本地改动" })
    return ya.spot_table(job, ui.Table(rows):area(ui.Pos { "center", w = 90, h = 20 }))
  end

  local lines = {}
  for l in out:gmatch("[^\r\n]*") do
    if l ~= "" then
      lines[#lines + 1] = l
    end
  end

  rows[#rows + 1] = ui.Row({ " Lines:", tostring(#lines) })

  local SPOT_MAX = 15
  for i = 1, math.min(#lines, SPOT_MAX) do
    local l = lines[i]
    local head = l:sub(1, 1)
    local style = head == "+" and ui.Style():fg("green")
      or head == "-" and ui.Style():fg("red")
      or head == "@" and ui.Style():fg("blue")
      or nil
    local row = ui.Row({ " " .. l })
    rows[#rows + 1] = style and row:style(style) or row
  end
  if #lines > SPOT_MAX then
    rows[#rows + 1] = ui.Row({ (" ... 还有 %d 行，用 peek 或 vd 查看"):format(#lines - SPOT_MAX) })
  end

  ya.spot_table(
    job,
    ui.Table(rows)
      :area(ui.Pos { "center", w = 100, h = math.min(#rows + 2, 30) })
      :row(1)
      :col(1)
      :col_style(th.spot and th.spot.tbl_col or ui.Style())
      :widths({ ui.Constraint.Length(12), ui.Constraint.Fill(1) })
  )
end

--- `peek` 回调：预览面板 hover 即见 diff。
---
--- ⚠️ 与 linemode 不同，peek 是 async 的，可以 spawn 进程。
--- 但必须**先查缓存做短路**：无改动的文件直接回落到 yazi 默认预览器，
--- 否则在源码目录里上下移动光标会一卡一顿。
---
--- @param job table
function diff.peek(job)
  local path = tostring(job.file.url)
  local sign = state.sign_for(path)

  -- 无状态 → 不接管，让 yazi 用默认预览器
  if not sign then
    return
  end

  -- ⚠️ 关键：不能 diff 的状态（? / I / X / ~ / !）必须**在这就交还控制权**。
  --    如果放行到 diff.raw，它会返回 nil，于是下面 `if not out then return end`
  --    直接返回 —— 既不渲染 diff、也不回落到默认预览器，
  --    结果就是**预览面板一片空白**（比不接管还糟）。
  --    典型受害者：未版本化文件（?），svn diff 报 E155010。
  if diff.blocked_reason(path) then
    return
  end

  local out = diff.raw(path)
  if not out then
    return
  end

  local lines = {}
  for l in out:gmatch("[^\r\n]*") do
    lines[#lines + 1] = ui.Line({ span_for(l) })
  end
  ya.preview_widget(job, ui.Text(lines):area(job.area))
end


-- ==========================================================================
-- commit —— 提交流程
-- ==========================================================================

--- @since 25.2.13

-- svnui.yazi / commit
--
-- 提交流程：范围 → 确认 → 输信息 → 执行 → 回执版本号。

-- ============================================================ commit

-- 提交动作。
--
-- 流程：刷新 → 收集目标 → confirm 确认范围 → input 输入信息 → 执行 → notify 回执。
--
-- ⚠️ 调 CLI 时**一律带 --yes**：确认已经在 yazi 的 ya.confirm 里做过了。
--    不要让用户在 TUI 里被问两次。


local commit = {}

--- 收集本次要提交的路径。
---
--- 有选中就用选中；没有则提示"提交全部变更"并列出清单。
--- @return string[]|nil
local function collect()
  local selected = util.targets()
  -- util.targets 在无选中时会回落到 hovered，这里需要区分：
  -- 真正"只选了一个"和"没选"在提交语义上完全不同。
  local n_selected = util.selection_count()

  if n_selected > 0 then
    return selected
  end
  return state.changed_paths()
end

--- 该状态能否进入提交候选。
---
--- ⚠️ `?` 未版本化**默认不进候选**：svn 不会提交没 add 的文件，
---    把它列进去只会让人以为"勾了就会提交"，结果静默漏掉。
---    想加进去请先 `va`（svn add）。
---
--- I(ignored) / X(external) 同理，它们根本不属于版本控制。
local function committable(xy)
  if not xy or #xy < 1 then
    return false
  end
  local x = xy:sub(1, 1)
  return x ~= "?" and x ~= "I" and x ~= "X" and x ~= " " and x ~= "_"
end

--- 提交候选（已按 committable 过滤）。
--- @return string[]
local function candidates()
  local out = {}
  for _, p in ipairs(state.changed_paths()) do
    if committable(state.porcelain_for(p)) then
      out[#out + 1] = p
    end
  end
  return out
end

--- 统计勾选数量。
--- @param picks table<string,boolean>
--- @return integer
local function count_picks(picks)
  local n = 0
  for _, on in pairs(picks) do
    if on then n = n + 1 end
  end
  return n
end

--- 按 all 的顺序取出已勾选路径，保持顺序稳定。
--- @return string[]
local function picked_list(all, picks)
  local out = {}
  for _, p in ipairs(all) do
    if picks[p] then
      out[#out + 1] = p
    end
  end
  return out
end

local function short(paths, limit)
  limit = limit or 30
  local out = {}
  for i, p in ipairs(paths) do
    if i > limit then
      out[#out + 1] = string.format("  ... 还有 %d 项", #paths - limit)
      break
    end
    local xy = state.porcelain_for(p) or "  "
    out[#out + 1] = string.format("  %s  %s", xy, p)
  end
  return out
end

--- 用 ya.which 让用户逐项调整勾选。
---
--- ⚠️ yazi **没有 ui.Popup 组件**，做不出带 [x] 复选框的面板。
---    官方只有这几种交互原语：ya.confirm（多行可滚动）/ ya.input（单行）
---    / ya.which（按键菜单）/ ya.notify（回执）。
---
--- 所以"勾选面板"用 which 循环近似：每次列出当前勾选状态，
--- 让用户按键切换；选"完成"退出。视觉上是菜单不是复选框，
--- 但**语义完全等价**，且不需要自绘组件。
---
--- @param picks table<string,boolean> 勾选状态，会被就地修改
local function pick_loop(picks, all)
  while true do
    local cands = {}
    for _, p in ipairs(all) do
      local mark = picks[p] and "[x]" or "[ ]"
      local xy = state.porcelain_for(p) or "  "
      cands[#cands + 1] = {
        on = tostring(#cands + 1):sub(-1),
        desc = string.format("%s %s  %s", mark, xy, p:match("([^/]+)$") or p),
      }
      if #cands >= 9 then
        break
      end
    end
    cands[#cands + 1] = { on = "a", desc = "全选" }
    cands[#cands + 1] = { on = "n", desc = "全不选" }
    cands[#cands + 1] = { on = "d", desc = "完成，进入下一步" }

    local idx = util.which(cands)
    if not idx then
      return false
    end

    local pick = cands[idx]
    if pick.on == "d" then
      return true
    elseif pick.on == "a" then
      for _, p in ipairs(all) do picks[p] = true end
    elseif pick.on == "n" then
      for _, p in ipairs(all) do picks[p] = false end
    else
      local n = tonumber(pick.on)
      if n and all[n] then
        picks[all[n]] = not picks[all[n]]
      end
    end
  end
end

--- 渲染候选清单（供 confirm 展示）。
local function render_picks(all, picks)
  local lines = {}
  for _, p in ipairs(all) do
    local mark = picks[p] and "[x]" or "[ ]"
    local xy = state.porcelain_for(p) or "  "
    lines[#lines + 1] = string.format("  %s %s  %s", mark, xy, p)
  end
  return lines
end

--- 用 yazi 原生选中机制"标记"待提交文件。
---
--- ⚠️ 这是整个插件最关键的交互设计。
---
--- yazi **没有 ui.Popup 组件**，`ya.confirm` 只有 [Y]es/(N)o 两个按钮，
--- 做不出带 [x] 复选框、能 ↑↓/Space 操作的面板。自绘也不现实 ——
--- 插件拿不到弹框内的按键事件。
---
--- 但 yazi 自己**早就有一套成熟的选中机制**：
---   Space 逐个切换 / v 进 visual mode 批量选 / Ctrl+A 全选
---   / Ctrl+R 反选 / ESC 清空 / 搜索结果里 v 全选
---
--- 所以把"勾选提交范围"这件事**外包给 yazi 原生选中**：
---   第一次 vc → 自动选中所有候选（toggle_all）
---   用户用原生操作调整
---   第二次 vc → 读 cx.active.selected 提交
---
--- 比自绘弹框强得多：visual mode、鼠标、搜索批量选全都白拿。
---
--- @param paths string[] 要选中的绝对路径
local function select_paths(paths)
  -- 先清空，避免把用户之前的选中混进来
  pcall(ya.emit, "toggle_all", { state = "off" })
  if #paths == 0 then
    return
  end
  -- toggle_all 支持一次传多个 url（避免循环 emit 导致 N 次重绘）
  local args = { state = "on" }
  for _, p in ipairs(paths) do
    args[#args + 1] = p
  end
  pcall(ya.emit, "toggle_all", args)
end

--- 读取当前 yazi 选中的文件。
--- 从 `cx.active.selected` 安全地取出**绝对路径**列表。
---
--- ⚠️⚠️ 这个 helper 存在的唯一理由：`Selected` 的 `__pairs` 返回值顺序
---    **文档和实现不一致**。
---
---    官方文档（Context → tab::Selected）写：
---        __pairs(self) → fun(t: self, k: any): integer, Url
---    即 (index, Url)。
---
---    但 yazi 26.9 实际返回的是 **(Url, index)**。
---
---    之前按文档写 `for _, u in pairs(sel)`，结果 u 拿到的是 index ——
---    于是把 "1" "2" "3" "4" 当路径传给 svn，报：
---        W155010: '/wc/1' is not found
---        E200009: 部分目标不存在
---
---    修法不是"猜一个正确顺序"，而是**两个返回值都检查、按类型挑**：
---      · index 一定是 number → 排除
---      · 路径一定是以 "/" 开头的 string → 只要它
---    这样无论 yazi 用哪种顺序，都不会再错。
---
--- @return string[]
local function selected_paths(sel)
  local out = {}
  if not sel then
    return out
  end
  local ok, it, t, k = pcall(pairs, sel)
  if not ok then
    return out
  end
  for a, b in it, t, k do
    for _, v in ipairs({ a, b }) do
      if v ~= nil and type(v) ~= "number" then
        local str = tostring(v)
        -- 绝对路径以 / 开头；Url 元表 __tostring 给的就是路径
        if str:sub(1, 1) == "/" then
          out[#out + 1] = str
        end
      end
    end
  end
  return out
end

local get_selected = ya.sync(function()
  return selected_paths(cx.active.selected)
end)

--- 清空 yazi 选中。
local function clear_selection()
  pcall(ya.emit, "toggle_all", { state = "off" })
end

--- 提交。两阶段：先"标范围"，再"提交"。
---
--- 状态机（state.commit_mode）：
---   nil/false → 第一次 vc：自动选中候选，进入待确认态
---   true      → 第二次 vc：读选中，提交
---
--- 为什么用状态机而不是"看有没有选中"：
---   用户可能本来就有选中（想提交这几个），
---   这时不该又去全选一遍覆盖掉。
function commit.run()
  local cwd = util.cwd()

  util.refresh(cwd)
  if not state.in_wc then
    util.notify("svnui", "不在 SVN 工作副本内", "warn")
    return
  end

  local all = candidates()
  if #all == 0 then
    util.notify("svnui", "没有可提交的变更（未版本化文件请先 va 执行 svn add）", "info")
    return
  end

  -- ── 阶段一：还没进入提交模式 → 标记范围
  if not state.commit_mode then
    -- 用户已有选中 → 尊重它，直接进入阶段二
    local cur = get_selected()
    if #cur > 0 then
      state.commit_mode = true
    else
      -- 没有选中 → 自动全选候选（排除 ? / I / X），然后等用户调整
      select_paths(all)
      state.commit_mode = true
      set_field("commit_mode", true)
      do_render()
      util.notify(
        "svnui",
        string.format("已选中 %d 项待提交。用 Space / v / ESC 调整后，再按 vc 提交", #all),
        "info"
      )
      return
    end
  end

  -- ── 阶段二：读当前选中 → 提交
  local sel = get_selected()
  if #sel == 0 then
    state.commit_mode = false
    set_field("commit_mode", false)
    util.notify("svnui", "没有选中任何文件。按 vc 重新选择范围", "warn")
    return
  end

  -- 只保留真正可提交的（用户可能选了未版本化的文件）
  local final, skipped = {}, {}
  local ok_set = {}
  for _, p in ipairs(all) do
    ok_set[p] = true
  end
  for _, p in ipairs(sel) do
    if ok_set[p] then
      final[#final + 1] = p
    else
      skipped[#skipped + 1] = p
    end
  end

  if #final == 0 then
    state.commit_mode = false
    set_field("commit_mode", false)
    util.notify("svnui", "选中的文件都不可提交（未版本化请先 va）", "warn")
    return
  end

  -- 提交信息
  local msg = util.input("Commit message" .. string.format("（%d 项）", #final) .. ":")
  state.commit_mode = false
  set_field("commit_mode", false)
  if not msg or msg:gsub("%s", "") == "" then
    util.notify("svnui", "提交信息为空，已取消（选中已保留）", "warn")
    return
  end

  -- 显式传路径
  local args = { "commit", "--yes", "-m", msg, "--" }
  for _, p in ipairs(final) do
    args[#args + 1] = p
  end

  local out, err = util.svnui(cwd, args)
  if not out then
    util.notify("svnui", err or "提交失败", "error")
    return
  end

  clear_selection()
  util.notify("svnui", out .. (#skipped > 0 and string.format("（跳过 %d 个不可提交项）", #skipped) or ""), "info")
  util.refresh(cwd)
  do_render()
end

--- 取消提交模式：清空选中并退出。
function commit.cancel()
  state.commit_mode = false
  set_field("commit_mode", false)
  clear_selection()
  do_render()
  util.notify("svnui", "已取消提交", "info")
end

-- ==========================================================================
-- log —— 日志（变量 svnlog，避免遮蔽 log）
-- ==========================================================================

--- @since 25.2.13

-- svnui.yazi / log
--
-- 日志查看。变量叫 svnlog 而不是 log —— 避免与 Lua 的 math.log 混淆。

-- ============================================================ log

-- 日志动作。
--
-- `vl` → 单行日志列表（ya.confirm 弹框，可滚动）
-- `vL` → 先选条数，再选某条看详情

local svnlog = {}

--- 展示最近 N 条。
--- @param limit integer
function svnlog.oneline(limit)
  local cwd = util.cwd()
  limit = limit or 30

  local out, err = util.svnui(cwd, { "log", "--oneline", "-l", tostring(limit) })
  if not out then
    util.notify("svnui", err or "取日志失败", "error")
    return
  end
  if out:gsub("%s", "") == "" then
    util.notify("svnui", "没有提交历史", "warn")
    return
  end

  util.popup(string.format("SVN log (最近 %d 条)", limit), out, 110, 30)
end

--- 完整格式（含正文）。
function svnlog.verbose(limit)
  local cwd = util.cwd()
  limit = limit or 10

  local out, err = util.svnui(cwd, { "log", "-l", tostring(limit) })
  if not out then
    util.notify("svnui", err or "取日志失败", "error")
    return
  end

  -- 完整日志可能很长，超出阈值就交给 pager（svnui 会自动分页）
  util.popup(string.format("SVN log — 最近 %d 条", limit), out, 110, 30)
end

--- 交互式：先选条数。
function svnlog.menu()
  local idx = util.which({
    { on = "1", desc = "最近 20 条（单行）" },
    { on = "2", desc = "最近 50 条（单行）" },
    { on = "3", desc = "最近 10 条（含正文）" },
    { on = "4", desc = "查看指定版本" },
  })
  if not idx then
    return
  end

  if idx == 1 then
    svnlog.oneline(20)
  elseif idx == 2 then
    svnlog.oneline(50)
  elseif idx == 3 then
    svnlog.verbose(10)
  elseif idx == 4 then
    local rev = util.input("Revision (如 1234，或区间 1200:1234):")
    if not rev or rev:gsub("%s", "") == "" then
      return
    end
    -- ⚠️ 是 -r 不是 -c：svn 的 -c N 表示"该版本引入的变化"（类似 git show），
    -- 而这里用户想看的是"该版本的日志条目"，用 -r N。
    local out, err = util.svnui(util.cwd(), { "log", "-r", rev })
    if not out then
      util.notify("svnui", err or "取日志失败", "error")
      return
    end
    util.popup("SVN log — r" .. rev, out, 110, 30)
  end
end


-- ==========================================================================
-- rescue —— 冲突 / 清理 / 体检
-- ==========================================================================

--- @since 25.2.13

-- svnui.yazi / rescue
--
-- 冲突处理 / 清理 / 体检。
--
-- SVN 的重灾区：文本冲突、属性冲突、树冲突（tree conflict）三套语义完全不同，
-- 外加 .mine/.rOLD/.rNEW 残留文件。这里统一收口。

-- ============================================================ rescue

-- 救援：冲突处理 / 清理 / 体检。
--
-- SVN 的冲突比 git 麻烦得多（文本冲突 + 属性冲突 + 树冲突三类，
-- 树冲突还会多打一行说明），这里把它做成两级菜单而不是让人手敲命令。


local rescue = {}

local STRATEGIES = {
  { on = "1", desc = "mine-full   全部用我的版本（丢弃服务端改动）", value = "mine-full" },
  { on = "2", desc = "theirs-full 全部用服务端版本（丢弃本地改动）", value = "theirs-full" },
  { on = "3", desc = "working     保留我手工编辑后的结果（推荐）", value = "working" },
  { on = "4", desc = "base        回到更新前的基线版本", value = "base" },
}

--- 冲突处理。
function rescue.conflicts()
  local cwd = util.cwd()
  util.refresh(cwd)

  local paths = state.conflict_paths()
  if #paths == 0 then
    util.notify("svnui", "没有冲突", "info")
    return
  end

  -- 第一步：确认要处理哪些
  local body = { string.format("检测到 %d 项冲突：", #paths), "" }
  for i, p in ipairs(paths) do
    if i > 30 then
      body[#body + 1] = string.format("  ... 还有 %d 项", #paths - 30)
      break
    end
    body[#body + 1] = "  " .. p
  end
  body[#body + 1] = ""
  body[#body + 1] = "选择解决策略："

  if not util.confirm("SVN 冲突", table.concat(body, "\n")) then
    return
  end

  -- 第二步：选策略
  local idx = util.which(STRATEGIES)
  if not idx or not STRATEGIES[idx] then
    return
  end
  local strategy = STRATEGIES[idx].value

  -- 树冲突用 theirs-full 前再确认一次 —— 可能丢整个目录的本地改动
  if strategy == "theirs-full" then
    if not util.confirm("确认", "theirs-full 会丢弃本地改动，且无法撤销。\n确定继续？") then
      return
    end
  end

  local args = { "resolve", "--yes", "-a", strategy }
  for _, p in ipairs(paths) do
    args[#args + 1] = p
  end

  local out, err = util.svnui(cwd, args)
  if not out then
    util.notify("svnui", err or "解决冲突失败", "error")
    return
  end

  util.notify("svnui", out, "info")
  util.refresh(cwd)

  -- resolve 之后立刻复检：如果还有冲突，说明策略没覆盖全部情况
  local left = state.conflict_paths()
  if #left > 0 then
    util.notify("svnui", string.format("仍有 %d 项冲突未解决", #left), "warn")
  end
end

--- 体检报告。
function rescue.doctor()
  local cwd = util.cwd()
  local data, err = util.svnui_json(cwd, { "doctor" })
  if not data then
    util.notify("svnui", err or "体检失败", "error")
    return
  end

  local lines = {
    data.healthy and "工作副本健康" or "存在问题：",
    "",
    string.format("  锁          %d", data.locked and #data.locked or 0),
    string.format("  冲突        %d", data.conflicts and #data.conflicts or 0),
    string.format("  缺失        %d", data.missing and #data.missing or 0),
    string.format("  阻碍        %d", data.obstructed and #data.obstructed or 0),
    string.format("  .mine 残留  %d", data.residual_mine and #data.residual_mine or 0),
    string.format("  未版本化    %d", data.unversioned_count or 0),
  }

  if data.conflicts and #data.conflicts > 0 then
    lines[#lines + 1] = ""
    lines[#lines + 1] = "冲突文件："
    for i, p in ipairs(data.conflicts) do
      if i > 20 then
        lines[#lines + 1] = string.format("  ... 还有 %d 项", #data.conflicts - 20)
        break
      end
      lines[#lines + 1] = "  " .. p
    end
  end
  if data.locked and #data.locked > 0 then
    lines[#lines + 1] = ""
    lines[#lines + 1] = "有锁 → 先跑 vx 选「仅解锁」"
  end

  util.popup("SVN 体检", table.concat(lines, "\n"), 80, 24)
end

--- 清理菜单。
function rescue.cleanup()
  local idx = util.which({
    { on = "1", desc = "仅解锁（不删任何文件）" },
    { on = "2", desc = "清理 .svn/pristine 垃圾" },
    { on = "3", desc = "删除未版本化文件    ⚠️高危" },
    { on = "4", desc = "删除被忽略的文件    ⚠️高危" },
  })
  if not idx then
    return
  end

  local cwd = util.cwd()

  if idx == 1 then
    local out, err = util.svnui(cwd, { "cleanup", "--yes" })
    if out then
      util.notify("svnui", out, "info")
    else
      util.notify("svnui", err or "清理失败", "error")
    end
    util.refresh(cwd)
    return
  end

  if idx == 2 then
    local out, err = util.svnui(cwd, { "cleanup", "--yes", "--vacuum-pristines" })
    if out then
      util.notify("svnui", out, "info")
    else
      util.notify("svnui", err or "清理失败", "error")
    end
    return
  end

  -- 高危：必须先列出将被删除的文件
  local flag = idx == 3 and "--remove-unversioned" or "--remove-ignored"

  local targets, err = util.svnui_json(cwd, { "status", "--ignored" })
  if not targets then
    util.notify("svnui", err or "无法取得待删清单", "error")
    return
  end

  local will_delete = {}
  for _, e in ipairs(targets or {}) do
    if (idx == 3 and e.sign == "?") or (idx == 4 and e.sign == "I") then
      will_delete[#will_delete + 1] = e.path
    end
  end

  if #will_delete == 0 then
    util.notify("svnui", "没有匹配的文件", "info")
    return
  end

  local body = { string.format("将永久删除 %d 项（无法撤销）：", #will_delete), "" }
  for i, p in ipairs(will_delete) do
    if i > 40 then
      body[#body + 1] = string.format("  ... 还有 %d 项", #will_delete - 40)
      break
    end
    body[#body + 1] = "  " .. p
  end

  if not util.confirm("危险操作", table.concat(body, "\n")) then
    return
  end

  local out, err2 = util.svnui(cwd, { "cleanup", "--yes", flag })
  if out then
    util.notify("svnui", out, "info")
  else
    util.notify("svnui", err2 or "清理失败", "error")
  end
  util.refresh(cwd)
end


-- ==========================================================================
-- 入口（setup / entry / peek / seek / spot）
-- ==========================================================================

--- @since 25.2.13

-- svnui.yazi —— SVN 状态集成（yazi 26.x）
--
-- 入口文件。拆成多模块只是为了可读性，**不是**为了复用：
--   json.lua    自带 JSON 解码器（yazi 没有 json 全局）
--   state.lua   状态缓存中心（唯一数据源，Linemode 只读它）
--   util.lua    调 svnui CLI + 官方弹框
--   diff.lua    diff 三种呈现（弹框 / peek / spot）
--   commit.lua  提交流程
--   log.lua     日志
--   rescue.lua  冲突 / 清理 / 体检
--
-- ⚠️ 关于 require：
--   · yazi 的 require **能**加载插件内部文件，规则是 `require("svnui.state")`
--     → `plugins/svnui.yazi/state.lua`（按第一个 `.` 切成 插件名 + 入口名）
--   · 必须写全 `svnui.xxx`，裸 `require("state")` 会去找 `plugins/state.yazi/`
--   · entry 名必须 kebab-case；**子目录不可靠**，所以文件全部平铺
--   · 循环依赖要避免：state ← util ← {diff,commit,log,rescue} ← main（单向）
--
-- 用法：
--   1. require("svnui"):setup {}          —— 注册 Linemode 染色 + 状态栏
--   2. plugin svnui -- <action>           —— keymap 触发
--   3. yazi.toml 注册 previewer/spotter   —— peek / spot


-- ---------------------------------------------------------------- cx 访问
--
-- ya.sync() 必须在**入口文件顶层**调用：
-- 它注册的回调按模块 ID 存放，执行时按"当前模块"定位。
-- 放在被 require 的子模块里，从 main 调用会定位错槽位。
-- 定义完再注入 util，供其他模块使用。
--
-- 返回值必须是 Sendable 类型（string / table of string），可安全跨线程。

local get_cwd = ya.sync(function()
  return tostring(cx.active.current.cwd)
end)

local get_targets = ya.sync(function()
  local out = selected_paths(cx.active.selected)
  if #out > 0 then
    table.sort(out)
    return out
  end
  local h = cx.active.current.hovered
  if h then
    out[#out + 1] = tostring(h.url)
  end
  return out
end)

--- 选中数量。用来区分"只选了一个"和"没选" —— 提交语义上完全不同。
--- 当前光标文件 url（诊断用）。
---
--- ⚠️ cx 只能在 sync 上下文读。diagnose 跑在 async（entry），
---    直接写 `cx.active.current.hovered` 会拿不到 —— 必须包 ya.sync。
local get_hovered = ya.sync(function()
  local h = cx.active.current.hovered
  return h and tostring(h.url) or nil
end)

local get_selection_count = ya.sync(function()
  local n = 0
  for _ in pairs(cx.active.selected) do
    n = n + 1
  end
  return n
end)

-- 注入给子模块（必须在任何 action 执行前完成）
util.cwd = get_cwd
util.targets = get_targets
util.selection_count = get_selection_count

-- ============================================================ 入口

local M = {}

--- 状态符号 → 颜色。与 theme.toml 的 [svn] 段语义一致。
--- 硬编码一份是为了不依赖用户是否合并了 theme.toml：
--- 颜色错了只是难看，配色段缺失导致 Lua 报错才是灾难。
local COLOR = {
  A = "green", G = "green", M = "yellow",
  D = "red", ["!"] = "red", C = "red", T = "red",
  R = "magenta", S = "magenta",
  X = "blue", K = "blue",
  I = "darkgray", ["?"] = "darkgray", ["~"] = "lightred",
}

--- 冒泡优先级，与 Rust 侧 StatusKind::priority 必须一致。
local PRIORITY = {
  T = 100, C = 100, ["!"] = 90, ["~"] = 80, R = 70,
  D = 60, A = 50, M = 40, G = 35, ["?"] = 20, I = 10, X = 5,
}

local function render(sign)
  if not sign or sign == " " or sign == "" then
    return ""
  end
  local span = ui.Span(" " .. sign)
  if COLOR[sign] then
    span = span:fg(COLOR[sign])
  end
  if sign == "C" or sign == "T" then
    span = span:bold()
  end
  return ui.Line({ span })
end

M.cfg = { order = 1500, show_branch = true }

--- 注册染色。在 ~/.config/yazi/init.lua 中：
---   require("svnui"):setup { order = 1500 }
function M:setup(opts)
  opts = opts or {}
  state.setup_done = true
  -- ⚠️ 必须落进 plugin state：setup 跑在 sync 上下文，
  --    而 entry 跑在**全新的 async 实例**，模块级 state 是空的。
  --    不落盘的话 entry 每次都读到 false，于是每次都弹"未初始化"。
  set_field("setup_done", true)
  if opts.order then self.cfg.order = opts.order end
  if opts.show_branch ~= nil then self.cfg.show_branch = opts.show_branch end
  if opts.svnui_path then util.set_bin(opts.svnui_path) end

  -- ⚠️ 染色必须用 get_sign（走 plugin state）。
  --    直接读模块级 `state` 会永远是空 —— 因为 async 侧的刷新写进了
  --    另一个 Lua 实例，而这里是 sync 上下文（见同步层说明）。
  Linemode:children_add(function(self)
    local file = self and self._file
    if not file then return "" end
    -- 只读内存缓存。这里绝不能 spawn 进程 —— 它每渲染一行都会跑一次。
    return render(get_sign(tostring(file.url)))
  end, self.cfg.order)

  if self.cfg.show_branch then
    Status:children_add(function() return get_status_line() end, 500, Status.RIGHT)
  end
end

-- ---------------------------------------------------------- 动作分发

--- 自检。把内部状态全打出来，用来定位"按了没反应"这类问题。
---
--- ⚠️ 这个动作存在的唯一理由：yazi 插件的错误信息极难排查，
---    （Lua 报错只在通知里闪一下，看不见堆栈），
---    所以宁可一次性把所有中间量摊开看。
local function diagnose()
  local cwd = get_cwd()
  local L = {}

  L[#L + 1] = "── 环境 ──"
  local sd = get_field("setup_done")
  L[#L + 1] = (sd and "[ok] " or "[!!] ") .. "setup 已调用 : " .. tostring(sd)
  if not state.setup_done then
    L[#L + 1] = "     → 没有染色/状态栏！init.lua 里需要："
    L[#L + 1] = "       require(\"svnui\"):setup {}"
  end
  L[#L + 1] = "svnui 路径    : " .. tostring(CFG.bin)
  L[#L + 1] = "ui.render    : " .. tostring(type(ui.render))
  L[#L + 1] = "ya.render    : " .. tostring(type(ya.render))
  L[#L + 1] = "cwd          : " .. tostring(cwd)

  -- 直接 spawn 一次，看原始输出
  L[#L + 1] = ""
  L[#L + 1] = "── 原始调用（q）──"
  local raw, rerr = util.svnui(cwd, { "--json", "q", "--dir", cwd, "--since", "0" })
  if raw then
    L[#L + 1] = "返回 " .. #raw .. " 字节:"
    L[#L + 1] = raw:sub(1, 600)
  else
    L[#L + 1] = "失败: " .. tostring(rerr)
  end

  -- 再跑一次 status 做对照
  L[#L + 1] = ""
  L[#L + 1] = "── 原始调用（status）──"
  local raw2, rerr2 = util.svnui(cwd, { "--json", "status" })
  if raw2 then
    L[#L + 1] = "返回 " .. #raw2 .. " 字节:"
    L[#L + 1] = raw2:sub(1, 600)
  else
    L[#L + 1] = "失败: " .. tostring(rerr2)
  end

  -- 刷一次，看缓存变成什么样
  L[#L + 1] = ""
  L[#L + 1] = "── 缓存 ──"
  local ok, err = util.refresh(cwd)
  L[#L + 1] = "refresh      : " .. tostring(ok) .. (err and ("  err=" .. err) or "")
  L[#L + 1] = "in_wc        : " .. tostring(state.in_wc)
  L[#L + 1] = "root         : " .. tostring(state.root)
  L[#L + 1] = "branch       : " .. tostring(state.branch)
  L[#L + 1] = "paths 数量   : " .. tostring(#state.paths)
  L[#L + 1] = "map 键数量   : " .. (function()
    local n = 0
    for _ in pairs(state.map) do n = n + 1 end
    return n
  end)()

  local shown = 0
  for _, pth in ipairs(state.paths) do
    if shown >= 8 then break end
    L[#L + 1] = string.format("  %s  %s", state.map[pth] or "??", pth)
    shown = shown + 1
  end

  -- 当前 hovered 能不能查到
  L[#L + 1] = ""
  L[#L + 1] = "── 当前文件查表 ──"
  local u = get_hovered()
  if u then
    L[#L + 1] = "hovered url  : " .. u
    L[#L + 1] = "查表结果     : " .. tostring(state.map[u])
    L[#L + 1] = "sign_for     : " .. tostring(state.sign_for(u))
  else
    L[#L + 1] = "（没有 hovered 文件）"
  end

  util.popup("svnui 自检", table.concat(L, "\n"), 120, 40)
  do_render()
end

--- 刷新 SVN 状态并重绘。
---
--- `vs` 的定位是**同步**，不是"打开一个状态弹框"：
---   1. 刷缓存（spawn svnui）
---   2. 让 yazi 重绘文件列表（行尾标记才会更新）
---   3. 用短通知回报结果
---
--- ⚠️ 第 2 步不能省：linemode 的回调只在 yazi 重绘时被调用，
---    只更新 Lua 侧的 map 而不触发重绘，界面上是看不到变化的 ——
---    表现为"按了没反应"。
---
--- `vR` 是强制刷新（清掉 daemon 版本戳），日常用 `vs` 够了。
local function do_refresh(force)
  local cwd = get_cwd()
  if force then
    state.stamp = 0
  end

  local ok, err = util.refresh(cwd)
  do_render()

  if not ok then
    if not state.in_wc then
      util.notify("svnui", "不在 SVN 工作副本内", "warn")
    else
      util.notify("svnui", err or "刷新失败", "error")
    end
    return
  end

  local n = #state.changed_paths()
  local c = state.conflict_count
  local msg
  if n == 0 then
    msg = "工作副本干净"
  elseif c > 0 then
    msg = string.format("已刷新：%d 项变更（! %d 处冲突，按 vk 处理）", n, c)
  else
    msg = string.format("已刷新：%d 项变更", n)
  end
  util.notify("svnui", msg, c > 0 and "warn" or "info")
end

local ACTIONS = {
  status = function() do_refresh(false) end,

  diff = function()
    local t = get_targets()
    if #t == 0 then util.notify("svnui", "没有目标文件", "warn"); return end
    diff.popup(t[1])
  end,

  commit = commit.run,
  log = function() svnlog.menu() end,

  blame = function()
    local t = get_targets()
    if #t == 0 then return end
    local out, err = util.svnui(get_cwd(), { "blame", "--", t[1] })
    if out then util.popup("SVN blame", out, 120, 30)
    else util.notify("svnui", err or "blame 失败", "error") end
  end,

  add = function()
    util.refresh(get_cwd())
    local t = get_targets()

    -- ⚠️ svn add 对**已在版本控制下**的文件会报 E150002，
    --    而且是"部分成功"：混一个已版本化文件，整批一起失败。
    --    所以先按状态过滤，只留 ?（未版本化）。
    local want, already = {}, {}
    for _, p in ipairs(t) do
      local xy = state.porcelain_for(p)
      if not xy or xy:sub(1, 1) == "?" then
        want[#want + 1] = p
      else
        already[#already + 1] = string.format("%s（状态 %s）", p:match("([^/]+)$") or p, xy)
      end
    end

    if #want == 0 then
      util.notify(
        "svnui",
        #already > 0 and ("已跳过：已在版本控制下 — " .. table.concat(already, "、"))
          or "没有未版本化的目标",
        "warn"
      )
      return
    end

    local args = { "add" }
    for _, p in ipairs(want) do args[#args + 1] = p end
    local out, err = util.svnui(get_cwd(), args)
    if out then
      util.notify("svnui", out .. (#already > 0 and string.format("（跳过 %d 个已版本化）", #already) or ""), "info")
    else
      util.notify("svnui", err or "add 失败", "error")
    end
    util.refresh(get_cwd())
    do_render()
  end,

  update = function()
    local out, err = util.svnui(get_cwd(), { "update", "--yes" })
    if out then util.notify("svnui", out, "info")
    else util.notify("svnui", err or "update 失败", "error") end
    util.refresh(get_cwd())
  end,

  revert = function()
    local t = get_targets()
    if #t == 0 then return end
    local body = { "将回滚（不可撤销）：", "" }
    for i, p in ipairs(t) do
      if i > 30 then body[#body + 1] = string.format("  ... 还有 %d 项", #t - 30); break end
      body[#body + 1] = "  " .. p
    end
    if not util.confirm("SVN Revert", table.concat(body, "\n")) then return end

    local args = { "revert", "--yes" }
    for _, p in ipairs(t) do args[#args + 1] = p end
    local out, err = util.svnui(get_cwd(), args)
    if out then util.notify("svnui", out, "info")
    else util.notify("svnui", err or "revert 失败", "error") end
    util.refresh(get_cwd())
  end,

  conflicts = rescue.conflicts,
  resolve = rescue.conflicts,
  cleanup = rescue.cleanup,
  doctor = rescue.doctor,

  refresh = function() do_refresh(true) end,
  diagnose = diagnose,
  cancel = commit.cancel,
}

function M:entry(job)
  -- async 实例是全新的，模块级 state 是空壳。
  -- 先把 plugin state 拉回来，否则读到的是过期/初始值。
  local snap = pull_state()
  if snap then
    for k, v in pairs(snap) do
      if v ~= nil then state[k] = v end
    end
  end

  -- 没 setup 过：染色和状态栏都不会有。明确告知，别让人以为是 bug。
  -- ⚠️ 从 plugin state 读，不能读模块级 state（async 实例是空的）。
  local setup_done = get_field("setup_done")
  if not setup_done then
    util.notify(
      "svnui",
      "未初始化：init.lua 里加 require(\"svnui\"):setup {} 后重启 yazi（否则无染色/状态栏）",
      "warn"
    )
  end

  local name = job.args and job.args[1]
  if not name then util.notify("svnui", "缺少动作参数", "error"); return end
  local fn = ACTIONS[name]
  if not fn then util.notify("svnui", "未知动作: " .. tostring(name), "error"); return end
  local ok, err = pcall(fn)
  if not ok then util.notify("svnui", "执行出错: " .. tostring(err), "error") end
end

-- ---------------------------------------------------------- 预览

function M:peek(job) diff.peek(job) end
function M:spot(job) diff.spot(job) end

--- 预览面板滚动。previewer 接口要求实现 seek，少了它预览区滚不动，
--- 长 diff 只能看到开头几行。
---
--- ⚠️ 26.x 要点：
---   · `ya.manager_emit()` 已废弃（#2653），用 `ya.emit()`
---   · seek 跑在 **sync 上下文**，可以直接读 `cx.active.preview.skip`
---   · 第一参数是新的 skip 值（位置参数），不是 `skip =` 具名参数
---   · 带上 `only_if` 防止滚动期间文件已切换导致错位
function M:seek(job)
  local cur = cx.active.preview.skip or 0
  local units = job.units or 0
  local step = units > 0 and math.min(units, 1) or math.max(units, -1)
  local next_skip = math.max(0, cur + step)
  ya.emit("peek", { next_skip, only_if = job.file.url })
end

M.PRIORITY = PRIORITY
M.COLOR = COLOR

return M
