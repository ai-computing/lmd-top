# Regenerating the README demo

The animation at the top of the README (`assets/demo.gif`) is a live recording of lmd-top against a
real heterogeneous cluster (GPU, RBLN and RNGD nodes). `assets/demo.cast` is the same recording as
an [asciicast v2](https://docs.asciinema.org/manual/asciicast/v2/) file; play it with
`asciinema play assets/demo.cast` or upload it to any asciinema player.

Node names and addresses in both files are replaced before publishing: node names by public
aliases (`npu-node-1`, `control-node`, …), IPv4 addresses by TEST-NET values (`192.0.2.x`) and home
directories by `/home/<user>`.

## Steps

```bash
cargo build --release
LMD_DEMO_ALIASES=/path/outside/the/repo/aliases.txt bash scripts/record-demo.sh
```

Requirements: a kubectl context and lmd-top config that reach the cluster (the same ones `lmd-top`
uses), `python3`, and [agg](https://github.com/asciinema/agg) (`cargo install --locked agg`). No tmux
or asciinema recorder is needed.

The alias file maps each real name to its public alias, one `real -> alias` per line (`#` starts a
comment). Keep it outside the repository and never commit it:

```text
# real -> alias
my-gpu-host-01 -> gpu-node-1
/home/alice -> /home/<user>
```

## What the scripts do

| Script | Role |
|---|---|
| `scripts/record-demo.sh` | Runs the two steps below with a scratch `HOME`, history file and audit log, then renders the GIF with `agg --theme github-dark --font-size 14` |
| `scripts/record-demo.py` | Starts `lmd-top --mode observe` in a 120×34 pseudo-terminal, sends a fixed key script (sections `0`–`6`, `←`/`→` sub-tabs, `j`) and records the output with timestamps. It only switches views and moves the selection, so the cluster is not changed |
| `scripts/sanitize-cast.py` | Replays the raw recording through a small terminal model, replaces names on each rendered screen row (keeping table columns aligned, including names the TUI cut short with `…`), writes the result as row redraws and fails if any name from the alias file, private IPv4 address or home directory is left |

To change the tour, edit `SCRIPT` in `scripts/record-demo.py` (time in seconds, keys, caption).
Check the GIF before committing: the cluster state shown is whatever was running at recording time.
