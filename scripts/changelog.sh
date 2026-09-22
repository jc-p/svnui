#!/usr/bin/env bash
# 打印两个版本之间的提交，按 Conventional Commits 的 type 分好组，
# 直接粘进 CHANGELOG.md 对应版本下。
#
# 用法：
#   scripts/changelog.sh                 # 最新 tag .. HEAD
#   scripts/changelog.sh v0.2.4          # v0.2.4 .. HEAD
#   scripts/changelog.sh v0.2.0 v0.2.4   # 指定区间
#   scripts/changelog.sh --tags          # 只列已有 tag，方便核对版本顺序
set -euo pipefail

cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

if [[ "${1:-}" == "--tags" ]]; then
  echo "已有 tag（按创建时间倒序）："
  git tag --sort=-creatordate | sed 's/^/  /' || echo "  （无）"
  echo
  echo "最新 tag : $(git describe --tags --abbrev=0 2>/dev/null || echo '无')"
  echo "Cargo 版本: $(grep -m1 '^version' Cargo.toml 2>/dev/null || echo '?')"
  exit 0
fi

FROM="${1:-}"
TO="${2:-HEAD}"

if [[ -z "$FROM" ]]; then
  FROM="$(git describe --tags --abbrev=0 2>/dev/null || true)"
  if [[ -z "$FROM" ]]; then
    echo "没有找到任何 tag，改为列出全部提交。" >&2
    FROM="$(git rev-list --max-parents=0 HEAD 2>/dev/null || true)"
  fi
fi

RANGE="$FROM..$TO"
if [[ -z "$FROM" ]]; then
  RANGE="$TO"
fi

echo "范围: $RANGE"
echo "提交数: $(git rev-list --count "$RANGE" 2>/dev/null || echo 0)"
echo

# type -> CHANGELOG 小节标题
BUCKETS=(
  "feat:Added"
  "fix:Fixed"
  "perf:Performance"
  "refactor:Changed"
  "docs:Documentation"
  "test:Tests"
  "build:Build"
  "ci:CI"
  "chore:Chore"
)

tmp="$(mktemp)"
git log --no-merges --pretty=format:'- %s (%h)' "$RANGE" > "$tmp" || true

for pair in "${BUCKETS[@]}"; do
  ctype="${pair%%:*}"
  title="${pair#*:}"
  # 匹配 "feat:" / "feat(scope):"
  items="$(grep -E "^- ${ctype}(\(|!:|:)" "$tmp" || true)"
  # 注意：不能写成 `[[ -z "$items" ]] && continue`
  # set -e 下条件为假时整条语句返回 1，脚本会在这里静默退出。
  if [[ -z "$items" ]]; then
    continue
  fi
  echo "### $title"
  echo "$items"
  echo
done

# 不符合规范的老提交（比如"初始化"），单列出来提醒整理
others="$(grep -vE "^- (feat|fix|perf|refactor|docs|test|build|ci|chore)(\(|!:|:)" "$tmp" || true)"
if [[ -n "$others" ]]; then
  echo "### 待整理（不符合 Conventional Commits 的提交）"
  echo "$others"
  echo
fi

rm -f "$tmp"
