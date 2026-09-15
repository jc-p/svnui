#!/usr/bin/env bash
# 一条命令发版：改版本号 → 提交 → push → 打包 → 发 Release → 更新 tap。
#
#   ./scripts/release.sh 0.2.0
#   ./scripts/release.sh 0.2.0 --host-only     # 之后的参数原样透传给 dist.sh
#
# 等价于手动三步：
#   ./scripts/bump.sh 0.2.0
#   git push origin main
#   ./scripts/dist.sh
#
# 为什么要先 push main 再跑 dist.sh：
#   gh release create 建的 tag 指向远端默认分支的 HEAD。本地 commit 了但没推，
#   tag 就会指到上一个提交 —— 二进制是对的（本地编的），但 tag 对不上源码，
#   以后翻历史会困惑。
#
# 只走本地发布。想改用 GitHub Actions，就别跑这个脚本，改成：
#   ./scripts/bump.sh 0.2.0
#   git push origin main && git push origin 0.2.0
# 两条路线别同时跑，否则本地和 CI 抢着上传同名 asset，
# tap 里的 sha256 可能对不上最终文件。

set -euo pipefail

cd "$(dirname "$0")/.."

NEW="${1:-}"
if [ -z "$NEW" ]; then
  echo "用法: $0 <新版本号> [dist.sh 参数...]"
  echo ""
  echo "  例如:"
  echo "    $0 0.2.0                # 完整发布"
  echo "    $0 0.2.0 --host-only    # 只编本机架构"
  echo "    $0 0.2.0 --no-publish   # 只打包，不碰远端"
  exit 1
fi
shift

# ① 改版本号 + commit + 打本地 tag
./scripts/bump.sh "$NEW"

# ② 推提交（tag 不需要推，dist.sh 里 gh release create 会在远端建）
BR="$(git rev-parse --abbrev-ref HEAD)"
echo ""
echo "── 推送 ${BR} ──"
git push origin "$BR"
echo "  ✓"

# ③ 打包 + 发布 + 更新 tap
echo ""
./scripts/dist.sh "$@"
