# Resource inventory JSON

`lmd-top --json` exposes an additive `resource_inventory` object in
`lmd-top/agent-state/v2`. Existing fields keep their meanings. Consumers should
ignore unknown fields within a supported schema version.

- `state`: `fresh`, `empty`, `stale`, `error`, `unknown`, or `unsupported`, using the
  same quality rules as `source_states.inventory`.
- `last_success_at`: Unix seconds of the successful API observation, or `null`.
- `allocatable_by_node`: node name → Kubernetes resource key → integer units.
- `requested_by_node`: node name → resource key → effective assigned pod requests.
- `totals`: objects with `resource`, `allocatable`, and `requested`. Cluster requests
  also include unscheduled pods, so they may exceed the sum of node requests.

A successful empty pod list records explicit zero requests for observed nodes.
Missing keys or an empty object without successful source evidence mean unknown,
not zero. Failed refreshes may retain old counts: consumers must inspect `state`
and `last_success_at` before using them. Detailed errors remain in `sources.inventory`.

Counts use Kubernetes extended-resource units, not exporter metric-row counts or
utilization percentages. Effective requests account for ordinary containers,
sequential init peaks, persistent init sidecars, limits fallback and pod overhead.
These are resource observations, not a guarantee of schedulability. Node health,
cordons, taints, affinity, other resource demands and runtime compatibility remain
separate constraints. This object intentionally does not emit a boolean `can_deploy`.
