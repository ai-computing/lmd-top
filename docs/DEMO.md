# Regenerating the README demo

The animation at the top of the README (`assets/demo.gif`) is a live recording of lmd-top against a
real heterogeneous cluster (GPU, RBLN and RNGD nodes). `assets/demo.cast` is the same recording as
an [asciicast v2](https://docs.asciinema.org/manual/asciicast/v2/) file; play it with
`asciinema play assets/demo.cast` or upload it to any asciinema player.

Node names and addresses in both files are replaced before publishing: node names by public
aliases (`npu-node-1`, `control-node`, …), IPv4 addresses by TEST-NET values (`192.0.2.x`) and home
directories by `/home/<user>`.

## Steps

1. **Serve a model and put load on it.** The demo is only useful while numbers move. Scale up a
   model you can spare accelerators for, wait until its pods are Ready, then drive a steady stream
   of requests through the gateway route for the whole recording (about 1 req/s is enough). For
   example, with [rm-bench](https://github.com/ai-computing/rm-bench) replaying a real conversation
   trace:

   ```bash
   rm-bench e2e --scenario trace-azure-conv --api completions \
     --gateway http://<gateway>/<route>/completions --model <model-id> \
     --speedup 0.85 --max-total-tokens 512 --duration 1500
   ```

   Start the load a few minutes before recording so the rate windows fill. Readiness-probe
   failures while the pods start are Warning events, and the Overview status line counts them until
   they expire (the API server keeps events for one hour by default), so record after that or
   accept the warning.

2. **Record.**

   ```bash
   cargo build --release
   LMD_DEMO_ALIASES=/path/outside/the/repo/aliases.txt bash scripts/record-demo.sh
   ```

3. **Restore** the cluster: stop the load and scale the model back to where it was.

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
| `scripts/record-demo.py` | Starts `lmd-top --mode observe` in a 120×34 pseudo-terminal, sends a fixed key script (sections `0`–`3`, `←`/`→` sub-tabs, `O` to sort the busiest device first) and records the output with timestamps. It only switches views and sort order, so the cluster is not changed |
| `scripts/sanitize-cast.py` | Replays the raw recording through a small terminal model, replaces names on each rendered screen row (keeping table columns aligned, including names the TUI cut short with `…`), writes the result as row redraws and fails if any name from the alias file, private IPv4 address or home directory is left |

To change the tour, edit `SCRIPT` in `scripts/record-demo.py` (time in seconds, keys, caption). The
tour stays longest on Overview, Serving: Perf and Infra: Devices, where utilisation, power and
throughput change under load.
Check the GIF before committing: the cluster state shown is whatever was running at recording time.

The request rate, TTFT and E2E columns in the header and in Serving: Perf come from vLLM or
Furiosa-LLM metrics in Prometheus, and EPP routing decisions from the endpoint picker's metrics. A
model server that exports neither (for example a custom prefill/decode worker that reports only
`airm_*` series) still shows as serving, with device utilisation and power moving, but those
columns stay `–`; its `airm_*` values appear in the ai-rm panel under Infra: Devices.
