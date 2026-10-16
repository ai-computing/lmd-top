"""Target-filtered dependency inventory, without network or toolchain mutation."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'build_inventory', Path(__file__).resolve().parents[1] / 'scripts/build-inventory.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class InventoryTests(unittest.TestCase):
    def test_only_resolved_target_packages_and_no_local_paths(self):
        metadata = {
            'resolve': {'nodes': [{'id': 'root', 'features': ['z', 'a']}]},
            'packages': [
                {'id': 'root', 'name': 'tool', 'version': '1.0', 'source': None,
                 'license': 'Apache-2.0', 'manifest_path': '/private/work/Cargo.toml'},
                {'id': 'other-platform', 'name': 'unused', 'version': '2', 'source': None},
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'Cargo.lock').write_text('locked dependencies\n')
            with patch.object(MODULE.subprocess, 'check_output', side_effect=[
                'rustc test\nhost: host-platform\n', json.dumps(metadata), 'cargo test\n'
            ]) as command:
                result = MODULE.inventory(root, 'chosen-platform')
        self.assertEqual(result['target'], 'chosen-platform')
        self.assertEqual(len(result['packages']), 1)
        self.assertEqual(result['packages'][0]['enabled_features'], ['a', 'z'])
        self.assertEqual(result['packages'][0]['source'], 'local')
        self.assertNotIn('/private/work', json.dumps(result))
        args = command.call_args_list[1].args[0]
        self.assertIn('--locked', args)
        self.assertIn('--offline', args)
        self.assertEqual(args[-2:], ['--filter-platform', 'chosen-platform'])
        self.assertEqual(len(result['cargo_lock_sha256']), 64)

    def test_metadata_failure_does_not_return_a_partial_inventory(self):
        with patch.object(MODULE.subprocess, 'check_output', side_effect=[
            'rustc test\nhost: host-platform\n', MODULE.subprocess.CalledProcessError(1, 'cargo')
        ]):
            with self.assertRaises(MODULE.subprocess.CalledProcessError):
                MODULE.inventory(Path('.'))
