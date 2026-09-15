#!/usr/bin/env bash
# 改版本号 + 提交 + 打 tag。
#
#   ./scripts/bump.sh 0.2.0
#
# 打完 tag 后手动 push（脚本不自动 push，留一步给你反悔）：
#   git push origin main && git push origin v0.2.0
#
# push 之后 GitHub Actions 会自动：
#   构建两个架构 → 创建 Release → 更新 homebrew tap

set -euo pipefail

NEW="${1:-}"
if [ -z "$NEW" ]; then
  echo "用法: $0 <新版本号>   例如: $0 0.2.0"
  exit 1
fi

# 允许 "v0.2.0" 和 "0.2.0" 两种写法
NEW="${NEW#v}"

# 只允许 数字.数字.数字（可带 -rc.1 这类后缀）
if ! echo "$NEW" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
  echo "✗ 版本号格式不对: $NEW"
  echo "  需要形如 0.2.0 或 0.2.0-rc.1"
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "✗ 工作区不干净，先提交或 stash："
  git status --short
  exit 1
fi

OLD="$(python3 -c "
import re,sys
for line in open('Cargo.toml',encoding='utf-8'):
    m = re.match(r'^version\s*=\s*\"(.*)\"\s*$', line)
    if m:
        print(m.group(1)); sys.exit(0)
print('')
")"

if [ -z "$OLD" ]; then
  echo "✗ 没在 Cargo.toml 里找到 version"
  exit 1
fi

if [ "$OLD" = "$NEW" ]; then
  echo "✗ 版本号没变（当前就是 $OLD）"
  exit 1
fi

echo "版本: $OLD → $NEW"

# 只替换 [package] 段之后的第一个 version。
# 依赖块里也有很多 version = "..."，所以不能全局替换。
python3 - "$OLD" "$NEW" <<'PY'
import re, sys, pathlib

old, new = sys.argv[1], sys.argv[2]
p = pathlib.Path("Cargo.toml")
lines = p.read_text(encoding="utf-8").split("\n")

in_package = False
done = False
for i, line in enumerate(lines):
    if re.match(r'^\s*\[', line):
        in_package = line.strip() == "[package]"
        continue
    if in_package and not done:
        m = re.match(r'^(\s*version\s*=\s*")(.*)("\s*)$', line)
        if m:
            lines[i] = f"{m.group(1)}{new}{m.group(3)}"
            done = True

if not done:
    sys.exit("✗ 没找到 [package] 段里的 version，未做任何修改")

p.write_text("\n".join(lines), encoding="utf-8")
PY

# 让 Cargo.lock 跟着更新（version 变了不刷新 lock 会有警告）
cargo check --quiet 2>/dev/null || cargo metadata --format-version 1 >/dev/null

git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $NEW"
git tag "$NEW"

echo ""
echo "✓ 已提交并打好 tag $NEW"
echo ""
echo "接下来（这一步会触发发布，确认无误再执行）："
echo "  git push origin main && git push origin $NEW"
echo ""
echo "反悔："
echo "  git tag -d $NEW && git reset --hard HEAD~1"
