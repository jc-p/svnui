#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""svnui 通用更新脚本 —— 后续所有更新都走这一条命令。

用法
    python3 scripts/apply_update.py <更新包.zip 或 目录> [--apply]

不传 --apply 只预演，一个字节都不改。
传了 --apply 才会：备份 → 打补丁 → 增补新文件 → 校验。

设计上刻意避开的三个坑（都是以前真踩过的）：
  1. **绝不整包替换 src/** —— 只跑补丁 + 增补新文件。
     整包替换会把本地手动改动冲掉，还会造成文件间版本错配。
  2. **自动定位项目根** —— 不依赖"当前目录正好是项目根"。
     以前在临时目录里跑脚本，脚本按 cwd 找 src/ 直接报"文件不存在"。
  3. **预演与应用分离** —— 先看清楚改什么，再决定要不要落盘。
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

# ── 终端输出 ────────────────────────────────────────────────
def ok(s):   print(f"  \033[32m✓\033[0m {s}")
def bad(s):  print(f"  \033[31m✗\033[0m {s}")
def warn(s): print(f"  \033[33m!\033[0m {s}")
def head(s): print(f"\n\033[36m── {s} ──\033[0m")


def brief(out, limit=30):
    """压缩补丁输出，但**绝不丢失败行**。

    原来直接取最后 15 行 —— 而失败项常常排在最前面（第一个锚点就对不上），
    于是只看到「失败 1」却不知道是哪个，等于让人猜。
    """
    ls = (out or "").strip().splitlines()
    if not ls:
        return []
    key = ("✗", "⚠", "期望找到", "最接近", "没有相似行", "找不到", "请先")
    bad = [l for l in ls if any(k in l for k in key)]
    rest = [l for l in ls if l not in bad]
    show = bad[:12]
    if rest:
        if len(rest) > 10:
            show.append("      …")
        show.extend(rest[-10:])
    return show[:limit]


def find_project_root(start: Path):
    """从 start 向上找含 Cargo.toml 且含 src/ 的目录。"""
    p = start.resolve()
    for d in [p, *p.parents]:
        if (d / "Cargo.toml").is_file() and (d / "src").is_dir():
            return d
    return None


def extract_package(src: Path, workdir: Path) -> Path:
    """zip 解压到临时目录；目录则直接返回。"""
    if src.is_dir():
        return src
    out = workdir / "pkg"
    out.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as z:
        for info in z.infolist():
            name = info.filename
            # zip 里中文名常以 cp437 存，解码回来
            for enc in ("utf-8", "gbk"):
                try:
                    name = info.filename.encode("cp437").decode(enc)
                    break
                except Exception:
                    continue
            target = out / name
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with z.open(info) as f, open(target, "wb") as w:
                    shutil.copyfileobj(f, w)
    return out


def resolve_pkg_root(pkg: Path) -> Path:
    """剥掉解压后常见的单层包装目录。

    打包时内容常整体放进 svnui_update_vXX/ 里，直接按 pkg 找会一无所获
    ——v31 就栽在这里：识别到 0 个补丁，却照样打印一堆 ✓。
    """
    entries = [p for p in pkg.iterdir() if not p.name.startswith("__MACOSX")]
    if len(entries) == 1 and entries[0].is_dir():
        inner = entries[0]
        for marker in ("patches", "files"):
            if (inner / marker).is_dir():
                return inner
        if any(inner.glob("fix_*.py")):
            return inner
    return pkg


def collect_patches(pkg: Path):
    """优先 patches/，其次包根的 fix_*.py。按名字排序保证依赖顺序。"""
    skip = {"patchkit.py", "_patchkit.py"}
    d = pkg / "patches"
    if d.is_dir():
        ps = sorted(x for x in d.glob("*.py")
                    if x.name not in skip and not x.name.startswith("_"))
        if ps:
            return ps
    # 兜底：递归找一层，避免 patches/ 嵌得更深时又漏掉
    ps = sorted(pkg.glob("*/patches/*.py"))
    if ps:
        return ps
    return sorted(pkg.glob("fix_*.py"))


def run_patch(py: Path, root: Path, extra):
    r = subprocess.run(
        [sys.executable, str(py), *extra],
        cwd=str(root), capture_output=True, text=True,
    )
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def copy_files(pkg: Path, root: Path, apply: bool):
    """把 files/ 下的内容按相对路径拷进项目根。覆盖式，天然幂等。"""
    files_dir = pkg / "files"
    if not files_dir.is_dir():
        return 0
    n = 0
    for f in sorted(files_dir.rglob("*")):
        if not f.is_file():
            continue
        rel = f.relative_to(files_dir)
        dst = root / rel
        if dst.exists() and dst.read_bytes() == f.read_bytes():
            ok(f"{rel} 已存在且一致，跳过")
            continue
        act = "覆盖" if dst.exists() else "新增"
        if apply:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(f, dst)
        ok(f"{act} {rel}")
        n += 1
    return n


def verify(root: Path):
    """校验：CR 残留 + 明显括号失衡（只报，不阻断）。"""
    head("校验")
    cr = []
    imba = []
    for f in (root / "src").rglob("*.rs"):
        try:
            b = f.read_bytes()
        except Exception:
            continue
        if b"\r" in b:
            cr.append(f.relative_to(root))
        try:
            s = b.decode("utf-8")
        except Exception:
            continue
        for open_ch, close_ch in ("{}", "()", "[]"):
            d = s.count(open_ch) - s.count(close_ch)
            if abs(d) >= 4:  # 阈值放宽：字符串/注释里的字面量会造成小偏差
                imba.append(f"{f.relative_to(root)} {open_ch}{close_ch} 差 {d}")
    if cr:
        bad(f"存在 CR 残留: {[str(x) for x in cr][:5]}")
    else:
        ok("无 CR 残留")
    if imba:
        warn("括号差值较大（可能是字符串/注释里的字面量，也可能是真缺括号）:")
        for x in imba[:8]:
            print(f"      {x}")
    else:
        ok("括号无明显失衡")


def auto_find_package():
    """在 ~/Downloads（以及 ~/下载、当前目录）下找最新的 svnui*.zip。

    挑"最新"用 mtime——下载时间越近越可能是你要装的那版。
    同目录下有多个时，按版本号（v后面的数字）优先，其次 mtime。
    """
    import re
    # 下载目录名在不同系统/语言下大小写、用词都不同（Downloads / downloads / 下载），
    # 硬编码一个名字会在别人机器上静默找不到 —— 所以按名字特征扫一遍。
    bases = []
    home = Path.home()
    if home.is_dir():
        for d in home.iterdir():
            if not d.is_dir():
                continue
            n = d.name.lower()
            if n in ("downloads", "download", "下载", "desktop", "桌面"):
                bases.append(d)
    bases.append(Path.cwd())
    cands = []
    for base in bases:
        if not base.is_dir():
            continue
        for f in base.glob("svnui*.zip"):
            if not f.is_file():
                continue
            m = re.search(r"[vV](\d+)", f.name)
            ver = int(m.group(1)) if m else -1
            cands.append((ver, f.stat().st_mtime, f))
    if not cands:
        return None
    cands.sort(key=lambda t: (t[0], t[1]), reverse=True)
    return cands[0][2]


def chained_preview(patches, root, workdir, dry, dry_fail):
    """在 src 副本上按顺序应用前面通过的补丁，再复核失败的那些。

    依赖型补丁（config_llm 要找的 next_suggestion 正是 commit_gen 插入的）
    单独预演必然失败 —— 可它其实没问题，只是还没轮到前置补丁。
    """
    scratch = workdir / "scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root / "Cargo.toml", scratch / "Cargo.toml")
    if (scratch / "src").exists():
        # 重跑时清掉旧副本：否则"已存在"的判定会让补丁跳过本该做的改动，
        # 链式复核看到的就不是真实的前置状态。
        shutil.rmtree(scratch / "src")
    shutil.copytree(root / "src", scratch / "src")
    pre_fail = []
    for py in patches:
        if py.name not in dry_fail:
            code, _ = run_patch(py, scratch, ["--apply"])
            if code != 0:
                # 以前这行不检查返回码：前置补丁其实没应用成功，
                # 链式复核于是必然失败，而界面上一句提示都没有。
                pre_fail.append(py.name)
    if pre_fail:
        warn(f"前置补丁在副本上应用失败: {pre_fail} —— 链式复核结果不可信")
    rescued = []
    for py in patches:
        if py.name not in dry_fail:
            continue
        code, out = run_patch(py, scratch, [])
        if code == 0:
            dry[py.name] = (0, out + "\n      （链式预演通过：依赖的前置补丁已应用）")
            rescued.append(py.name)
    return rescued


def self_bootstrap(pkg, root, argv):
    """把自己升到包里的新版，并用新代码重跑一次。

    ——v34 就栽在这里：新脚本只在 --apply 阶段才拷进项目，而多数人第一次
    是跑预演。于是"链式预演""失败行不截断"这些修复永远轮不到生效，
    看到的还是老脚本的报错。脚本得先会升级自己，别的修复才有机会执行。
    """
    if os.environ.get("SVNUI_UPDATE_BOOTSTRAPPED") == "1":
        return
    cand = pkg / "files" / "scripts" / "apply_update.py"
    if not cand.is_file():
        return
    data = cand.read_bytes()
    me = Path(__file__).resolve()
    target = (root / "scripts" / "apply_update.py").resolve()
    changed = False
    for dest in {me, target}:
        try:
            if dest.exists() and dest.read_bytes() == data:
                continue
        except Exception:
            pass
        try:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(data)
            ok(f"更新脚本已升级: {dest}")
            changed = True
        except Exception as e:
            warn(f"更新脚本写入失败 {dest}: {e}")
    if not changed:
        return
    print("      用新版脚本重跑一次，确保预演/应用逻辑是最新的\n")
    os.environ["SVNUI_UPDATE_BOOTSTRAPPED"] = "1"
    os.execv(sys.executable, [sys.executable, str(target if target.exists() else me), *argv])


def main():
    ap = argparse.ArgumentParser(description="svnui 通用更新脚本")
    ap.add_argument("package", nargs="?", default=None,
                    help="更新包 .zip 或已解压目录；省略则自动找 ~/Downloads 下最新的 svnui_*.zip")
    ap.add_argument("--apply", action="store_true", help="真正写入（默认只预演）")
    ap.add_argument("--no-backup", action="store_true", help="不备份 src/")
    args = ap.parse_args()

    if args.package:
        src = Path(args.package).expanduser()
        if not src.exists():
            bad(f"找不到更新包: {src}")
            return 2
    else:
        src = auto_find_package()
        if src is None:
            bad("没传更新包，也没在 ~/Downloads 下找到 svnui_*.zip")
            print("      把下载好的 zip 放进 ~/Downloads，或显式指定路径：")
            print("        python3 scripts/apply_update.py <包路径>")
            return 2
        ok(f"自动选用更新包: {src.name}")

    root = find_project_root(Path.cwd())
    if root is None:
        bad("定位不到项目根（含 Cargo.toml 与 src/ 的目录）。")
        print("      请先 cd 到 svnui 项目目录再跑，例如：")
        print("        cd ~/learn/Lazy/svnui && python3 scripts/apply_update.py <包>")
        return 2
    ok(f"项目目录: {root}")

    workdir = Path(tempfile.mkdtemp(prefix="svnui_update_"))
    pkg = resolve_pkg_root(extract_package(src, workdir))
    patches = collect_patches(pkg)
    if not patches:
        bad("包内没有找到任何补丁脚本 —— 什么都不做，避免假装成功。")
        print(f"      解包位置: {pkg}")
        got = [str(p.relative_to(pkg)) for p in pkg.rglob("*")
               if p.is_file() and not p.name.startswith(".")][:20]
        print("      包内文件:")
        for g in got:
            print(f"        {g}")
        print("      若确实应有补丁，请把上面这份清单发来。")
        return 2
    ok(f"识别到 {len(patches)} 个补丁")
    for py in patches:
        print(f"        · {py.name}")

    # 先把脚本自身升到包里的版本 —— 预演阶段也要升，否则新逻辑永远轮不到。
    self_bootstrap(pkg, root, sys.argv[1:])

    # ── 预演 ──
    head("预演（不写入）")
    dry = {}
    for py in patches:
        dry[py.name] = run_patch(py, root, [])
    dry_fail = [py.name for py in patches if dry[py.name][0] != 0]
    for py in patches:
        code, out = dry[py.name]
        (ok if code == 0 else bad)(f"{py.name}: {'通过' if code == 0 else '失败'}")
        if code != 0:
            print("\n".join("      " + l for l in brief(out)))
    if dry_fail:
        rescued = chained_preview(patches, root, workdir, dry, dry_fail)
        for name in rescued:
            ok(f"{name}: 链式预演通过（依赖的前置补丁应用后正常）")
        dry_fail = [n for n in dry_fail if n not in rescued]
    if dry_fail:
        warn(f"以下补丁预演失败，本次将跳过（其余照常应用）: {dry_fail}")
        print("      本地代码与该补丁预期基线不同，把上面的 ✗ 行发来即可对症修。")

    if not args.apply:
        head("预演结束")
        print("  确认无误后加 --apply 真正写入：")
        print(f"    python3 {sys.argv[0]} {src} --apply")
        return 0

    # ── 备份 ──
    if not args.no_backup:
        head("备份")
        stamp = subprocess.run(
            ["date", "+%Y%m%d_%H%M%S"], capture_output=True, text=True
        ).stdout.strip() or "bak"
        bak = Path(tempfile.gettempdir()) / f"svnui_src_backup_{stamp}"
        shutil.copytree(root / "src", bak)
        ok(f"src/ 已备份到 {bak}")

    # ── 增补文件（放在补丁之前：补丁可能要引用这些新文件）──
    head("增补文件")
    copy_files(pkg, root, apply=True)

    # ── 应用补丁 ──
    head("应用补丁")
    applied, skipped = [], []
    for py in patches:
        if py.name in dry_fail:
            warn(f"{py.name}: 跳过（预演失败）")
            skipped.append(py.name)
            continue
        code, out = run_patch(py, root, ["--apply"])
        if code == 0:
            # 区分"这次真改了"和"早就改过了" —— 幂等要看得见，不能都叫"已应用"。
            # 不能靠"出现过跳过字样"来判断：只要有一个新文件已存在就会命中，
            # 明明改了 13 处也会被标成"已是最新"。
            m = re.search(r"命中\s*(\d+)", out)
            hits = int(m.group(1)) if m else None
            if hits == 0:
                label = "已是最新（未重复改动）"
            elif hits is not None:
                label = f"已应用（{hits} 处改动）"
            else:
                label = "已应用"
            ok(f"{py.name}: {label}")
        else:
            bad(f"{py.name}: 失败")
        if code != 0:
            print("\n".join("      " + l for l in brief(out)))
            bad(f"{py.name} 应用时失败，已停止。可用备份回滚 src/")
            return 1
        applied.append(py.name)

    verify(root)

    head("下一步")
    print("  cd " + str(root))
    print("  cargo build --release")
    print("  ./scripts/install-local.sh --no-build")
    if skipped:
        print(f"\n  本次跳过（预演失败）: {skipped}")
        print("  把预演输出发来即可对症修，其余补丁已正常应用。")
    if "bak" in dir():
        print(f"\n  回滚（如需）: rm -rf src && cp -r {bak} src")
    return 0


if __name__ == "__main__":
    sys.exit(main())
