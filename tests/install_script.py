"""下载与原子安装回归：真实归档/校验，curl 与 uname 仅使用临时替身。"""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
STUB = r'''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if name == 'uname':
    print(os.environ.get('TEST_OS', 'Darwin') if args == ['-s'] else os.environ.get('TEST_ARCH', 'arm64'))
    raise SystemExit(0)
url = args[-1]
with open(os.environ['TEST_CALLS'], 'a') as log:
    log.write(json.dumps(args) + '\n')
assert url.startswith('https://github.com/jc-p/svnui/releases/')
assert args[args.index('--proto') + 1] == '=https'
assert args[args.index('--proto-redir') + 1] == '=https'
if url.endswith('/latest'):
    print(os.environ.get('TEST_LATEST', 'https://github.com/jc-p/svnui/releases/tag/0.7.1'), end='')
    raise SystemExit(0)
if os.environ.get('TEST_DOWNLOAD_FAIL') == '1':
    raise SystemExit(22)
source = pathlib.Path(os.environ['TEST_RELEASE']) / url.rsplit('/', 1)[1]
shutil.copyfile(source, args[args.index('--output') + 1])
'''


class Installer(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix='svnui installer tests ')
        self.root = Path(self.tmp.name)
        self.release = self.root / 'release'
        self.release.mkdir()
        self.bin = self.root / 'user bin'
        self.bin.mkdir()
        self.destination = self.bin / 'svnui'
        self.destination.write_bytes(b'OLD_BINARY')
        self.stubs = self.root / 'stubs'
        self.stubs.mkdir()
        for command in ['curl', 'uname']:
            path = self.stubs / command
            path.write_text(STUB)
            path.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{self.stubs}{os.pathsep}{os.environ["PATH"]}',
                        TEST_RELEASE=str(self.release), TEST_CALLS=str(self.root / 'calls'))
        self.package()

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def package(self, architecture: str = 'aarch64', *, version: str = '0.7.1',
                member: str = 'svnui', link: bool = False) -> Path:
        asset = self.release / f'svnui-0.7.1-{architecture}-apple-darwin.tar.gz'
        payload = f'#!/bin/sh\necho "svnui {version}"\n'.encode()
        with tarfile.open(asset, 'w:gz') as archive:
            entry = tarfile.TarInfo(member)
            entry.mode = 0o755
            if link:
                entry.type = tarfile.SYMTYPE
                entry.linkname = '/bin/sh'
                archive.addfile(entry)
            else:
                entry.size = len(payload)
                archive.addfile(entry, io.BytesIO(payload))
        digest = hashlib.sha256(asset.read_bytes()).hexdigest()
        (self.release / 'SHA256SUMS').write_text(f'{digest}  {asset.name}\n')
        return asset

    def run_installer(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(['bash', str(ROOT / 'install.sh'), '--bin-dir', str(self.bin), *args],
                              env=self.env, text=True, capture_output=True, timeout=20)

    def assert_preserved(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.destination.read_bytes(), b'OLD_BINARY')
        self.assertFalse(list(self.bin.glob('.svnui-install.*')))

    def test_latest_release_installs_and_reports_path_without_editing_shell(self) -> None:
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(subprocess.check_output([self.destination, '--version'], text=True).strip(), 'svnui 0.7.1')
        calls = [json.loads(line) for line in (self.root / 'calls').read_text().splitlines()]
        self.assertTrue(any(call[-1].endswith('/latest') for call in calls))
        self.assertIn('export PATH=', result.stdout)
        self.assertFalse(list(self.bin.glob('.svnui-install.*')))

    def test_intel_and_explicit_version_skip_latest_query(self) -> None:
        self.package('x86_64')
        self.env['TEST_ARCH'] = 'x86_64'
        result = self.run_installer('--version', '0.7.1')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = (self.root / 'calls').read_text()
        self.assertNotIn('/latest', calls)
        self.assertIn('x86_64-apple-darwin', calls)

    def test_invalid_platform_architecture_and_version_do_not_write(self) -> None:
        for overrides, args in [({'TEST_OS': 'Linux'}, []), ({'TEST_ARCH': 'riscv64'}, []),
                                ({}, ['--version', '../invalid']), ({}, ['--version'])]:
            with self.subTest(overrides=overrides, args=args):
                before = self.env.copy()
                self.env.update(overrides)
                self.assert_preserved(self.run_installer(*args))
                self.env = before
        self.assertFalse((self.root / 'calls').exists())

    def test_download_failure_and_unexpected_latest_redirect_preserve_old_binary(self) -> None:
        self.env['TEST_DOWNLOAD_FAIL'] = '1'
        self.assert_preserved(self.run_installer('--version', '0.7.1'))
        self.env.pop('TEST_DOWNLOAD_FAIL')
        self.env['TEST_LATEST'] = 'https://example.com/tag/0.7.1'
        self.assert_preserved(self.run_installer())

    def test_wrong_missing_and_duplicate_checksums_preserve_old_binary(self) -> None:
        sums = self.release / 'SHA256SUMS'
        original = sums.read_text()
        for checksum in ['0' * 64 + '  ' + original.split()[1] + '\n', '', original * 2]:
            with self.subTest(checksum=checksum):
                sums.write_text(checksum)
                self.assert_preserved(self.run_installer('--version', '0.7.1'))

    def test_archive_traversal_and_symlink_are_rejected(self) -> None:
        self.package(member='../escape')
        self.assert_preserved(self.run_installer('--version', '0.7.1'))
        self.package(link=True)
        self.assert_preserved(self.run_installer('--version', '0.7.1'))

    def test_wrong_binary_version_preserves_old_binary(self) -> None:
        self.package(version='0.7.0')
        self.assert_preserved(self.run_installer('--version', '0.7.1'))


if __name__ == '__main__':
    unittest.main()
