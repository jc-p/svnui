#!/usr/bin/env bash
# 编译并更新本机 svnui；安装失败立即退出，避免把旧程序误报为新版本。
set -euo pipefail

APP="svnui"
BIN_DIR="/usr/local/bin"
NO_BUILD=0

usage() {
  echo "用法: $0 [--no-build] [--bin-dir DIR]"
  echo "  默认编译 Release 并安装到 /usr/local/bin，需要时提示输入 sudo 密码。"
  echo "  --no-build     直接安装已有的 Release 二进制"
  echo "  --bin-dir DIR  指定已存在的安装目录"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) NO_BUILD=1; shift ;;
    --bin-dir)
      if [[ $# -lt 2 || -z "$2" || "$2" == --* ]]; then
        echo "✗ --bin-dir 需要目录参数" >&2
        exit 2
      fi
      BIN_DIR="$2"
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "✗ 未知参数: $1" >&2; usage >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# 自定义相对安装目录以调用者所在目录为准，随后构建固定在项目根执行。
if [[ ! -d "$BIN_DIR" ]]; then
  echo "✗ 安装目录不存在: $BIN_DIR" >&2
  exit 1
fi
BIN_DIR="$(cd "$BIN_DIR" && pwd -P)"
DEST="$BIN_DIR/$APP"
cd "$ROOT"

if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "── 编译 Release ──"
  cargo build --release --locked --offline
fi

SRC="${CARGO_TARGET_DIR:-$ROOT/target}/release/$APP"
if [[ ! -x "$SRC" ]]; then
  echo "✗ 找不到可执行的 $SRC，请先去掉 --no-build 重新编译" >&2
  exit 1
fi
# 先验证新程序能启动，再覆盖本机已安装版本。
"$SRC" --version

echo "── 安装到 $DEST ──"
if [[ -w "$BIN_DIR" && ( ! -e "$DEST" || -w "$DEST" ) ]]; then
  install -m 755 "$SRC" "$DEST"
else
  echo "需要管理员权限，请在终端中输入 sudo 密码。"
  sudo install -m 755 "$SRC" "$DEST"
fi

# 修复前后版本号可能相同，必须比较二进制内容，不能只检查 --version。
cmp -s "$SRC" "$DEST" || {
  echo "✗ 安装后的二进制与构建产物不一致" >&2
  exit 1
}
"$DEST" --version
hash -r
FIRST="$(command -v "$APP" || true)"
# 比较文件身份，兼容 macOS /var 与 /private/var 等指向同一位置的路径。
if [[ "$FIRST" -ef "$DEST" ]]; then
  echo "✓ 已更新，PATH 指向 ${DEST}。退出旧界面后重新运行 svnui tui。"
else
  echo "✓ 新二进制已安装到 $DEST"
  echo "⚠ 当前 PATH 实际指向: ${FIRST:-（未找到）}"
  echo "  可直接试用: $DEST tui"
fi
