#!/usr/bin/env bash
# 重新生成 tests/fixtures/ 下的样本。
#
# 需要 svnadmin。只有 svn 客户端的机器上跑不了 —— 那也没关系，
# 仓库里已经有一份手工构造的样本，能跑通测试；这份脚本的作用是
# **换 svn 版本后重新校准**，避免样本与真实输出脱节导致解析悄悄失效。
#
# 用法：  bash tests/fixtures/gen.sh
# 会写到 tests/tmp/ 再拷贝进 tests/fixtures/

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP="$ROOT/tests/tmp"
FIX="$ROOT/tests/fixtures"
mkdir -p "$TMP" "$FIX"

if ! command -v svnadmin >/dev/null 2>&1; then
  echo "✗ 找不到 svnadmin，无法生成样本。" >&2
  echo "  仓库里已有一份手工样本，测试仍可跑。" >&2
  echo "  Ubuntu: sudo apt install subversion" >&2
  echo "  macOS:  brew install subversion" >&2
  exit 0
fi

REPO="$TMP/repo"
WC="$TMP/wc"
rm -rf "$REPO" "$WC"

echo "→ 建仓库 $REPO"
svnadmin create "$REPO"
REPO_URL="file://$REPO"

svn co "$REPO_URL" "$WC" --non-interactive -q

cd "$WC"

echo "→ 造场景"
# 基础文件
mkdir -p src/deep vendor
printf 'a\n' > src/a.txt
printf 'b\n' > src/deep/b.txt
printf 'c\n' > src/c.txt
printf 'keep\n' > keep.txt
svn add -q src keep.txt
svn ci -m "init" --non-interactive -q

# 覆盖各种状态
printf 'a-modified\n' > src/a.txt                 # M
printf 'new\n'        > src/new.txt               # ?
svn add -q src/new.txt                            # A
svn ps svn:externals "^/vendor ext" . -q          # 属性改动 _M
svn rm -q --keep-local src/c.txt                  # D
rm -f src/deep/b.txt                              # ! 缺失
mkdir -p build && printf 'x\n' > build/out.o      # I 忽略（需 svn:ignore）
svn ps svn:ignore "build" . -q
printf 'mine\n' > src/中文 文件.txt                # 中文 + 空格路径
svn add -q "src/中文 文件.txt"

# ---- 导出"干净"样本（无冲突）
echo "→ 导出 status_plain.txt"
svn status --non-interactive > "$FIX/status_plain.txt" 2>/dev/null || true
echo "→ 导出 status.xml"
svn status --xml --non-interactive > "$FIX/status.xml" 2>/dev/null || true

# ---- 日志 / info
echo "→ 导出 log.xml / info.xml"
svn log --xml -v --non-interactive > "$FIX/log.xml" 2>/dev/null || true
svn info --xml --non-interactive > "$FIX/info.xml" 2>/dev/null || true

# ---- 冲突样本（需要两次提交制造冲突）
echo "→ 制造冲突"
svn ci -m "wip" --non-interactive -q
printf 'local-edit\n' > conflict.txt
svn add -q conflict.txt
svn ci -m "add conflict.txt" --non-interactive -q
svn up -q --non-interactive

# 用另一个工作副本改同一文件，再回来 update 造冲突
WC2="$TMP/wc2"
svn co "$REPO_URL" "$WC2" --non-interactive -q
printf 'remote-edit\n' > "$WC2/conflict.txt"
svn ci -m "remote edit" --non-interactive -q -F <(echo "remote edit") "$WC2/conflict.txt" 2>/dev/null || \
  (cd "$WC2" && svn ci -m "remote edit" --non-interactive -q)

printf 'local-again\n' > conflict.txt
svn up --non-interactive --accept postpone > /dev/null 2>&1 || true

echo "→ 导出 status_conflict.txt"
svn status --non-interactive > "$FIX/status_conflict.txt" 2>/dev/null || true

# ---- 校验：样本必须能被解析（非空）
for f in status_plain.txt status_conflict.txt status.xml log.xml info.xml; do
  if [ ! -s "$FIX/$f" ]; then
    echo "⚠ $f 是空的，保留仓库里已有的手工样本" >&2
    git -C "$ROOT" checkout -- "tests/fixtures/$f" 2>/dev/null || true
  fi
done

echo "✓ 样本已更新到 tests/fixtures/"
echo "  跑 just test 验证解析是否仍全绿。"
