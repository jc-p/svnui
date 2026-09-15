#!/usr/bin/env bash
# 打包 + 发布（一条命令走完）。
#
#   ./scripts/dist.sh                  # 全流程：编译 → 打包 → 更新 formula → 发 Release → push tap
#   ./scripts/dist.sh --host-only      # 只编本机架构（交叉编译出问题时先用它）
#   ./scripts/dist.sh --no-publish     # 只打包，不碰任何远端
#   ./scripts/dist.sh --no-push        # 发 Release，但 tap 仓库只 commit 不 push
#   ./scripts/dist.sh --tap-dir <dir>  # 手动指定 homebrew-svnui 仓库路径
#
# 环境变量：
#   TAP_OWNER   覆盖 owner（默认从 git remote 解析）
#   TAP_REPO    覆盖 owner/repo（默认从 git remote 解析）
#
# 关于 tag：统一不带 v 前缀。
#   Release 的 tag 名、formula 里的 download 路径、bump.sh 打的 tag 必须一致，
#   差一个 v 就是 404 —— 这个坑已经踩过三次，别再拆开维护。

set -euo pipefail

cd "$(dirname "$0")/.."

HOST_ONLY=0
NO_PUBLISH=0
NO_PUSH=0
TAP_DIR_ARG=""

while [ $# -gt 0 ]; do
  case "$1" in
    --host-only)  HOST_ONLY=1; shift ;;
    --no-publish) NO_PUBLISH=1; shift ;;
    --no-push)    NO_PUSH=1; shift ;;
    --tap-dir)    TAP_DIR_ARG="${2:-}"; shift 2 ;;
    -h|--help)
      sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "✗ 未知参数: $1"; exit 1 ;;
  esac
done

# ── 0. 先查清楚用的是哪套 Rust ────────────────────────────────
#
# 必须的，因为 macOS 上极易出现「rustup 装了 target，但 cargo 是另一套
# （Homebrew 的 rust）」：
#   rustup target add x86_64-apple-darwin    → 装进 rustup 那套工具链
#   cargo build --target x86_64-apple-darwin → 用的是 /opt/homebrew/bin/cargo
# 两套互不相干，于是 rustup 说装好了、cargo 说没装，报的错还是
# 「target may not be installed」这种误导性文案，极难自查。

echo "── 检查 Rust 环境 ──"

CARGO_BIN="$(command -v cargo || true)"
if [ -z "$CARGO_BIN" ]; then
  echo "✗ 没找到 cargo"
  exit 1
fi
echo "  cargo:   ${CARGO_BIN}"

CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
IS_RUSTUP=0
case "$CARGO_BIN" in
  "$CARGO_HOME_DIR/bin/"*) IS_RUSTUP=1 ;;
esac

if [ "$IS_RUSTUP" = "1" ]; then
  echo "  来源:    rustup（shim）"
  ACTIVE="$(rustup show active-toolchain 2>/dev/null | head -1 | awk '{print $1}')"
  echo "  工具链:  ${ACTIVE}"
else
  echo "  来源:    非 rustup ⚠️"
  echo ""
  echo "  ✗ cargo 不是 rustup 的：${CARGO_BIN}"
  echo ""
  echo "  rustup 装的 target 只对 rustup 的 cargo 有效，你现在这套看不到。"
  echo ""
  echo "    A. 让 rustup 排到 PATH 最前（推荐）："
  echo "         echo 'export PATH=\"\$HOME/.cargo/bin:\$PATH\"' >> ~/.zshrc"
  echo "         exec zsh"
  echo "    B. 只编本机架构："
  echo "         $0 --host-only"
  exit 1
fi

# ── 1. 版本号 ────────────────────────────────────────────────
VER="$(python3 -c "
import re,sys
for line in open('Cargo.toml',encoding='utf-8'):
    m = re.match(r'^version\s*=\s*\"(.*)\"\s*\$', line)
    if m:
        print(m.group(1)); sys.exit(0)
sys.exit('✗ 没在 Cargo.toml 里找到 version')
")"
echo "  版本:    ${VER}（tag 不带 v）"

# ── 2. 仓库 / owner ──────────────────────────────────────────
REPO="${TAP_REPO:-}"
if [ -z "$REPO" ]; then
  REPO="$(git remote get-url origin 2>/dev/null || true)"
  REPO="$(printf '%s' "$REPO" \
    | sed -E 's#^https://github\.com/##; s#^git@github\.com:##; s#\.git$##')"
fi
if [ -z "$REPO" ] || [ "$REPO" = "origin" ]; then
  echo "✗ 解析不出 owner/repo，用 TAP_REPO=owner/repo 指定"
  exit 1
fi
OWNER="${TAP_OWNER:-${REPO%%/*}}"
echo "  仓库:    ${REPO}"
echo ""

# ── 3. 决定编哪些 target ─────────────────────────────────────
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  arm64)  HOST_TARGET=aarch64-apple-darwin ;;
  x86_64) HOST_TARGET=x86_64-apple-darwin ;;
  *) echo "✗ 未知架构: $HOST_ARCH"; exit 1 ;;
esac

if [ "$HOST_ONLY" = "1" ]; then
  TARGETS=("$HOST_TARGET")
  echo "  模式:    只编本机架构（--host-only）"
else
  TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
  echo "  模式:    双架构"
fi
echo ""

# ── 4. 编译 + 打包 ───────────────────────────────────────────
rm -rf dist
mkdir -p dist

for t in "${TARGETS[@]}"; do
  echo "── 编译 ${t} ──"

  if [ -n "${ACTIVE:-}" ]; then
    rustup target add --toolchain "$ACTIVE" "$t" >/dev/null
  else
    rustup target add "$t" >/dev/null
  fi

  # 装完立刻验证：不验的话失败时会报「target may not be installed」，
  # 看起来像脚本写错，实际是没装上。
  if ! rustup target list --installed --toolchain "$ACTIVE" 2>/dev/null | grep -qx "$t"; then
    echo "  ✗ target 装不上：${t}"
    echo "  已装的 target："
    rustup target list --installed --toolchain "$ACTIVE" | sed 's/^/    /'
    echo "  可以先只编本机架构：$0 --host-only"
    exit 1
  fi

  cargo build --release --target "$t"

  BIN="target/${t}/release/svnui"

  # 冒烟测试只在本机架构下做：arm64 跑不了 x86_64 二进制（Bad CPU type）
  if [ "$t" = "$HOST_TARGET" ]; then
    echo "  冒烟测试:"
    "$BIN" --version | sed 's/^/    /'
  else
    echo "  跳过冒烟测试（本机 ${HOST_ARCH} 跑不了 ${t} 的二进制）"
  fi

  NAME="svnui-${VER}-${t}"
  # tar 里平铺，不套目录：套目录的话 bin.install 的路径每次都得跟着改。
  STAGE="$(mktemp -d)"
  cp "$BIN" "$STAGE/svnui"
  tar -czf "dist/${NAME}.tar.gz" -C "$STAGE" svnui
  rm -rf "$STAGE"

  echo "  ✓ dist/${NAME}.tar.gz"
  echo ""
done

# ── 5. 校验和 ────────────────────────────────────────────────
echo "── 校验和 ──"

sha_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    sha256sum "$1" | cut -d' ' -f1
  fi
}

PLACEHOLDER="0000000000000000000000000000000000000000000000000000000000000000"
SHA_ARM="$PLACEHOLDER"
SHA_X86="$PLACEHOLDER"
HAVE_ARM=0
HAVE_X86=0

if [ -f "dist/svnui-${VER}-aarch64-apple-darwin.tar.gz" ]; then
  SHA_ARM="$(sha_of "dist/svnui-${VER}-aarch64-apple-darwin.tar.gz")"
  HAVE_ARM=1
  echo "  arm64:  ${SHA_ARM}"
fi
if [ -f "dist/svnui-${VER}-x86_64-apple-darwin.tar.gz" ]; then
  SHA_X86="$(sha_of "dist/svnui-${VER}-x86_64-apple-darwin.tar.gz")"
  HAVE_X86=1
  echo "  x86_64: ${SHA_X86}"
fi

# ── 6. 渲染一份完整 formula 到 dist/ ─────────────────────────
#
# 从 tpl 整体渲染（不是逐行替换），用于「全新生成」。
# 第 7 步的就地更新才用于日常维护已经手工调过的 Formula/svnui.rb。
OWNER="$OWNER" \
VERSION="$VER" \
SHA_ARM="$SHA_ARM" \
SHA_X86="$SHA_X86" \
python3 - <<'PY'
import os, pathlib, sys

tpl = pathlib.Path("packaging/homebrew/svnui.rb.tpl").read_text(encoding="utf-8")
out = (tpl
    .replace("@@OWNER@@",   os.environ["OWNER"])
    .replace("@@VERSION@@", os.environ["VERSION"])
    .replace("@@SHA_ARM@@", os.environ["SHA_ARM"])
    .replace("@@SHA_X86@@", os.environ["SHA_X86"]))

# 还有 @@ 残留说明某个 token 没替换上，静默生成错文件代价太大
if "@@" in out:
    sys.exit(f"✗ 模板仍有未替换的占位符:\n{out}")

pathlib.Path("dist/svnui.rb").write_text(out, encoding="utf-8")
print("  ✓ dist/svnui.rb（全新渲染，供对照）")
PY

# ── 7. 就地更新 tap 仓库的 Formula/svnui.rb ──────────────────
#
# 只改三样：url 里的版本号、两个 sha256、test 里的版本断言。
# 其余结构（注释、depends_on、on_macos 块的写法）原样保留 ——
# 你手工调过的 formula 不会被覆盖回模板的样子。

echo ""
echo "── 更新 Formula ──"

# 所有可能的本地 tap 克隆位置。
# 注意可能不止一份：你自己 clone 的一份在工作目录（方便编辑 + push），
# brew 自己 tap 出来的那份在 /opt/homebrew/Library/Taps/（brew 实际读它）。
# 只更新其中一份，另一份就会停在旧版本 —— brew upgrade 拿到的还是旧 formula。
tap_candidates() {
  if [ -n "$TAP_DIR_ARG" ]; then
    printf '%s\n' "$TAP_DIR_ARG"
    return
  fi
  local d seen=""
  for d in \
    "$HOME/Library/Taps/${OWNER}/homebrew-svnui" \
    "/opt/homebrew/Library/Taps/${OWNER}/homebrew-svnui" \
    "/usr/local/Homebrew/Library/Taps/${OWNER}/homebrew-svnui" \
    "$(brew --repository 2>/dev/null || true)/Library/Taps/${OWNER}/homebrew-svnui" \
    "Formula"
  do
    [ -n "$d" ] || continue
    [ -f "$d/Formula/svnui.rb" ] || continue
    # 去重：/opt/homebrew/... 会被字面量和 brew --repository 各匹配一次，
    # 不去掉就会同步两遍（无害但输出重复，看着像出错）
    case "$seen" in
      *"|$d|"*) continue ;;
    esac
    seen="${seen}|$d|"
    # 归一成绝对路径，否则 brew 那份可能以相对路径出现，跟 TAP_DIR 比不上
    printf '%s\n' "$(cd "$d" && pwd)"
  done

  # 兜底：owner 解析不到（SSH 别名、自建 git 服务器、remote 改过名）时，
  # 上面拼出的路径会变成 .../Taps//homebrew-svnui 而全部落空。
  # 这时直接在常见前缀下 glob 找 homebrew-svnui，避免静默跳过。
  for base in "$HOME/Library/Taps" "/opt/homebrew/Library/Taps" \
              "/usr/local/Homebrew/Library/Taps" \
              "$(brew --repository 2>/dev/null || true)/Library/Taps"
  do
    [ -n "$base" ] && [ -d "$base" ] || continue
    for d in "$base"/*/homebrew-svnui "$base"/homebrew-svnui; do
      [ -f "$d/Formula/svnui.rb" ] || continue
      d="$(cd "$d" && pwd)"
      case "$seen" in
        *"|$d|"*) continue ;;
      esac
      seen="${seen}|$d|"
      printf '%s\n' "$d"
    done
  done
  return 0   # 没匹配到是正常情况，不算失败
}

find_tap() { tap_candidates | head -1; }

TAP_DIR="$(find_tap || true)"

if [ -z "$TAP_DIR" ] || [ ! -f "$TAP_DIR/Formula/svnui.rb" ]; then
  echo "  ⚠️ 没找到 tap 仓库的 Formula/svnui.rb，跳过就地更新"
  echo "     用 --tap-dir <路径> 指定，或先 brew tap ${OWNER}/svnui"
  TAP_DIR=""
else
  echo "  tap:     ${TAP_DIR}"
  TARGET_FORMULA="$TAP_DIR/Formula/svnui.rb" \
  VERSION="$VER" \
  SHA_ARM="$SHA_ARM" \
  SHA_X86="$SHA_X86" \
  HAVE_ARM="$HAVE_ARM" \
  HAVE_X86="$HAVE_X86" \
  python3 - <<'PY'
import os, re, sys, pathlib

p    = pathlib.Path(os.environ["TARGET_FORMULA"])
ver  = os.environ["VERSION"]
shas = {"aarch64": os.environ["SHA_ARM"], "x86_64": os.environ["SHA_X86"]}
have = {"aarch64": os.environ["HAVE_ARM"] == "1", "x86_64": os.environ["HAVE_X86"] == "1"}

s = p.read_text(encoding="utf-8")

# 1) 版本号：只动 url 路径 / 文件名 / test 断言，不做全局字符串替换
for pat, rep in [
    (r'(releases/download/)v?[0-9][0-9A-Za-z.\-]*/', r'\g<1>' + ver + '/'),
    (r'(svnui-)[0-9][0-9A-Za-z.\-]*?(-(?:aarch64|x86_64)-apple-darwin)', r'\g<1>' + ver + r'\g<2>'),
    (r'(assert_match ")[0-9][0-9A-Za-z.\-]*(")', r'\g<1>' + ver + r'\g<2>'),
]:
    s, n = re.subn(pat, rep, s)

# 2) sha256：定位「含架构 token 的 url」之后紧跟的那个 sha256。
#    允许中间夹注释或空行，但不允许跨到下一个 url。
for token, sha in shas.items():
    pat = re.compile(
        r'(url\s+"[^"]*' + token + r'[^"]*"(?:(?!\burl\b)[\s\S])*?sha256\s+")[^"]*(")')
    s, n = pat.subn(lambda m, v=sha: m.group(1) + v + m.group(2), s, count=1)
    if n == 0:
        if have[token]:
            sys.exit(f"✗ formula 里没找到 {token} 的 url/sha256 配对，未做修改")
        print(f"  ⚠️ formula 里没有 {token} 的条目，跳过（本次也没编它）")
    else:
        print(f"  ✓ {token} sha256 已更新")

p.write_text(s, encoding="utf-8")
PY

  if grep -q '@@' "$TAP_DIR/Formula/svnui.rb"; then
    echo "✗ formula 里还有未替换的占位符，已停止"
    exit 1
  fi
fi

# ── 8. 发布 Release ──────────────────────────────────────────
echo ""
echo "── 发布到 GitHub ──"

if [ "$NO_PUBLISH" = "1" ]; then
  echo "  --no-publish：跳过"
else
  if ! command -v gh >/dev/null 2>&1; then
    echo "✗ 没装 gh（brew install gh && gh auth login）"
    exit 1
  fi
  if ! gh auth status >/dev/null 2>&1; then
    echo "✗ gh 未登录：gh auth login"
    exit 1
  fi

  # 已存在就补传（immutable release 不能重建，只能覆盖同名 asset）；
  # 不存在才 create。
  if gh release view "$VER" --repo "$REPO" >/dev/null 2>&1; then
    echo "  Release ${VER} 已存在，覆盖上传 assets"
    gh release upload "$VER" dist/svnui-"${VER}"-*.tar.gz --repo "$REPO" --clobber
  else
    echo "  创建 Release ${VER}"
    if ! gh release create "$VER" dist/svnui-"${VER}"-*.tar.gz \
         --repo "$REPO" --generate-notes; then
      echo ""
      echo "  ✗ 创建失败。常见原因："
      echo "    1. 仓库 Rulesets 禁止创建 tag"
      echo "       → https://github.com/${REPO}/settings/rules"
      echo "         删掉那条规则，或把自己加进 Bypass list"
      echo "         （Rulesets 不自动豁免 owner，必须显式加）"
      echo "    2. 版本号被 immutable release 占住（删了 release 也不释放）"
      echo "       → 升版本号重来：./scripts/bump.sh <新版本>"
      exit 1
    fi
  fi
  echo "  ✓ https://github.com/${REPO}/releases/tag/${VER}"
fi

# ── 9. 提交 tap 仓库 ─────────────────────────────────────────
echo ""
echo "── 提交 tap ──"

if [ -z "$TAP_DIR" ]; then
  echo "  无 tap 目录，跳过"
elif [ "$NO_PUBLISH" = "1" ]; then
  echo "  --no-publish：跳过"
else
  (
    cd "$TAP_DIR"
    if ! git rev-parse --git-dir >/dev/null 2>&1; then
      echo "  ⚠️ ${TAP_DIR} 不是 git 仓库，跳过提交"
      exit 0
    fi
    if git diff --quiet -- Formula/svnui.rb && git diff --cached --quiet -- Formula/svnui.rb; then
      echo "  formula 无变化，跳过提交"
      exit 0
    fi
    git add Formula/svnui.rb
    git commit -m "svnui ${VER}"
    echo "  ✓ 已 commit"
    if [ "$NO_PUSH" = "1" ]; then
      echo "  --no-push：留给你手动推"
      echo "    cd ${TAP_DIR} && git push"
    else
      # 推当前分支（main 或 master），不写死
      BR="$(git rev-parse --abbrev-ref HEAD)"
      git push origin "$BR"
      echo "  ✓ 已 push 到 origin/${BR}"
    fi
  )
fi

# ── 9.5 同步其余本地 tap 克隆 ────────────────────────────────
#
# 除了上面提交 push 的那份，本机可能还有 brew 自己 tap 出来的那份：
#   /opt/homebrew/Library/Taps/<owner>/homebrew-svnui/Formula/svnui.rb
# brew 读的是那一份。不拉最新的话，刚发完版 brew upgrade 拿到的还是
# 旧 formula（旧 url + 旧 sha256），表现为 404 或 checksum mismatch。
# 这些克隆是同一个仓库的另一份副本，所以 ff-only 拉取即可，不产生提交。

echo ""
echo "── 同步本地 tap 克隆 ──"

SYNCED=0
OTHERS=0
while IFS= read -r d; do
  [ -n "$d" ] || continue
  [ "$d" = "$TAP_DIR" ] && continue          # 主目录刚 push 过，跳过
  [ -d "$d/.git" ] || continue
  # 拿不到 remote 就别动它，可能是无关目录
  git -C "$d" remote get-url origin >/dev/null 2>&1 || continue
  OTHERS=$((OTHERS+1))

  printf '  %s\n' "$d"
  if [ -n "$(git -C "$d" status --porcelain -- Formula/svnui.rb 2>/dev/null)" ]; then
    echo "    ⚠️ 本地有未提交改动，跳过（先处理：cd $d && git diff）"
    continue
  fi
  # --ff-only：只允许快进。有分叉就报出来让你看，绝不悄悄覆盖本地改动
  if git -C "$d" pull --ff-only --quiet 2>/dev/null; then
    echo "    ✓ 已拉取到最新"
    SYNCED=$((SYNCED+1))
  else
    echo "    ⚠️ 快进失败（本地有落后/分叉的提交），手动处理："
    echo "       cd $d && git status"
  fi
done < <(tap_candidates)

if [ "$OTHERS" = "0" ] && [ -n "$TAP_DIR" ]; then
  echo "  只有一份 tap 克隆（${TAP_DIR}），无需额外同步"
elif [ "$SYNCED" != "$OTHERS" ]; then
  echo "  ⚠️ 有 ${OTHERS} 份其他克隆，只同步了 ${SYNCED} 份 —— 看上面的警告"
fi

if [ "$NO_PUBLISH" = "0" ] && [ "$NO_PUSH" = "0" ]; then
  # brew 自己也会缓存 formula 解析结果，这一步让 brew 立刻看到新版本
  if command -v brew >/dev/null 2>&1; then
    brew update --force >/dev/null 2>&1 \
      && echo "  ✓ brew update 已刷新" \
      || echo "  ⚠️ brew update 失败，手动跑一次：brew update --force"
  fi
fi

# ── 10. 收尾 ─────────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════"
echo "✓ svnui ${VER}"
ls -la dist/

if [ "$HOST_ONLY" = "1" ]; then
  echo ""
  echo "⚠️ 只编了 ${HOST_TARGET}，另一侧的 sha256 是占位值，那边装会失败。"
  echo "   自用/团队都是 Apple Silicon 可以先这样，之后不加 --host-only 重跑补齐。"
fi

if [ "$NO_PUBLISH" = "1" ]; then
  echo ""
  echo "下一步："
  echo "  gh release create ${VER} dist/svnui-${VER}-*.tar.gz --repo ${REPO} --generate-notes"
  echo "  cp dist/svnui.rb <tap>/Formula/ && 提交推送"
else
  echo ""
  echo "验证："
  echo "  brew upgrade svnui"
  echo "  svnui --version          # ${VER}"
  echo "  brew test svnui"
  echo ""
  echo "给别人装（新版 brew 必须 trust，否则静默忽略整个 tap）："
  echo "  brew tap ${OWNER}/svnui && brew trust ${OWNER}/svnui && brew install svnui"
fi
