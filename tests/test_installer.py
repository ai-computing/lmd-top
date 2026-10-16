"""Offline release installation tests: no network, cluster or real user config."""
import hashlib
import io
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / 'install.sh'


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / 'install with spaces'
        mock = self.root / 'mock'
        mock.mkdir()
        arch = {'arm64': 'aarch64', 'amd64': 'x86_64'}.get(platform.machine(), platform.machine())
        asset = f'lmd-top-v0.36.0-{arch}-{platform.system().lower()}'
        archive = self.root / 'release.tgz'
        with tarfile.open(archive, 'w:gz') as tar:
            data = b'#!/bin/sh\necho test-release\n'
            entry = tarfile.TarInfo(f'{asset}/lmd-top')
            entry.size = len(data)
            entry.mode = 0o755
            tar.addfile(entry, io.BytesIO(data))
        (self.root / 'release.sha256').write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  release.tgz\n')
        curl = mock / 'curl'
        curl.write_text('''#!/usr/bin/env python3
import os, sys, shutil
from pathlib import Path
args = sys.argv[1:]
root = Path(os.environ['INSTALL_FIXTURE'])
if '-o' not in args:
    print('{"tag_name":"v0.36.0"}')
else:
    dest = args[args.index('-o') + 1]
    checksum = any(a.endswith('.sha256') for a in args)
    if checksum and os.environ.get('FAIL_CHECKSUM'):
        sys.exit(22)
    shutil.copyfile(root / ('release.sha256' if checksum else 'release.tgz'), dest)
''')
        curl.chmod(0o755)
        self.env = dict(os.environ, PATH=str(mock) + os.pathsep + os.environ['PATH'], INSTALL_FIXTURE=str(self.root))

    def run_installer(self, *args):
        return subprocess.run(['bash', str(INSTALLER), *args], env=self.env, capture_output=True, text=True)

    def test_separate_options_in_any_order_and_atomic_upgrade(self):
        self.bin.mkdir()
        (self.bin / 'lmd-top').write_text('old release')
        result = self.run_installer('--bin-dir', str(self.bin), '--version', 'v0.36.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('test-release', (self.bin / 'lmd-top').read_text())
        self.assertTrue(os.access(self.bin / 'lmd-top', os.X_OK))
        self.assertEqual(list(self.bin.glob('.lmd-top.*')), [])

    def test_latest_and_equals_options(self):
        result = self.run_installer('--bin-dir=' + str(self.bin))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.bin / 'lmd-top').exists())

    def test_missing_or_bad_checksum_preserves_existing_binary(self):
        self.bin.mkdir()
        target = self.bin / 'lmd-top'
        target.write_text('keep me')
        for missing in [True, False]:
            if missing:
                self.env['FAIL_CHECKSUM'] = '1'
            else:
                self.env.pop('FAIL_CHECKSUM')
                (self.root / 'release.sha256').write_text('0' * 64 + '  release.tgz\n')
            result = self.run_installer('--version=v0.36.0', '--bin-dir', str(self.bin))
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(target.read_text(), 'keep me')

    def test_bad_arguments_fail_before_install(self):
        for args in [('--version',), ('--bin-dir',), ('--typo',), ('--version', '../bad'), ('--with-demo',), ('--version=',)]:
            self.assertEqual(self.run_installer(*args).returncode, 2, args)
        self.assertFalse(self.bin.exists())

    def test_source_build_honors_destination_and_lock(self):
        cargo = self.root / 'mock' / 'cargo'
        cargo.write_text("""#!/usr/bin/env python3
import sys
from pathlib import Path
args = sys.argv[1:]
assert '--locked' in args
root = Path(args[args.index('--root') + 1])
(root / 'bin').mkdir()
(root / 'bin' / 'lmd-top').write_text('source-build')
""")
        cargo.chmod(0o755)
        result = self.run_installer('--from-source', '--bin-dir', str(self.bin))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.bin / 'lmd-top').read_text(), 'source-build')
