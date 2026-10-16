# Source quality in agent-state/v2

`--json` exposes `sources` (observation metadata) and `source_states` (quality at
serialization time). Consumers must read these alongside the associated values.
Retained values are not proof of current health.

`pool_bindings` maps InferencePool names to sorted Deployment names verified from
currently selected Pods and their Pod → ReplicaSet → Deployment controller UID
chains, within the same namespace. It requires a nonempty supported matchLabels
selector and complete ownership evidence for every selected Pod. Empty pools,
unsupported selectors or unknown owners produce no binding. Multiple verified
owners are preserved; the TUI only pivots to a model when exactly one is known.
This is observed membership, not readiness, EPP cache state or successful inference.
Read it with the `pools`, `pods`, `replicasets` and `workloads` source states.
Those lists are collected separately, so rollout races can make a binding
temporarily unavailable. Relationships are not retained on prerequisite failure.

| Source key | Associated observations |
|---|---|
| `nodes` | Node conditions and host metrics |
| `workloads` | Deployments and model workload identity |
| `pods` | Namespace pod list |
| `routes` | HTTPRoutes |
| `pools` | InferencePools |
| `events` | Kubernetes events (the UI retains the newest 40) |
| `gateway` | Gateway address and Programmed condition |
| `jobs` | Compile/prefetch Job observations |
| `epp_config` | Parsed EPP ConfigMap configuration |
| `model_store` | Fetched model inventory ConfigMap |
| `inventory` | Kubernetes accelerator capacity and reservations |

States are `unknown`, `fresh`, `empty`, `unsupported`, `stale`, or `error`.
A successful empty list is `empty`. A failed refresh with a previous successful
observation retains its values and timestamp and is `stale`; initial failure is
`error`. Missing source keys must be treated as unknown. `samples` counts source
observations, which need not equal rendered rows (for example, only compile and
prefetch Jobs appear in the activity feed).

A success timestamp later than lmd-top's current clock is `unknown`, not
fresh. This can occur after clock adjustments; its age cannot be established until
the clock catches up. Error details remain available separately in `sources`.

`store_scanned_at` is an additive nullable Unix timestamp in seconds, recorded by
the discovery scanner. It remains null for legacy, invalid or future timestamps.
It is retained with the inventory after a failed fetch. `model_store: fresh` means
the ConfigMap was fetched successfully; it does not mean the filesystem scan is
recent. Compare `store_scanned_at` with the consumer's clock and acceptable scan
age. The supplied discovery CronJob runs every ten minutes. A scan is not an atomic
filesystem snapshot and fetch success does not verify artifact integrity.

Similarly, fresh EPP configuration does not establish that a running EPP process
has loaded it, and fresh Job status does not imply successful pod log collection.

`epp_profiles` lists all parsed scheduling profiles in configuration order. Each
entry contains `name`, `scorers` (`plugin_ref`, `weight`), nullable
`configured_picker`, and `unclassified_plugins`. Picker references are never
counted as scorers. A null picker means it was not resolved by the known-type
parser; it does not assert that the running EPP has no picker. Read these entries
with `source_states.epp_config`. They describe configuration, not runtime profile
selection. In the TUI, `P` on the EPP view cycles profiles and clears local weight
simulations; that view choice never modifies this JSON configuration inventory.

Endpoint observations have independent `epp_decisions`, `epp_queues` and `epp_scores`
source keys. Each failed refresh retains only that source's previous values and
success timestamp. A successful empty response clears that source's observations.
Duplicate pod-only series are treated as ambiguous and reported as a source error.
These keys do not establish namespace/pool/profile attribution.

The interactive latency detail uses the configured namespace plus the selected
Service label, and displays a five-minute window. Exporters/scrape configuration
must provide `namespace` and `service`; missing labels produce unavailable detail
rather than a global fallback. Performance summaries, per-Service rows, running/waiting requests and KV-cache
queries also require the configured namespace. Proxy metrics are grouped by their observed Service labels, with no fixed
Service-name filter. Cluster-level node/device metrics remain cluster-wide.
EPP endpoint/pool aggregate isolation remains a separate, unfinished audit.

EPP endpoint decisions and scores select the configured endpoint namespace. When
Prometheus renames an exporter namespace label to `exported_namespace`, that label
is authoritative; the scrape namespace is used only when no exported namespace is
present. Scores similarly restore `exported_pod` as the endpoint identity before
joining the pod-only view. Pool and per-pod queue metrics in the inspected GIE
v1.5.0 source lack an exporter namespace label and therefore require the EPP scrape
namespace. Cross-namespace EPP-to-pool deployment is not yet resolved by this path.
Multiple pool/profile observations for one endpoint remain ambiguous rather than
being selected arbitrarily. See [Prometheus label-conflict handling](https://prometheus.io/docs/prometheus/latest/configuration/configuration/).

Each `per_model_perf` row now adds `service` (nullable observed query target) and
`metric_families` (`vllm`, `furiosa`, `proxy`; empty means unobserved, multiple means
ambiguous). These describe collector evidence, not a hardware-name inference.
Interactive detail captures that service/family with the request. Furiosa detail
currently supports TTFT and inter-token latency; E2E is explicitly unsupported
until an exporter metric contract is established. Unknown/ambiguous families do
not trigger guessed vLLM queries. A displayed Deployment alias can differ from the
observed Service; see the endpoint and controller verification requirements below.

Deployment aliases now require an unambiguous declared Service selector relation
within the configured namespace; substring/name similarity is no longer used for
these performance joins. `services` is an independently reported collection source.
Unbound Service observations use `service/<name>` row labels. Selector matching is
configuration evidence, not proof of current EndpointSlice membership or exclusive
ownership across non-Deployment workload kinds.

`service_bindings` exposes currently verified Deployment → Service candidates.
Verification now additionally requires Service-owned EndpointSlices, matching Pod
UIDs, and controller UID chains through ReplicaSets to the current Deployment.
All endpoint Pods must match the selector, and selector-matching Pods owned by
another controller invalidate the relation. Empty endpoint sets and missing
ownership evidence do not bind. `replicasets` and `endpointslices` source states
report prerequisite collection quality. Separate list requests are not a single
Kubernetes transaction; concurrent rollout changes may conservatively leave a
relation unavailable until the next collection.

## Route configuration and runtime evidence

`route_configuration` contains `state` (the routes source quality), nullable
`inference_pool_backend_observed`, and `runtime_verified` (currently false).
The backend observation is available only with fresh or successfully empty route
collection. Unknown, failed, unsupported or stale collection produces null.
All backend references in each HTTPRoute rule are inspected for a named
`InferencePool` in API group `inference.networking.k8s.io`, including references
after the first one. This is configuration presence, regardless of traffic weight,
not proof of attachment, acceptance, endpoint readiness, EPP execution, or traffic.

The legacy `epp_in_path` boolean remains for v2 compatibility and may retain an
old observation. Its name must not be interpreted as runtime verification; use
`route_configuration` for new consumers. False does not prove EPP bypass: Service
backends can themselves target proxies or other routing infrastructure. The route
rows still summarize the first backend, while the configuration observation checks
all references; full weighted-backend route editing remains a separate concern.

The TUI uses Service/Pool bindings for topology and navigation only while every
prerequisite source is fresh at the current clock. Service relationships require
`routes`, `services`, `workloads`, `pods`, `replicasets`, and `endpointslices`;
Pool relationships require `routes`, `pools`, `workloads`, `pods`, and
`replicasets`. A missing, empty, failed or expired source makes these relationships
unavailable. JSON consumers must make the same quality checks before using a
retained binding map as a current relationship.

## Structured route inventory

The additive `routes` array in agent-state/v2 contains one entry per observed
HTTPRoute match (including implicit matches). Each entry has:

- `name`, effective `path`, nullable `uid` and `resource_version`.
- Nullable `configuration`: `namespace`, zero-based `rule_index` and `match_index`,
  `implicit_path`, original `match_conditions`, and the full `backend_refs` array.
  Each backend reference preserves its fields, including weight, namespace, group,
  port and filters. An implicit match has empty original conditions; effective
  path defaults are separately reflected in `path` and `implicit_path`.
- `verified_deployments`: currently verified local ownership candidates. Empty
  means unresolved, unsupported, ambiguous backend configuration, or unavailable
  prerequisites; it does not mean no model is serving. Multiple Pool owners may
  be listed. The current resolver requires one supported local backend reference.

Read retained route entries with `source_states.routes`. Row positions are scoped
to the recorded resource version and must not be used as stable object identities.
This inventory does not expose route acceptance, attachment or runtime traffic
verification. Long terminal descriptions may be clipped; this structured JSON
preserves the collected match/backend configuration without display truncation.

## Cluster aggregate completeness

`cluster.util_avg_pct`, `vram_used_gb`, `vram_total_gb`, `power_w`,
`disk_used_gb`, and `disk_total_gb` are nullable numbers. Each is null when its
observed population is empty or any member lacks a finite value for that metric.
Measured zero stays zero. Each metric is assessed independently: missing memory
usage does not discard an available capacity sum. `cluster.busy` is a nullable
integer, known only when all observed accelerators have finite utilization;
its threshold is utilization greater than 10 percent, shared with the terminal
summary. This corrects the earlier JSON-only 5-percent threshold: values above
5 but at or below 10 are now counted as idle in both outputs.

Consumers must now accept null for `busy` and empty-population totals instead
of interpreting them as zero. Counts such as nodes/accelerators remain observed
row counts and need source-quality checks. Complete numeric coverage does not
prove discovery completeness or freshness; inspect the corresponding sources.
Memory totals sum exporter-reported device values, which can include shared
memory pools and are not a deduplicated physical-memory inventory.

Human-readable `--snapshot` output includes a `[sources]` section with the same
quality names, samples, last-success Unix timestamps, age and errors. Missing or
future timestamps have unknown age. The Gateway line includes observation quality
beside its retained Programmed value. This section provides collection context;
use JSON for machine parsing rather than relying on text layout.
