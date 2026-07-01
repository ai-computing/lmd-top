# Gateway API Inference Extension (GIE / GAIE) — Engineering Briefing

*Compiled 2026-07-01. Facts verified against the project source (release tags up to v1.5.0), the official docs site, the endpoint-picker protocol proposal, and Kubernetes/CNCF blog posts. Field/CR/scorer names are pinned to specific versions; uncertainties are flagged inline.*

> **One-slide summary.** GIE is a Kubernetes SIG-Network extension that turns any Envoy/ext_proc-capable Gateway API implementation into an "Inference Gateway." It adds one GA custom resource (**InferencePool**) plus alpha resources for model objectives, and delegates per-request pod selection to an out-of-band gRPC service, the **Endpoint Picker (EPP)**, that scores model-server replicas on live signals (queue depth, KV-cache utilization, prefix-cache and LoRA affinity) instead of doing round-robin.

---

## 1. What GIE is, and how it relates to standard Gateway API

The **Gateway API Inference Extension** is an official Kubernetes SIG project (`kubernetes-sigs/gateway-api-inference-extension`) that "optimizes self-hosting Generative Models on Kubernetes." It "improves the tail latency and throughput of LLM completion requests against Kubernetes-hosted model servers using an extensible request scheduling algorithm that is kv-cache and request cost aware, avoiding evictions or queueing as load increases." It does not replace your gateway — it "transform[s] your existing gateway into an Inference Gateway." ([README](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/README.md), [docs home](https://gateway-api-inference-extension.sigs.k8s.io/))

**It is purely additive to Gateway API** — it "builds on the existing Gateway API, adding inference-specific routing capabilities while retaining the familiar model of Gateways and HTTPRoutes." ([Kubernetes blog, 2025-06-05](https://kubernetes.io/blog/2025/06/05/introducing-gateway-api-inference-extension/))

| Gateway API primitive | Role under GIE |
|---|---|
| **GatewayClass** | Reused unchanged — binds the controller/implementation. |
| **Gateway** | Reused unchanged — the traffic entry point (Envoy-based). |
| **HTTPRoute** | Reused — but a route points its `backendRefs` at an **InferencePool** instead of a Kubernetes `Service`. |
| **InferencePool** *(new)* | A pool of model-server pods; "similar to a Service but specialized for AI/ML serving." |
| **Endpoint Picker (EPP)** *(new)* | An ext_proc gRPC service, bound to the pool, that makes the per-request pod choice. |

Integration mechanism: **Envoy External Processing (ext_proc)**. Any gateway supporting both ext_proc and Gateway API can become an inference gateway without a rewrite. Implementations include **Envoy Gateway / Envoy AI Gateway, kgateway, GKE Gateway, Istio, Agentgateway, and NGINX Gateway Fabric**. ([docs home](https://gateway-api-inference-extension.sigs.k8s.io/), [implementations](https://gateway-api-inference-extension.sigs.k8s.io/implementations/gateways/))

### The gap it fills (why normal HTTP LB is inadequate)

LLM inference "sessions are often long-running, resource-intensive, and partially stateful," unlike "typical short-lived, stateless web requests," so "traditional load balancers focused on HTTP path or round-robin lack the specialized capabilities" and "don't account for model identity or request criticality." ([Kubernetes blog](https://kubernetes.io/blog/2025/06/05/introducing-gateway-api-inference-extension/)) Round-robin / least-connection ignore GPU realities: replica count says nothing about accelerator saturation, queue backlog, or which model/adapter is resident. ([CNCF deep-dive, 2025-04-21](https://www.cncf.io/blog/2025/04/21/deep-dive-into-the-gateway-api-inference-extension/))

GIE adds:
- **KV-cache awareness** — steer around KV-cache-saturated pods to avoid evictions and queueing. ([README](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/README.md), [CNCF](https://www.cncf.io/blog/2025/04/21/deep-dive-into-the-gateway-api-inference-extension/))
- **Model-aware routing** — the `model` name is parsed from the OpenAI-format request body and routed to pods actually serving it. ([CNCF](https://www.cncf.io/blog/2025/04/21/deep-dive-into-the-gateway-api-inference-extension/))
- **LoRA-adapter affinity** — prefer endpoints that already have the requested adapter loaded. ([README](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/README.md))
- **Request criticality / load shedding** — protect critical/interactive traffic and shed best-effort traffic first under contention. ([CNCF](https://www.cncf.io/blog/2025/04/21/deep-dive-into-the-gateway-api-inference-extension/))
- **Live-metrics scheduling** — a filter/score pipeline picks the pod with lowest expected latency / best accelerator efficiency. ([CNCF](https://www.cncf.io/blog/2025/04/21/deep-dive-into-the-gateway-api-inference-extension/))

---

## 2. The custom resources

> **Read this first — there are two API groups in play at once**, which is the source of most confusion. At the **v1.0.0 GA** cycle, `InferencePool` graduated to the stable group `inference.networking.k8s.io/v1`, while the AI/ML-owner resource was renamed `InferenceModel → InferenceObjective` **but stayed alpha** in the pre-GA group `inference.networking.x-k8s.io/v1alpha2`. ([GA migration guide](https://gateway-api-inference-extension.sigs.k8s.io/guides/ga-migration/), [proposal 1199](https://github.com/kubernetes-sigs/gateway-api-inference-extension/tree/main/docs/proposals/1199-inferencemodel-api-evolution))

| Group | Version | Kinds | Status |
|---|---|---|---|
| `inference.networking.k8s.io` | `v1` | **InferencePool** | GA / stable |
| `inference.networking.x-k8s.io` | `v1alpha2` | **InferenceObjective**, InferenceModelRewrite | alpha |
| `inference.networking.x-k8s.io` | `v1alpha1` | InferencePoolImport (multi-cluster) | alpha |

### 2a. InferencePool

- **`inference.networking.k8s.io/v1`, kind `InferencePool`** (GA). Source: [`api/v1/inferencepool_types.go` @ v1.5.0](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api-inference-extension/v1.5.0/api/v1/inferencepool_types.go)

| `spec` field (v1) | Meaning |
|---|---|
| `selector` | Label selector that **must exactly match labels on the model-server Pods** in the pool. |
| `targetPorts` | List of `{ number: <port> }` — port(s) on the selected Pods (e.g. 8000). |
| `endpointPickerRef` | Reference to the **EPP** service that performs per-request selection. Sub-fields: `group` (default core), `kind` (default `Service`), `name` (required), `port`, `failureMode` (`FailOpen`/`FailClose`, **default `FailClose`**). |

> **Field-name caveat (alpha vs GA):** the old **v1alpha2** InferencePool used singular `targetPortNumber` (int) and `extensionRef`. The **v1** schema renamed these to `targetPorts` (a list) and `endpointPickerRef`. If a doc shows `targetPortNumber`/`extensionRef`, it is the alpha API. ([GA migration guide](https://gateway-api-inference-extension.sigs.k8s.io/guides/ga-migration/))

**Relationship to Service:** it is **not** a drop-in Service replacement. It groups pods that share accelerator type, base model, and model server, and is targeted by an HTTPRoute *instead of* a Service — but it delegates the actual endpoint choice to the EPP rather than doing round-robin. ([InferencePool docs](https://gateway-api-inference-extension.sigs.k8s.io/api-types/inferencepool/))

**Analogy:** a "smart Service for LLM servers." A Service picks any ready endpoint; an InferencePool asks the EPP "which replica has this LoRA adapter loaded / the warmest KV-cache / the shortest queue?" before routing.

### 2b. InferenceModel → InferenceObjective (the rename)

The old `InferenceModel` was **split into two resources** at GA — do not assume `InferenceObjective` is just a renamed `InferenceModel`; the fields moved.

**Old `InferenceModel`** (pre-GA, `x-k8s.io/v1alpha2`, ≤ v0.5.x). Source: [`inferencemodel_types.go` @ v0.3.0](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api-inference-extension/v0.3.0/api/v1alpha2/inferencemodel_types.go). `spec` had:
- `modelName` — value of the request's `model` field (unique per pool).
- `criticality` — enum **`Critical` | `Standard` | `Sheddable`** (drives shedding order).
- `targetModels` — `[{ name, weight }]` traffic split across model/LoRA versions.
- `poolRef` — `{ group, kind, name }`.

**Current `InferenceObjective`** (v1.x, `x-k8s.io/v1alpha2`). Source: [`inferenceobjective_types.go` @ v1.5.0](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api-inference-extension/v1.5.0/apix/v1alpha2/inferenceobjective_types.go). The spec is **radically slimmed** to only:
- `priority` — an **integer** (higher = more critical; negatives allowed; unset = 0). This **replaces the `Critical`/`Standard`/`Sheddable` enum** with an open integer scale, used by flow control under contention.
- `poolRef` — required, the InferencePool it applies to.

> **Important — despite the name "Objective," there is no SLO/latency field.** As of v1.5.0, `InferenceObjective` has **no `modelName`, no `criticality`, no `targetModels`, and no explicit SLO field** — only `priority` and `poolRef`. Any doc showing an explicit SLO field is aspirational or version-specific. Latency objectives are handled by the EPP/scheduler, not the CRD. *(Flag: still an actively evolving alpha area.)*

**`InferenceModelRewrite`** (v1.x, `x-k8s.io/v1alpha2`) absorbed the `modelName`-matching and `targetModels` traffic-splitting: `spec` = `poolRef` + `rules[]` with `matches[]` (`model: { type, value }`) and `targets[]` (`{ weight, modelRewrite }`). Source: [`inferencemodelrewrite_types.go` @ v1.5.0](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api-inference-extension/v1.5.0/apix/v1alpha2/inferencemodelrewrite_types.go). *(Flag: not present in v1.0.0; introduced somewhere in v1.1–v1.5.)*

**Analogy:** the old single "model config" was decomposed by concern — `InferenceObjective` = "how important is this workload's traffic" (priority/QoS); `InferenceModelRewrite` = "match this model name and rewrite/split it to these backend versions" (like an HTTPRoute rewrite, but for the `model` field).

### 2c. HTTPRoute referencing an InferencePool

```yaml
apiVersion: gateway.networking.k8s.io/v1
kind: HTTPRoute
spec:
  rules:
  - backendRefs:
    - group: inference.networking.k8s.io   # v1 GA pool (NOT x-k8s.io)
      kind: InferencePool
      name: <pool-name>
```

Verified from a conformance manifest: [`httproute_multiple_gateways_different_pools.yaml` @ v1.5.0](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api-inference-extension/v1.5.0/conformance/tests/httproute_multiple_gateways_different_pools.yaml). For a legacy v1alpha2 pool the `group` would instead be `inference.networking.x-k8s.io` — match the group to the pool version you deployed.

---

## 3. The Endpoint Picker (EPP)

### What it is — confirmed ext_proc

The EPP is an **Envoy External Processing (`ext_proc`) gRPC service** the gateway calls per request to pick a backend from an InferencePool. Per the protocol proposal (status *Implemented*): "The EPP MUST implement the Envoy external processing service protocol" and "MUST support streaming mode." ([Proposal 004](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/docs/proposals/004-endpoint-picker-protocol/README.md)) The gateway/Envoy still performs the actual routing; the EPP only *selects* the destination.

### Scheduling model — a kube-scheduler-style plugin framework

The scheduler is pluggable, modeled on kube-scheduler. A request runs through one or more **Scheduler Profiles**, each an ordered pipeline of three phases: **Filter → Score → Pick**. ([Proposal 0845 — scheduler architecture](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/docs/proposals/0845-scheduler-architecture-proposal/README.md))

Extension-point interfaces (`framework/plugins.go`): `ProfileHandler` (picks which profiles run — exactly one per scheduler), `Filter` (`[]Pod → []Pod`, eliminative, no weight), `Scorer` (`map[Pod]float64` normalized to **[0,1]**), `Picker` (chooses final pod(s), returns ≥1). A profile may have any number of filters and scorers, but **exactly one picker**.

**Filters vs scorers:** filters run first and are boolean/eliminative — they *remove* unfit pods (zero survivors → EPP returns 503). Scorers run on survivors and are quantitative — each yields a [0,1] score, weighted and summed. Filters have no weight; scorers do. Note the v1.x line **collapsed most former filters into scorers** (soft scoring rather than hard cuts) and moved hard sheddable-drop behavior into a separate **Saturation Detector**.

### Scorers — exact identifiers (source-verified at v1.3.0)

| Scorer | Type string | Score math (per pod) |
|---|---|---|
| Queue depth | `queue-scorer` | `(maxQ − podQ)/(maxQ − minQ)`; all equal → 1.0 |
| KV-cache utilization | `kv-cache-utilization-scorer` | `1 − KVCacheUsagePercent` |
| LoRA affinity | `lora-affinity-scorer` | active adapter → 1.0; capacity to load → 0.8; queued → 0.6; at max → 0.0 |
| Running requests | `running-requests-size-scorer` | min/max normalized on running-request count (lower better) |
| Prefix-cache affinity | `prefix-cache-scorer` | `matchedPrefixBlocks / totalPrefixBlocks` |

Sources: [`scorer/queue.go`](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/v1.3.0/pkg/epp/scheduling/framework/plugins/scorer/queue.go), [`kvcache_utilization.go`](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/v1.3.0/pkg/epp/scheduling/framework/plugins/scorer/kvcache_utilization.go), [`lora_affinity.go`](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/v1.3.0/pkg/epp/scheduling/framework/plugins/scorer/lora_affinity.go), [`multi/prefix/plugin.go`](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/v1.3.0/pkg/epp/scheduling/framework/plugins/multi/prefix/plugin.go).

**The `prefix-cache-scorer` is the LRU one.** It maintains an **approximate LRU index** of which pod holds which prompt-prefix blocks ("approximation to the actual prefix LRU cache state on the model servers"). Params: `blockSize` (prompt block size for hashing; docs default 64), `maxPrefixBlocksToMatch` (default 256), `lruCapacityPerServer` (default 31250 entries/pod), `autoTune` (size to real GPU blocks if the pod reports `CacheNumGPUBlocks`).

> **Flag on your scorer list:** `queue-scorer`, `kv-cache-utilization-scorer`, and `prefix-cache-scorer` are confirmed exact upstream names, and `prefix-cache-scorer` is the LRU one. A literal **`load-aware-scorer`** and **`sheddable-capacity-filter`** were **not found** as registered upstream types (v0.5.1/v1.3.0) — sheddability lives in the **Saturation Detector** (429 under load); those names may be from an older or downstream variant (e.g. `llm-d-inference-scheduler`).

**Classic filters** (first-class in **v0.5.1**, mostly folded into scorers since): `low-queue-filter`, `lora-affinity-filter`, `least-queue-filter`, `least-kv-cache-filter`, `decision-tree-filter` (`pkg/epp/scheduling/framework/plugins/filter/*.go` @ v0.5.1).

### How weighted scorers combine → the winning pod

From `scheduler_profile.go` / `weighted_scorer.go` (v1.3.0):
1. Each pod's accumulator starts at 0.
2. For each scorer, per pod: `weightedScore[pod] += enforceScoreRange(score) * scorer.Weight()`. `enforceScoreRange` clamps each raw score to [0,1] before weighting. Weights are plain integers (default 1). The final score is a **simple weighted sum**.
   > *Discrepancy to note:* the docs say weighting is "relative to the sum of weights," but the code does **not** divide by total weight — verify against the exact release you run.
3. The **Picker** chooses. Default `max-score-picker`: random tie-break shuffle, sort by score descending, keep top `maxNumOfEndpoints` (default 1). Alternatives: `random-picker`, `weighted-random-picker`. ([`picker/max_score_picker.go`](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/v1.3.0/pkg/epp/scheduling/framework/plugins/picker/max_score_picker.go))

Worked example (docs, weights 2/2/3): pod score = `2·queue + 2·kvcache + 3·prefix`, then max-score-picker takes the highest.

### Plugin framework / config

There is a startup config object `EndpointPickerConfig` (`apiVersion: inference.networking.x-k8s.io/v1alpha1`) — it **looks like a CRD but is not** (read once at startup via `--config-file`/`--config-text`, not reconciled). Sections: `plugins` (`{name?, type, parameters?}`), `schedulingProfiles` (`{name, plugins:[{pluginRef, weight?}]}`), `saturationDetector`, `data`, `featureGates`. ([EPP config guide](https://gateway-api-inference-extension.sigs.k8s.io/guides/epp-configuration/config-text/))

```yaml
apiVersion: inference.networking.x-k8s.io/v1alpha1
kind: EndpointPickerConfig
plugins:
- type: prefix-cache-scorer
  parameters: { blockSize: 5, maxPrefixBlocksToMatch: 256, lruCapacityPerServer: 31250 }
schedulingProfiles:
- name: default
  plugins:
  - pluginRef: prefix-cache-scorer
    weight: 50
```

**Multiple profiles** enable **disaggregated prefill/decode** (a `prefill` profile and a `decode` profile with their own filter/score/pick lists). ([Proposal 0845 example](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/docs/proposals/0845-scheduler-architecture-proposal/examples/example.yaml))

---

## 4. The request path (exact mechanism)

1. **Client → Gateway.** Client sends e.g. `POST /v1/completions`. An `HTTPRoute` whose `backendRef` is an **InferencePool** matches. The gateway does not load-balance itself.
2. **Gateway → EPP over ext_proc.** Decision is made at the request-headers phase; if the `model` name requires the body, the header response is deferred until after the request-body phase, then endpoint selection runs. ([Proposal 004](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/docs/proposals/004-endpoint-picker-protocol/README.md))
3. **EPP scores and selects** a pod (queue/KV-cache/prefix/LoRA), producing a target `ip:port`.
4. **EPP → Envoy — dual return (verified in source).** In one ext_proc response the EPP sets **both**:
   - a **header** named **`x-gateway-destination-endpoint`**, and
   - **dynamic metadata**: namespace **`envoy.lb`**, key **`x-gateway-destination-endpoint`**, value `ip:port` (comma-separated list allowed for fallback/retry).
   - It also sets **`ClearRouteCache: true`** so Envoy re-evaluates routing.

   Verified constants `DestinationEndpointNamespace = "envoy.lb"`, `DestinationEndpointKey = "x-gateway-destination-endpoint"`: [`pkg/lwepp/metadata/consts.go`](https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/pkg/lwepp/metadata/consts.go). The spec requires header and metadata values be identical and deliberately does not fix which one the proxy consumes (portability).
5. **Envoy → pod.** The final wiring is **implementation-specific** (the one implementation-dependent link):
   - **Header path:** an Envoy `ORIGINAL_DST` cluster reads `x-gateway-destination-endpoint` to set the upstream address (`ClearRouteCache` forces the re-route). ([design issue #19](https://github.com/kubernetes-sigs/gateway-api-inference-extension/issues/19))
   - **Metadata path:** `envoy.lb` dynamic metadata used for subset load balancing — the path **GKE Gateway** uses.

Errors: no ready endpoints → **503**; shed under load → **429**. Optional input subsetting constrains EPP choices via request filter metadata `envoy.lb.subset_hint` / key `x-gateway-destination-endpoint-subset`; the served endpoint is reported back via `x-gateway-destination-endpoint-served`.

---

## 5. Versioning and maturity

- **Latest release: `v1.5.0`, published 2026-04-19.** GA (`v1.0.0`) was **2025-09-09**. *(Flag: a web summary mis-dated v1.5.0 to "April 2025"; the GitHub API date is 2026-04-19.)*
- **Maturity (CRDs at v1.5.0):** **InferencePool = v1 (GA)**; **InferenceObjective = v1alpha2 (alpha)**; InferencePoolImport = v1alpha1; InferenceModelRewrite = alpha.
- **Common misconception to correct:** the `InferenceModel → InferenceObjective` rename is real, but **InferenceObjective did NOT graduate to v1** — only **InferencePool** is v1. Objective remains alpha under `inference.networking.x-k8s.io`.

**Timeline (three headline changes landed together in the v1.0.0 GA cycle):**

| Date | Version | Change |
|---|---|---|
| 2025-02-06 | v0.1.0 | First release. `InferenceModel` + `InferencePool` under `inference.networking.x-k8s.io`. |
| 2025-06-05 | (blog) | "Introducing Gateway API Inference Extension" (pre-rename, alpha). |
| 2025-07-21 | v0.5.0 | Still `inferencemodels` + `inferencepools` (v1alpha2). |
| **2025-09-09** | **v1.0.0 GA** | (a) `InferencePool` → **v1**, group → **`inference.networking.k8s.io`** (dropped `x-`). (b) `InferenceModel` **renamed `InferenceObjective`** (stays v1alpha2). (c) `criticality` enum → integer `priority`. |

Sources: [v1.0.0 release notes](https://github.com/kubernetes-sigs/gateway-api-inference-extension/releases/tag/v1.0.0), [GA migration guide](https://gateway-api-inference-extension.sigs.k8s.io/guides/ga-migration/).

**Repo split an engineer must know:** As of `main`, the GIE repo ships mainly `InferencePool` (v1) + `InferencePoolImport` + a **Lightweight EPP (LWEPP)** + conformance tests. The **full EPP, InferenceObjective, InferenceModelRewrite, and Body-Based Router have moved to `llm-d/llm-d-router`** (issue #2430). Track that repo for the scheduler/objective pieces going forward.

**Ownership & llm-d relationship:** Official Kubernetes SIG project, now **sponsored by SIG-Network** (originated in WG-Serving, which concluded ~2026-02-26). The relationship with **llm-d is bidirectional**: llm-d consumes GIE's InferencePool + EPP for inference-aware routing, and GIE's EPP data-plane components have moved *into* `llm-d/llm-d-router`. GIE is the upstream K8s standard; it did **not** originate from llm-d. ([CNCF WG-Serving conclusion](https://www.cncf.io/blog/2026/02/26/kubernetes-wg-serving-concludes-following-successful-advancement-of-ai-inference-support/), [Kubernetes blog](https://kubernetes.io/blog/2025/06/05/introducing-gateway-api-inference-extension/))

---

## Uncertainty flags (consolidated)

- **`InferenceObjective` has no SLO field** in v1.5.0 despite its name — only `priority` + `poolRef`. SLO/latency handling lives in the EPP/scheduler.
- **`load-aware-scorer` / `sheddable-capacity-filter`** not found as upstream registered plugin types; likely downstream (llm-d-inference-scheduler) or older names. Sheddability = Saturation Detector (429).
- **Weighted-sum normalization**: code does a plain weighted sum; docs say "relative to the sum of weights." Verify against your release.
- **Envoy → pod wiring** (ORIGINAL_DST header vs `envoy.lb` metadata subset LB) is implementation-dependent, not fixed by the GIE spec.
- Scorer/field details pinned to specific tags (v1.3.0 for EPP, v1.5.0 for CRDs, v0.5.1 for classic filters) — verify against your installed version, as alpha resources evolve between minor releases.
- `prefix-cache-scorer` `blockSize` default: docs say 64; confirm the code constant if the exact number matters.
