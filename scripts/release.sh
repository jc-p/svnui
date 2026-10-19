#!/usr/bin/env bash
# ./scripts/release.sh <版本> [dist.sh 参数]
# --no-publish 只打包当前版本，必须在任何版本/Git 写操作前分流。
set -euo pipefail
cd "$(dirname "$0")/.."
NEW="${1:-}"
[ -n "${NEW}" ] || { echo "用法: $0 <版本> [--no-publish] [--offline] [--tap-dir <dir>]" >&2; exit 1; }
shift
NEW="${NEW#v}"
NO_PUBLISH=0 TAP_ONLY=0
for arg in "$@"; do
  case "$arg" in --no-publish) NO_PUBLISH=1 ;; --tap-only) TAP_ONLY=1 ;; esac
done
CURRENT="$(python3 - <<'PY'
import pathlib, re
package = re.search(r'(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)', pathlib.Path('Cargo.toml').read_text())
print(re.search(r'(?m)^version\s*=\s*"([^"]+)"', package[1])[1])
PY
)"
if [ "${NO_PUBLISH}" = 1 ] || [ "${TAP_ONLY}" = 1 ]; then
  [ "${NEW}" = "${CURRENT}" ] || { echo "纯打包/恢复只接受当前版本 ${CURRENT}，不修改版本或 Git" >&2; exit 1; }
  exec ./scripts/dist.sh "$@"
fi
# 提前验证参数，避免拼错选项后才发现已经产生版本提交。
args=("$@")
while [ "$#" -gt 0 ]; do
  case "$1" in
    --offline|--no-push|--brew-update) shift ;;
    --tap-dir) [ "$#" -ge 2 ] && [ -n "$2" ] || exit 1; shift 2 ;;
    *) echo "完整发布不支持参数: $1" >&2; exit 1 ;;
  esac
done
if [ "${NEW}" != "${CURRENT}" ]; then
  ./scripts/bump.sh "${NEW}"
fi
COMMIT="$(git rev-parse HEAD)"
echo "发布提交: ${COMMIT}；后续失败保留提交和 tag，不自动回退"
git push origin HEAD
# macOS Bash 3 在 nounset 下展开空数组会报错，空参数单独处理。
if [ "${#args[@]}" -gt 0 ]; then
  exec ./scripts/dist.sh "${args[@]}"
fi
exec ./scripts/dist.sh
