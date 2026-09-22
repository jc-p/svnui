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
