#!/usr/bin/env bash
# 下载官方 macOS 安装包；先校验，再原子替换用户目录中的程序，不使用 sudo。
set -euo pipefail

VERSION="latest"
BIN_DIR="${HOME}/.local/bin"
STAGE=""
PENDING=""
RELEASES="https://github.com/jc-p/svnui/releases"

usage() {
  echo '用法: bash install.sh [--version 0.7.3] [--bin-dir DIR]'
  echo '默认安装最新正式 Release 到 ~/.local/bin，不安装 SVN、不修改 PATH 配置。'
}
fail() { echo "错误: $*" >&2; exit 1; }
cleanup() {
  [ -z "$PENDING" ] || rm -f -- "$PENDING"
  [ -z "$STAGE" ] || rm -rf -- "$STAGE"
}
trap cleanup EXIT

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version|--bin-dir)
      [ "$#" -ge 2 ] && [ -n "$2" ] && [[ "$2" != --* ]] || fail "$1 缺少参数"
      if [ "$1" = --version ]; then VERSION="$2"; else BIN_DIR="$2"; fi
      shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail "未知参数: $1" ;;
  esac
done

[ "$(uname -s)" = Darwin ] || fail '预编译安装包目前仅支持 macOS；其他系统请按 README 从源码构建'
case "$(uname -m)" in
  arm64|aarch64) TARGET=aarch64-apple-darwin ;;
  x86_64) TARGET=x86_64-apple-darwin ;;
  *) fail '不支持此 CPU 架构；请在 Release 页面确认可用安装包' ;;
esac
for cmd in curl tar shasum awk mktemp install; do
  command -v "$cmd" >/dev/null || fail "缺少命令: $cmd"
done

# 只使用 HTTPS 的官方 Release；跟随最新版本跳转，不依赖 gh、jq、Rust 或 API token。
download() {
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
    --fail --silent --show-error --location --retry 2 \
    --connect-timeout 15 --max-time 180 "$@"
}
if [ "$VERSION" = latest ]; then
  LATEST="$(download --output /dev/null --write-out '%{url_effective}' "$RELEASES/latest")" \
    || fail '无法查询最新 Release；检查网络，或使用 --version 0.7.3 指定版本'
  [[ "$LATEST" == "$RELEASES/tag/"* ]] || fail '最新 Release 跳转地址异常'
  VERSION="${LATEST##*/}"
fi
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || fail "版本格式无效: $VERSION"

ASSET="svnui-${VERSION}-${TARGET}.tar.gz"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/svnui-install.XXXXXX")"
echo "下载 svnui ${VERSION} (${TARGET})"
download --output "$STAGE/$ASSET" "$RELEASES/download/$VERSION/$ASSET" \
  || fail '安装包下载失败；已有安装未修改'
download --output "$STAGE/SHA256SUMS" "$RELEASES/download/$VERSION/SHA256SUMS" \
  || fail '校验清单下载失败；已有安装未修改'
EXPECTED="$(awk -v name="$ASSET" '$2 == name { print $1 }' "$STAGE/SHA256SUMS")"
[[ "$EXPECTED" =~ ^[0-9a-f]{64}$ ]] || fail '校验清单缺少此安装包，或存在重复/无效记录'
ACTUAL="$(shasum -a 256 "$STAGE/$ASSET")"
[ "${ACTUAL%% *}" = "$EXPECTED" ] || fail 'SHA256 校验失败；已有安装未修改，请重新下载官方附件'

# 官方安装包只有一个普通可执行文件；拒绝额外路径和链接，避免解压越界。
CONTENTS="$(tar -tzf "$STAGE/$ASSET")" || fail '安装包不是有效的 tar.gz'
[ "$CONTENTS" = svnui ] || fail '安装包文件清单异常'
DETAIL="$(tar -tvzf "$STAGE/$ASSET")" || fail '无法检查安装包类型'
[[ "$DETAIL" == -* ]] || fail '安装包中的 svnui 必须是普通文件'
mkdir "$STAGE/unpack"
tar -xzf "$STAGE/$ASSET" -C "$STAGE/unpack"
[ -f "$STAGE/unpack/svnui" ] && [ ! -L "$STAGE/unpack/svnui" ] || fail '安装包中没有普通 svnui 文件'
chmod 755 "$STAGE/unpack/svnui"
REPORTED="$("$STAGE/unpack/svnui" --version)" || fail '新程序无法运行；检查架构与 macOS 安全提示，已有安装未修改'
[ "$REPORTED" = "svnui $VERSION" ] || fail '新程序版本与 Release 不一致；已有安装未修改'

mkdir -p -- "$BIN_DIR"
BIN_DIR="$(cd "$BIN_DIR" && pwd -P)"
DEST="$BIN_DIR/svnui"
[ ! -d "$DEST" ] || fail "目标是目录，不能替换: $DEST"
# 临时文件与目标同目录，失败不会覆盖旧程序，最终重命名保持原子性。
PENDING="$(mktemp "$BIN_DIR/.svnui-install.XXXXXX")"
install -m 755 "$STAGE/unpack/svnui" "$PENDING"
mv -f -- "$PENDING" "$DEST"
PENDING=""
echo "已安装: ${DEST} (${REPORTED}，SHA256 已校验)"
FIRST="$(command -v svnui || true)"
if [ -n "$FIRST" ] && [ "$FIRST" -ef "$DEST" ]; then
  echo '退出旧界面后重新运行 svnui tui。'
else
  echo '当前 PATH 尚未命中新程序，在当前终端执行：'
  # 这里输出供用户执行的命令，保留 $PATH 字面量，不展开安装进程的环境。
  # shellcheck disable=SC2016
  printf '  export PATH=%q:"$PATH"\n' "$BIN_DIR"
  echo '  hash -r'
  echo '  svnui --version'
  echo '需要永久生效时，将上面的 export 加入 ~/.zshrc 或 ~/.bashrc。'
fi
if ! command -v svn >/dev/null; then
  echo 'SVN 操作还需要 svn 命令：macOS 可执行 brew install subversion。'
fi
