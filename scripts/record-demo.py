#!/usr/bin/env python3
"""Record a live lmd-top session as an asciicast v2 file, driven by a fixed key script.

Runs lmd-top in a pseudo-terminal of a fixed size, sends keys on a timed schedule and writes
every byte the TUI prints with its timestamp. No tmux or asciinema needed (stdlib only).
The result contains real cluster names: run scripts/sanitize-cast.py on it before publishing.

    python3 scripts/record-demo.py --out /tmp/raw.cast -- lmd-top
"""
import argparse
import codecs
import fcntl
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

# (seconds after start, keys, caption): sections 0-3, ←/→ sub-tabs and O (sort direction). Views
# only; nothing here opens an action menu, so the run changes nothing in the cluster.
# Record while a model serves traffic (docs/DEMO.md): the tour stays longest on the views whose
# numbers move under load — Overview, Serving: Perf and Infra: Devices.
RIGHT = "\x1b[C"
SCRIPT = [
    (7.0, "1", "Traffic: Flow"),
    (12.0, "2", "Serving"),
    (16.0, RIGHT, "Serving: Perf (device utilisation over time)"),
    (24.0, "3", "Infra: Nodes"),
    (28.0, RIGHT, "Infra: Devices + ai-rm panel"),
    (29.5, "O", "sort busiest device first"),
    (37.0, "0", "Overview"),
    (44.0, None, "stop (ends on Overview so the GIF loops cleanly)"),
]


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("--cols", type=int, default=120)
    ap.add_argument("--rows", type=int, default=34)
    ap.add_argument("--warmup", type=float, default=6.0,
                    help="seconds before the key script starts (first data load); trim with sanitize-cast.py --start")
    ap.add_argument("--max-seconds", type=float, default=70.0)
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    cmd = a.cmd[1:] if a.cmd[:1] == ["--"] else a.cmd
    if not cmd:
        ap.error("command to record is required (after --)")

    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
    pid, fd = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", a.rows, a.cols, 0, 0))
        os.execvpe(cmd[0], cmd, env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", a.rows, a.cols, 0, 0))
    dec = codecs.getincrementaldecoder("utf-8")("replace")
    header = {"version": 2, "width": a.cols, "height": a.rows,
              "env": {"TERM": "xterm-256color", "SHELL": "/bin/bash"}, "title": "lmd-top"}
    start = time.monotonic()
    pending = [(t + a.warmup, keys, caption) for t, keys, caption in SCRIPT]
    with open(a.out, "w", encoding="utf-8") as out:
        out.write(json.dumps(header) + "\n")
        while True:
            now = time.monotonic() - start
            if now > a.max_seconds:
                os.kill(pid, signal.SIGTERM)
                break
            while pending and pending[0][0] <= now:
                _, keys, caption = pending.pop(0)
                print("%5.1fs  %s" % (now, caption), file=sys.stderr)
                if keys is None:  # stop without recording the terminal restore
                    pending = None
                    break
                os.write(fd, keys.encode())
            if pending is None:
                os.kill(pid, signal.SIGTERM)
                break
            try:
                r, _, _ = select.select([fd], [], [], 0.05)
            except InterruptedError:
                continue
            if not r:
                continue
            try:
                data = os.read(fd, 65536)
            except OSError:  # EIO: child closed the pty
                break
            if not data:
                break
            if b"\x1b[6n" in data:  # cursor position query: answer like a terminal would
                os.write(fd, b"\x1b[1;1R")
            text = dec.decode(data)
            if text:
                out.write(json.dumps([round(now, 6), "o", text], ensure_ascii=False) + "\n")
    for _ in range(30):  # give lmd-top 3 s to exit on SIGTERM, then kill it
        if os.waitpid(pid, os.WNOHANG)[0]:
            return
        time.sleep(0.1)
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)


if __name__ == "__main__":
    main()
