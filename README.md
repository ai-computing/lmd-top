# lmd-top

> **A terminal observability & operations tool for [llm-d](https://llm-d.ai) clusters.**
> See the whole serving stack — Gateway, EPP routing, model servers, and heterogeneous accelerators — on one screen, from a single binary.

**English** · [한국어](README.ko.md)

[![release](https://img.shields.io/github/v/release/gspark-etri/lmd-top?logo=github)](https://github.com/gspark-etri/lmd-top/releases/latest)
[![license](https://img.shields.io/github/license/gspark-etri/lmd-top)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white)

`lmd-top` correlates the four layers of an llm-d serving stack — Gateway → EPP (Endpoint
Picker) → model server → infrastructure — across heterogeneous accelerators (NVIDIA GPU,
Rebellions RBLN, Furiosa RNGD, and host CPU). It reads your existing Prometheus and
Kubernetes; it stores no data of its own.

## Demo

Record a demo locally with `lmd-top --cast && agg docs/demo.cast lmd-top.gif` (soft theme, live braille
timelines and cross-layer drill-down, rendered from synthetic data).

## Highlights

- **Four layers on one screen.** The Gateway, EPP/InferencePool, model servers, and hardware are correlated, so you can answer *which model runs where, how requests are routed, and how load is distributed* without switching tools.
- **Heterogeneous accelerators, unified.** NVIDIA GPU, Rebellions RBLN, and Furiosa RNGD sit side by side. The exact GPU model and its VRAM are auto-detected, unified-memory parts (GB10, GH200) are recognized and marked `∪`, and per-node disk usage is tracked too.
- **EPP-aware.** It reads the EPP `ConfigMap` (active scorers, weights, and picker), visualizes routing decisions and per-pod queues, and diagnoses whether an HTTPRoute actually flows through the InferencePool or bypasses it.
- **Lifecycle split — provision vs. run.** **Serving** (section 2) is the running side: every deployment in a sortable table with a live phase (serving / starting / degraded / failed, cross-referenced from pods) plus scale/restart/stop, so you can tell whether a model *is serving, still trying, or has failed*. **Deploy** (section 4) is the provision side: a **Model List** of what you can deploy (catalog feasibility + store builds) and an **Activity** feed that unifies compile Jobs and deploy rollouts with status and progress.
- **A rich terminal UI.** An LED device grid, a stacked VRAM bar, braille timelines, active alerting, log tailing, and a `scale` action — with four themes and understated animations, all in a single static Rust binary that has no C dependencies.

Policy development (unreleased): [workspace and EPP feature guide](docs/EPP-CAPABILITIES.md).
Use `lmd-top --policy init --workspace ./my-policy` to scaffold a pinned Go project;
`:Routing policy` opens the configuration and rollout preview form.

## Views

Navigation has two axes that mirror the request path (Gateway → EPP → Model → Infra):
pick a **section** with the number keys `0`–`6` (or cycle with `Tab` / `Shift+Tab`), then cycle
its **sub-tabs** with `←` / `→` (or `[` / `]`). In multi-panel views, `Ctrl+w` enters panel-focus
mode (then `h`/`j`/`k`/`l` or arrows move focus, `Esc` exits) — the vi/tmux window model.

| # | Section | Sub-tabs | Shows |
|---|---|---|---|
| 0 | **Overview** | — | Cluster summary, LED device grid, VRAM bar, accelerators by kind/node, EPP path, models, and a one-line diagnosis |
| 1 | **Traffic** | Flow · EPP | **Flow**: Gateway → HTTPRoute → backend → pods, with InferencePool/EPP/SLO and the EPP-bypass diagnosis (`⏎` jumps to the backend model). **EPP**: scorers/weights, the picker, InferencePool endpoints, and request distribution |
| 2 | **Serving** | Serving · Perf · Pods | **Serving**: running deployments in a sortable table — phase (`● serving` / `◑ starting` / `⚠ degraded` / `✗ failed` / `○ stopped`, cross-referenced from pods), engine, target, replicas, `@node`, tok/s; `o`/`O` sort, `⏎` → Scale/Restart/Stop/Objective/YAML/Logs, plus **Provenance** (`P`) on a node-local artifact — a read-only probe Job on that node reads the toolchain the artifact recorded, which is the only way to see it for hand-compiled `hostPath` builds. **Perf**: p95 latency QUEUE→PREFILL→DECODE→TPOT→E2E, tok/s, + SLO advisor. **Pods**: `llm-serving` pods (ready/phase/node/restarts) |
| 3 | **Infra** | Nodes · Devices · Topology | **Nodes**: health (CPU/mem/disk/load + devices). **Devices**: per-device util/VRAM/temp/power. **Topology**: Canvas Gateway→EPP→Pool flow + device pressure heatmap |
| 4 | **Deploy** | Library / Zoo | **Library** — provision, not runtime — two panels stacked (`Ctrl+w` switches focus). The header carries the shared store's free space, since this is where builds are created and removed. **Model List** (top): everything you can deploy — org-catalog feasibility (`✓ ready` / `⚙ needs-compile` / `✗ no-capacity`) + placement targets and store builds, grouped by family; `⏎` → Deploy/Compile, and on a store build also **Move** (`V`, relocate/rename inside the store), **Remove** (`M`, delete the build) and **Rescan** (`r`). Each store build shows **what built it** — the toolchain version read out of the artifact itself (`optimum-rbln=0.10.2`, `furiosa-compiler=2026.3.0`) — on the row and in the detail panel, and the compile pre-flight repeats it for the family you are about to build, because two builds with identical options can behave differently when different compilers produced them. Move and Remove run as a Job you review as a manifest first, and both refuse while a Deployment is serving from that path. Rescan runs the store-discovery CronJob once immediately, so a removed build stops being listed straight away instead of lingering for up to its 10-minute schedule. The deploy form fills options first, then `⏎` opens a **placement** picker — candidate nodes listed with free/total devices, util, mem, and schedulability — and picking one generates the manifest. **Activity** (bottom): a unified feed of compile Jobs (with % progress bar) and running/starting/failed deploy rollouts, each with a STARTED age (5m/3h/2d), serving node, and outcome; finished compile jobs auto-clean after 30 min; `⏎` → Logs/Delete. **Zoo**: the vendor (Furiosa/Rebellions) model zoo — public HF models auto-fetched from the [furiosa-ai HF org](https://huggingface.co/furiosa-ai) and [rbln-model-zoo](https://github.com/RBLN-SW/rbln-model-zoo) (bundled baseline via `scripts/fetch-zoo.sh`; `r` live-refreshes the Furiosa list via `curl`; override with `LMD_ZOO`). Each row shows compilable vendors (from the NPU compat list) and a situational STATUS (`● built` / `◐ compiling` / `⇊ prefetch` / `⇩ gated` / `○ available`); `o` sorts. Stacked below is the same **Activity** feed (compile + **prefetch** + deploy). `⏎` → **Prefetch** (download weights into the shared store cache) / **Compile→vendor**, then deploy the resulting build from Library |
| 5 | **Events** | — | Kubernetes + llm-d events, newest first; `⏎` shows the full message |
| 6 | **Setup** | — | Bootstrap **Doctor** for a fresh cluster — checks the llm-d platform prerequisites (Gateway API + Inference-Extension CRDs, `llm-d-gateway`, shared EPP Roles, `model-store` PVC, `hf-token` secret, accelerator device plugins, Prometheus) as `✓`/`!`/`✗`. `⏎` on a row acts by risk: objects lmd-top authors exactly (namespace, Gateway with the detected `gatewayClassName`) → review→apply; CRDs → `kubectl apply -f <upstream release URL>` (admin, confirm); site-specific / Helm-managed items (PVC, EPP Roles, secret, device plugins) → shows the exact command to run yourself. Read-only until you confirm |

List headers show a `Σ` aggregate of the shown rows (all rows, or just the filtered ones). `y` shows any selected resource's live YAML (read-only).


Setup and maintenance: [quick start](docs/QUICKSTART.md). New local onboarding commands (`--init`, `--check-setup`) are available from source and pending release.

## Install

It's a single binary (glibc on Linux) that drives your `kubectl` — so installing is just "put the binary on PATH." Pick whichever fits; **all features (incl. compile/deploy) are identical** across them.

**kubectl plugin** (recommended for cluster operators) — run the full TUI as `kubectl lmd-top`:

```bash
# self-hosted manifest (until it lands in the central krew-index):
kubectl krew install --manifest-url https://raw.githubusercontent.com/gspark-etri/lmd-top/main/plugins/lmd-top.yaml
kubectl lmd-top                      # or: kubectl lmd-top --mode admin
```

Running under `kubectl` guarantees `kubectl` is present, so scale/stop/restart and RBLN/Furiosa compile & deploy (which `kubectl apply` generated manifests) work out of the box with your kubeconfig's permissions.

**One-line installer** (prebuilt binary → `~/.local/bin`, no Rust toolchain):

```bash
curl -fsSL https://raw.githubusercontent.com/gspark-etri/lmd-top/main/install.sh | bash
#   ... | bash -s -- --version v0.34.0            # pin a version
#   ... | bash -s -- --bin-dir /usr/local/bin     # system-wide (needs write perms)
```

Auto-detects OS/arch (Linux/macOS · x86_64/aarch64), downloads the release tarball, verifies its `.sha256`, and installs it. Manual equivalent:

```bash
VER=v0.34.0   # latest: https://github.com/gspark-etri/lmd-top/releases/latest
curl -fsSL "https://github.com/gspark-etri/lmd-top/releases/download/$VER/lmd-top-$VER-x86_64-linux.tar.gz" | tar xz
sudo install -m 0755 "lmd-top-$VER-x86_64-linux/lmd-top" /usr/local/bin/
```

**From source** (developers; needs a Rust toolchain and a C linker):

```bash
git clone https://github.com/gspark-etri/lmd-top.git && cd lmd-top
./install.sh --from-source        # cargo install (+ prereqs); --with-demo also regens the GIF
# by hand: cargo install --path .
```

**Runtime requirements:**

- `kubectl` with kubeconfig access, and network reachability to Prometheus. It never SSHes into accelerator nodes.
- A truecolor terminal with a font covering box-drawing and braille glyphs is recommended; otherwise run with `LMD_THEME=default`.
- The binary links only glibc — no OpenSSL, pkg-config, or cmake. `xdg-open` is optional (used only by the `g` key).

## Usage

```bash
lmd-top                      # launch the TUI (permission mode: observe)
lmd-top --mode admin         # allow operational actions (scale / restart / apply)
lmd-top --json               # print machine-readable agent state (JSON; includes pods + store capacity)
lmd-top --doctor             # survey Prometheus: exporters, metric coverage, gaps
lmd-top --audit              # print the audit log of applied mutations
lmd-top --snapshot | --render | --cast   # headless text / CI render / demo asciicast
LMD_PROM=192.0.2.10:30090 LMD_NS=my-ns lmd-top   # point at another cluster
```

**Permission modes** (`--mode`, shown as a header badge) gate actions:
`observe` (default, view only) → `debug` (adds logs, `l`) → `admin` (scale, restart, stop,
compile/deploy apply, cordon, route rename/retarget, store move/rescan) → `danger` (delete
pod/job/route rule, remove a build from the store).
Mutating actions open a confirmation popup that defaults to **No**.
Every applied mutation (scale, stop, restart, cordon, delete, route edit, apply) is appended
to an **audit log** (`~/.config/lmd-top/audit.log`, or `$LMD_AUDIT`) with timestamp, mode,
action, target, and result — view it with `lmd-top --audit`.

**Keys.**

| | |
|---|---|
| Navigate | `0-6`/`Tab` section · `←`/`→` (`[ ]`) sub-tab · `Ctrl+w` then `hjkl`/arrows panel focus · `↑↓`/`kj` select · `g`/`G` top/bottom · `Ctrl+u`/`Ctrl+d` half-page · `Esc` back |
| Act | `⏎`/`a` action menu (drill when none) · `p i r e m` cross-layer pivot (also in the menu as **Go: …**) · `/` filter · `:` command palette (jump to any view / run any display action) · `o`/`O` sort column / direction · `y` live YAML · `l` logs · menu → Compile/Deploy/Scale/Restart/Stop/Delete/Cordon/Objective (mode-gated `⊘`, default-No confirm) · on a store row: `V` move · `M` remove · `r` rescan |
| Display | `t` theme · `f` animations · `z` zoom · `Space` pause · `A` alerts · `?` help · `q` quit · `:graf` Grafana · `R` reset session energy |

Collection quality: press `:` and select **Data sources** to inspect source status, last successful collection, and errors. Failed refreshes retain previous values with their original timestamps. `–` means unavailable, not zero; a dashed gauge has no valid measurement. JSON includes `sources`, `source_states`, and `warnings`. The report is a snapshot; reopen it to refresh.

Workload identity: `:` → **Workload classification** shows model, prefill, decode, gateway, EPP, and unknown deployments with runtime evidence. Names alone do not establish model identity. For a custom inference runtime, annotate its Deployment with `lmd-top.io/workload-role: model` (or `prefill` / `decode`). Runtime classification and Kubernetes Ready counts do not prove successful inference. JSON `workloads` also includes non-model deployments.

Performance objectives: `:` → **Performance assessment** lists each session-local SLO target as met, violated, or unknown. Missing/stale measurements and absent observed request traffic do not pass a target. Unknown metrics are shown in the Perf panel. Ambiguous model-name matches require a more specific target. Stage p95 values are independent percentiles; the report does not treat them as additive contributions or prove accelerator capacity.

Mutation modes are enforced again by the operation executor. Store deletion retains its `danger` requirement through preview and manifest editing. Headless `--plan … --apply` now requires `--mode admin` or `--mode danger`; planning and server dry-run do not require an apply mode. These modes prevent accidental actions; Kubernetes RBAC remains the authorization boundary.

Audit records now include the pinned context, default/request namespace, reviewed UID/resourceVersion where available, and elapsed time. CLI plan/apply is audited too. If logging fails after a successful operation, the UI reports that separately: do not rerun the operation to retry logging. Older audit rows remain readable.

Kubernetes target: the first cluster access snapshots the current kubeconfig for the session. All subsequent kubectl reads and writes use that snapshot, so switching your shell’s current context does not retarget a running lmd-top. Restart lmd-top to select another context or recover from initial configuration failure. The confirmation and JSON `kubernetes_context` show the selected context. The private snapshot is removed on normal exit.

**Environment.**

- `LMD_PROM`, `LMD_NS` (default `llm-serving`), `LMD_GRAFANA` — point it at your cluster.
- `LMD_THEME` — startup theme: `soft`, `default`, `high-contrast`, or `colorblind`.
- `LMD_AUDIT` — audit log path (default: `~/.config/lmd-top/audit.log`).
- `LMD_W` / `LMD_H` — the `--render` size.
- `LMD_COMPILE_IMAGE_RBLN`, `LMD_COMPILE_IMAGE_FURIOSA`, `LMD_SERVING_IMAGE` — container images for the generated compile/deploy manifests. Until set, those fields are `TODO-…` placeholders and the in-app apply (`a`) is blocked; `w` still saves the manifest to edit by hand.
- `LMD_SAVE_DIR` — where `w` writes saved manifests (default: current dir).
- `LMD_HISTORY` — compile/serving outcome log (default: `~/.config/lmd-top/history.jsonl`),
  read back by `lmd-top --history`.
- `LMD_COMPILE_ENV` — extra env injected into generated compile Jobs, as `K=V,K=V`. Intended
  for vendor debug switches; RBLN builds on TVM, so `TVM_LOG_DEBUG=1` / `TVM_BACKTRACE=1`
  reach the compiler. A flag the SDK rejects is classified as `vendor-flag` and kept out of
  the option evidence, so a bad switch cannot teach the advisor that your options fail.
- `LMD_RBLN_TOOLCHAIN` — a wheel directory in the shared store to install the RBLN toolchain
  from, e.g. `/mnt/store/rbln-toolchain/0.10.3`. The compile Job then runs on
  `python:3.10-slim` and `pip install --no-index --find-links=<dir>`, which pins every version
  together and stops the build inheriting whatever the node's Python happens to have. Needs no
  hostPath, no nodeSelector and no vendor registry credentials — just the wheels.
- Optional `~/.config/lmd-top/lmd-top.yaml` customizes column order.

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
| Store inventory / provenance | `model-inventory` cm | build path, size, compile target, the toolchain the artifact records, and the store's own `df` (`manifests/model-store-discovery.yaml` scans for it) |

Data arrives on two tiers: a fast tier (~1 s) for accelerators and nodes, and a full
snapshot (~3 s) for everything else. It's pure Rust — Prometheus is queried over raw `tokio`
HTTP/1.0, and Kubernetes through `kubectl`.

## Status & roadmap

**Works today, with no traffic required.** All twelve views; GPU/RBLN/RNGD and node/disk
monitoring with auto-detection and unified memory; the Flow topology and EPP-bypass
diagnosis; EPP ConfigMap introspection; active alerting; the `scale` and `logs` actions; the
Serving section (running deployments with a serving/starting/failed phase) and the Deploy
section's Model List + Activity (catalog feasibility, and RBLN/Furiosa compile & deploy
manifest generation with a mode-gated apply, tracked in one activity feed); the headless
`--json`, `--doctor`, `--snapshot`, and `--cast` modes; and themes, animation, zoom, and
permission modes.

**Fills in once real traffic flows through the EPP and vLLM exposes metrics.** Per-model p95
latency breakdown, tok/s, per-pod queue distribution, KV%/TTFT/E2E, and EPP request
distribution. (The EPP weight `+`/`-` is a local weight-share simulation — it does not apply
to the cluster.)

**Planned.** Applied control-plane actions beyond the current apply flow (endpoint drain,
traffic/policy-weight apply, and rollout, each as dry-run → confirm → audit) and an EPP
per-endpoint score debugger. (NPU **compile & deploy automation** — generating RBLN/Furiosa
compile Jobs and serving Deployments from the Deploy view, gated by permission mode — has
since shipped; see Highlights.) See [ROADMAP.md](ROADMAP.md) and [CHANGELOG.md](CHANGELOG.md).

## Maturity

Verified against a live heterogeneous cluster (8 nodes; GB10, RBLN, and RNGD accelerators;
EPP, routes, and models live). It's experimental (0.x), so interfaces may still change.

## ai-rm

이 레포는 [ai-rm](https://github.com/ai-computing/ai-rm)(AI 반도체 통합 자원관리)의 **`l5-ops` 계층, kind `tool`** 구성요소이며, ai-rm의 **관측 콘솔**이다. 가속기(A100·GB10·RBLN ATOM·Furiosa RNGD)와 노드, 모델 서버, InferencePool·EPP 라우팅, TTFT/TPOT/SLO를 한 화면에서 보고, 권한 모드(`--mode`)와 감사 로그 아래에서 scale·drain·route 편집·NPU 컴파일/배포 같은 운영 액션을 실행한다. 메타데이터는 루트의 `ai-rm.yaml`에 있다.

예전 rm_monitor의 역할은 나뉘었다. 모니터링 스택(Prometheus, exporter, Grafana 대시보드)은 `llm-d-setup/monitoring/`(l4-cluster)으로, 부하 도구는 `rm-bench`(l5-ops)로 옮겼다. lmd-top은 그 스택이 모은 지표를 읽어 상관 분석하는 콘솔이고, 자체 데이터 저장소는 없다.

**k8s + llm-d 연결.** 클러스터에 상주하지 않는다. control-plane 등 `kubectl`이 되는 호스트에서 돌며, Kubernetes는 `kubectl`로(Deployment, Pod, HTTPRoute, Gateway, InferencePool, EPP ConfigMap), 지표는 Prometheus HTTP API(`LMD_PROM`)로 읽는다. 쓰기 액션도 같은 `kubectl` 경로로 적용한다. EPP 라우팅 정책 플러그인(`heterogeneous-capacity` filter/scorer, `epp_endpoint_score` 내보내기)은 `contrib/epp-score-observer/`에 있고, 이미지 빌드·배포는 아직 하지 않았다.

**설치·테스트.** 가속기나 클러스터 없이 돈다.

```bash
cargo install --locked --path .                         # lmd-top 바이너리 설치
cargo test --locked                                     # Rust 단위·골든 테스트
python3 -m unittest discover -s tests -p 'test_*.py'    # 설치 스크립트 회귀 테스트
(cd contrib/epp-score-observer && go test ./...)        # EPP 정책 플러그인 (Go)
```

**인터페이스.** 제공: `Metrics`(`--json` agent 상태 `lmd-top/agent-state/v2`, `--doctor` 지표 커버리지), `Placement`(heterogeneous-capacity EPP 정책, `--policy render/plan/replay`). 사용: `Metrics`(DCGM·RBLN·Furiosa·vLLM·EPP Prometheus 지표), `Accelerator`(가속기 팩 `src/accel/`, 노드 device plugin 리소스), `Workload`(serving Deployment, NPU 컴파일 Job, npu-lora 학습 Job `--plan train`), `Placement`(InferencePool·EPP·HTTPRoute).

**KPI.** 이종자원수(GPU·RBLN·RNGD를 한 콘솔에서 관측·배포), SLO(TTFT/TPOT/E2E p95, 큐·KV 상태), 전력효율(가속기 전력 지표 표시).

**사이트 설정 (`cluster.local.env`).** 실제 주소는 커밋하지 않는다. `cp cluster.example.env cluster.local.env`(untracked) 후 값을 채우고 `set -a; . ./cluster.local.env; set +a`로 내보낸 뒤 실행한다. 환경 변수가 없으면 `~/.config/lmd-top/lmd-top.yaml`, 그다음 localhost 기본값을 쓴다.

| 변수 | 뜻 |
|---|---|
| `LMD_PROM` | Prometheus `host:port` (평문 HTTP, NodePort 또는 port-forward) |
| `LMD_NS` | 모델 서버·EPP가 있는 네임스페이스 (기본 `llm-serving`) |
| `LMD_GRAFANA` | Grafana 기본 URL (`:graf` 명령) |

## Contributing & license

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).
Licensed under [Apache-2.0](LICENSE).
