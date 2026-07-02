# lmd-top

> **A terminal observability & operations tool for [llm-d](https://llm-d.ai) clusters.**
> `k9s` navigation · `all-smi` accelerator cards · **first-class EPP routing awareness** — one screen, one static binary.

![Rust](https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white)
![single static binary](https://img.shields.io/badge/single%20static%20binary-no%20C%20deps-success)
![for llm-d](https://img.shields.io/badge/for-llm--d-8839ef)
![views](https://img.shields.io/badge/correlated%20views-10-89b4fa)

`lmd-top` **correlates all four layers** of an llm-d serving stack —
`Gateway → EPP (Endpoint Picker) → Model Server → Infrastructure` — for
**heterogeneous accelerator** fleets (NVIDIA GPU · Rebellions RBLN · Furiosa RNGD · host CPU).
It reads your existing Prometheus + Kubernetes; it owns no data of its own.

## Demo

*Demo recording omitted from the public export.*
*Demo recording omitted from the public history.* -->

<sub>Soft (Catppuccin) theme · live braille timelines · cross-layer drill-down.</sub>

```
⠙ lmd-top [observe]   llm-d · 8 nodes   ⌂ gw 192.0.2.6 ●   · updated 2s ago
● SERVING 5/11   req/s 6.2  TTFT 92ms  E2E 0.8s  │ accel 9/14 busy  VRAM 67%  ⚡409W  ⚠1 alert
⇥  0:Overview  1:Accel  2:Models  3:EPP  4:Flow  5:Pods  6:Perf  7:Launch  8:Events  9:Nodes
╭ Cluster ───────────────────────────────────────────────────────────────────────────────╮
│ 14 accel  GB10×2 RBLN×4 RNGD×8 │ util 41% temp 52°C │ VRAM 489/735GB 67% ⚡409W │ 5/11    │
│ VRAM  █████│████│████│████│████│████│█░░░│░░░░│░░░░│░░  489/735GB used                     │
│ GB10 ● ●   RBLN ● ● ● ●   RNGD ● ● ● ● ● ● ● ●                                             │
╰───────────────────────────────────────────────────────────────────────────────────────────╯
╭ Status ──────────────────────────────────────────────────────────────────────────────────╮
│ ● 5 models serving, accelerators have headroom                                            │
╰───────────────────────────────────────────────────────────────────────────────────────────╯
╭ Accelerators (by kind / node) ────────────────────────────────────────────────────────────╮
│ ● GB10×1 @dgx-spark0   ●●●●●○○○○○  47%  mem ●●●●●●●●●●  124/131GB  trend ▁▂▄▅▆▅▄▃            │
│ ● RBLN×4 @etri-001     ●●●○○○○○○○  31%  mem ●●●●●●●●··   54/ 68GB  trend ▂▃▃▄▃▂▁▁            │
╰───────────────────────────────────────────────────────────────────────────────────────────╯
 ↑↓ sel  ⏎ detail  / filter  o sort  l logs  t theme  f anim  z zoom  ? help  q quit
```

---

## Why lmd-top?

The llm-d ecosystem has **no live, operator-facing terminal tool** — only Grafana web
dashboards, benchmark harnesses, and `helm`/`kubectl`. `lmd-top` fills that gap, and
uniquely **observes and explains EPP routing decisions**.

| | Sees | llm-d / EPP awareness | Accelerators | K8s actions | Terminal |
|---|---|---|---|---|---|
| `k9s` | K8s objects | ❌ | ❌ | ✅ | ✅ |
| `all-smi` | Infra (accelerators) only | ❌ | ✅✅ | ❌ | ✅ |
| `llmtop` | single-host psutil | ❌ | ⚠️ | ❌ | ✅ |
| Grafana | all-layer metrics | ⚠️ | ✅ | ❌ | ❌ web |
| **lmd-top** | **4-layer correlation** | ✅✅ EPP `Filter→Score→Pick` | ✅ | ✅ | ✅ |

---

## Highlights

- **Four layers, one screen.** Gateway, EPP/InferencePool, model servers, and hardware
  are correlated so you can answer *"which model runs where, how requests are routed,
  and how load is distributed."*
- **Heterogeneous accelerators, unified.** NVIDIA GPU (`DCGM_*`), Rebellions RBLN
  (`RBLN_DEVICE_STATUS:*`), and Furiosa RNGD (`furiosa_npu_*`) are shown side by side —
  vendor identity by color, health by glyph. The exact GPU model (A100 / GB10 / H100 …)
  and its total VRAM are **auto-detected** from DCGM (`modelName` / `FB_TOTAL`), not
  hardcoded. **Unified-memory** parts (GB10 / GH200 / GB200) are recognized — their memory
  reflects the host-shared pool and is marked `∪`.
- **EPP-aware.** Introspects the EPP `ConfigMap` (active scorers, weights, picker),
  visualizes routing decisions and per-pod queues, and **auto-diagnoses whether an
  HTTPRoute goes through the InferencePool (EPP) or bypasses it** (a common misconfig
  that leaves EPP metrics empty).
- **all-smi-style visuals.** Per-device gauges, inline sparklines, btop/nvtop-style
  **area-fill timelines**, an at-a-glance **LED device grid**, and a **stacked VRAM
  composition bar** (by vendor).
- **Active alerting.** Threshold/health conditions (throttle, not-alive, hot, node
  NotReady/cordon/pressure, pod restarts/Failed) trigger a summary-bar flash + a toast,
  and are collected into an **alert history** (`A`).
- **Operator ergonomics.** Row selection with scrollbars & position counters, substring
  filtering, sorting, drill-down detail, pod/model **logs overlay**, `scale` action,
  a **data-freshness clock**, responsive tabs, focus highlight on the active pane, a
  **zoom/focus** mode, tasteful **animations** (toggle with `f`), and four themes —
  **soft (Catppuccin, default)** / classic / high-contrast / **colorblind-safe**.
- **Pure Rust, single static binary.** No TLS/heavy HTTP crates: Prometheus is queried
  over raw `tokio` HTTP/1.0, Kubernetes via `kubectl`. Nothing to install on GPU nodes.

---

## Views

Ten correlated views — switch with the top number keys (`0`–`9`) or `Tab`:

| # | View | What it shows |
|---|---|---|
| **0** | **Overview** | Cluster Σ summary · **LED device grid** · **VRAM composition bar** · accelerators by kind/node · EPP path & pools · models table · **one-line cross-layer diagnosis** |
| 1 | **Accel** | Per-device rows: util bar / VRAM / temp / power + inline util trend. GPU · RBLN · RNGD unified. `⏎` → full util/VRAM timeline |
| 2 | **Models** | Per-model accelerator/node · ready · running/waiting · KV% · tok/s · routing path · status |
| 3 | **EPP** | Active scorers & weights (ConfigMap introspect) + picker + InferencePool endpoints + **request distribution** (routing decisions) |
| 4 | **Flow** | **Whole topology at a glance** — Gateway → HTTPRoute → backend (model status/accelerator/node) + InferencePool/EPP/**SLO** (InferenceObjective) + autoscalers + **EPP-bypass diagnosis**. `⏎` → backend model detail |
| 5 | **Pods** | `llm-serving` pod status (ready / phase / node / restarts) |
| 6 | **Perf** | **For EPP policy tuning** — system timelines + per-model p95 latency broken down **QUEUE → PREFILL(P) → DECODE(D) → TPOT → E2E** + preemptions, tok/s, per-pod queue distribution. *Lists every launched model (idle ones show `–`).* |
| 7 | **Launch** | Model **catalog** × live accelerator inventory → placement feasibility (`✓` ready / `⚙` needs-artifact / `✗` no-capacity). Read-only; catalog = `catalog/models.yaml` |
| 8 | **Events** | Unified k8s + llm-d events (newest first), warnings highlighted |
| 9 | **Nodes** | Node health / placement — status · kubelet · CPU · load · memory · accelerators per node |

> **Flow** answers *where does each model run, how is it routed, and does traffic actually
> pass through the EPP?* **Perf** gathers the latency/token/distribution signals you need
> to design EPP scorer policy (populated once EPP-path traffic + vLLM metrics are present).

---

## Install

### Prerequisites

- **Rust** toolchain (`rustup`) + a **C linker** (`gcc`/`cc`), required for Rust linking:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  sudo apt-get install -y build-essential      # gcc/cc linker
  ```
- Runtime: `kubectl` (with kubeconfig access) + reachability to Prometheus.
  **No SSH to accelerator nodes is needed** — everything goes through Prometheus.

### Build & install

```bash
git clone https://github.com/gspark-etri/lmd-top.git && cd lmd-top
cargo install --path .        # → ~/.cargo/bin/lmd-top
# or just build:
cargo build --release         # → target/release/lmd-top
```

---

## Usage

```bash
lmd-top                       # launch the TUI (permission mode: observe)
lmd-top --mode admin          # allow scale/rollout actions (see Permission modes)
lmd-top --snapshot            # collect once, print text (headless / debug)   [alias: -s]
lmd-top --json                # collect once, print machine-readable agent state (JSON)
lmd-top --doctor              # survey Prometheus: exporters, metric coverage, gaps, new signals
lmd-top --render              # render every view to text via TestBackend (CI / verification)

# point at a different cluster / namespace
LMD_PROM=192.0.2.3:30090 LMD_NS=my-ns lmd-top
```

### Permission modes

Mutating actions are gated by a startup mode (`--mode observe|debug|admin|danger`,
default `observe`), shown as a badge in the header. This prevents fat-finger accidents
in a shared cluster.

| Mode | Allows | Gated keys |
|---|---|---|
| **observe** *(default)* | view only | — |
| **debug** | + logs / dry-run | `l` |
| **admin** | + scale / rollout | `s` |
| **danger** | + delete / force | *(future)* |

Admin+ mutating actions (e.g. `s` scale) ask for a `y`/`n` confirmation before applying.

### Agent state (JSON)

`lmd-top --json` prints a curated, machine-readable state tree (schema
`lmd-top/agent-state/v1`) so an AI agent can consume cluster/accelerator/model/pool
status, `diagnosis`, `alerts`, and the available `actions` (each with a `risk` level and
`requires_confirmation` flag) without scraping the terminal. `NaN` metrics serialize as
`null`. This is the machine-readable half of a human-in-the-loop console.

### Diagnostics (`--doctor`)

`lmd-top --doctor` surveys Prometheus and prints: the accelerator/host **exporters**
detected (job labels), a **coverage table** of every metric lmd-top reads (present/absent,
with the concrete impact of each missing one — e.g. "EPP metrics missing → EPP views
empty", "FB_TOTAL missing → unified-mem falls back to host"), and a list of **unused
accelerator metrics present in the cluster** — candidate new signals to wire. Use it to
answer *"why is this view empty?"* and *"what new metrics does this hardware expose?"*
without hand-writing PromQL.

### Keybindings

| Key | Action |
|---|---|
| `0`–`9` | switch view |
| `Tab` | next view |
| `↑`/`↓` (or `k`/`j`), mouse scroll | select row |
| `Enter` | **drill-down detail** (accel · model · pod · node · event; in **Flow** → backend model; in **Perf** → p50/p95/p99 + timelines) |
| `←`/`→` | previous / next item (in a node detail, `↑`/`↓` picks a device) |
| `/` | **filter** (substring) — type, then Enter/Esc |
| `o` | cycle **sort** (Accel: util/temp/mem/name · Models: name/status/ready · Pods: name/phase/restarts) |
| `l` | **logs** overlay for selected pod/model (scroll, `r` refresh) |
| `s` | **scale** selected model (desired 0↔1 toggle) |
| `A` | **alert history** overlay (threshold / health events) |
| `t` | cycle **theme** (soft / classic / high-contrast / colorblind-safe) |
| `f` | toggle **animations** on/off |
| `g` | open **Grafana** dashboard in browser |
| `z` | **zoom / focus** (hide header + tabs, maximize body) |
| `Space` | **pause** updates (freeze data for reading) |
| `Esc` | **back only** (close detail / filter / zoom — does *not* quit) |
| `?` | help / color legend overlay |
| `q` | quit |

### Semantic colors & glyphs

Color encodes **severity** or **identity**; state is carried by a separate **glyph**
(so the two never collide, and it stays legible in the colorblind theme):

| Element | Meaning |
|---|---|
| 🟢 green | healthy / low load / serving |
| 🟡 yellow | warning / mid load / pending / throttling |
| 🔴 red | critical / high load / error / device down / **active alert** |
| 🔵 cyan | accent / headers / interactive values |
| ⚫ dark gray | idle / absent (`–`) / labels |
| vendor color | GPU = green · **RBLN = magenta** · RNGD = cyan |
| glyphs | `●` up/healthy · `○` idle/scaled-0 · `◐` pending · `⚠` throttle · `⊘` cordoned · `✗` down |
| thresholds | util `>85`🔴 `>60`🟡 · mem `>90`🔴 `>70`🟡 · temp `>80`🔴 `>60`🟡 |

Metrics that aren't present yet (workload off) render as `–`/`offline` and fill in
automatically once the workload comes up. The header shows a **freshness clock**
(`updated Ns ago`, turning yellow when data goes stale).

### Configuration (optional) — `~/.config/lmd-top/lmd-top.yaml`

Customize column visibility/order:

```yaml
columns:
  models: [name, accel, status, tps]   # only these columns, in this order (default: all)
```

### Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `LMD_PROM` | `192.0.2.5:30090` | Prometheus `host:port` (plain HTTP) |
| `LMD_NS` | `llm-serving` | target namespace |
| `LMD_GRAFANA` | `http://192.0.2.5:30300` | Grafana base URL opened by `g` |
| `LMD_THEME` | `soft` | startup theme: `soft` / `default` / `high-contrast` / `colorblind` (or `0`–`3`) |
| `LMD_W` / `LMD_H` | `100` / `26` | render size for `--render` |

---

## Data path

lmd-top **owns no data** — it reads your existing stack and correlates it.

| Layer | Source | Example metrics / resources |
|---|---|---|
| Infra (accelerators) | Prometheus | `furiosa_npu_*`, `RBLN_DEVICE_STATUS:*`, `DCGM_FI_DEV_*`, `node_*` |
| Model server | Prometheus | `vllm:num_requests_running/waiting`, `vllm:*_latency_seconds_bucket`, `vllm:generation_tokens_total`, `vllm:kv_cache_usage_perc` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| Topology / status / actions | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool, InferenceObjective |

Data flows in on two tiers: a **~1 s fast tier** (accelerators + nodes) and a
**~3 s full snapshot** (everything else). Per-model perf joins vLLM metrics by the
`service` label (= Deployment name), the same key the Models view uses.

> **To see every metric**, some exporters/ServiceMonitors may be required (RBLN and EPP;
> Furiosa is on by default). See the companion setup repo (`llm-d-setup`):
> `manifests/epp-servicemonitor.yaml`, `manifests/rbln-metrics-servicemonitor.yaml`.

---

## Architecture

```
 kubectl ─┐                                              ┌─ Overview ─┐
 Prom    ─┤→ collectors → Snapshot (metric bus) → panels ┤  Accel …   │ → ratatui → terminal
 (cm)    ─┘   (data IN, one-way)          (render OUT)    └─ Nodes ────┘
```

- **Pure Rust, no C-library deps.** Prometheus is queried directly over `tokio` TCP
  (HTTP/1.0, no TLS); Kubernetes via shelling out to `kubectl`. Result: a single static
  binary with no heavy TLS/HTTP crates.
- `collectors` **only write** to the `Snapshot`; `panels` **only read** it. New data =
  add a collector; new screen = add a panel.
- Dependencies: `ratatui`, `tokio`, `serde`/`serde_json`/`serde_yaml`, `anyhow`,
  `unicode-width`.

```
src/
  main.rs      entry point · event loop · --snapshot/--json/--doctor/--render/--cast
  collect.rs   Snapshot types + prom/kube collection      config.rs   env/yaml settings
  prom.rs      pure-tokio HTTP/1.0 Prometheus client       metrics.rs  metric-name registry
  kube.rs      kubectl shelling + scale action             catalog.rs  Launch model catalog
  app.rs       UI state (view / selection / history / alerts / permission modes)
  agent.rs     --json agent state    doctor.rs   --doctor survey    cast.rs   --cast demo
  ui/          mod.rs (views) · theme.rs (palette) · widgets.rs · panel.rs · fx.rs (animation)
```

---

## Roadmap

- **Phase 1 — Monitor** ✅ *(current)* — 10 correlated views, active alerting, logs, scale action.
- **Phase 2 — Control plane** *(next)* — permission modes (observe/debug/admin/danger), a
  machine-readable **agent JSON** state (`--snapshot --json`), and safe actions (endpoint
  drain / traffic & policy weight / rollout) with dry-run → confirm → audit.
- **Phase 3 — LLM-native depth** *(infra-gated; skeleton-first)* — PD-aware dashboard, EPP
  decision debugger with per-endpoint score table, KV/prefix cache locality, SLO/goodput
  diagnosis. Fills in once traffic goes through the EPP with vLLM/tracing metrics.
- **ModelService-native** — shift Models/Launch from raw Deployments to llm-d ModelService
  CRDs, enabling real deploy from the catalog.

See `ROADMAP.md` for the detailed plan and `CHANGELOG.md` for release history.

## Status

Verified against a live cluster (5 nodes, 12 accelerators, EPP/routes/models live).
Experimental (0.x) — interfaces may change.
