#!/usr/bin/env bash
# install-local.sh — 编译 svnui 并装到本地 bin，随后校验是否被旧版本遮蔽
# 用法: ./scripts/install-local.sh [--no-build]
set -uo pipefail

BIN_DIR="/usr/local/bin"
APP="svnui"
NO_BUILD=0

for a in "$@"; do
  case "$a" in
    --no-build) NO_BUILD=1 ;;
    -h|--help)  echo "用法: $0 [--no-build]"; exit 0 ;;
    *)          echo "未知参数: $a"; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT" || exit 1

if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "── 编译 ──"
  cargo build --release || { echo "✗ 编译失败，先解决编译错误"; exit 1; }
fi

SRC="$ROOT/target/release/$APP"
if [[ ! -f "$SRC" ]]; then
  echo "✗ 找不到 $SRC"
  exit 1
fi

echo "── 安装 ──"
if [[ ! -d "$BIN_DIR" ]]; then
  echo "✗ $BIN_DIR 不存在。先执行：sudo mkdir -p $BIN_DIR"
  exit 1
fi

if [[ -w "$BIN_DIR" ]]; then
  cp "$SRC" "$BIN_DIR/$APP"
else
  sudo cp "$SRC" "$BIN_DIR/$APP"
fi
chmod +x "$BIN_DIR/$APP"
hash -r 2>/dev/null || true

echo "  ✓ $BIN_DIR/$APP"
"$BIN_DIR/$APP" --version

echo "── 校验 PATH 遮蔽 ──"
ALL=()
while IFS= read -r line; do
  [[ -n "$line" ]] && ALL+=("$line")
done < <(which -a "$APP" 2>/dev/null || true)

FIRST="${ALL[0]:-}"
if [[ "$FIRST" == "$BIN_DIR/$APP" ]]; then
  echo "  ✓ which -a 第一条就是它"
else
  echo "  ⚠️ 被遮蔽：敲 '$APP' 实际跑到 ${FIRST:-（无）}"
  printf '    全部路径:\n'
  printf '      %s\n' "${ALL[@]}"
  echo "  修法: echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zshrc && exec zsh"
  echo "  或者卸掉冲突的那份: brew uninstall $APP"
fi
