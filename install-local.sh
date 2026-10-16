#!/usr/bin/env bash
# 根目录入口：转交统一安装脚本，从任意工作目录调用都能定位到本项目。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$ROOT/scripts/install-local.sh" "$@"
