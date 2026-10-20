<div align="center">

<h1>lmd-top</h1>

<h3>Observe and operate heterogeneous Kubernetes + llm-d clusters from the terminal</h3>

<p>
  <strong>English</strong> · <a href="README.ko.md">한국어</a>
</p>

<p>
  <a href="https://github.com/ai-computing/lmd-top/actions/workflows/ci.yml"><img src="https://github.com/ai-computing/lmd-top/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/ai-computing/ai-rm"><img src="https://img.shields.io/badge/ai--rm-L5%20observe%20%26%20operate-f472b6" alt="ai-rm layer: L5 observe and operate"></a>
  <a href="https://github.com/ai-computing/rm-core/blob/v0.1.1/PROTOCOL.md"><img src="https://img.shields.io/badge/protocol-v0.1-7c5cff" alt="Protocol v0.1"></a>
  <a href="https://github.com/ai-computing/lmd-top/releases/latest"><img src="https://img.shields.io/github/v/release/ai-computing/lmd-top?logo=github" alt="Latest release"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/Rust-single%20binary-000000?logo=rust&amp;logoColor=white" alt="Rust, single binary"></a>
  <a href="#-architecture"><img src="https://img.shields.io/badge/target-Kubernetes%20%2B%20llm--d-326ce5?logo=kubernetes&amp;logoColor=white" alt="Target: Kubernetes and llm-d"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-64748b" alt="License: Apache-2.0"></a>
</p>

<p>
  <a href="#-quick-start"><b>Install</b></a> ·
  <a href="#-usage"><b>Usage</b></a> ·
  <a href="#-ai-rm-integration"><b>ai-rm</b></a> ·
  <a href="#-architecture"><b>Architecture</b></a> ·
  <a href="#-documentation"><b>Docs</b></a> ·
  <a href="#-roadmap"><b>Roadmap</b></a>
</p>

</div>

---

**lmd-top** is a terminal UI that correlates the four layers of an [llm-d](https://llm-d.ai) serving stack
(Gateway → EPP (Endpoint Picker) → model server → infrastructure) across heterogeneous accelerators: NVIDIA GPU,
Rebellions RBLN, Furiosa RNGD and host CPU. It reads your existing Prometheus and Kubernetes and stores no data of
its own; operational actions run through `kubectl` behind a permission mode and an audit log. In
[ai-rm](https://github.com/ai-computing/ai-rm) it is the **L5 observe & operate** console, and it shows the ai-rm
protocol v0.1 objects (AcceleratorProfiles, `airm_*` metrics, placement decisions) next to the vendor data.

It is experimental (0.x): interfaces may still change.

## ✨ Highlights

<table>
<tr>
<td width="33%" valign="top">
<b>Four layers on one screen</b><br>
Gateway, EPP/InferencePool, model servers and hardware are correlated, so you can see which model runs where, how requests are routed and how load is spread without switching tools.
</td>
<td width="33%" valign="top">
<b>Heterogeneous accelerators, unified</b><br>
NVIDIA GPU, Rebellions RBLN and Furiosa RNGD side by side. GPU model and VRAM are auto-detected, unified-memory parts (GB10, GH200) are marked <code>∪</code>, and per-node disk use is tracked.
</td>
<td width="33%" valign="top">
<b>EPP-aware</b><br>
Reads the EPP <code>ConfigMap</code> (active scorers, weights, picker), shows routing decisions and per-pod queues, and diagnoses whether an HTTPRoute really flows through the InferencePool or bypasses it.
</td>
</tr>
<tr>
<td width="33%" valign="top">
<b>Provision vs. run</b><br>
<b>Serving</b> lists running deployments with a live phase (serving, starting, degraded, failed) plus scale, restart and stop. <b>Deploy</b> lists what you can deploy and an activity feed of compile Jobs and rollouts.
</td>
<td width="33%" valign="top">
<b>Guarded operations</b><br>
Permission modes (<code>observe</code> → <code>debug</code> → <code>admin</code> → <code>danger</code>), confirmation popups that default to No, and an audit log of every applied mutation.
</td>
<td width="33%" valign="top">
<b>One static binary</b><br>
LED device grid, stacked VRAM bar, braille timelines, alerting, log tailing and four themes in a single Rust binary that links only glibc. Headless <code>--json</code>, <code>--doctor</code> and <code>--snapshot</code> modes too.
</td>
</tr>
</table>

The public repository is a sanitized export: the demo recording is not included. Record one locally with
`lmd-top --cast && agg docs/demo.cast lmd-top.gif` (soft theme, live braille timelines and cross-layer drill-down,
rendered from synthetic data).

## 🚀 Quick start

lmd-top is a single binary that drives your `kubectl`, so installing means putting the binary on `PATH`. All
features, compile and deploy included, are the same however you install it.

**Release binary** (no Rust toolchain). Latest release: v0.35.0; targets `x86_64-linux`, `aarch64-linux`,
`aarch64-darwin`.

```bash
VER=v0.35.0   # latest: https://github.com/ai-computing/lmd-top/releases/latest
curl -fsSLO "https://github.com/ai-computing/lmd-top/releases/download/$VER/lmd-top-$VER-x86_64-linux.tar.gz"
curl -fsSLO "https://github.com/ai-computing/lmd-top/releases/download/$VER/lmd-top-$VER-x86_64-linux.tar.gz.sha256"
sha256sum -c "lmd-top-$VER-x86_64-linux.tar.gz.sha256" && tar xzf "lmd-top-$VER-x86_64-linux.tar.gz"
sudo install -m 0755 "lmd-top-$VER-x86_64-linux/lmd-top" /usr/local/bin/
```

**From source** (needs a Rust toolchain and a C linker). This is also the way to get the unreleased commands
(`--init`, `--check-setup`, `--version`) described in the [quick start guide](docs/QUICKSTART.md):

```bash
git clone https://github.com/ai-computing/lmd-top.git && cd lmd-top
./install.sh --from-source        # cargo install (+ prereqs); --with-demo also regenerates the demo GIF
# by hand: cargo install --locked --path .
```

**Run it** against a cluster:

```bash
lmd-top                                            # observe mode
LMD_PROM=192.0.2.10:30090 LMD_NS=my-ns lmd-top     # point at another cluster
```

<details>
<summary><b>Installer script and kubectl plugin</b></summary>
<br>

> [!NOTE]
> `install.sh` and `plugins/lmd-top.yaml` still download release assets from the project's previous repository
> location, not from `ai-computing/lmd-top`. Until they are updated, use the release binary or a source build above.

**kubectl plugin** (for cluster operators): run the full TUI as `kubectl lmd-top`.

```bash
# self-hosted manifest (until it lands in the central krew-index):
kubectl krew install --manifest-url https://raw.githubusercontent.com/ai-computing/lmd-top/main/plugins/lmd-top.yaml
kubectl lmd-top                      # or: kubectl lmd-top --mode admin
```

Running under `kubectl` guarantees `kubectl` is present, so scale/stop/restart and RBLN/Furiosa compile & deploy
(which `kubectl apply` generated manifests) work with your kubeconfig's permissions.

**One-line installer** (prebuilt binary → `~/.local/bin`):

```bash
curl -fsSL https://raw.githubusercontent.com/ai-computing/lmd-top/main/install.sh | bash
#   ... | bash -s -- --version v0.35.0            # pin a version
#   ... | bash -s -- --bin-dir /usr/local/bin     # system-wide (needs write perms)
```

It auto-detects OS/arch (Linux/macOS · x86_64/aarch64), downloads the release tarball, verifies its `.sha256`
and installs it.
</details>

**Runtime requirements**

- `kubectl` with kubeconfig access, and network reachability to Prometheus. lmd-top never SSHes into accelerator nodes.
- A truecolor terminal with a font covering box-drawing and braille glyphs is recommended; otherwise run with `LMD_THEME=default`.
- The binary links only glibc: no OpenSSL, pkg-config or cmake. `xdg-open` is optional (used only by the `g` key).

Setup, connection and maintenance: [quick start guide](docs/QUICKSTART.md).

## 🧭 Usage

```bash
lmd-top                      # launch the TUI (permission mode: observe)
lmd-top --mode admin         # allow operational actions (scale / restart / apply)
lmd-top --json               # print machine-readable agent state (JSON; includes pods + store capacity)
lmd-top --doctor             # survey Prometheus: exporters, metric coverage, gaps
lmd-top --audit              # print the audit log of applied mutations
lmd-top --snapshot | --render | --cast   # headless text / CI render / demo asciicast
```

Navigation follows the request path (Gateway → EPP → Model → Infra): pick a **section** with `0`–`6` (or
`Tab` / `Shift+Tab`), then cycle its **sub-tabs** with `←` / `→` (or `[` / `]`). In multi-panel views `Ctrl+w`
enters panel-focus mode (`h`/`j`/`k`/`l` or arrows move, `Esc` exits), like vi/tmux windows.

| # | Section | Sub-tabs |
|---|---|---|
| 0 | **Overview** | — |
| 1 | **Traffic** | Flow · EPP |
| 2 | **Serving** | Serving · Perf · Pods |
| 3 | **Infra** | Nodes · Devices · Topology |
| 4 | **Deploy** | Library / Zoo |
| 5 | **Events** | — |
| 6 | **Setup** | — |

**Permission modes** (`--mode`, shown as a header badge) gate actions: `observe` (default, view only) → `debug`
(adds logs, `l`) → `admin` (scale, restart, stop, compile/deploy apply, cordon, route rename/retarget, store
move/rescan) → `danger` (delete pod/job/route rule, remove a build from the store). Mutating actions open a
confirmation popup that defaults to **No**. Every applied mutation (scale, stop, restart, cordon, delete, route
edit, apply) is appended to an **audit log** (`~/.config/lmd-top/audit.log`, or `$LMD_AUDIT`) with timestamp,
mode, action, target and result; view it with `lmd-top --audit`.

<details>
<summary><b>What each view shows</b></summary>
<br>

| # | Section | Sub-tabs | Shows |
|---|---|---|---|
| 0 | **Overview** | — | Cluster summary, LED device grid, VRAM bar, accelerators by kind/node, EPP path, models, and a one-line diagnosis |
| 1 | **Traffic** | Flow · EPP | **Flow**: Gateway → HTTPRoute → backend → pods, with InferencePool/EPP/SLO and the EPP-bypass diagnosis (`⏎` jumps to the backend model). **EPP**: scorers/weights, the picker, InferencePool endpoints, and request distribution |
| 2 | **Serving** | Serving · Perf · Pods | **Serving**: running deployments in a sortable table — phase (`● serving` / `◑ starting` / `⚠ degraded` / `✗ failed` / `○ stopped`, cross-referenced from pods), engine, target, replicas, `@node`, tok/s; `o`/`O` sort, `⏎` → Scale/Restart/Stop/Objective/YAML/Logs, plus **Provenance** (`P`) on a node-local artifact — a read-only probe Job on that node reads the toolchain the artifact recorded, which is the only way to see it for hand-compiled `hostPath` builds. **Perf**: p95 latency QUEUE→PREFILL→DECODE→TPOT→E2E, tok/s, + SLO advisor. **Pods**: `llm-serving` pods (ready/phase/node/restarts) |
| 3 | **Infra** | Nodes · Devices · Topology | **Nodes**: health (CPU/mem/disk/load + devices). **Devices**: per-device util/VRAM/temp/power, plus the ai-rm panel ([below](#-ai-rm-integration)). **Topology**: Canvas Gateway→EPP→Pool flow + device pressure heatmap |
| 4 | **Deploy** | Library / Zoo | **Library** — provision, not runtime — two panels stacked (`Ctrl+w` switches focus). The header carries the shared store's free space, since this is where builds are created and removed. **Model List** (top): everything you can deploy — org-catalog feasibility (`✓ ready` / `⚙ needs-compile` / `✗ no-capacity`) + placement targets and store builds, grouped by family; `⏎` → Deploy/Compile, and on a store build also **Move** (`V`, relocate/rename inside the store), **Remove** (`M`, delete the build) and **Rescan** (`r`). Each store build shows **what built it** — the toolchain version read out of the artifact itself (`optimum-rbln=0.10.2`, `furiosa-compiler=2026.3.0`) — on the row and in the detail panel, and the compile pre-flight repeats it for the family you are about to build, because two builds with identical options can behave differently when different compilers produced them. Move and Remove run as a Job you review as a manifest first, and both refuse while a Deployment is serving from that path. Rescan runs the store-discovery CronJob once immediately, so a removed build stops being listed straight away instead of lingering for up to its 10-minute schedule. The deploy form fills options first, then `⏎` opens a **placement** picker — candidate nodes listed with free/total devices, util, mem, and schedulability — and picking one generates the manifest. **Activity** (bottom): a unified feed of compile Jobs (with % progress bar) and running/starting/failed deploy rollouts, each with a STARTED age (5m/3h/2d), serving node, and outcome; finished compile jobs auto-clean after 30 min; `⏎` → Logs/Delete. **Zoo**: the vendor (Furiosa/Rebellions) model zoo — public HF models auto-fetched from the [furiosa-ai HF org](https://huggingface.co/furiosa-ai) and [rbln-model-zoo](https://github.com/RBLN-SW/rbln-model-zoo) (bundled baseline via `scripts/fetch-zoo.sh`; `r` live-refreshes the Furiosa list via `curl`; override with `LMD_ZOO`). Each row shows compilable vendors (from the NPU compat list) and a situational STATUS (`● built` / `◐ compiling` / `⇊ prefetch` / `⇩ gated` / `○ available`); `o` sorts. Stacked below is the same **Activity** feed (compile + **prefetch** + deploy). `⏎` → **Prefetch** (download weights into the shared store cache) / **Compile→vendor**, then deploy the resulting build from Library |
| 5 | **Events** | — | Kubernetes + llm-d events, newest first; `⏎` shows the full message |
| 6 | **Setup** | — | Bootstrap **Doctor** for a fresh cluster — checks the llm-d platform prerequisites (Gateway API + Inference-Extension CRDs, `llm-d-gateway`, shared EPP Roles, `model-store` PVC, `hf-token` secret, accelerator device plugins, Prometheus) as `✓`/`!`/`✗`. `⏎` on a row acts by risk: objects lmd-top authors exactly (namespace, Gateway with the detected `gatewayClassName`) → review→apply; CRDs → `kubectl apply -f <upstream release URL>` (admin, confirm); site-specific / Helm-managed items (PVC, EPP Roles, secret, device plugins) → shows the exact command to run yourself. Read-only until you confirm |

List headers show a `Σ` aggregate of the shown rows (all rows, or just the filtered ones). `y` shows any selected
resource's live YAML (read-only).
</details>

<details>
<summary><b>Keybindings</b></summary>
<br>

| | |
|---|---|
| Navigate | `0-6`/`Tab` section · `←`/`→` (`[ ]`) sub-tab · `Ctrl+w` then `hjkl`/arrows panel focus · `↑↓`/`kj` select · `g`/`G` top/bottom · `Ctrl+u`/`Ctrl+d` half-page · `Esc` back |
| Act | `⏎`/`a` action menu (drill when none) · `p i r e m` cross-layer pivot (also in the menu as **Go: …**) · `/` filter · `:` command palette (jump to any view / run any display action) · `o`/`O` sort column / direction · `y` live YAML · `l` logs · menu → Compile/Deploy/Scale/Restart/Stop/Delete/Cordon/Objective (mode-gated `⊘`, default-No confirm) · on a store row: `V` move · `M` remove · `r` rescan |
| Display | `t` theme · `f` animations · `z` zoom · `Space` pause · `A` alerts · `?` help · `q` quit · `:graf` Grafana · `R` reset session energy |

**Colors and glyphs.** Color encodes severity or identity, while state is carried by a separate glyph (`●` up,
`○` idle, `◐` pending, `⚠` throttling, `⊘` cordoned, `✗` down) so the UI stays legible in the colorblind theme.
Metrics that aren't present yet render as `–` and fill in once the workload is up.
</details>

<details>
<summary><b>Environment variables</b></summary>
<br>

- `LMD_PROM`, `LMD_NS` (default `llm-serving`), `LMD_GRAFANA` — point it at your cluster.
- `LMD_AIRM_NS` — namespace of the ai-rm ConfigMap `airm-placement` (default: `LMD_NS`).
- `LMD_THEME` — startup theme: `soft`, `default`, `high-contrast`, or `colorblind`.
- `LMD_AUDIT` — audit log path (default: `~/.config/lmd-top/audit.log`).
- `LMD_W` / `LMD_H` — the `--render` size.
- `LMD_COMPILE_IMAGE_RBLN`, `LMD_COMPILE_IMAGE_FURIOSA`, `LMD_SERVING_IMAGE` — container images for the generated compile/deploy manifests. Until set, those fields are `TODO-…` placeholders and the in-app apply (`a`) is blocked; `w` still saves the manifest to edit by hand.
- `LMD_SAVE_DIR` — where `w` writes saved manifests (default: current dir).
- `LMD_HISTORY` — compile/serving outcome log (default: `~/.config/lmd-top/history.jsonl`), read back by `lmd-top --history`.
- `LMD_COMPILE_ENV` — extra env injected into generated compile Jobs, as `K=V,K=V`. Intended for vendor debug switches; RBLN builds on TVM, so `TVM_LOG_DEBUG=1` / `TVM_BACKTRACE=1` reach the compiler. A flag the SDK rejects is classified as `vendor-flag` and kept out of the option evidence, so a bad switch cannot teach the advisor that your options fail.
- `LMD_RBLN_TOOLCHAIN` — a wheel directory in the shared store to install the RBLN toolchain from, e.g. `/mnt/store/rbln-toolchain/0.10.3`. The compile Job then runs on `python:3.10-slim` and `pip install --no-index --find-links=<dir>`, which pins every version together and stops the build inheriting whatever the node's Python happens to have. Needs no hostPath, no nodeSelector and no vendor registry credentials — just the wheels.
- `LMD_RBLN_HOST_SITE_PACKAGES` — the RBLN node's user-level Python site-packages, e.g. `/home/<user>/.local/lib/python3.10/site-packages`, for nodes where the vendor stack was installed with `pip install --user`. Only used by the hostPath fallback (no `LMD_COMPILE_IMAGE_RBLN`, no `LMD_RBLN_TOOLCHAIN`); the compile Job and the RBLN serving Deployment then mount it at the same path and put it first on `PYTHONPATH`. Unset, only the system Python locations are mounted.
- Optional `~/.config/lmd-top/lmd-top.yaml` customizes column order.

Site values: copy `cluster.example.env` to `cluster.local.env` (untracked), fill it in and export it with
`set -a; . ./cluster.local.env; set +a` before running. Without environment variables lmd-top falls back to
`~/.config/lmd-top/lmd-top.yaml`, then to localhost defaults.
</details>

<details>
<summary><b>Operating notes: data quality, workload identity, SLOs, safety</b></summary>
<br>

**Collection quality.** Press `:` and select **Data sources** to inspect source status, last successful collection,
and errors. Failed refreshes retain previous values with their original timestamps. `–` means unavailable, not zero;
a dashed gauge has no valid measurement. JSON includes `sources`, `source_states`, and `warnings`. The report is a
snapshot; reopen it to refresh.

**Workload identity.** `:` → **Workload classification** shows model, prefill, decode, gateway, EPP, and unknown
deployments with runtime evidence. Names alone do not establish model identity. For a custom inference runtime,
annotate its Deployment with `lmd-top.io/workload-role: model` (or `prefill` / `decode`). Runtime classification and
Kubernetes Ready counts do not prove successful inference. JSON `workloads` also includes non-model deployments.

**Performance objectives.** `:` → **Performance assessment** lists each session-local SLO target as met, violated,
or unknown. Missing/stale measurements and absent observed request traffic do not pass a target. Unknown metrics are
shown in the Perf panel. Ambiguous model-name matches require a more specific target. Stage p95 values are
independent percentiles; the report does not treat them as additive contributions or prove accelerator capacity.

**Mode enforcement.** Mutation modes are enforced again by the operation executor. Store deletion retains its
`danger` requirement through preview and manifest editing. Headless `--plan … --apply` requires `--mode admin` or
`--mode danger`; planning and server dry-run do not require an apply mode. These modes prevent accidental actions;
Kubernetes RBAC remains the authorization boundary.

**Audit records** include the pinned context, default/request namespace, reviewed UID/resourceVersion where
available, and elapsed time. CLI plan/apply is audited too. If logging fails after a successful operation, the UI
reports that separately: do not rerun the operation to retry logging. Older audit rows remain readable.

**Kubernetes target.** The first cluster access snapshots the current kubeconfig for the session. All subsequent
kubectl reads and writes use that snapshot, so switching your shell's current context does not retarget a running
lmd-top. Restart lmd-top to select another context or recover from initial configuration failure. The confirmation
and JSON `kubernetes_context` show the selected context. The private snapshot is removed on normal exit.

**Routing policy development** (unreleased): `lmd-top --policy init --workspace ./my-policy` scaffolds a pinned Go
project, and `:Routing policy` opens the configuration and rollout preview form. See the
[workspace and EPP feature guide](docs/EPP-CAPABILITIES.md).
</details>

## 🔌 ai-rm integration

lmd-top is the **`l5-ops` layer, kind `tool`** of [ai-rm](https://github.com/ai-computing/ai-rm) and its observation
console; metadata is in [`ai-rm.yaml`](ai-rm.yaml). The old rm_monitor role was split: the monitoring stack
(Prometheus, exporters, Grafana dashboards) moved to [hetero-llm-d](https://github.com/ai-computing/hetero-llm-d)
`monitoring/` (L4) and load generation to rm-bench (L5). lmd-top reads what that stack collects; it does not import
rm-core, it reads rm-core formats.

**ai-rm panel (protocol v0.1, read-only).** Infra ▸ Devices shows an ai-rm panel when the cluster has any ai-rm source:

| Source | What the panel shows |
|---|---|
| `AcceleratorProfile` (`airm.ai-computing.io/v1alpha1`) | vendor, model, memory and peak with its source; the selected device's profile is marked |
| Prometheus `airm_*` series | values with rm-core v0.1 names and units; vendor-derived `tok_s` / `req_s` / `power_w` / `accelerators` shown as `[vendor]` when no airm series exists. The device PWR column prefers `airm_power_w{scope="card"}` |
| ConfigMap `airm-placement` (namespace `LMD_AIRM_NS`, default `LMD_NS`) | placement decisions and their reasons. Both layouts are read: llm-d-setup's documented `decisions.json` document (a `decisions` list, shown as `<pool>/<kind>`), and one key per workload with `{"accelerator_ids": [...], "kv_transport": "...", "reason": "..."}` (JSON or YAML). An empty `decisions` list shows nothing |

Without the CRD, the series and the ConfigMap the view is unchanged and nothing is warned. Write actions are not
affected by ai-rm data.

**Interfaces.** Provides `Metrics` (`--json` agent state `lmd-top/agent-state/v2`, `--doctor` metric coverage) and
`Placement` (the heterogeneous-capacity EPP policy, `--policy render/plan/replay`). Uses `Metrics` (DCGM, RBLN,
Furiosa, vLLM and EPP Prometheus metrics), `Accelerator` (accelerator packs in `src/accel/`, node device-plugin
resources), `Workload` (serving Deployments, NPU compile Jobs, npu-lora training Jobs via `--plan train`) and
`Placement` (InferencePool, EPP, HTTPRoute). KPIs: heterogeneous resources (GPU, RBLN and RNGD observed and deployed
from one console), SLO (TTFT/TPOT/E2E p95, queue and KV state) and power efficiency (accelerator power metrics).

**Install and test** without accelerators or a cluster:

```bash
cargo install --locked --path .                         # install the lmd-top binary
cargo test --locked                                     # Rust unit and golden tests
python3 -m unittest discover -s tests -p 'test_*.py'    # installer and recipe regression tests
(cd contrib/epp-score-observer && go test ./...)        # EPP policy plugin (Go)
```

## 🏗 Architecture

lmd-top does not run in the cluster. It runs on any host where `kubectl` works (control plane, workstation), reads
Kubernetes through `kubectl` and metrics through the Prometheus HTTP API (`LMD_PROM`), and applies write actions
through the same `kubectl` path.

| Layer | Source | Examples |
|---|---|---|
| Accelerators / host | Prometheus | `DCGM_FI_DEV_*`, `RBLN_DEVICE_STATUS:*`, `furiosa_npu_*`, `node_*` |
| Model server | Prometheus | `vllm:*_latency_seconds_bucket`, `vllm:num_requests_*`, `vllm:*kv_cache*` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| Topology / status / actions | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool, InferenceObjective |
| Store inventory / provenance | `model-inventory` cm | build path, size, compile target, the toolchain the artifact records, and the store's own `df` (`manifests/model-store-discovery.yaml` scans for it) |
| ai-rm (optional) | `kubectl` + Prometheus | `AcceleratorProfile`, `airm_*`, ConfigMap `airm-placement` |

Data arrives on two tiers: a fast tier (~1 s) for accelerators and nodes, and a full snapshot (~3 s) for everything
else. It is pure Rust: Prometheus is queried over raw `tokio` HTTP/1.0, and Kubernetes through `kubectl`.

The EPP routing-policy plugin (`heterogeneous-capacity` filter/scorer, `epp_endpoint_score` export) lives in
[`contrib/epp-score-observer/`](contrib/epp-score-observer/README.md); no image has been built or deployed from it yet.

## 📚 Documentation

| Goal | Guides |
|---|---|
| 🚀 **Install and connect** | [Quick start](docs/QUICKSTART.md) · [Contributing](CONTRIBUTING.md) |
| 🔀 **EPP and routing** | [EPP feature guide](docs/EPP-CAPABILITIES.md) · [EPP plugins](docs/EPP-PLUGINS.md) · [Heterogeneous analysis](docs/HETEROGENEOUS-ANALYSIS.md) |
| 🧾 **Data and provenance** | [JSON inventory](docs/JSON-INVENTORY.md) · [JSON source quality](docs/JSON-SOURCE-QUALITY.md) · [Build provenance](docs/BUILD-PROVENANCE.md) |
| 🛡 **Quality and operations** | [Operation hardening](docs/OPERATION-HARDENING.md) · [QA test plan](docs/QA-TESTPLAN.md) · [1.0 readiness](docs/V1-READINESS.md) |
| 📍 **Progress** | [Changelog](CHANGELOG.md) · [Roadmap](ROADMAP.md) (Korean) |

## 🗺 Roadmap

**Works today, with no traffic required.** All twelve views; GPU/RBLN/RNGD and node/disk monitoring with
auto-detection and unified memory; the Flow topology and EPP-bypass diagnosis; EPP ConfigMap introspection; active
alerting; the `scale` and `logs` actions; the Serving section and the Deploy section's Model List + Activity
(catalog feasibility, and RBLN/Furiosa compile & deploy manifest generation with a mode-gated apply); the headless
`--json`, `--doctor`, `--snapshot` and `--cast` modes; themes, animation, zoom and permission modes; the read-only
ai-rm panel.

**Fills in once real traffic flows through the EPP and vLLM exposes metrics.** Per-model p95 latency breakdown,
tok/s, per-pod queue distribution, KV%/TTFT/E2E, and EPP request distribution. (The EPP weight `+`/`-` is a local
weight-share simulation; it does not apply to the cluster.)

- [x] NPU compile & deploy automation from the Deploy view, gated by permission mode
- [x] Permission modes, audit log, `--json` agent state and safe actions (v0.35)
- [ ] Applied control-plane actions beyond the current apply flow (traffic/policy-weight apply and rollout, each as dry-run → confirm → audit)
- [ ] EPP per-endpoint score debugger: the score source exists in `contrib/epp-score-observer` and shows `–` until that plugin is deployed
- [ ] Release the unreleased changes in [CHANGELOG.md](CHANGELOG.md) (`Cargo.toml` is at 0.36.0; the latest tag is v0.35.0); they still need real-cluster and NPU validation

**Maturity.** Verified against a live heterogeneous cluster (8 nodes; GB10, RBLN and RNGD accelerators; EPP,
routes and models live). It is experimental (0.x), so interfaces may still change. Direction and priorities:
[ROADMAP.md](ROADMAP.md).

## 🤝 Contributing

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).

## 📄 License

[Apache-2.0](LICENSE).
