#!/usr/bin/env bash
# 修改 package/lock 版本并提交、创建本地 tag；不联网、不构建、不推送。
set -euo pipefail
cd "$(dirname "$0")/.."
NEW="${1:-}"; NEW="${NEW#v}"
[[ "${NEW}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo '需要版本号，例如 0.4.2' >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo '工作区不干净，请先完成当前改动' >&2; exit 1; }
if git show-ref --verify --quiet "refs/tags/${NEW}"; then
  echo "tag ${NEW} 已存在，未修改任何文件" >&2; exit 1
fi
BACKUP="$(git rev-parse HEAD)"
python3 - "${NEW}" <<'PY'
import pathlib, re, sys
version = sys.argv[1]
toml = pathlib.Path('Cargo.toml')
lock = pathlib.Path('Cargo.lock')
text, locked = toml.read_text(), lock.read_text()
package = re.search(r'(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)', text)
if not package:
    raise SystemExit('未找到 [package]')
old = re.search(r'(?m)^version\s*=\s*"([^"]+)"', package[1])
if not old or old[1] == version:
    raise SystemExit('版本未变化或未找到 package version')
entries = locked.split('[[package]]')
matches = [i for i, entry in enumerate(entries) if re.search(r'(?m)^name = "svnui"$', entry)]
if len(matches) != 1 or not re.search(r'(?m)^version = "' + re.escape(old[1]) + r'"$', entries[matches[0]]):
    raise SystemExit('Cargo.lock 源版本不一致；未修改')
new_package = package[1][:old.start(1)] + version + package[1][old.end(1):]
text = text[:package.start(1)] + new_package + text[package.end(1):]
entries[matches[0]] = re.sub(r'(?m)^version = "[^"]+"$', lambda _: f'version = "{version}"', entries[matches[0]], count=1)
# 所有解析完成后才落盘；失败保留文件，调用方明确处理，不自动 reset。
toml.write_text(text)
lock.write_text('[[package]]'.join(entries))
PY
trap 'echo "版本步骤失败；保留现场。修改前提交: ${BACKUP}；请检查 git status，不要自动重写历史" >&2' ERR
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to ${NEW}" -m 'AIGC: release helper'
git tag "${NEW}"
echo "版本提交与本地 tag 已创建: ${NEW}；尚未推送"
