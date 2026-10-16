#!/usr/bin/env python3
"""Opt-in Unix PTY smoke check; no Kubernetes or Prometheus access required."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/lmd-top'))
    parser.add_argument('--slow-zoo', action='store_true',
                        help='Verify input/quit while fake vendor requests are blocked')
    parser.add_argument('--manifest-review', choices=['success', 'conflict'],
                        help='Exercise prepared manifest confirmation and audit with fake kubectl')
    args = parser.parse_args()
    if args.slow_zoo and args.manifest_review:
        parser.error('choose one pending-operation scenario')
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='lmd-terminal-') as directory:
        # Prevent inherited kubeconfig/context from contacting a real cluster.
        kubectl = Path(directory) / 'kubectl'
        kubectl.write_text('#!/bin/sh\nexit 1\n')
        kubectl.chmod(0o700)
        if args.slow_zoo:
            curl = Path(directory) / 'curl'
            curl.write_text("""#!/usr/bin/env python3
import os, pathlib, time
root = pathlib.Path(os.environ['LMD_TERMINAL_TEST_DIR'])
(root / ('started.' + str(os.getpid()))).write_text('started')
time.sleep(20)
(root / ('finished.' + str(os.getpid()))).write_text('finished')
print('[]')
""")
            curl.chmod(0o700)
        if args.manifest_review:
            kubectl.write_text(r"""#!/usr/bin/env python3
import json, os, pathlib, sys, time
root = pathlib.Path(os.environ['LMD_TERMINAL_TEST_DIR'])
args = sys.argv[1:]
if args[:2] == ['config', 'view']:
    print(json.dumps({'current-context':'test','contexts':[{'name':'test','context':{'cluster':'test'}}], 'clusters':[{'name':'test','cluster':{'server':'https://invalid.test'}}]}))
    sys.exit(0)
assert args[0] == '--kubeconfig'
args = args[2:]
if '-f' not in args:
    if '-o' in args and args[args.index('-o')+1] == 'json':
        print('{"items":[]}')
    sys.exit(0)
body = sys.stdin.read()
with (root / 'requests.jsonl').open('a') as output:
    output.write(json.dumps({'args':args,'body':body})+'\n')
current = {'apiVersion':'v1','kind':'Namespace','metadata':{'name':'review-test','uid':'reviewed-uid','resourceVersion':'42','labels':{'keep':'unchanged'}}}
if args[0] == 'get':
    (root / 'preparing').touch()
    deadline = time.monotonic()+20
    while not (root / 'release').exists():
        if time.monotonic() > deadline: sys.exit(1)
        time.sleep(.05)
    print(json.dumps(current))
elif args[0] == 'apply':
    assert '--dry-run=server' in args
    print(json.dumps(current))
elif args[0] == 'replace':
    assert json.loads(body) == current
    if '--dry-run=server' not in args and os.environ['LMD_TERMINAL_REVIEW_CASE'] == 'conflict':
        print('Conflict: resourceVersion changed', file=sys.stderr)
        sys.exit(1)
    print('namespace/review-test replaced')
else:
    raise AssertionError('unexpected manifest verb: '+repr(args))
""")
        master, slave = pty.openpty()
        before = termios.tcgetattr(slave)
        output = bytearray()
        process = None

        def resize(width, height):
            fcntl.ioctl(slave, termios.TIOCSWINSZ,
                        struct.pack('HHHH', height, width, 0, 0))

        def drain(seconds):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        output.extend(os.read(master, 65536))
                    except OSError:
                        break

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

        try:
            resize(80, 24)
            env = dict(os.environ, TERM='xterm-256color',
                       PATH=directory + os.pathsep + os.environ.get('PATH', ''),
                       KUBECONFIG=str(Path(directory) / 'missing-config'),
                       LMD_PROM='127.0.0.1:1',
                       LMD_TERMINAL_TEST_DIR=directory)
            if args.manifest_review:
                env.update(LMD_NS='review-test', LMD_AUDIT=str(Path(directory) / 'audit.log'),
                           LMD_TERMINAL_REVIEW_CASE=args.manifest_review)
            process = subprocess.Popen(
                [str(binary), '--mode', 'admin' if args.manifest_review else 'observe'], env=env,
                stdin=slave, stdout=slave, stderr=slave,
                preexec_fn=controlling_terminal)
            deadline = time.monotonic() + 10
            while b'\x1b[?1049h' not in output and time.monotonic() < deadline:
                drain(0.1)
                if process.poll() is not None:
                    raise RuntimeError('Program exited before entering alternate screen')
            if b'\x1b[?1049h' not in output:
                raise RuntimeError('Terminal startup exceeded 10 seconds')
            for width, height in [(40, 12), (140, 45), (80, 24)]:
                resize(width, height)
                os.kill(process.pid, signal.SIGWINCH)
                drain(0.5)
            quit_started = None
            if args.slow_zoo:
                os.write(master, b':zoo')
                drain(0.3)
                os.write(master, b'\r')
                drain(0.3)
                os.write(master, b'r')
                deadline = time.monotonic() + 5
                while not list(Path(directory).glob('started.*')) and time.monotonic() < deadline:
                    drain(0.1)
                if not list(Path(directory).glob('started.*')):
                    raise RuntimeError('Zoo refresh did not start; test would not exercise a pending read')
                drain(0.3)
                started_count = len(list(Path(directory).glob('started.*')))
                os.write(master, b'rrr')
                drain(0.3)
                if len(list(Path(directory).glob('started.*'))) != started_count:
                    raise RuntimeError('Repeated refresh launched duplicate workers')
                quit_started = time.monotonic()
            if args.manifest_review:
                def until(predicate, description):
                    deadline = time.monotonic() + 10
                    while not predicate() and time.monotonic() < deadline:
                        drain(.1)
                    if not predicate():
                        raise RuntimeError('Timed out: ' + description + '; terminal tail: ' + repr(bytes(output[-6000:])))

                root = Path(directory)
                def requests():
                    path = root / 'requests.jsonl'
                    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

                # Setup's missing namespace action provides a real generated
                # manifest without any special application test entrypoint.
                drain(2)
                for key in (b':setup', b'\r', b'j', b'\r', b'a'):
                    os.write(master, key)
                    drain(.4)
                until(lambda: (root / 'preparing').exists(), 'manifest preparation started')
                os.write(master, b'y')
                drain(.4)
                if any(call['args'][0] != 'get' for call in requests()):
                    raise RuntimeError('Confirmation executed before preparation completed')
                (root / 'release').touch()
                until(lambda: any(c['args'][0] == 'apply' for c in requests()), 'server merge completed')
                drain(.5)
                os.write(master, b'p')
                until(lambda: b'reviewed-uid' in output, 'reviewed identity visible in target panel')
                os.write(master, b'\x1b[F')  # End: exercise scrolling in target review.
                drain(.2)
                os.write(master, b'\x1b')
                drain(.3)
                os.write(master, b'y')
                until(lambda: (root / 'audit.log').exists(), 'confirmed operation audited')
                fields = (root / 'audit.log').read_text().strip().split('\t')
                if fields[2] != 'apply' or fields[4] != ('ok' if args.manifest_review == 'success' else 'FAIL'):
                    raise RuntimeError('Unexpected audit outcome: '+repr(fields[:6]))
                targets = json.loads(fields[11])
                if targets[0]['uid'] != 'reviewed-uid' or targets[0]['resource_version'] != '42':
                    raise RuntimeError('Audit lost the confirmed identity')
                calls = requests()
                if [c['args'][0] for c in calls] != ['get', 'apply', 'replace', 'replace']:
                    raise RuntimeError('Unexpected request order/rebase: '+repr(calls))
                if '--dry-run=server' not in calls[2]['args'] or '--dry-run=server' in calls[3]['args']:
                    raise RuntimeError('Expected validation before real conditional update')
                drain(.3)
                os.write(master, b'\x1b')  # Close failure details, if any.
                drain(.3)
            for key in (b'?', b'\x1b', b'q', b'y'):
                os.write(master, key)
                drain(0.5)
            deadline = time.monotonic() + 15
            while process.poll() is None and time.monotonic() < deadline:
                drain(0.1)
            if process.poll() is None:
                raise RuntimeError('Confirmed quit exceeded 15 seconds')
            drain(0.2)
            checks = {
                'exit_success': process.returncode == 0,
                'alternate_enter': b'\x1b[?1049h' in output,
                'alternate_leave': b'\x1b[?1049l' in output,
                'termios_restored': before == termios.tcgetattr(slave),
                'no_panic': b'panicked' not in output,
            }
            if args.slow_zoo:
                checks['quit_while_read_pending'] = time.monotonic() - quit_started < 5
                checks['read_not_completed'] = not list(Path(directory).glob('finished.*'))
            if args.manifest_review:
                checks['manifest_review_' + args.manifest_review] = True
            print(json.dumps(checks))
            if not all(checks.values()):
                raise RuntimeError('Terminal lifecycle check failed')
        finally:
            if process is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            # Curl has its own process group; clean test descendants even if a
            # regression prevents normal runtime cancellation from reaping them.
            for marker in Path(directory).glob('started.*'):
                try:
                    os.killpg(int(marker.name.split('.')[1]), signal.SIGKILL)
                except ProcessLookupError:
                    pass
            os.close(master)
            os.close(slave)


if __name__ == '__main__':
    main()
