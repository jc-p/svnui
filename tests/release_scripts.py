"""发布脚本的故障回归；仅在临时仓库中运行 PATH 替身，不构建/发布真实包。"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
REAL_GIT = shutil.which('git')

# 替身仅模拟脚本的外部契约；Git 元数据来自临时真实仓库。
STUB = r'''#!/usr/bin/env python3
import json, os, pathlib, shutil, subprocess, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['CALL_LOG'], 'a') as log:
    log.write(json.dumps([name] + args) + '\n')
if name == 'git':
    if 'ls-remote' in args:
        print(os.environ.get('REMOTE_TAG', ''), end='')
        sys.exit(0)
    if 'push' in args:
        sys.exit(1 if os.environ.get('FAIL_PUSH') == '1' else 0)
    sys.exit(subprocess.call([os.environ['REAL_GIT']] + args))
if name == 'rustup':
    if args != ['target', 'list', '--installed']:
        sys.exit('禁止安装 target')
    print('aarch64-apple-darwin\nx86_64-apple-darwin')
    sys.exit(0)
if name == 'cargo':
    if os.environ.get('MACOSX_DEPLOYMENT_TARGET') != '11.0':
        sys.exit('本地包最低 macOS 版本必须为 11.0')
    if '--locked' not in args or ('--offline' not in args and os.environ.get('ALLOW_ONLINE_BUILD') != '1'):
        sys.exit('构建必须锁定且离线')
    target = args[args.index('--target') + 1]
    if os.environ.get('FAIL_BUILD') == target:
        sys.exit(1)
    binary = pathlib.Path('target') / target / 'release/svnui'
    binary.parent.mkdir(parents=True, exist_ok=True)
    version = os.environ.get('BINARY_VERSION', '0.4.2')
    binary.write_text('#!/bin/sh\necho "svnui ' + version + '"\n')
    binary.chmod(0o755)
    sys.exit(0)
if name == 'gh':
    state = pathlib.Path(os.environ['RELEASE_STATE'])
    if args[:2] == ['auth', 'status']:
        sys.exit(0)
    if args[:1] == ['api']:
        if os.environ.get('FAIL_API') == '1':
            sys.exit(1)
        if state.exists():
            print('123')
        sys.exit(0)
    if args[:2] == ['release', 'create']:
        if state.exists() or os.environ.get('FAIL_CREATE') == '1':
            sys.exit(1)
        state.write_text('created')
        sys.exit(0)
    if args[:2] == ['release', 'download']:
        destination = pathlib.Path(args[args.index('--dir') + 1])
        source = pathlib.Path('dist/releases/0.4.2')
        for path in source.iterdir():
            if path.suffix == '.gz' or path.name in ('manifest.json', 'SHA256SUMS'):
                shutil.copy2(path, destination / path.name)
        if os.environ.get('CORRUPT_DOWNLOAD') == '1':
            next(destination.glob('*.tar.gz')).write_text('corrupt')
        sys.exit(0)
    sys.exit('禁止未预期 gh 操作')
sys.exit('禁止外部命令: ' + name)
'''


@unittest.skipUnless(REAL_GIT, '脚本测试需要 Git')
class ReleaseScripts(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix='svnui release tests ')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        (self.repo / 'scripts').mkdir()
        for name in ('dist.sh', 'bump.sh', 'release.sh'):
            shutil.copy2(ROOT / 'scripts' / name, self.repo / 'scripts' / name)
        template = self.repo / 'packaging/homebrew'
        template.mkdir(parents=True)
        shutil.copy2(ROOT / 'packaging/homebrew/svnui.rb.tpl', template)
        (self.repo / 'Cargo.toml').write_text('[package]\nname = "svnui"\nversion = "0.4.2"\n')
        (self.repo / 'Cargo.lock').write_text('version = 4\n\n[[package]]\nname = "svnui"\nversion = "0.4.2"\n')
        (self.repo / '.gitignore').write_text('/dist/\n/target/\n')
        self.git('init', '-q')
        self.git('config', 'user.name', 'Release Test')
        self.git('config', 'user.email', 'release@example.invalid')
        self.git('remote', 'add', 'origin', 'https://github.com/jc-p/svnui.git')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        for name in ('cargo', 'rustup', 'gh', 'git', 'brew'):
            script = self.bin / name
            script.write_text(STUB)
            script.chmod(0o755)
        self.log = self.root / 'calls.jsonl'
        self.state = self.root / 'release-state'
        self.env = dict(os.environ, PATH=f'{self.bin}{os.pathsep}{os.environ["PATH"]}',
                        REAL_GIT=REAL_GIT, CALL_LOG=str(self.log), RELEASE_STATE=str(self.state))
        # 用户的 Git/Cargo 环境不应改变临时测试边界。
        for key in ('TAP_REPO', 'TAP_OWNER', 'GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE'):
            self.env.pop(key, None)

    def git(self, *args: str, cwd: Path | None = None) -> str:
        result = subprocess.run([REAL_GIT, *args], cwd=cwd or self.repo,
                                capture_output=True, text=True, check=True, timeout=15)
        return result.stdout.strip()

    def run_script(self, *args: str, script: str = 'dist.sh', success: bool = True,
                   extra: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(['bash', f'scripts/{script}', *args], cwd=self.repo,
                                env=dict(self.env, **(extra or {})), capture_output=True,
                                text=True, errors="replace", timeout=30)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def calls(self) -> list[list[str]]:
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def tap(self, with_formula: bool = True) -> Path:
        tap = self.root / 'tap with spaces'
        tap.mkdir()
        self.git('init', '-q', cwd=tap)
        self.git('config', 'user.name', 'Tap Test', cwd=tap)
        self.git('config', 'user.email', 'tap@example.invalid', cwd=tap)
        self.git('remote', 'add', 'origin', 'https://github.com/jc-p/homebrew-svnui.git', cwd=tap)
        if with_formula:
            (tap / 'Formula').mkdir()
            formula = (ROOT / 'packaging/homebrew/svnui.rb.tpl').read_text()
            for token, value in {'OWNER': 'jc-p', 'VERSION': '0.4.1', 'SHA_ARM': 'a' * 64, 'SHA_X86': 'b' * 64}.items():
                formula = formula.replace(f'@@{token}@@', value)
            (tap / 'Formula/svnui.rb').write_text(formula)
            self.git('add', '.', cwd=tap)
        self.git('commit', '--allow-empty', '-qm', 'tap fixture', cwd=tap)
        return tap

    def test_no_publish_has_no_git_or_external_writes(self) -> None:
        tap = self.tap()
        before = (tap / 'Formula/svnui.rb').read_bytes()
        head = self.git('rev-parse', 'HEAD')
        old = self.repo / 'dist/releases/0.4.1/keep'
        old.parent.mkdir(parents=True)
        old.write_text('keep')
        self.run_script('--offline', '--no-publish', '--tap-dir', str(tap),
                        extra={'MACOSX_DEPLOYMENT_TARGET': '15.0'})
        self.assertEqual(before, (tap / 'Formula/svnui.rb').read_bytes())
        self.assertEqual(head, self.git('rev-parse', 'HEAD'))
        self.assertEqual(old.read_text(), 'keep')
        forbidden = {'push', 'pull', 'commit', 'tag', 'add', 'reset'}
        for call in self.calls():
            self.assertNotIn(call[0], ('gh', 'brew'))
            if call[0] == 'git':
                self.assertFalse(forbidden.intersection(call[1:]), call)
        manifest = json.loads((self.repo / 'dist/releases/0.4.2/manifest.json').read_text())
        self.assertEqual(manifest['commit'], head)
        self.assertEqual(len(manifest['artifacts']), 2)

    def test_release_no_publish_splits_before_bump_push(self) -> None:
        self.run_script('0.4.2', '--no-publish', '--offline', script='release.sh')
        self.assertFalse(any(call[0] == 'git' and 'push' in call for call in self.calls()))
        self.run_script('0.4.3', '--no-publish', '--offline', script='release.sh', success=False)
        self.assertEqual(self.git('tag'), '')

    def test_build_failure_keeps_old_artifacts_and_cleans_stage(self) -> None:
        self.run_script('--offline', '--no-publish', success=False,
                        extra={'FAIL_BUILD': 'x86_64-apple-darwin'})
        self.assertFalse((self.repo / 'dist/releases/0.4.2').exists())
        self.assertEqual(list((self.repo / 'dist').glob('.stage-*')), [])
        self.assertEqual(list((self.repo / 'dist').glob('*.lock')), [])

    def test_binary_and_lock_mismatch_stop_packaging(self) -> None:
        self.run_script('--offline', '--no-publish', success=False,
                        extra={'BINARY_VERSION': '9.9.9'})
        self.assertFalse((self.repo / 'dist/releases/0.4.2').exists())
        p = self.repo / 'Cargo.lock'
        p.write_text(p.read_text().replace('0.4.2', '0.4.0'))
        self.run_script('--offline', '--no-publish', success=False)

    def test_tag_mismatch_and_existing_release_never_upload(self) -> None:
        self.git('tag', '0.4.2')
        (self.repo / 'README').write_text('new')
        self.git('add', 'README')
        self.git('commit', '-qm', 'later')
        self.run_script('--offline', success=False)
        self.assertFalse(any(call[:3] == ['gh', 'release', 'create'] for call in self.calls()))
        self.git('tag', '-d', '0.4.2')
        self.state.write_text('existing')
        self.run_script('--offline', success=False)
        self.assertFalse(any(call[:2] == ['gh', 'release'] for call in self.calls()))

    def test_remote_tag_mismatch_and_api_failure_stop(self) -> None:
        self.run_script('--offline', success=False,
                        extra={'REMOTE_TAG': 'f' * 40 + '\trefs/tags/0.4.2\n'})
        self.run_script('--offline', success=False, extra={'FAIL_API': '1'})
        self.assertFalse(any(call[:3] == ['gh', 'release', 'create'] for call in self.calls()))

    def test_publish_failure_or_bad_download_does_not_touch_tap(self) -> None:
        tap = self.tap()
        before = (tap / 'Formula/svnui.rb').read_bytes()
        self.run_script('--offline', '--tap-dir', str(tap), success=False,
                        extra={'FAIL_CREATE': '1'})
        self.assertEqual(before, (tap / 'Formula/svnui.rb').read_bytes())
        self.run_script('--offline', '--tap-dir', str(tap), success=False,
                        extra={'CORRUPT_DOWNLOAD': '1'})
        self.assertTrue(self.state.exists())
        self.assertEqual(before, (tap / 'Formula/svnui.rb').read_bytes())

    def test_empty_tap_creates_and_commits_formula_before_push(self) -> None:
        tap = self.tap(with_formula=False)
        initial = self.git('rev-parse', 'HEAD', cwd=tap)
        self.run_script('--offline', '--tap-dir', str(tap))
        self.assertNotEqual(initial, self.git('rev-parse', 'HEAD', cwd=tap))
        self.assertEqual(self.git('ls-files', 'Formula/svnui.rb', cwd=tap), 'Formula/svnui.rb')
        self.assertEqual(self.git('status', '--porcelain', cwd=tap), '')
        formula = (tap / 'Formula/svnui.rb').read_text()
        self.assertIn('0.4.2', formula)
        self.assertNotIn('@@', formula)
        calls = self.calls()
        added = next(i for i, call in enumerate(calls) if call[0] == 'git' and call[-2:] == ['add', 'Formula/svnui.rb'])
        committed = next(i for i, call in enumerate(calls) if call[0] == 'git' and 'commit' in call)
        pushed = next(i for i, call in enumerate(calls) if call[0] == 'git' and 'push' in call)
        self.assertLess(added, committed)
        self.assertLess(committed, pushed)

    def test_tap_push_failure_can_retry_without_rebuild_upload(self) -> None:
        tap = self.tap()
        self.run_script('--offline', '--tap-dir', str(tap), success=False,
                        extra={'FAIL_PUSH': '1'})
        self.assertTrue(self.state.exists())
        tap_head = self.git('rev-parse', 'HEAD', cwd=tap)
        self.assertEqual(self.git('status', '--porcelain', cwd=tap), '')
        calls_before = len(self.calls())
        self.run_script('--tap-only', '--tap-dir', str(tap))
        retried = self.calls()[calls_before:]
        self.assertEqual(tap_head, self.git('rev-parse', 'HEAD', cwd=tap))
        self.assertFalse(any(call[0] == 'git' and 'commit' in call for call in retried))
        pushes = [call for call in retried if call[0] == 'git' and 'push' in call]
        self.assertEqual(len(pushes), 1)
        self.assertEqual(pushes[0][:2], ['git', '-C'])
        self.assertEqual(Path(pushes[0][2]).resolve(), tap.resolve())
        self.assertEqual(pushes[0][3:], ['push', 'origin', 'HEAD'])
        self.assertFalse(any(call[0] == 'cargo' or call[:3] == ['gh', 'release', 'create'] for call in retried))
        self.assertIn('0.4.2', (tap / 'Formula/svnui.rb').read_text())

    def test_reused_artifact_checksum_is_validated(self) -> None:
        self.run_script('--offline', '--no-publish')
        next((self.repo / 'dist/releases/0.4.2').glob('*.tar.gz')).write_text('tampered')
        self.run_script('--offline', success=False)
        self.assertFalse(self.state.exists())

    def test_host_only_stays_local(self) -> None:
        self.run_script('--offline', '--no-publish', '--host-only')
        manifest = json.loads((self.repo / 'dist/releases/0.4.2/manifest.json').read_text())
        self.assertEqual(len(manifest['targets']), 1)
        self.run_script('--offline', '--host-only', success=False)
        self.assertFalse(self.state.exists())

    def test_release_without_options_handles_empty_array_on_macos_bash(self) -> None:
        self.run_script('0.4.2', script='release.sh', extra={'ALLOW_ONLINE_BUILD': '1'})
        self.assertTrue(self.state.exists())
        self.assertTrue(any(call[0] == 'git' and 'push' in call for call in self.calls()))

    def test_release_bad_argument_is_rejected_before_version_write(self) -> None:
        head = self.git('rev-parse', 'HEAD')
        self.run_script('0.4.3', '--bad-option', script='release.sh', success=False)
        self.assertEqual(head, self.git('rev-parse', 'HEAD'))
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_bump_refuses_existing_tag_without_mutation(self) -> None:
        self.git('tag', '0.4.3')
        before = (self.repo / 'Cargo.toml').read_bytes()
        self.run_script('0.4.3', script='bump.sh', success=False)
        self.assertEqual(before, (self.repo / 'Cargo.toml').read_bytes())

    def test_bump_updates_only_local_package_and_lock(self) -> None:
        before = self.git('rev-parse', 'HEAD')
        self.run_script('0.4.3', script='bump.sh')
        self.assertNotEqual(before, self.git('rev-parse', 'HEAD'))
        self.assertEqual(self.git('rev-parse', '0.4.3'), self.git('rev-parse', 'HEAD'))
        self.assertIn('0.4.3', (self.repo / 'Cargo.lock').read_text())
        self.assertFalse(any(call[0] in ('cargo', 'rustup', 'gh', 'brew') for call in self.calls()))


if __name__ == '__main__':
    unittest.main()
