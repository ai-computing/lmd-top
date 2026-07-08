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

## Deploy

1. **Enable the picker in the EPP config** (`deploy/default-plugins.yaml` — adds the
   plugin type + picker ref to the mounted ConfigMap):
   ```bash
   kubectl create cm llmd-router-epp -n llm-serving \
     --from-file=default-plugins.yaml=deploy/default-plugins.yaml \
     --dry-run=client -o yaml | kubectl apply -f -
   ```
   (Repeat per-serving EPP ConfigMap if you run more than one, e.g.
   `serve-*-epp`, `gemma4-rbln-epp`.)
2. **Swap the image** (reversible — re-run with the original image to roll back):
   ```bash
   deploy/swap-image.sh <registry>/epp-score-observer:v1 llm-serving
   ```
3. **Verify** the metric appears (needs traffic through the gateway to populate scores):
   ```bash
   kubectl port-forward deploy/llmd-router-epp -n llm-serving 9090:9090 &
   curl -sk https://localhost:9090/metrics | grep epp_endpoint_score
   ```
   Then in `lmd-top` the EPP view's `score` column fills in per endpoint.

## Rollback

```bash
deploy/swap-image.sh ghcr.io/llm-d/llm-d-router-endpoint-picker-dev:main llm-serving
# and restore the original ConfigMap (drop the endpoint-score-observer lines).
```

## lmd-top side

`lmd-top` scrapes `epp_endpoint_score` (PromQL by `pod`) and fills the EPP
view's per-endpoint `score` column — `–` until the metric exists, auto-fills
once this plugin is deployed and traffic flows. No lmd-top redeploy needed to
turn it on beyond the version that added the query.
