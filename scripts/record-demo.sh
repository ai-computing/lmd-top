#!/usr/bin/env bash
# Regenerate the README demo (assets/demo.cast, assets/demo.gif) from a live cluster.
#
#   cargo build --release
#   LMD_DEMO_ALIASES=/path/outside/the/repo/aliases.txt bash scripts/record-demo.sh
#
# lmd-top runs in observe mode with the key script in scripts/record-demo.py (views and selection
# only), so the cluster is not changed. The run uses a scratch HOME, history and audit log, and your
# usual kubeconfig and lmd-top config. The alias file maps real names to public ones, one
# `real -> alias` per line; keep it out of git. Needs python3 and agg (cargo install --locked agg).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

: "${LMD_DEMO_ALIASES:?set LMD_DEMO_ALIASES to an untracked alias file (real -> alias per line)}"
BIN=${LMD_BIN:-target/release/lmd-top}
for tool in python3 agg; do
  command -v "$tool" >/dev/null 2>&1 || { echo "Missing required tool: $tool" >&2; exit 2; }
done
[ -x "$BIN" ] || { echo "No lmd-top binary at $BIN: run cargo build --release or set LMD_BIN" >&2; exit 2; }
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/home"
export KUBECONFIG=${KUBECONFIG:-$HOME/.kube/config}
export LMD_CONFIG=${LMD_CONFIG:-$HOME/.config/lmd-top/lmd-top.yaml}
HOME="$work/home" LMD_HISTORY="$work/history.jsonl" LMD_AUDIT="$work/audit.log" \
  python3 scripts/record-demo.py --cols 120 --rows 34 --warmup 6 --out "$work/raw.cast" -- "$BIN" --mode observe

# Replays the raw cast, replaces names/addresses and fails if any real name is left.
# --start drops the first-load screen ("connecting…") recorded during the warm-up.
python3 scripts/sanitize-cast.py --start 6 "$work/raw.cast" assets/demo.cast
agg --theme github-dark --font-size 14 --idle-time-limit 2 --last-frame-duration 4 \
  assets/demo.cast assets/demo.gif
ls -l assets/demo.cast assets/demo.gif
