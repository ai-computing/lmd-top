# lmd-top

> **A terminal observability & operations tool for [llm-d](https://llm-d.ai) clusters.**
> See the whole serving stack — Gateway, EPP routing, model servers, and heterogeneous accelerators — on one screen, from a single static binary.

**English** · [한국어](README.ko.md)

[![release](https://img.shields.io/github/v/release/gspark-etri/lmd-top?logo=github)](https://github.com/gspark-etri/lmd-top/releases/latest)
![Rust](https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white)
![single static binary](https://img.shields.io/badge/single%20static%20binary-no%20C%20deps-success)
![for llm-d](https://img.shields.io/badge/for-llm--d-8839ef)
![views](https://img.shields.io/badge/correlated%20views-10-89b4fa)

`lmd-top` correlates the four layers of an llm-d serving stack — Gateway → EPP (Endpoint
Picker) → model server → infrastructure — across heterogeneous accelerators (NVIDIA GPU,
Rebellions RBLN, Furiosa RNGD, and host CPU). It reads your existing Prometheus and
Kubernetes; it stores no data of its own.

## Demo

*Demo recording omitted from the public history.*

*Demo recording omitted from the public export.*

## Highlights

- **Four layers on one screen.** The Gateway, EPP/InferencePool, model servers, and hardware are correlated, so you can answer *which model runs where, how requests are routed, and how load is distributed* without switching tools.
- **Heterogeneous accelerators, unified.** NVIDIA GPU, Rebellions RBLN, and Furiosa RNGD sit side by side. The exact GPU model and its VRAM are auto-detected, unified-memory parts (GB10, GH200) are recognized and marked `∪`, and per-node disk usage is tracked too.
- **EPP-aware.** It reads the EPP `ConfigMap` (active scorers, weights, and picker), visualizes routing decisions and per-pod queues, and diagnoses whether an HTTPRoute actually flows through the InferencePool or bypasses it.
- **Deployment lifecycle.** The Deploy view groups each model's compiled variants (tensor/pipeline parallelism, quantization, RBLN/Furiosa NPU options), shows which node and disk they live on, and highlights where there is free capacity to place them.
- **A rich terminal UI.** An LED device grid, a stacked VRAM bar, braille timelines, active alerting, log tailing, and a `scale` action — with four themes and understated animations, all in a single static Rust binary that has no C dependencies.

## Views

Switch views with the number keys `0`–`9`, or cycle with `Tab` / `Shift+Tab`.

| # | View | Shows |
|---|---|---|
| 0 | **Overview** | Cluster summary, LED device grid, VRAM bar, accelerators by kind/node, EPP path, models, and a one-line diagnosis |
| 1 | **Accel** | Per-device util / VRAM / temp / power with a trend; `⏎` opens the util & VRAM timeline |
| 2 | **Models** | Per-model accelerator/node, ready count, running/waiting, KV%, tok/s, route, and status |
| 3 | **EPP** | Scorers and weights, the picker, InferencePool endpoints, and request distribution |
| 4 | **Flow** | Gateway → HTTPRoute → backend → pods, with InferencePool/EPP/SLO and the EPP-bypass diagnosis; `⏎` jumps to the backend model |
| 5 | **Pods** | `llm-serving` pods (ready / phase / node / restarts) |
| 6 | **Perf** | Per-device history plus per-model p95 latency broken down QUEUE → PREFILL → DECODE → TPOT → E2E, tok/s, and queues; `⏎` opens p50/95/99 + timelines |
| 7 | **Deploy** | The model lifecycle: compiled variants (family → build, with options, `@node /path`, and status), deploy targets (free capacity per node), and catalog feasibility |
| 8 | **Events** | Kubernetes + llm-d events, newest first; `⏎` shows the full message |
| 9 | **Nodes** | Node health — CPU, memory, disk, load, and devices per node; press `⏎`, then `↑↓` to pick a device |

## Install

To build, you need a Rust toolchain and a C linker (`cc`/`gcc`) — that's it. The binary
links only glibc; there are no native or C-library dependencies (no OpenSSL, pkg-config, or
cmake). At runtime it needs `kubectl` with kubeconfig access and network reachability to
Prometheus; it never SSHes into accelerator nodes. A truecolor terminal with a font that
covers box-drawing and braille glyphs is recommended (otherwise run with `LMD_THEME=default`).
`xdg-open` is optional and only used by the `g` key.

```bash
git clone https://github.com/gspark-etri/lmd-top.git && cd lmd-top
./install.sh                 # installs any missing prereqs, then runs `cargo install`
#   ./install.sh --check     # report what's present/missing, install nothing
#   ./install.sh --with-demo # also install agg and regenerate the demo GIF
# by hand: cargo install --path .   (cargo fetches the Rust crates automatically)
```

## Usage

```bash
lmd-top                      # launch the TUI (permission mode: observe)
lmd-top --mode admin         # allow scale / rollout actions
lmd-top --json               # print machine-readable agent state (JSON)
lmd-top --doctor             # survey Prometheus: exporters, metric coverage, gaps
lmd-top --snapshot | --render | --cast   # headless text / CI render / demo asciicast
LMD_PROM=192.0.2.3:30090 LMD_NS=my-ns lmd-top   # point at another cluster
```

**Permission modes** (`--mode`, shown as a header badge) gate mutating actions:
`observe` (default, view only) → `debug` (adds logs, `l`) → `admin` (adds `scale`, with a
y/n confirmation) → `danger` (reserved). Admin actions always ask before applying.

**Keys.** `↑↓`/`kj` select a row, `Enter` drills into detail, `w` moves focus between panels
in multi-panel views, and `←→` step through items. `/` filters, `o` cycles the sort, `l`
shows logs, `s` scales, and `A` opens the alert history. `t` cycles the theme, `f` toggles
animations, `z` zooms, `Space` pauses, `g` opens Grafana, `?` shows help, and `q` quits.

**Environment.** `LMD_PROM`, `LMD_NS` (default `llm-serving`), and `LMD_GRAFANA` point it at
your cluster; `LMD_THEME` picks the startup theme (`soft`, `default`, `high-contrast`, or
`colorblind`); `LMD_W`/`LMD_H` set the `--render` size. An optional
`~/.config/lmd-top/lmd-top.yaml` customizes column order.

**Colors and glyphs.** Color encodes severity or identity, while state is carried by a
separate glyph (`●` up, `○` idle, `◐` pending, `⚠` throttling, `⊘` cordoned, `✗` down) so the
UI stays legible in the colorblind theme. Metrics that aren't present yet render as `–` and
fill in once the workload is up.

## Data path

lmd-top reads your existing stack and correlates it; it owns no data.

| Layer | Source | Examples |
|---|---|---|
| Accelerators / host | Prometheus | `DCGM_FI_DEV_*`, `RBLN_DEVICE_STATUS:*`, `furiosa_npu_*`, `node_*` |
| Model server | Prometheus | `vllm:*_latency_seconds_bucket`, `vllm:num_requests_*`, `vllm:*kv_cache*` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| Topology / status / actions | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool, InferenceObjective |

Data arrives on two tiers: a fast tier (~1 s) for accelerators and nodes, and a full
snapshot (~3 s) for everything else. It's pure Rust — Prometheus is queried over raw `tokio`
HTTP/1.0, and Kubernetes through `kubectl`.

## Status & roadmap

**Works today, with no traffic required.** All ten views; GPU/RBLN/RNGD and node/disk
monitoring with auto-detection and unified memory; the Flow topology and EPP-bypass
diagnosis; EPP ConfigMap introspection; active alerting; the `scale` and `logs` actions; the
Deploy view (compiled variants, storage node, deploy targets); the headless `--json`,
`--doctor`, `--snapshot`, and `--cast` modes; and themes, animation, zoom, and permission modes.

**Fills in once real traffic flows through the EPP and vLLM exposes metrics.** Per-model p95
latency breakdown, tok/s, per-pod queue distribution, KV%/TTFT/E2E, and EPP request
distribution. (The EPP weight `+`/`-` is a local weight-share simulation — it does not apply
to the cluster.)

**Planned.** Applied control-plane actions (endpoint drain, traffic/policy-weight apply, and
rollout, each as dry-run → confirm → audit); an EPP per-endpoint score debugger; and **NPU
compile & deploy automation** — from the Deploy view, compile a model for RBLN or Furiosa (as
a Kubernetes Job running the vendor toolchain) and deploy the artifact through ModelService,
gated by permission mode. See `ROADMAP.md` and `CHANGELOG.md`.

## Maturity

Verified against a live heterogeneous cluster (8 nodes; GB10, RBLN, and RNGD accelerators;
EPP, routes, and models live). It's experimental (0.x), so interfaces may still change.
