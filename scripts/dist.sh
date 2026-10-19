#!/usr/bin/env bash
# 打包到 dist/releases/<version>，验证后发布，最后按需更新显式指定的 tap。
# --no-publish [--offline] [--host-only]：只生成本地包，绝不操作 tap 或远端。
# --tap-only --tap-dir <dir>：验证已发布附件后仅重试 tap，不重新构建或上传。
# --no-push：tap 只提交不推送；--brew-update：成功推送后显式刷新 brew。
set -euo pipefail
cd "$(dirname "$0")/.."

# 与发布 workflow 一致，避免本机构建意外提高最低 macOS 版本。
export MACOSX_DEPLOYMENT_TARGET=11.0

NO_PUBLISH=0 OFFLINE=0 HOST_ONLY=0 TAP_ONLY=0 NO_PUSH=0 BREW_UPDATE=0
TAP_DIR=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-publish) NO_PUBLISH=1; shift ;;
    --offline) OFFLINE=1; shift ;;
    --host-only) HOST_ONLY=1; shift ;;
    --tap-only) TAP_ONLY=1; shift ;;
    --no-push) NO_PUSH=1; shift ;;
    --brew-update) BREW_UPDATE=1; shift ;;
    --tap-dir) [ "$#" -ge 2 ] && [ -n "$2" ] || { echo '缺少 --tap-dir 路径' >&2; exit 1; }; TAP_DIR="$2"; shift 2 ;;
    -h|--help) sed -n '2,5p' "$0"; exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 1 ;;
  esac
done
fail() { echo "错误: $*" >&2; exit 1; }
[ "${NO_PUBLISH}" = 0 ] || [ "${TAP_ONLY}" = 0 ] || fail '--tap-only 不能与 --no-publish 同用'
[ "${TAP_ONLY}" = 0 ] || [ -n "${TAP_DIR}" ] || fail '--tap-only 需要 --tap-dir'

# 从 [package] 和对应 lock entry 读取版本，避免误取依赖的 version。
VER="$(python3 - <<'PY'
import pathlib, re
text = pathlib.Path('Cargo.toml').read_text()
package = re.search(r'(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)', text)
version = re.search(r'(?m)^version\s*=\s*"([^"]+)"', package[1] if package else '')
if not version or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?', version[1]):
    raise SystemExit('Cargo.toml package version 无效')
entries = pathlib.Path('Cargo.lock').read_text().split('[[package]]')
locked = [entry for entry in entries if re.search(r'(?m)^name = "svnui"$', entry)]
if len(locked) != 1 or not re.search(r'(?m)^version = "' + re.escape(version[1]) + r'"$', locked[0]):
    raise SystemExit('Cargo.lock 中 svnui 版本不一致')
print(version[1])
PY
)"
COMMIT="$(git rev-parse HEAD)"
DIRTY=0
[ -z "$(git status --porcelain --untracked-files=normal)" ] || DIRTY=1
[ "${DIRTY}" = 0 ] || fail '打包需要干净提交，以便 manifest 可核验；请先提交当前改动'
OUT="dist/releases/${VER}"
mkdir -p dist/releases
LOCK="dist/.release-${VER}.lock"
mkdir "${LOCK}" 2>/dev/null || fail "已有同版本任务运行: ${LOCK}（确认无任务后再手动清理）"
STAGE=""
cleanup() { [ -z "${STAGE}" ] || rm -rf "${STAGE}"; rmdir "${LOCK}" 2>/dev/null || true; }
trap cleanup EXIT

# 纯打包只读 Git 元数据；发布才检查 tag/远端并使用 gh。
REPO="${TAP_REPO:-}"
if [ -z "${REPO}" ]; then
  REPO="$(git remote get-url origin)"
  REPO="${REPO#git@github.com:}"; REPO="${REPO#https://github.com/}"; REPO="${REPO%.git}"
fi
[[ "${REPO}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail 'TAP_REPO 需要 owner/repo'
OWNER="${REPO%%/*}"
if [ "${NO_PUBLISH}" = 0 ]; then
  if git show-ref --verify --quiet "refs/tags/${VER}"; then
    [ "$(git rev-parse "refs/tags/${VER}^{commit}")" = "${COMMIT}" ] || fail '本地 tag 与源码提交不一致'
  fi
  REMOTE_TAGS="$(git ls-remote origin "refs/tags/${VER}" "refs/tags/${VER}^{}")"
  REMOTE_COMMIT="$(printf '%s\n' "${REMOTE_TAGS}" | awk 'NF {sha=$1} END {print sha}')"
  [ -z "${REMOTE_COMMIT}" ] || [ "${REMOTE_COMMIT}" = "${COMMIT}" ] || fail '远端 tag 与源码提交不一致'
  command -v gh >/dev/null || fail '需要 gh'
  gh auth status >/dev/null 2>&1 || fail 'gh 未登录'
  # 用列表查询区分不存在和网络/权限失败；后者不得当成新 Release 创建。
  RELEASE_ID="$(gh api "repos/${REPO}/releases" --paginate --jq ".[] | select(.tag_name == \"${VER}\") | .id")"
  if [ "${TAP_ONLY}" = 0 ]; then
    [ -z "${RELEASE_ID}" ] || fail "Release ${VER} 已存在，拒绝覆盖；仅重试 tap 使用 --tap-only"
  else
    [ -n "${RELEASE_ID}" ] || fail 'tap 恢复需要已存在的 Release'
  fi
fi

case "$(uname -m)" in
  arm64|aarch64) HOST_TARGET=aarch64-apple-darwin ;;
  x86_64) HOST_TARGET=x86_64-apple-darwin ;;
  *) fail '本地打包仅支持 macOS 架构' ;;
esac
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
[ "${HOST_ONLY}" = 0 ] || TARGETS=("${HOST_TARGET}")
if [ "${NO_PUBLISH}" = 0 ] && [ "${HOST_ONLY}" = 1 ]; then
  fail '正式发布与双架构 formula 需要两个架构，--host-only 仅用于 --no-publish'
fi

if [ ! -e "${OUT}" ]; then
  [ "${TAP_ONLY}" = 0 ] || fail "缺少已验证本地产物 ${OUT}；保留发布时产物后重试"
  STAGE="$(mktemp -d "dist/.stage-${VER}.XXXXXX")"
  BUILD_ARGS=(--release --locked)
  [ "${OFFLINE}" = 0 ] || BUILD_ARGS+=(--offline)
  for target in "${TARGETS[@]}"; do
    rustup target list --installed | grep -qx "$target" || fail "缺少 $target；请先显式 rustup target add，不会自动安装"
    cargo build "${BUILD_ARGS[@]}" --target "$target"
    BIN="target/$target/release/svnui"
    if [ "$target" = "${HOST_TARGET}" ]; then
      [ "$("${BIN}" --version)" = "svnui ${VER}" ] || fail '二进制版本不一致'
    fi
    mkdir "${STAGE}/payload"
    cp "${BIN}" "${STAGE}/payload/svnui"
    tar -czf "${STAGE}/svnui-${VER}-$target.tar.gz" -C "${STAGE}/payload" svnui
    rm -rf "${STAGE}/payload"
  done
  python3 - "${STAGE}" "${VER}" "${COMMIT}" "${DIRTY}" "${TARGETS[@]}" <<'PY'
import hashlib, json, pathlib, sys
root, version, commit, dirty, *targets = sys.argv[1:]
p = pathlib.Path(root)
artifacts = {f'svnui-{version}-{target}.tar.gz': hashlib.sha256((p / f'svnui-{version}-{target}.tar.gz').read_bytes()).hexdigest() for target in targets}
(p / 'SHA256SUMS').write_text(''.join(f'{sha}  {name}\n' for name, sha in sorted(artifacts.items())))
(p / 'manifest.json').write_text(json.dumps(dict(version=version, commit=commit, dirty=dirty == '1', targets=targets, artifacts=artifacts), indent=2) + '\n')
PY
  # 同版本任务已持锁；只有所有构建完成后才出现最终目录。
  mv "${STAGE}" "${OUT}"
  STAGE=""
fi

# 复用必须来自同一提交与工作区状态；不得把旧版本目录当成当前构建。
python3 - "${OUT}" "${VER}" "${COMMIT}" "${DIRTY}" "${TARGETS[@]}" <<'PY'
import hashlib, json, pathlib, sys
root, version, commit, dirty, *targets = sys.argv[1:]
p = pathlib.Path(root)
m = json.loads((p / 'manifest.json').read_text())
if (m['version'], m['commit'], m['dirty']) != (version, commit, dirty == '1'):
    raise SystemExit('manifest 版本/源码提交/工作区状态不匹配；保留旧目录，停止复用')
if m['dirty']:
    raise SystemExit('脏工作区产物不能复用；请使用干净提交重新打包')
expected = {f'svnui-{version}-{target}.tar.gz' for target in targets}
if set(m['artifacts']) != expected or set(m['targets']) != set(targets):
    raise SystemExit('产物架构集合不匹配')
for name, sha in m['artifacts'].items():
    if hashlib.sha256((p / name).read_bytes()).hexdigest() != sha:
        raise SystemExit(f'产物校验失败: {name}')
checksums = ''.join(f'{sha}  {name}\n' for name, sha in sorted(m['artifacts'].items()))
if (p / 'SHA256SUMS').read_text() != checksums:
    raise SystemExit('SHA256SUMS 与 manifest 不一致')
PY
if [ "${NO_PUBLISH}" = 1 ]; then
  echo "本地打包完成: ${OUT}（未操作远端或 tap）"
  exit 0
fi

if [ "${TAP_ONLY}" = 0 ]; then
  gh release create "${VER}" "${OUT}"/*.tar.gz "${OUT}/SHA256SUMS" "${OUT}/manifest.json" \
    --repo "${REPO}" --target "${COMMIT}" --generate-notes
  echo "Release 已创建；验证失败时保留附件，不更新 tap: https://github.com/${REPO}/releases/tag/${VER}"
fi
STAGE="$(mktemp -d "dist/.verify-${VER}.XXXXXX")"
gh release download "${VER}" --repo "${REPO}" --dir "${STAGE}" --pattern '*.tar.gz' --pattern SHA256SUMS --pattern manifest.json
python3 - "${OUT}" "${STAGE}" <<'PY'
import hashlib, json, pathlib, sys
local, remote = map(pathlib.Path, sys.argv[1:])
for name in ['manifest.json', 'SHA256SUMS']:
    if (local / name).read_bytes() != (remote / name).read_bytes():
        raise SystemExit(f'发布后 {name} 不一致；停止 tap 更新')
for name, sha in json.loads((local / 'manifest.json').read_text())['artifacts'].items():
    if hashlib.sha256((remote / name).read_bytes()).hexdigest() != sha:
        raise SystemExit(f'发布后附件校验失败: {name}；停止 tap 更新')
PY
rm -rf "${STAGE}"; STAGE=""
[ -n "${TAP_DIR}" ] || { echo '附件验证完成；未指定 --tap-dir，跳过 tap'; exit 0; }
[ -d "${TAP_DIR}" ] || fail 'tap 目录不存在；Release 已保留，可 --tap-only 重试'
TAP_DIR="$(cd "${TAP_DIR}" && pwd)"
[ -z "$(git -C "${TAP_DIR}" status --porcelain)" ] || fail 'tap 工作区不干净；Release 已保留'
TAP_REMOTE="$(git -C "${TAP_DIR}" remote get-url origin)"
case "${TAP_REMOTE}" in
  "git@github.com:${OWNER}/homebrew-svnui.git"|"https://github.com/${OWNER}/homebrew-svnui.git"|"https://github.com/${OWNER}/homebrew-svnui") ;;
  *) fail 'tap origin 与发布仓库 owner 不一致' ;;
esac
# 只操作显式 tap，不自动扫描、pull 其他工作区。
python3 - "${OUT}" "${TAP_DIR}" "${OWNER}" <<'PY'
import json, pathlib, re, sys
root, tap, owner = sys.argv[1:]
m = json.loads((pathlib.Path(root) / 'manifest.json').read_text())
version = m['version']
formula = pathlib.Path(tap) / 'Formula/svnui.rb'
if formula.exists():
    text = formula.read_text()
    for target in m['targets']:
        name = f'svnui-{version}-{target}.tar.gz'
        pattern = re.compile(r'(url\s+")[^"]*svnui-[^"/]+-' + re.escape(target) + r'\.tar\.gz("(?:(?!\burl\b)[\s\S])*?sha256\s+")[^"]*(")')
        url = f'https://github.com/{owner}/svnui/releases/download/{version}/{name}'
        text, count = pattern.subn(lambda match: match[1] + url + match[2] + m['artifacts'][name] + match[3], text)
        if count != 1:
            raise SystemExit(f'formula 中 {target} 条目数量异常；未修改')
    text = re.sub(r'(assert_match ")[^"]*(", shell_output)', lambda match: match[1] + version + match[2], text)
else:
    text = pathlib.Path('packaging/homebrew/svnui.rb.tpl').read_text()
    for token, value in {'OWNER': owner, 'VERSION': version, 'SHA_ARM': m['artifacts'][f'svnui-{version}-aarch64-apple-darwin.tar.gz'], 'SHA_X86': m['artifacts'][f'svnui-{version}-x86_64-apple-darwin.tar.gz']}.items():
        text = text.replace(f'@@{token}@@', value)
if '@@' in text:
    raise SystemExit('formula 仍有占位符；未修改')
formula.parent.mkdir(parents=True, exist_ok=True)
formula.write_text(text)
PY
# 先暂存显式目标再检查索引，确保首次创建的未跟踪 formula 也会提交。
git -C "${TAP_DIR}" add Formula/svnui.rb
if ! git -C "${TAP_DIR}" diff --cached --quiet -- Formula/svnui.rb; then
  git -C "${TAP_DIR}" commit -m "svnui ${VER}"
fi
if [ "${NO_PUSH}" = 0 ]; then
  git -C "${TAP_DIR}" push origin HEAD
  [ "${BREW_UPDATE}" = 0 ] || brew update
fi
echo "发布及 tap 更新完成: ${VER}；失败恢复仅执行 --tap-only --tap-dir <同一目录>"
