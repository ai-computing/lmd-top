"""Run the shell recipe with recording commands; no packages, files or models installed."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

RECIPE = Path(__file__).resolve().parents[1] / 'assets/recipes/furiosa-compile.sh'


class FuriosaRecipeTests(unittest.TestCase):
    def run_recipe(self, failure=False):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ['apt-get', 'mkdir', 'cp']:
                p = root / name
                p.write_text('#!/bin/sh\nexit 0\n')
                p.chmod(0o755)
            fxb = root / 'fxb'
            fxb.write_text('#!/usr/bin/python3\nimport json,os,sys\n'
                           'open(os.environ["RECORD"],"w").write(json.dumps(sys.argv[1:]))\n'
                           'sys.exit(int(os.environ["FAIL"]))\n')
            fxb.chmod(0o755)
            python = root / 'python3'
            python.write_text('#!/bin/sh\nif [ "$1" = "-" ]; then /bin/cat >/dev/null; '
                              'else /usr/bin/touch "$PUBLISHED"; fi\n')
            python.chmod(0o755)
            result = subprocess.run(['sh', str(RECIPE)], env={
                'PATH': f'{root}:/usr/bin:/bin', 'RECORD': str(root / 'args'),
                'PUBLISHED': str(root / 'published'), 'FAIL': '7' if failure else '0',
                'MODEL_ID': 'org/model with spaces', 'OUTPUT': '/unused/output',
                'TP': '8', 'PP': '2', 'MAX_LEN': '4096',
                'FXB_CONCURRENCY': '3', 'FXB_OPTIM_LEVEL': 'O0',
            }, capture_output=True, text=True, timeout=5)
            return result, json.loads((root / 'args').read_text()), (root / 'published').exists()

    def test_selected_options_reach_fxb_without_shell_splitting(self):
        result, args, published = self.run_recipe()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(args, ['build', 'org/model with spaces', '/work/out/model',
                               '-tp', '8', '-pp', '2', '--max-model-len', '4096',
                               '--concurrency', '3', '--optim-level', 'O0'])
        self.assertTrue(published)

    def test_failed_compile_never_publishes(self):
        result, _, published = self.run_recipe(failure=True)
        self.assertEqual(result.returncode, 7)
        self.assertFalse(published)


if __name__ == '__main__':
    unittest.main()
