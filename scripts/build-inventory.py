#!/usr/bin/env python3
"""Record resolved Rust dependencies and toolchain identity (not a license audit)."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def inventory(root, target=None):
    rustc = subprocess.check_output(['rustc', '-vV'], cwd=root, text=True).strip()
    host = next(line.split(': ', 1)[1] for line in rustc.splitlines()
                if line.startswith('host: '))
    target = target or host
    metadata = json.loads(subprocess.check_output([
        'cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
        '--filter-platform', target,
    ], cwd=root, text=True))
    resolved = {node['id']: node for node in metadata['resolve']['nodes']}
    packages = []
    for package in metadata['packages']:
        if package['id'] not in resolved:
            continue
        packages.append({
            'name': package['name'],
            'version': package['version'],
            'source': package['source'] or 'local',
            'declared_license': package.get('license'),
            'license_file_declared': package.get('license_file') is not None,
            'enabled_features': sorted(resolved[package['id']]['features']),
        })
    return {
        'schema': 'lmd-top-rust-build-inventory-v1',
        'target': target,
        'rustc': rustc,
        'cargo': subprocess.check_output(['cargo', '--version'], cwd=root, text=True).strip(),
        'cargo_lock_sha256': hashlib.sha256((root / 'Cargo.lock').read_bytes()).hexdigest(),
        'scope': 'Cargo target-filtered resolution, including build/test dependencies; excludes Go plugin and external SDK/runtime images',
        'packages': sorted(packages, key=lambda p: (p['name'], p['version'], p['source'])),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    result = inventory(root, args.target)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')
    print(f"Recorded {len(result['packages'])} packages for {result['target']}")


if __name__ == '__main__':
    main()
