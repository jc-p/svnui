#!/usr/bin/env bash
# 本地打包（半自动发布流程的第 2 步）。
#
#   ./scripts/dist.sh
#
# 做三件事：
#   1. 编两个架构（arm64 + x86_64）
#   2. 打 tar.gz
#   3. 算 sha256，并把填好值的 formula 渲染到 dist/svnui.rb
#
# 不做（故意留给你手动）：
#   - 上传 GitHub Release
#   - push 到 tap 仓库
# 原因是这两步会改动远端、不可逆，脚本里自动跑容易误伤。
#
# 产物在 dist/ 下，下一步照 PUBLISH_MANUAL.md 做。

set -euo pipefail

cd "$(dirname "$0")/.."

VER="$(python3 -c "
import re,sys
for line in open('Cargo.toml',encoding='utf-8'):
    m = re.match(r'^version\s*=\s*\"(.*)\"\s*$', line)
    if m:
        print(m.group(1)); sys.exit(0)
sys.exit('✗ 没在 Cargo.toml 里找到 version')
")"
echo "版本: ${VER}"

# 当前机器架构。Apple Silicon 是 arm64，Intel 是 x86_64。
HOST_ARCH="$(uname -m)"

# 两个 target 都要出：
#   - arm64  : Apple Silicon 机器原生跑
#   - x86_64 : Intel 机器，以及 Apple Silicon 上跑 Rosetta 的场景
# macOS 跨架构编译不需要额外 linker（都用系统自带 SDK），
# 纯 Rust 依赖链（本项目全是）直接加 target 就能编。
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

rm -rf dist
mkdir -p dist

for t in "${TARGETS[@]}"; do
  echo ""
  echo "── 编译 ${t} ──"

  if [ "$HOST_ARCH" = "arm64" ] && [ "$t" = "x86_64-apple-darwin" ]; then
    # Apple Silicon 编 x86_64：需要 Rosetta 才能跑冒烟测试，
    # 但**编译**本身不需要 —— Rust 的 macOS target 是交叉编译，
    # 只要 SDK 路径对就行。SDK 由 xcrun 提供，装了 Xcode CLT 就有。
    echo "（交叉编译，冒烟测试会跳过）"
  fi

  rustup target add "$t" >/dev/null
  cargo build --release --target "$t"

  BIN="target/${t}/release/svnui"

  # 冒烟测试：能跑起来才继续。
  # 跨架构的二进制在本机跑不了（arm64 机器跑不了 x86_64 二进制），
  # 强行跑会得到 "Bad CPU type"，那是正常的，不是编译失败。
  case "$HOST_ARCH-$t" in
    arm64-aarch64-apple-darwin|x86_64-x86_64-apple-darwin)
      echo "  冒烟测试:"
      "$BIN" --version
      ;;
    *)
      echo "  跳过冒烟测试（本机 ${HOST_ARCH} 跑不了 ${t} 的二进制）"
      ;;
  esac

  NAME="svnui-${VER}-${t}"
  # tar 里平铺（不套子目录）：
  # 套目录的话目录名带版本号，formula 里 bin.install 的路径每次都得改。
  # 平铺能保证它永远是 bin.install "svnui"。
  STAGE="$(mktemp -d)"
  cp "$BIN" "$STAGE/svnui"
  tar -czf "dist/${NAME}.tar.gz" -C "$STAGE" svnui
  rm -rf "$STAGE"

  echo "  ✓ dist/${NAME}.tar.gz"
done

echo ""
echo "── 校验和 ──"

# macOS 用 shasum，Linux 用 sha256sum。
# Homebrew 要的是纯 64 位十六进制，cut 只取第一列。
if command -v shasum >/dev/null 2>&1; then
  SHA_ARM="$(shasum -a 256 "dist/svnui-${VER}-aarch64-apple-darwin.tar.gz" | cut -d' ' -f1)"
  SHA_X86="$(shasum -a 256 "dist/svnui-${VER}-x86_64-apple-darwin.tar.gz"        | cut -d' ' -f1)"
else
  SHA_ARM="$(sha256sum "dist/svnui-${VER}-aarch64-apple-darwin.tar.gz" | cut -d' ' -f1)"
  SHA_X86="$(sha256sum "dist/svnui-${VER}-x86_64-apple-darwin.tar.gz"  | cut -d' ' -f1)"
fi

echo "  arm64:  ${SHA_ARM}"
echo "  x86_64: ${SHA_X86}"

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
echo ""
echo "下一步（手动）："
echo "  1. 把 dist/*.tar.gz 上传到 GitHub Release v${VER}"
echo "  2. 把 dist/svnui.rb 覆盖到 homebrew-svnui 仓库的 Formula/"
echo "  3. brew upgrade svnui 验证"
echo ""
echo "详细步骤见 PUBLISH_MANUAL.md"
