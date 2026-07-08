# epp-score-observer

A drop-in **EPP (llm-d endpoint-picker) plugin** that exposes the router's real
**per-endpoint scheduler score** as a Prometheus metric, so tools like `lmd-top`
can answer "why did the router pick *this* pod?" from live data.

## What it does

The llm-d EPP scores every candidate endpoint each request (queue / kv-cache /
prefix-cache scorers, weighted), sums them into one composite score, and the
**Picker** selects the max. Those scores are normally computed in memory and
discarded — nothing exports them.

This plugin is a **Picker** (`type: endpoint-score-observer`) that wraps the
stock `max-score-picker`: on every `Pick` it records each candidate's composite
score, then delegates selection unchanged. Metrics are registered on the EPP's
existing controller-runtime registry (served on `:9090`), so the current
ServiceMonitor scrapes them with no extra wiring.

Selection behavior is **identical** to before (it delegates to max-score).

### Metrics exported

| metric | type | labels | meaning |
|---|---|---|---|
| `epp_endpoint_score` | gauge | `pod`, `namespace`, `pool` | composite score of each candidate in the most recent cycle (higher = preferred) |
| `epp_endpoint_picked_total` | counter | `pod`, `namespace`, `pool` | cumulative picks per endpoint |

## Why a custom binary (not a fork)

`main.go` reuses the upstream runner verbatim
(`github.com/llm-d/llm-d-router/cmd/epp/runner`) and only calls
`plugin.Register("endpoint-score-observer", Factory)` before `Run()`. All stock
flags and plugins are unchanged; we just add one plugin type to the registry.

## Status — validated

- **Compiles** against `github.com/llm-d/llm-d-router@main` (pinned in `go.mod`/`go.sum`
  to `v0.4.0-rc.1.0.20260707084645-e194457dc507`).
- **Unit-tested** (`pkg/scoreobserver/picker_test.go`): a reconstructed Pick cycle
  exports the real scores as gauges and delegates the max-score selection.
- **Config-load verified**: running the built EPP binary with `deploy/default-plugins.yaml`
  loads the plugin and makes it the active Picker
  (`Picker: endpoint-score-observer/endpoint-score-observer`).
- **Not yet run in-cluster** with live traffic — that needs the image in a pullable
  registry (below) and requests through a gateway route.

## Build

`go.mod`/`go.sum` are committed and pinned, so a plain build is reproducible.
To match a *different* running EPP image, re-pin: `go get github.com/llm-d/llm-d-router@<commit>; go mod tidy`.

```bash
cd contrib/epp-score-observer
docker build -t <registry>/epp-score-observer:v1 .     # multi-stage; CGO_ENABLED=0 static
docker push  <registry>/epp-score-observer:v1
```
Use a registry the cluster can pull (ghcr anon is 403 here — use an internal/authed registry).

### No local docker? Build in-cluster with kaniko (pushes to your registry)
```bash
# context = this dir uploaded to a PVC/git; then a kaniko Job runs the Dockerfile.
# (kaniko does the go build itself — no Go/docker needed locally.)
```

### No registry at all? Run the static binary via the shared store PVC
Mirrors the cluster's host-stack pattern (see lmd-top e2e notes): build the static
binary, `kubectl cp` it onto the `model-store` PVC, and run a Deployment on a minimal
base image whose command is the mounted binary. Avoids images/registries entirely.

## Deploy — attach to an EXISTING llm-d environment (recommended)

The natural way to add this to any llm-d install: **swap the EPP image + inject the
picker into that EPP's existing config**. `deploy/install.sh` does both, for one or all
EPPs in a namespace, without assuming any names or a fixed config:

```bash
deploy/install.sh --image <registry>/epp-score-observer:<tag> --ns llm-serving
# --epp d1,d2   target specific EPP deployments (default: auto-discover by image)
# --dry-run     print the config diff + image change, mutate nothing
```

What it does per EPP (see `inject_picker.py`):
- resolves the EPP's own `--config-file` → its ConfigMap + key (handles per-env names),
- injects `endpoint-score-observer` into `plugins` and makes it the profile picker
  (removing any explicit max-score/random picker; **idempotent**, preserves the rest —
  including env-specific bits like Furiosa's `furiosa_llm_*` metric specs),
- swaps the image and `rollout restart`s.

Requirements: `kubectl`, `python3` + PyYAML, and an image the cluster can pull.

**Metrics scraping.** The metrics live on each EPP's existing `:9090` (plaintext HTTP).
If your Prometheus already scrapes EPP metrics (typical for kube-prometheus-stack based
llm-d installs), `epp_endpoint_score` appears automatically. If not, apply
`deploy/servicemonitor.yaml` (a PodMonitor).

**Verify** (scores populate once traffic flows through the gateway):
```bash
kubectl port-forward deploy/<epp> -n llm-serving 9090:9090 &
curl -s http://localhost:9090/metrics | grep epp_endpoint_score
```

## Rollback

```bash
deploy/install.sh --image ghcr.io/llm-d/llm-d-router-endpoint-picker-dev:main --ns llm-serving
# then drop the picker: kubectl edit cm <epp-cm> -n llm-serving  (remove the two
# endpoint-score-observer lines) — an unknown plugin type would otherwise fail config load.
```

## Verified in-cluster (2026-07-08)

Exercised end-to-end on a live llm-d cluster **without docker or a registry**: built the
static binary with Go, `kubectl cp`'d it onto the shared `model-store` PVC, ran it as an
EPP (busybox base, binary from PVC), wired a test gateway route, and sent requests —
`epp_endpoint_score` / `epp_endpoint_picked_total` populated with real per-endpoint values.
Notes that shaped the tooling:
- EPP ext-proc (`:9002`) is **TLS** — keep `--secure-serving` on (default); a plaintext
  ext-proc makes the gateway FailClose (500/503).
- Metrics (`:9090`) are **plaintext HTTP** even with secure-serving on.
- GIE won't let a pod belong to two InferencePools — a parallel test pool needs its own
  backend pods (or repoint an existing EPP in place, which is what `install.sh` does).

## lmd-top side

`lmd-top` scrapes `epp_endpoint_score` (PromQL by `pod`) and fills the EPP
view's per-endpoint `score` column — `–` until the metric exists, auto-fills
once this plugin is deployed and traffic flows. No lmd-top redeploy needed to
turn it on beyond the version that added the query.
