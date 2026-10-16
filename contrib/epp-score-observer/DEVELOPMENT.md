# Policy development workspace

Generated with `lmd-top --policy init --workspace ./my-policy`.
The Go module is pinned to router e194457dc507. New Go logic requires a new EPP image;
changing only policy parameters requires a new config and EPP restart.

1. `make test` — unit tests and pinned framework adapter tests; no cluster required.
2. Edit `pkg/heterogeneous/policy.go` (pure Evaluate), add scenarios to policy_test.go.
3. `make replay` — JSON candidate decisions, rejection reasons and tied winners.
   The replay uses the same Evaluate as the plugin. It is not traffic replay or a latency simulator.
4. Edit `policy.yaml` (target + parameters); `examples/parameters.json` is the replay input.
   Keep their parameters identical when comparing behavior. Example capacities are synthetic.
5. `lmd-top --policy render --file policy.yaml --base examples/base-epp.yaml > epp.yaml`
6. `make build` or `make image IMAGE=YOUR_REGISTRY/epp-policy:VERSION`.
   Publishing the image is a separate operator action.
7. `lmd-top --policy plan --file policy.yaml > rollout.yaml` reads the named live Deployment
   and ConfigMap and creates a reviewable manifest. Or open `:Routing policy` in the TUI,
   enter policy.yaml's path, review, validate (`v`), apply (`a`, admin + confirmation).
8. Watch the EPP Deployment rollout and send canary requests before widening traffic.
   Rollback with `kubectl -n NAMESPACE rollout undo deployment/NAME` or Serving → Rollback.
   The old ConfigMap remains so the old image/config pair is recoverable. Retain old images
   and Deployment revisions; garbage collect old ConfigMaps only after retiring those revisions.

Capacity is measured completed requests/sec for equivalent model revision, quantization,
input/output length distribution and concurrency. The heuristic is
`score = 1 / (1 + (waiting + running) / capacity)`, with an implicit 1-second reference.
It is not predicted TTFT/TPOT and does not guarantee an SLO. Other profile scorers still contribute.

Classes match pod labels, not node labels. Explicit selectors should include the model
revision/precision compatibility group when needed. The model allowlist matches the request's
TargetModel exactly. maxInputTokens is an optional input-only guard, not total context or
artifact ABI validation. With a guard, missing parser tokenization rejects the candidate.
No GPU↔NPU KV format interoperability is assumed.

The companion load extractor requires exactly one finite, nonnegative integer gauge series
for each configured waitingMetric/runningMetric per endpoint. Set the actual vendor/engine
metric names (the example uses vLLM names). It invalidates partial scrapes and timestamps
complete samples independently of unrelated telemetry. Stale, missing, ambiguous classes,
unknown models and excessive known input lengths are rejected; an empty candidate set fails
routing instead of bypassing compatibility. Configure and test a valid fallback class if needed.

The planner preserves existing plugins, scorer weights, profile handlers, and explicit data
sources. It adds the filter/scorer to only the selected profile. It adds a load extractor to
the selected Prometheus source. It does not replace an existing picker, create a gateway or
InferencePool, or claim that an arbitrary image contains this plugin. Use the pinned image
built here and validate its configuration with a scratch deployment before production.
