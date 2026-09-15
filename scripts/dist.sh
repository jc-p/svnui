#!/usr/bin/env bash
# 本地打包（半自动发布流程的第 2 步）。
#
#   ./scripts/dist.sh              # 编两个架构
#   ./scripts/dist.sh --host-only  # 只编本机架构（交叉编译出问题时用它先出包）
#
# 做三件事：
#   1. 编 arm64 + x86_64
#   2. 打 tar.gz
#   3. 算 sha256，并把填好值的 formula 渲染到 dist/svnui.rb
#
# 不做（故意留给你手动）：
#   - 上传 GitHub Release
#   - push 到 tap 仓库
# 原因是这两步会改动远端、不可逆，脚本里自动跑容易误伤。

set -euo pipefail

cd "$(dirname "$0")/.."

HOST_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --host-only) HOST_ONLY=1 ;;
    -h|--help)
      echo "用法: $0 [--host-only]"
      echo "  --host-only  只编本机架构（交叉编译有麻烦时用它先把包打出来）"
      exit 0 ;;
    *) echo "✗ 未知参数: $arg"; exit 1 ;;
  esac
done

# ── 0. 先查清楚用的是哪套 Rust ────────────────────────────────
#
# 这一步是必须的，因为 macOS 上极易出现「rustup 装了 target，
# 但 cargo 是另一套（Homebrew 的 rust）」的情况：
#   rustup target add x86_64-apple-darwin   → 装进 ~/.rustup 的工具链
#   cargo build --target x86_64-apple-darwin → 用的是 /opt/homebrew/bin/cargo
# 两套互不相干，于是 rustup 说"装好了"，cargo 说"没装"，报错信息还是
# "the target may not be installed" 这种误导性文案，很难自查。

echo "── 检查 Rust 环境 ──"

CARGO_BIN="$(command -v cargo || true)"
if [ -z "$CARGO_BIN" ]; then
  echo "✗ 没找到 cargo"
  exit 1
fi
echo "  cargo:   ${CARGO_BIN}"

# 判断 cargo 是否是 rustup 的 shim。
# rustup 的 shim 一定在 ~/.cargo/bin 下（或 $CARGO_HOME/bin）。
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
  echo "  rustup 装的 target 只对 rustup 的 cargo 有效，"
  echo "  你现在这套 cargo 看不到它 —— 这就是报错的原因。"
  echo ""
  echo "  二选一："
  echo "    A. 让 ~/.cargo/bin 排在 PATH 最前（推荐）："
  echo "         echo 'export PATH=\"\$HOME/.cargo/bin:\$PATH\"' >> ~/.zshrc"
  echo "         exec zsh"
  echo "    B. 只用本机架构，跳过交叉编译："
  echo "         $0 --host-only"
  exit 1
fi

# ── 1. 版本号 ────────────────────────────────────────────────
VER="$(python3 -c "
import re,sys
for line in open('Cargo.toml',encoding='utf-8'):
    m = re.match(r'^version\s*=\s*\"(.*)\"\s*$', line)
    if m:
        print(m.group(1)); sys.exit(0)
sys.exit('✗ 没在 Cargo.toml 里找到 version')
")"
echo "  版本:    ${VER}"

# ── 2. 决定要编哪些 target ───────────────────────────────────
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

# ── 3. 编译 ──────────────────────────────────────────────────
rm -rf dist
mkdir -p dist

for t in "${TARGETS[@]}"; do
  echo "── 编译 ${t} ──"

  # 装 target，并且显式指定工具链。
  # 不指定 --toolchain 时，rustup 用当前目录的 override / 默认工具链；
  # 显式指定能保证"装到哪"和"用哪个编"是同一套。
  if [ -n "${ACTIVE:-}" ]; then
    rustup target add --toolchain "$ACTIVE" "$t" >/dev/null
  else
    rustup target add "$t" >/dev/null
  fi

  # 装完立刻验证。
  # 不验的话，万一装失败（网络问题、该工具链没这个 target），
  # 后面 cargo 会报 "target may not be installed" 这种误导性错误，
  # 看起来像脚本写错了，实际是没装上。
  if ! rustup target list --installed --toolchain "$ACTIVE" 2>/dev/null | grep -qx "$t"; then
    echo "  ✗ target 装不上：${t}"
    echo ""
    echo "  当前工具链 ${ACTIVE} 已装的 target："
    rustup target list --installed --toolchain "$ACTIVE" | sed 's/^/    /'
    echo ""
    echo "  可以先只编本机架构把包打出来："
    echo "    $0 --host-only"
    exit 1
  fi

  cargo build --release --target "$t"

  BIN="target/${t}/release/svnui"

  # 冒烟测试只在"本机架构"下做。
  # arm64 机器跑不了 x86_64 二进制（Bad CPU type），反之亦然 ——
  # 那是正常的，不是编译失败。
  if [ "$t" = "$HOST_TARGET" ]; then
    echo "  冒烟测试:"
    "$BIN" --version | sed 's/^/    /'
  else
    echo "  跳过冒烟测试（本机 ${HOST_ARCH} 跑不了 ${t} 的二进制）"
  fi

  NAME="svnui-${VER}-${t}"
  # tar 里平铺（不套子目录）：
  # 套目录的话目录名带版本号，formula 里 bin.install 的路径每次都得改。
  # 平铺能保证它永远是 bin.install "svnui"。
  STAGE="$(mktemp -d)"
  cp "$BIN" "$STAGE/svnui"
  tar -czf "dist/${NAME}.tar.gz" -C "$STAGE" svnui
  rm -rf "$STAGE"

  echo "  ✓ dist/${NAME}.tar.gz"
  echo ""
done

# ── 4. 校验和 ────────────────────────────────────────────────
echo "── 校验和 ──"

# macOS 用 shasum，Linux 用 sha256sum。
# Homebrew 要的是纯 64 位十六进制，cut 只取第一列。
sha_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    sha256sum "$1" | cut -d' ' -f1
  fi
}

# 只有一个架构时，另一个的 sha 填占位值。
# formula 里两个 url 都在，缺一个会 404；用占位 sha 至少保证
# arm64 那条路径能正常安装，Intel 用户再单独补。
PLACEHOLDER="0000000000000000000000000000000000000000000000000000000000000000"
SHA_ARM="$PLACEHOLDER"
SHA_X86="$PLACEHOLDER"

if [ -f "dist/svnui-${VER}-aarch64-apple-darwin.tar.gz" ]; then
  SHA_ARM="$(sha_of "dist/svnui-${VER}-aarch64-apple-darwin.tar.gz")"
  echo "  arm64:  ${SHA_ARM}"
fi
if [ -f "dist/svnui-${VER}-x86_64-apple-darwin.tar.gz" ]; then
  SHA_X86="$(sha_of "dist/svnui-${VER}-x86_64-apple-darwin.tar.gz")"
  echo "  x86_64: ${SHA_X86}"
fi

# 从 tpl 渲染成品 formula。
# 用 Python 而不是 sed：sed 匹配不上会静默跳过，
# 推出去的 formula 就是错的，用户装的时候才炸。
OWNER="${TAP_OWNER:-jc-p}" \
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

# 兜底：还有 @@ 残留说明某个 token 没替换上
if "@@" in out:
    sys.exit(f"✗ 模板仍有未替换的占位符:\n{out}")

pathlib.Path("dist/svnui.rb").write_text(out, encoding="utf-8")
print("  ✓ dist/svnui.rb")
PY

echo ""
echo "════════════════════════════════════════"
echo "✓ 打包完成，产物在 dist/："
ls -la dist/

if [ "$HOST_ONLY" = "1" ]; then
  echo ""
  echo "⚠️ 只编了 ${HOST_TARGET}，另一侧架构的 sha256 是占位值。"
  echo "   Intel 机器装会失败。自用/团队都是 Apple Silicon 的话可以先这样，"
  echo "   之后再补编另一个架构（不加 --host-only 重跑即可）。"
fi

echo ""
echo "下一步（手动）："
echo "  1. 把 dist/*.tar.gz 上传到 GitHub Release v${VER}"
echo "  2. 把 dist/svnui.rb 覆盖到 homebrew-svnui 仓库的 Formula/"
echo "  3. brew upgrade svnui 验证"
echo ""
echo "详细步骤见 PUBLISH_MANUAL.md"
