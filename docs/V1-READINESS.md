# lmd-top 1.0 readiness

Status: active work, not a release declaration. Baseline inspected 2026-09-28:
Cargo version 0.36.0, 247 Rust unit tests + 8 CLI tests passing. The working tree
already contains unreleased improvements; those changes are preserved.

## Objective

Bring the complete tool to 1.0 readiness through refactoring, functional additions,
visual improvements, bug fixes and verification. A passing existing test suite or
a version-number change alone does not establish readiness. Published registration
attachments remain frozen snapshots; ongoing source improvements do not rewrite them.

## Acceptance gates and evidence

| Gate | Required behavior / deliverable | Authoritative verification | Status |
|---|---|---|---|
| Data correctness | Distinguish fresh/stale/failed/missing/unsupported data; do not turn collector failure into zero; independent source timestamps; consistent UI/JSON | Collector failure/recovery fixtures, snapshot merge tests, fake HTTP/kubectl integration, live read-only observations | In progress |
| Core refactoring | Testable collection/state merge, explicit operation plans/execution, view/input boundaries; remove duplicated policy and vendor assumptions | Module boundaries plus regression tests covering caller behavior | In progress |
| Operations | Permission enforcement at execution, explicit review/validation, conflict protection, bounded subprocesses, recoverable failure and useful audit records | Fake kubectl operation traces, CLI/TUI tests, isolated scratch-resource checks where needed | Partial; existing hardening requires audit |
| Model lifecycle | Reliable model/build/run identities; source/toolchain provenance; compatibility evidence; compile/deploy/retry/store workflows with immutable publication | Golden semantic tests, interrupted publication tests, vendor recipe checks and relevant runtime evidence | Partial; full provenance incomplete |
| Inference analysis | Evidence-based SLO/bottleneck diagnosis, comparable GPU/NPU metrics, PD role visibility and explicit unknown states | Deterministic mixed-device fixtures and negative tests; observed metrics where available | Partial; observation logic tested, matched runtime comparison unverified |
| Visualization and UX | Coherent overview/status, traceable cross-layer navigation, useful empty/error states, readable tables/charts across widths/themes, no blocking UI I/O | Render matrix including 80x24, narrow/resize tests, actual PTY captures | Partial; render/PTY checks pass, interaction audit ongoing |
| Interfaces | Validated CLI/config, stable documented machine schema, discoverable diagnostics and setup; no misleading success | CLI failure matrix, JSON contract/schema tests, install/update tests | Partial |
| Distribution | Reproducible source build, exact versions, license/dependency inventory, CI/release checks, migration/recovery docs | fmt/clippy/Rust/Python/Go checks, package extraction build, installer/release workflow review | Partial |
| Final audit | All gates above proven; known defects resolved or bounded with accurate behavior; user-facing docs match implementation | Versioned readiness report with actual commands/results and limitations | Not started |

## Work order

1. Collection quality and snapshot freshness, including failure/recovery UX and JSON.
2. Workload/route identity, metrics joins and resource capacity correctness.
3. Operation execution and validation safety; subprocess/terminal lifecycle.
4. Build provenance, compatibility and model lifecycle completion.
5. Inference diagnosis/PD/heterogeneous comparison and visual improvements.
6. CLI/config/schema, distribution, live validation and final 1.0 audit.

Future ideas in older ROADMAP documents are inputs, not evidence of implementation.
Unsupported vendor/runtime combinations must stay explicitly unknown; fake values,
synthetic demos or narrowly scoped tests cannot prove real inference compatibility.

## Progress log

- Performance table display and selection now share the same direction-aware,
  filtered row order. Previously the renderer used perf_rows_order directly,
  while selection reversed that result and applied a separate filter, so reversed
  or filtered views could select a different model than the visible row. Numeric
  columns use the common missing-last comparator in both directions; model names
  provide deterministic ties. Nonfinite-only metrics do not establish activity.
  Regression coverage compares displayed/selection order for four numeric columns
  in both directions and checks case-insensitive filtering and selected model.
  Full suite: 410 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. Real traffic/profile attribution and
  the separate per-Pod queue panel remain independent verification concerns.
- Numeric sorting for accelerator/node columns and Overview/Serving model metrics
  now uses a shared comparator. Finite values sort numerically, while None/NaN/
  infinities remain last in either direction with existing identity tie-breakers.
  This removes partial_cmp(None)-as-equal comparisons that could violate ordering
  transitivity when followed by a name tie-breaker. Tests check antisymmetry and
  transitivity over finite/nonfinite/absent triples in both directions and actual
  Node row order for four numeric columns. Full suite: 409 unit + 11 CLI passed,
  one opt-in capture ignored; strict Clippy, formatting and whitespace checks
  passed. Specialized performance ordering remains separately implemented.
- Pod/accelerator-to-model navigation no longer guesses Deployment identity from
  a Pod name prefix or falls back to the Pod name as a model. It requires unique
  controller-UID-derived membership with fresh Pods/ReplicaSets/workloads sources.
  All pivots into Serving now select the exact resolved Deployment artifact, not
  the first substring match, and retrace when none exists. Regression coverage
  resolves unrelated names, rejects lookalike names and ambiguous ownership,
  and expires each prerequisite independently; existing navigation tests pass.
  Full suite: 407 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. Other correlation-filter pivots still
  need auditing; this is not a claim that every cross-layer link is verified.
- Added a whole-view rendering regression injecting positive/negative infinity
  into node and accelerator metrics, across every view with detail both on/off.
  It reproduced an Overview disk NaN% (infinity divided by infinity). Disk ratio
  now uses the checked percentage/Metric helpers, and 19 NaN-only UI predicates
  now treat all nonfinite values as unavailable (clock, temperatures, bandwidth,
  CPU/load, prefix metrics and related displays). Full suite: 406 unit + 11 CLI
  passed, one opt-in capture ignored; strict Clippy, formatting and whitespace
  checks passed. This verifies nonfinite presentation for the injected fields;
  semantic range validation and live inference remain separate concerns.
- Headless snapshots now include source quality, sample counts, Unix last-success
  timestamps, age and read errors. Missing/future timestamps have unknown age;
  absent metadata is explicitly unknown. Gateway's retained Programmed value
  carries its observation state beside it instead of appearing unqualified.
  Regression coverage renders a failed refresh retaining a successful Gateway,
  an empty Pod list and a future Node timestamp, preserving their different
  meanings. Full suite: 405 unit + 11 CLI passed, one opt-in capture ignored;
  strict Clippy, formatting and whitespace checks passed. Source success still
  means collection success, not service readiness or end-to-end inference.
- Extracted headless snapshot formatting from the main loop into snapshot_text.
  The renderer receives Snapshot, Config and history records and returns the full
  text without filesystem, process or stdout access. Main retains collection,
  history loading and emission. Existing nonfinite/route formatting regressions
  moved with the renderer; an end-to-end formatting fixture checks all primary
  sections, missing node metrics, implicit route configuration and query warnings
  without a cluster or history file. Full suite: 404 unit + 11 CLI passed, one
  opt-in capture ignored; strict Clippy, formatting and whitespace checks passed.
  Source freshness still uses the current clock; this refactor does not prove
  runtime inference behavior or complete release readiness.
- Retarget validation now catches two predictable split-rule failures before
  changing a manifest: named-rule duplication/policy-attachment ambiguity and
  exceeding the 16-rule HTTPRoute limit. It leaves both cases unchanged with
  explicit-edit guidance, while preserving supported in-place updates of named
  single-match rules and routes at capacity. Tests assert no partial mutation and
  preserved rule names for in-place updates. Full suite: 403 unit + 11 CLI passed,
  one opt-in capture ignored; strict Clippy, formatting and whitespace checks
  passed. Automatic policy-aware splitting is not implemented; this guard does
  not establish route runtime convergence or attached-policy correctness.
- Aligned headless text snapshots with the routing/measurement semantics of TUI
  and JSON. Route output now uses current configuration evidence with explicit
  runtime-unverified wording instead of epp_in_path=true/false, and includes
  owning HTTPRoute, match descriptions and complete backend descriptions. Text
  metric formatting now suppresses both infinities and NaN, including optional
  per-model metrics, while preserving measured zero. Regressions exercise unknown
  versus fresh route sources, implicit POST/Pool configuration and nonfinite
  numeric formatting. Full suite: 401 unit + 11 CLI passed, one opt-in capture
  ignored; strict Clippy, formatting and whitespace checks passed. Text remains
  human-oriented; structured automation should use the documented JSON contract.
- Full local release gate passed after route decoding/topology/JSON and summary
  changes: 399 unit + 11 CLI, 24 Python tests, all four actual PTY scenarios,
  strict Clippy/format/shell syntax, dependency inventory, Rust/Go policy parity,
  Go tests/replay/build and isolated extracted-package verification. The package
  contains 178 files (3.9 MiB unpacked, 2.2 MiB compressed). A read-only live JSON
  run against llm-serving returned nine route rows. Independently fetched
  HTTPRoutes matched every row's UID/resourceVersion, rule/match conditions and
  complete backend references. All relationship prerequisite sources were fresh;
  route_configuration reported Pool configuration with runtime_verified=false.
  The observed cluster had 8 nodes, 13 accelerators and 0/22 models serving;
  incomplete memory/disk totals and absent inference metrics remained null.
  Evidence: /tmp/lmd-routing-summary-release-gate.log,
  /tmp/lmd-routing-summary-live.json, /tmp/lmd-routing-summary-raw-routes.json.
  These observations validate collection fidelity, not actual inference traffic,
  vendor compilation, hosted CI or cross-platform release readiness. No live
  resources were modified.
- Unified busy-count calculation between terminal and JSON. The terminal's
  existing utilization >10% threshold is now shared rather than JSON using >5%.
  Empty/incomplete utilization populations display unknown in the terminal and
  null in JSON; the summary's memory/power totals use the same complete-sum helper.
  Tests render the actual summary alongside JSON for missing, 7%, 10%, and 10.1%
  samples, plus pure empty/nonfinite cases. Full suite: 399 unit + 11 CLI passed,
  one opt-in capture ignored; strict Clippy, formatting and whitespace checks
  passed. Documented the threshold correction for JSON consumers. These are
  telemetry summaries, not accelerator allocation or scheduling availability.
- Agent cluster telemetry now uses complete-population sums, preserving null for
  empty or incomplete measurements rather than emitting empty sums as zero.
  Utilization average, accelerator memory/power and node disk totals are nullable;
  busy count is also null if any observed accelerator lacks finite utilization.
  Real measured zero remains zero and each metric's completeness is independent.
  Regression coverage checks empty, measured-zero and mixed complete/incomplete
  device populations. Full suite: 397 unit + 11 CLI passed, one opt-in capture
  ignored; strict Clippy, formatting and whitespace checks passed. Documented the
  consumer null-handling change and limits: row/discovery freshness still requires
  source quality, and shared-memory totals are not deduplicated physical capacity.
- Added structured per-match routes to agent-state/v2, including resource identity,
  namespace, rule/match positions, implicit-path information, original conditions,
  every backend reference and currently verified Deployment candidates. Collection
  retains typed configuration independently from display summaries. JSON contract
  coverage checks an implicit POST match with weighted cross-namespace backends,
  exact untruncated reference preservation, identity and unknown source quality.
  Full suite: 396 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. Schema additions are documented;
  complete schema validation and route runtime status remain separate work.
- Route decoding now retains a complete backendRefs description (including
  weights, namespaces, API groups and additional backends) for topology display.
  Model ownership/navigation require one named supported backend in the route's
  own namespace. Cross-namespace names, foreign API groups, absent targets and
  weighted/multiple targets no longer borrow the first backend's local binding
  and present it as the complete route relationship. Decoder/resolver regressions
  cover local defaults, explicit local namespaces, foreign namespaces/groups,
  multiple weighted references and no references. Full suite: 395 unit + 11 CLI
  passed, one opt-in capture ignored; strict Clippy, formatting and whitespace
  checks passed. Long backend descriptions may be clipped; complete YAML remains
  inspectable. Cross-namespace ownership resolution and weighted-backend editing
  are still incomplete, and runtime traffic is not inferred from these settings.
- Extracted HTTPRoute row decoding from the Kubernetes collector into the pure
  routing module. Rules with absent/empty matches or match conditions without
  explicit paths now remain visible using the implicit root path, with an
  explicit implicit-path annotation. Header/method conditions and UID identity
  are preserved. Root-path editing refuses implicit conditions before selecting
  a rule, preventing an implicit row from mutating an explicit-root sibling.
  Regressions cover default/explicit matches, empty lists, second-rule Pool
  detection and all three editing operations without unintended mutation.
  Full suite: 394 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. Default semantics checked against
  [Gateway API v1.2.0 HTTPRoute definitions](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api/v1.2.0/apis/v1/httproute_types.go).
  Condition-aware editing remains incomplete; implicit paths require explicit
  HTTPRoute editing. This is configuration visibility, not routing convergence.
- Verified route relationships now require fresh evidence at the time of use,
  not just a binding map left in the snapshot. Service relationships require
  routes/services/workloads/pods/replicasets/endpointslices; Pool relationships
  require routes/pools/workloads/pods/replicasets. Any missing, stale, failed or
  empty prerequisite suppresses model/pod topology links and route-to-model
  pivots. Both consumers use the same resolver. A matrix regression independently
  invalidates each required source for both backend kinds; existing navigation
  and rendering tests now establish explicit observation timestamps. Full suite:
  392 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy, formatting
  and whitespace checks passed. Concurrent list snapshots remain non-atomic, and
  freshness is not proof of runtime request traversal.
- Routing topology now resolves model ownership through the same verified
  Service/InferencePool binding helper used by navigation, instead of comparing
  backend and Deployment names. Pod children come from a new current
  Pod/ReplicaSet/Deployment controller-UID membership map, not name prefixes.
  Unknown/shared model relationships display unknown rather than no-serving;
  InferencePool configuration is labelled POOL, not proof of EPP execution.
  Tests reject changed owner UIDs and foreign namespaces, and render an unrelated
  but owned Pod while excluding a backend-prefix impostor. Full suite: 391 unit
  + 11 CLI passed, one opt-in capture ignored; strict Clippy, formatting and
  whitespace checks passed. Pod membership describes Deployment ownership, not
  readiness or the exact subset selected by a particular Service/Pool; live
  runtime traffic verification remains open.
- Routing topology now displays the owning HTTPRoute, one-based rule/match
  positions, path match type and all non-path condition fields beneath each route
  row. This exposes method/header/query distinctions previously hidden behind the
  same path. Collection preserves condition descriptions; pure formatting and
  rendered same-path GET/POST regressions verify both source positions and methods,
  including narrow rendering checks. Full suite: 389 unit + 11 CLI passed, one
  opt-in capture ignored; strict Clippy and formatting passed. Long conditions can
  be clipped by terminal width; YAML inspection remains available. This does not
  yet replace path-based editing with condition identity or add implicit matches.
- Fixed path-based HTTPRoute editing of duplicate paths within one rule. The
  previous selector rejected duplicates across rules but renamed/deleted/retargeted
  all matching conditions within a single rule, including distinct headers or
  methods that the selected row did not identify. All three operations now reject
  that ambiguity before mutation. Regressions cover path type, method, header and
  query differences and assert the entire object is unchanged after rejection.
  Full suite: 387 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. Full condition-aware route selection
  and editing remain future work; existing explicit HTTPRoute editing is required
  for these ambiguous cases.
- Separated configured InferencePool backends from proven EPP traversal. Agent
  JSON adds route_configuration with source quality, nullable current observation
  and runtime_verified=false; the legacy epp_in_path boolean remains for v2
  compatibility. Topology/EPP/overview/pivot displays no longer claim runtime
  traversal or bypass from that boolean, nor explain missing endpoint data as
  bypass. Observations now scan every backend reference, require the InferencePool
  API group and a name, and become unknown when route data is stale or failed.
  Tests cover second-position backends, wrong/missing groups, source failure/age,
  successful empty results and JSON field behavior. Full suite: 386 unit + 11 CLI
  passed, one opt-in capture ignored; strict Clippy and whitespace checks passed.
  First-backend route rows/editing and runtime routing convergence remain separate
  work; configuration presence does not validate traffic weights or attachment.
- Revalidated the current workspace after Gateway selection and Setup changes:
  384 Rust unit + 11 CLI, 24 Python tests, all four actual PTY scenarios (normal,
  pending Zoo read, manifest success, manifest conflict), strict Clippy, format,
  shell syntax, inventory, Rust/Go policy parity, Go tests/replay and plugin build
  passed. Used Go 1.25.11 from the existing isolated toolchain. Source packaging
  independently passed with an isolated target directory: 177 files, 3.9 MiB
  unpacked / 2.1 MiB compressed; inspected the archive for the new review/Setup
  modules and the updated gate files. Local release checks and reusable CI now
  require this offline locked extracted-package build, preventing packaging from
  silently remaining an optional manual check. Local checks allow dirty worktrees;
  CI requires clean source. Hosted CI and cross-platform release builds were not
  run, and this does not verify vendor compilation or real inference traffic.
- Gateway creation no longer prefers Cilium or the first list entry when several
  classes are installed. Setup exposes one candidate row per observed class;
  choosing a row opens the corresponding manifest for review with admin required
  for apply. A single class keeps the direct review flow. Failed prerequisites
  and stale observations disable every candidate, and an existing Gateway hides
  creation candidates. Regression tests exercise both choices through setup_enter,
  parse the previewed class, and check uncertainty/age/existing-object cases.
  Full suite: 384 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. Controller support and readiness are
  not inferred from a class name, and no live resource was changed.
- Setup reads the configured class from the named Gateway itself and displays it
  alongside an explicit readiness/EPP-routing-unverified qualifier. Removed the
  claim that any installed Cilium GatewayClass proves this Gateway bypasses EPP,
  including the unverified controller migration instructions. Existence remains
  an existence check, not routing health. Regression coverage exercises Istio and
  Cilium selections with both classes installed, unknown selection, and a real
  subprocess query whose selected class differs from the installed class list.
  Full suite: 383 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. Runtime routing convergence remains
  a separate unverified gate; no cluster changes were performed.
- Platform prerequisite collection now overlaps seven independent CRD/Role/
  Secret/PVC reads in a fixed-size group after namespace access is established.
  GatewayClass and Gateway reads form a second concurrent group only when the
  Gateway CRD is observed present. Existing per-command bounds and per-row unknown
  tracking remain intact. A subprocess fake-kubectl test withholds every first-
  group response until all seven queries have started, proving overlap rather
  than assuming speed from a timing threshold. It also returns one Forbidden
  response and one successful absence and verifies their distinct states alongside
  successful results. Full suite: 382 unit + 11 CLI passed, one opt-in capture
  ignored; strict Clippy and formatting passed. No live latency benchmark or
  cluster mutation was performed.
- Extracted SetupProbe and prerequisite collection into `setup_probe.rs`, removing
  roughly 110 lines of platform-specific query/error bookkeeping from the main
  collector. The collector retains orchestration and re-exports the observation
  type; Setup/CLI behavior and the existing unknown/age regressions are unchanged.
  Full suite: 381 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy,
  formatting and whitespace checks passed. This structural change adds no runtime
  compatibility or release-readiness claim.
- Setup records probe completion time and expires its namespace/CRD/Gateway/
  Role/Secret/PVC assessments using the configured stale threshold (minimum ten
  seconds). Missing/future timestamps and expired observations render unknown
  without setup write actions; independent Prometheus and inventory observations
  retain their own assessment paths. Fresh probe results restore guided actions.
  Also corrected common source quality so a future success timestamp is unknown
  rather than fresh after saturating subtraction. Tests cover absent/aged/future
  probe times, recovery and source-clock catch-up. Full suite: 381 unit + 11 CLI
  passed, one opt-in capture ignored; strict Clippy and formatting passed.
  Rebuilt binary also passed both Setup-to-manifest success/conflict PTY scenarios,
  confirming fresh probes still expose the intended guided action and audit flow.
- Setup's accelerator/device-plugin assessment now requires fresh or successfully
  empty inventory evidence. Initial collection failure, missing source status,
  stale retained capacity or an expired observation render unknown with no plugin
  installation suggestion. A verified empty inventory still produces the existing
  warning; fresh advertised capacity remains visible. The check uses current time
  rather than the snapshot timestamp, so pausing does not preserve a false fresh
  assessment. Regression covers missing, empty, fresh, failed-retained and aged
  observations. Full suite: 379 unit + 11 CLI passed, one opt-in capture ignored;
  strict Clippy and formatting passed. Advertised capacity does not establish
  device-plugin health or successful inference.
- Setup now distinguishes failed CRD/GatewayClass/Gateway/Role/Secret/PVC probes
  from observed absence. Failed rows render `?`/unknown and offer no install,
  creation or token-entry action. Gateway prerequisite uncertainty propagates
  to its row; PVC lookup uses ignore-not-found so a missing object remains distinct
  from a failed query. Namespace query failure says access unknown, not proof that
  the cluster is unreachable. Both the TUI summary and `--check-setup` count unknown
  checks; CLI returns failure for unknown or missing prerequisites. Token presence
  no longer claims download authorization. Regression covers unknown action
  suppression, recovery to known absence, and offline CLI output/failure status.
  Full suite: 378 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy
  and formatting passed. Probe errors currently provide generic access guidance;
  vendor plugin readiness and credential validity still require runtime evidence.
- Extracted confirmation execution-slot handling into `app/confirmation.rs`.
  Approval previously discarded the confirmation before checking whether another
  operation was active. Busy approval now preserves the reviewed target/plan,
  resets the choice to No and requires an explicit retry. Cancellation remains
  available while busy; unprepared manifests cannot acquire the slot. Regression
  verifies preserved UID/version, unchanged active operation, explicit retry and
  cancellation. Full suite: 377 unit + 11 CLI passed, one opt-in capture ignored;
  strict Clippy passed. Rebuilt binary passed both manifest success/conflict PTY
  scenarios, including terminal restoration and audit checks. No live cluster
  mutation was performed.
- Fixed route action-menu subject ambiguity: deletion previously looked up the
  first row with the selected path, while rename/retarget reread live selection.
  Menus now capture the entire selected Route (HTTPRoute name/path/UID/version)
  and all three mutations inherit it without path-only or current-selection
  fallback. Menu titles identify the owning HTTPRoute. Regression opens a menu
  on the second of two identical paths, removes that row from the live snapshot,
  changes selection and verifies all three operations retain the original target.
  Execution's existing identity guard then rejects a changed/deleted resource.
  Full suite: 375 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy
  and formatting passed. No live route changes were performed.
- Added observed InferencePool → Deployment relationships from same-namespace
  matchLabels membership and complete Pod/ReplicaSet/Deployment controller UID
  chains. At least one selected Pod is required; missing/foreign ownership or
  unsupported selector keys prevent a binding. Multiple verified owners are
  retained in additive JSON `pool_bindings`, while model pivots require exactly
  one owner. Regressions cover matching unrelated names, recreated owner UIDs,
  namespace mismatch, empty pools, unsupported selectors, foreign controllers,
  multiple owners and UI ambiguity handling. Full suite: 374 unit + 11 CLI passed,
  one opt-in capture ignored; expanded binding tests and strict Clippy passed.
  Read-only live JSON contains the new field with all prerequisite source states
  fresh. It is empty: an independent current API read found six Pools and zero
  selector-matching Pods. No inferred model links or live mutations were made.
  Evidence: `/tmp/lmd-pool-binding-live.json`. This verifies membership logic and
  the current empty state, not inference readiness or EPP runtime convergence.
- Route-to-model navigation no longer treats the backend name as a Deployment.
  Service routes resolve through the verified Service/Deployment relationship;
  missing or ambiguous relationships show guidance and retain the current view.
  Serving navigation selects the exact resolved artifact rather than the first
  substring match. Tests cover differently named Service/Deployment objects,
  similarly named artifacts, unknown/ambiguous bindings, and InferencePool names
  that lack a verified Deployment relationship. The latter remain unavailable
  for model pivots rather than using the previous name guess; a verified Pool
  selector/ownership join is still needed for full Pool-to-model navigation.
  Full suite: 373 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy
  and formatting passed. Pod/device filtering after a pivot retains its existing
  correlation-search semantics and is not proof of complete endpoint ownership.
- Route retarget candidates now use actual namespace Service names from the
  tracked Services query, rather than treating Deployment/model names as Service
  names. Selectorless and currently endpoint-free Services remain valid candidates;
  names are deduplicated and namespace-filtered. Failed/empty Service collection
  does not fabricate Service candidates from workloads. If no Service or Pool
  candidates are observed, the action explains how to refresh/check sources
  instead of opening an empty form. Regressions cover namespace filtering,
  selectorless/idle Services, form choices and the empty state. Full suite:
  372 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy/fmt passed.
  Destination existence and ports remain rechecked at execution; selection does
  not establish serving readiness or freeze destination identity.
- Route rename/retarget/delete now carry the selected HTTPRoute UID/version from
  collection through the edit form and confirmation. Execution requires identity
  before I/O and rejects a loaded object that differs; existing conditional
  replace retains that identity through validation/application. Form tests prove
  background refresh cannot change the selected identity. Fake-kubectl traces
  cover all three operations, missing identity, UID replacement, version changes,
  validation conflicts and write conflicts, preserving other route fields and
  verifying no retry. Also connected previously guarded node/Pod/Job identities
  to the common confirmation/audit identity accessor. Full suite: 371 unit +
  11 CLI passed, one opt-in capture ignored; strict Clippy passed. Destination
  Service/InferencePool review identity and actual routing convergence remain
  separate limitations; no live route changes were performed.
- Added real PTY scenarios for manifest review success and conflict. They navigate
  Setup's generated namespace manifest, attempt confirmation while preparation is
  deliberately blocked, inspect the displayed reviewed UID, approve, and assert
  exact get/dry-run-merge/conditional-validation/update order plus matching audit
  UID/version and outcome. The fake kubectl asserts the full preserved payload;
  conflict has no re-read, retry or force. Both scenarios now run in CI and the
  local release gate. No real cluster writes occur.
  The first run exposed a validation-environment problem: earlier `cargo package`
  verification had left the development executable/depfile referring to the
  extracted package copy, and normal builds reported fresh. Cleaned only this
  package's Cargo artifacts and rebuilt; verified the depfile references current
  workspace sources. Earlier general PTY results alone did not prove the new
  confirmation behavior. Source package verification is now documented with a
  separate target directory to avoid contaminating development artifacts.
  The corrected full gate passed: 369 unit + 11 CLI, 24 Python, all four PTY
  scenarios, Go tests/replay/build, policy parity, dependency inventory, shell
  syntax, strict Clippy and formatting. Evidence:
  `/tmp/lmd-reviewed-real-binary-gate.log`. This closes the fake-cluster interactive
  confirmation gap; real admission/RBAC/controller and vendor runtime evidence
  remain separate requirements.
- Bound manifest preparation to confirmation and audit. `Pending::Apply` now
  carries an immutable prepared plan tied to the exact source and namespace;
  unprepared or edited plans fail before mutation. TUI preparation runs on one
  background worker, with generation/source/namespace checks discarding late
  results after cancellation, reopening or edits. Preparation failure stays
  failed until explicit retry, and confirmation defaults back to No when ready.
  `p` displays scrollable target operation/UID/version metadata, `e` invalidates
  preparation, and `v` refreshes it. Execution validates and uses those objects
  directly without re-preparing from newer live state. CLI explicit apply prepares
  the same bound plan. Audit adds a trailing JSON target array for success/failure,
  with resource data excluded and explicit/default namespaces distinguished.
  Tests cover stale responses, permission gating, namespace/source binding,
  unprepared execution rejection, narrow target review and audit payload omission.
  Verification: 369 unit + 11 CLI passed, one opt-in capture ignored; strict
  Clippy/build/fmt passed. Normal and slow-Zoo PTY checks passed on the new binary.
  Those PTY checks exercise general terminal lifecycle, not a complete interactive
  manifest confirmation/application; fake kubectl traces cover execution conflicts.
- Generic manifest execution now resolves non-Job targets before mutation.
  Absent targets use create; existing targets use a server dry-run apply to
  compute the complete merged object, verify the observed UID/resourceVersion
  survived preparation, then validate and execute replace with those conditions.
  This preserves unmentioned fields while refusing execution-time changes;
  no force or automatic conflict retry is used. The fake-kubectl trace covers
  new/existing targets, merge-time changes, dry-run conflicts, real update
  conflicts, concurrent creation, and Job conflicts. All validation precedes
  mutation. Existing-resource operations now additionally require get/update
  RBAC permissions. Reference semantics: [Kubernetes API updates](https://kubernetes.io/docs/reference/using-api/api-concepts/#updates-to-existing-resources)
  and [kubectl replace](https://kubernetes.io/docs/reference/kubectl/generated/kubectl_replace/).
  Live server dry-run on a ConfigMap confirmed full merged data and UID/version
  preservation, accepted conditional replace, and unchanged live version/labels.
  No real cluster mutation was executed. Full suite: 365 unit + 11 CLI passed,
  one opt-in capture ignored; strict Clippy passed. Remaining gap: preparation
  currently happens inside execution, after confirmation. Binding the reviewed
  target versions to the confirmation UI and audit record is still required.
- Current integrated checkpoint after manifest planning/Job-create changes:
  `scripts/check-release.sh` passed using the isolated Go 1.25.11 toolchain.
  Results: 365 Rust unit + 11 CLI tests, 24 Python tests, normal and slow-Zoo
  PTY checks (resize, alternate-screen restoration, termios restoration and quit
  during a pending read), Go tests/replay/build, Rust/Go policy rendering parity,
  78-package Rust dependency inventory, shell syntax, formatting and strict Clippy.
  `cargo package --allow-dirty --locked --offline` packaged 174 files and built
  the extracted source successfully. Archive membership was checked for the new
  manifest/local-file/latency/service-binding modules, inventory and terminal
  scripts, and inventory tests. Logs: `/tmp/lmd-current-integrated-release.log`
  and `/tmp/lmd-current-source-package.log` (local evidence, not shipped artifacts).
  This verifies the current Linux development environment, not hosted release
  jobs, other OS/architectures, real vendor compilation or inference performance.
  Remaining leading operations gap: generic non-Job manifest application lacks
  a target identity/version captured for review and enforced at execution.
- 2026-09-28: inspected current source and readiness/QA documents; reran baseline
  Rust tests (255 passed). Identified full-snapshot freshness overwritten by fast
  collection and discarded collection errors as the first correctness defects.
- 2026-09-28: separated source quality and snapshot merge into `quality.rs` and
  `snapshot.rs`. Full refresh timestamps no longer advance on fast samples;
  failed refreshes retain prior values with their original success timestamps;
  successful empty responses clear removed resources. Collector task completion
  timestamps are captured inside tasks rather than when their handles are awaited.
  Source status/state and warnings are exposed in agent JSON; the TUI header and
  diagnosis show degraded collection, and `:Data sources` provides error details.
  Node/device/performance histories only append new successful observations.
  Missing memory/temperature/power no longer default to zero during collection;
  the summary and dot gauges explicitly handle unavailable values.
  Verification: `cargo test --locked` (254 unit + 8 CLI passed),
  `cargo clippy --locked --all-targets -- -D warnings` and `cargo fmt` passed.
  Remaining data gate work: source coverage for inventory/events/jobs and detailed
  performance, fake collector integration, field-level partial failures, complete
  missing-value rendering across views, and live read-only validation. These unit
  checks do not establish the full data gate or 1.0 readiness.
- 2026-09-28: added shared measured-value formatting and capacity ratios; missing
  accelerator/host measurements render as placeholders in tables, details, trees,
  and overview aggregates. Dot/continuous/stacked gauges distinguish unknown from
  measured zero. Incomplete model metric totals no longer become zero or a partial
  total. The performance aggregate explicitly labels the mean of endpoint p95s
  rather than implying a combined p95. Missing throttle samples remain unknown.
  Render regression found and fixed NaN text in node aggregate headings.
  Verification: 258 unit + 8 CLI tests passed; strict Clippy and fmt passed.
  Render matrix: 13 views × normal/detail × 3 sizes (40x12, 80x24, 140x45) ×
  4 themes, checking for leaked NaN text and crashes with missing metrics.
  Live read-only `--json` completed: 8 nodes / 13 accelerators, valid v2 JSON with
  source states (performance empty, other tracked sources fresh). This proves
  current collection connectivity, not operations or inference workload identity.
  Remaining: health presence vs observed health, workload identity (proxy must not
  imply an LLM serving), missing detailed performance fields, and full collector
  failure/recovery integration coverage.
- 2026-09-28: added pure workload classification from command/image evidence and
  explicit role annotations. Removed deployment-name exclusion and blanket model
  inclusion. EPP/proxy/PD gateway deployments no longer inflate model counts;
  recognized prefill/decode workers remain model workloads. Image-entrypoint
  vLLM deployments are recognized even without `vllm` in their args. Unknown and
  non-model deployments remain discoverable in `:Workload classification` and
  JSON `workloads`, with evidence and selected container. Custom runtimes can opt
  in using `lmd-top.io/workload-role`. Classification is heuristic runtime evidence,
  not a loaded-model or successful-inference check. 262 unit + 8 CLI tests pass.
  Live read-only validation: 29 deployments classified as 12 model / 10 PD workers /
  2 gateways / 5 EPP. Model workload count is now 22 with 0 Ready, replacing the
  misleading previous 1-serving result caused by a Ready Python proxy. No unknown
  deployment in this observed cluster; custom runtimes elsewhere remain heuristic.
  Strict Clippy and formatting passed. Readiness does not imply an inference probe.
- 2026-09-28: extracted confirmed-operation execution into `mutation.rs` and added
  an execution-time mode gate for every Pending variant in `permissions.rs`.
  Store-delete danger privilege now survives preview, confirmation and editing;
  generic Apply cannot declare a requirement below admin. Negative scale targets
  are rejected before I/O. Headless plan/apply requires explicit admin+ mode before
  collection; docs/help describe the behavior change. Confirmation shows required
  and current modes. Regression tests call the actual worker for all Pending variants
  in observe/debug, cover destructive plans in admin and hidden secret values, and
  execute the CLI with no kubectl available to verify early refusal. Broader operation
  gate remains incomplete (context/UID pinning, executor trace and audit coverage).
  Validation: `cargo test --locked` passed 265 unit + 9 CLI tests; strict all-target
  Clippy and formatting passed. No real-cluster mutation was performed.
- 2026-09-28: introduced session kubeconfig pinning and unified all Kubernetes
  process construction through it. Current-context changes cannot redirect later
  collection or mutations; initial configuration failure is cached without falling
  back to the current context. Confirmation and agent JSON identify the context.
  Added exclusive 0600 snapshot creation and normal-exit cleanup (including setup
  failure return). No snapshot for commands that never access Kubernetes.
  Tests: fake kubectl changes source context immediately after config read and
  verifies every subsequent call uses the pinned configuration; checks permissions
  and cleanup. Unit tests reject invalid context mappings and verify file lifecycle.
  `cargo test --locked`: 266 unit + 10 CLI passed; strict Clippy and fmt passed.
  Resource UID/version drift and mutation audit context still require further work.
  Live read-only JSON verified pinned context `kubernetes-admin@kubernetes`, 8 nodes,
  13 accelerators, expected source states, and no remaining session snapshot file
  after normal exit. No cluster mutation was performed.
- 2026-09-28: scale/stop/restart now capture reviewed Deployment UID/resourceVersion
  in Pending and preserve them across background collection. Execution uses JSON
  Patch tests for both dry-run and application; no automatic retry on conflict.
  Restart copies existing template metadata before changing its restart annotation.
  Tests exercise the actual worker via fake kubectl subprocesses: successful scale
  and restart, recreation before validation, change after dry-run, preserved labels/
  annotations, and exact command traces without conflict retries. UI dispatch test
  verifies a later snapshot cannot replace the already-confirmed target identity.
  Real API validation (server dry-run only): matching identity accepted, mismatched
  UID rejected on vllm-qwen05b-gb10. No persisted cluster update requested.
  Remaining operation scope: rollback/node/deletion/generic-apply identity checks,
  context-aware audit records and stale preview handling for manifests.
- 2026-09-28: moved mutation audit recording into the worker before UI delivery and
  routed CLI plan/apply through that executor. Audit scope appends pinned context,
  request/default namespace, reviewed UID/version, and elapsed time while retaining
  legacy TSV columns. New files use mode 0600; every field strips control characters.
  Write failures surface separately without retrying completed operations. Secret
  apply no longer forwards raw command stdout/stderr, which can echo credentials.
  Tests verify worker audit scope/status for success and conflict, no duplicate record
  on UI delivery, explicit write errors, preserved successful UI state after audit
  failure, and Secret output suppression. This is not a crash-durable intent journal;
  actual manifest object namespaces and interruption recovery remain audit limitations.
  Validation: 274 unit + 10 CLI tests passed; strict all-target Clippy and formatting
  passed. Operation tests used fake kubectl; no real cluster mutation was performed.
- 2026-09-28: replaced unguarded rollout undo with a pure rollback planner and
  conditional Deployment template patch. History selection uses Deployment controller
  UID and numeric revision, refuses ambiguous/absent/invalid history and paused/no-op
  targets, and removes only the ReplicaSet-generated pod-template-hash. Pending keeps
  the original Deployment identity, and audit names the chosen revision/ReplicaSet.
  Fake-kubectl worker tests cover successful rollback, preserved replica count and
  template metadata, and conflict after server dry-run with no retry; pure tests cover
  foreign-owner histories, ambiguity, replacement, invalid revision and no-op.
  No real-cluster rollback was performed. Deletion/node/generic manifest identity,
  release validation, broader inference analysis and lifecycle gates remain open.
  Validation: 277 unit + 10 CLI tests passed; strict all-target Clippy and formatting
  passed. Runtime rollback tests used a fake API through real subprocess execution.
- 2026-09-28: extracted evidence-based SLO evaluation into `performance.rs`.
  Fixed vacuous all-met success when no measurements exist and partial-target success.
  Missing/nonfinite/negative values, stale collection, and absent positive request
  traffic produce explicit unknown results. Removed inferred accelerator headroom
  and automatic knob-direction advice based on missing/fuzzy-joined utilization.
  Stage p95 comparisons are described as independent observations, not additive
  latency contributions or proof of bottlenecks. Preemption is an observation rather
  than an assertion of memory thrashing. Added `:Performance assessment` report.
  Objective matching refuses ambiguous partial matches; invalid/nonpositive/nonfinite
  targets preserve the form and previous objective instead of silently clearing it.
  Validation: 283 unit + 10 CLI tests, strict Clippy, and formatting passed. Added
  rendered Perf-view checks for partial/stale SLO evidence and no false "met" text.
  Targets remain session-local; hardware comparisons, persisted targets, end-to-end
  probe evidence and complete inference analysis gate are not established by this work.
- 2026-09-28: expanded atomic build publication to provenance schema v2 with requested
  and effective options, observed package versions, recipe SHA-256s, requested image,
  model revision when recorded by saved config, host/Python and execution Pod UID.
  Canonical provenance fingerprints distinguish recorded inputs; artifact checksums,
  image digest/driver/firmware and runtime compatibility remain explicitly unknown.
  Reserved build metadata env cannot be overwritten through debug passthrough; token
  environment is not captured. Provenance probe displays the record and a real local
  shell test found/fixed a control-character bug in RBLN version extraction.
  Validation: 285 Rust unit + 10 CLI + 11 Python tests passed; strict Clippy and fmt
  passed. Compile/probe golden manifests regenerated and metadata env reviewed.
  Vendor compilers were not run. New finding: some compile form options are not
  applied by recipes (RBLN quantization; Furiosa batch/buckets/chunk/block/activation
  DQ). Requested/effective records expose this; adapters or explicit rejection remain
  necessary before 1.0. See BUILD-PROVENANCE.md for contract and limitations.
- 2026-09-28: compile numeric parameters now reject malformed, zero, negative and
  overflowing values before manifest generation for both vendors, preserving optional
  `chunk=none`. Regression tests exposed an unbounded device-choice allocation from
  user-supplied TP; choices now use observed device counts and default device arithmetic
  saturates. RBLN runtime/registry checks moved out of the Furiosa preflight branch.
  Validation: 20 compile tests including golden manifests, formatting and strict
  all-target Clippy passed. Vendor recipe option forwarding remains an open gate;
  these checks do not establish SDK compatibility or successful hardware compilation.
- 2026-09-28: wired RBLN quantization selection into the actual recipe as
  `rbln_quantization` dictionaries for w8a8/w4a16, following vendor SDK sources
  v0.9.1/v0.10.2 and current dev. Help explicitly requires prequantized checkpoints;
  selecting a format does not calibrate or quantize ordinary weights. Effective
  provenance includes the submitted dictionary. Unknown formats fail and SDK
  quantization errors are never retried by dropping the option. Four recipe tests
  execute against a recording SDK and check forwarding, provenance and failures.
  Validation: 287 unit + 10 CLI + 15 Python tests passed; compile golden recipes
  regenerated and inspected. Hardware compilation and Furiosa option adapters remain
  open; these mock SDK tests prove invocation behavior, not binary compatibility.
- 2026-09-28: added Furiosa FXB host compiler concurrency and bucket optimization
  controls, forwarded through manifest/recipe and effective provenance. Defaults are
  8 workers/O3; workers are explicitly independent of inference batching. Shell
  recipe tests record exact argv with custom values and prove failed compilation
  skips publication. Removed the incorrect furiosa-ai organization-only compatibility
  claim; repository names cannot establish supported architecture or quantization.
  Validation: 20 compile Rust tests, 18 Python tests, fmt and strict Clippy passed.
  Golden env/recipes inspected after regeneration. Actual SDK execution and legacy
  batch/chunk/block/activation-DQ option handling remain open lifecycle work.
- 2026-09-28: malformed --set syntax now exits 2 before collection; compile plans
  reject unknown option names and duplicate canonical/label aliases. Discrete vendor
  choices are validated before Job creation. Regression coverage includes misspellings,
  duplicate aliases, invalid enum values and CLI execution without kubectl. Full tests
  also exposed an obsolete assertion that repository organization proved FXB support;
  updated it to require explicit unverified compatibility for both organizations.
  Validation: 289 unit + 11 CLI tests passed; strict Clippy and formatting passed.
  Deploy override validation and unmapped Furiosa legacy options remain to audit.
- 2026-09-28: shared override-name/alias validation now covers deploy plans,
  including mount and RBLN build-selection fields where relevant. Invalid replicas,
  accelerator counts, ports and routing values are rejected before YAML generation;
  the editable form is retained. Zero replicas remains a valid stopped deployment.
  RBLN implicit artifact selection validates compile parameters rather than computing
  a path from invalid options. Regression tests exercise invalid plans, duplicate
  device aliases, synthetic artifact cleanup and valid zero-replica/65535-port input.
  Validation: 290 unit + 11 CLI tests, formatting and strict all-target Clippy passed.
  Remaining work includes runtime option support, capacity estimates while editing,
  resource-operation guards and the other open acceptance gates above.
- 2026-09-28: deploy capacity evaluation now validates live replica/device input
  before arithmetic, instead of substituting one for malformed text or overflowing
  i64 multiplication during editing. Invalid inputs render an unknown demand and
  correction hint; zero replicas has zero accelerator demand. Boundary tests cover
  empty, negative, zero-device, i64 extremes and maximum accepted i32 products.
  Validation: 291 unit + 11 CLI tests, formatting and strict Clippy passed.
  Scheduling estimates still need a separate audit of missing reservation evidence,
  selected node/Ready/cordon constraints and source freshness; this change establishes
  numeric correctness, not complete Kubernetes scheduling feasibility.
- 2026-09-28: deploy capacity now scopes candidate devices to the selected node,
  excludes NotReady/cordoned nodes, and marks missing node-state evidence unknown.
  Cluster-wide inventory cannot inflate free capacity beyond eligible candidate nodes.
  Tests cover selected cordoned/ready nodes, multi-replica demand, NotReady and missing
  node metadata; existing capacity fixtures now declare their node-state evidence.
  Validation: 292 unit + 11 CLI tests, formatting and strict Clippy passed.
  Remaining defect identified: missing node_alloc still falls back to metric idle;
  inventory collection currently hides pod-list failures and lacks explicit zero
  reservation entries. Correct that collection contract before removing the fallback.
  Taints, CPU/RAM, topology and source freshness are not yet full scheduler evidence.
- 2026-09-28: inventory collection now joins node/pod reads and publishes reservation
  evidence only when both lists validate. Successful empty pod lists create explicit
  zero reservations for every observed node/resource; failed/malformed lists clear
  unavailable evidence and report a warning. Extracted pure JSON parser rejects bad
  quantities, overflows and references to unobserved nodes. Deployment capacity no
  longer substitutes metric idle for missing reservations; it reports unknown.
  Validation: 294 unit + 11 CLI tests, formatting and strict Clippy passed, including
  empty/malformed lists, quantities, terminal pods and missing-reservation capacity.
  Remaining inventory work: init/sidecar scheduling request semantics, allocatable
  rather than metric-device counts per node, field freshness and all UI consumers.
- 2026-09-28: accelerator reservation totals now use effective pod requests:
  sum ordinary containers plus persistent init sidecars, compare with each sequential
  init stage plus already-started sidecars, then add pod overhead. Missing requests
  fall back to limits; checked arithmetic rejects overflow. Pure tests cover sequential
  init peaks, sidecar ordering, accumulated sidecars, limits-only and overhead cases.
  Reference: https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/
  Validation: 295 unit + 11 CLI tests, formatting and strict Clippy passed.
  Still not complete scheduling feasibility: per-node allocatable inventory, metric
  count differences, freshness, placement constraints and live validation remain open.
- 2026-09-28: retained per-node Kubernetes allocatable alongside reservation evidence
  and switched deployment capacity to those counts instead of exporter device rows.
  Selected/Ready/cordon filtering operates on Kubernetes inventory; exporter gaps no
  longer erase schedulable capacity, and explicit zero allocatable cannot appear free.
  Inventory parse tests verify the per-node capacity value; placement tests now cover
  missing exporter rows and zero advertised devices using explicit inventory fixtures.
  Validation: 295 unit + 11 CLI tests, formatting and strict Clippy passed.
  Source freshness, placement-picker consumers, taints/topology/CPU/RAM constraints
  and live observations still need completion before scheduling claims or 1.0 release.
- 2026-09-28: added inventory as an independently timestamped source. Full snapshot
  merge retains its previous capacity/reservation evidence and original success time
  on failure, while a successful empty response clears it. Existing source diagnostics
  and JSON source states expose the new status. Deployment fit refuses stale/failed
  inventory even when retained counts could otherwise indicate enough capacity.
  Tests cover retention, successful-empty recovery, expiry and initial errors.
  Validation: 296 unit + 11 CLI tests, formatting and strict Clippy passed.
  Placement picker and other inventory consumers still need the same evidence audit;
  this does not establish full scheduling feasibility or final 1.0 readiness.
- 2026-09-28: deployment placement picker now reuses deploy_fit per candidate,
  including any/spread, replacing duplicated metric-count/reservation-default logic.
  Kubernetes inventory supplies candidates even without exporter samples; unknown or
  stale evidence cannot produce green schedulability/free indicators. Missing memory
  metrics render placeholders. Picker tests cover inventory-only nodes, stale source
  status, and selected-node manifest preservation. Fixtures explicitly supply capacity.
  Validation: 296 unit + 11 CLI tests, formatting and strict Clippy passed.
  Capacity remains advisory: taints, affinity, topology, CPU/RAM and driver/runtime
  compatibility require additional evidence; placement selection is not a guarantee.
- 2026-09-28: extracted pure inventory parsing/effective-request accounting into
  inventory.rs, leaving collection I/O and quality handling in collect.rs. Added an
  opt-in captured-API test with no network or mutation. Read-only kubectl capture of
  8 nodes and 144 pods passed that test: allocatable GPU=2, RBLN=4, RNGD=7; observed
  requests were 0 for all three. Node sums match cluster totals. Temporary API files
  were removed after verification; no workload was deployed, scaled or compiled.
  Validation: 296 unit + 11 CLI tests passed (1 capture test ignored by default and
  passed separately), formatting and strict Clippy passed. Live zero-request evidence
  does not validate nonzero sidecar scheduling on hardware or prove all 1.0 gates.
- 2026-09-28: exposed resource_inventory as an additive agent-state/v2 JSON object:
  per-node allocatable and effective requests, cluster totals, quality and last-success
  time. Explicit zeros remain distinct from missing maps, and retained stale counts
  carry their original timestamp. Added JSON-INVENTORY.md consumer contract covering
  pending requests, extended-resource units and scheduling limitations.
  Validation: all 4 agent JSON contract tests, formatting and strict Clippy passed.
  This interface change does not complete the remaining lifecycle/operations/UI gates.
- 2026-09-28: deployment preflight now reports the actual configured Furiosa image,
  matching manifest generation. Image references are labelled configured with pull/
  runtime verification unknown, not ready. Driver-label evidence is scoped to the
  selected Ready, uncordoned candidate; absent labels no longer assert definite Pod
  Pending behavior. Unknown capacity requests evidence refresh rather than suggesting
  stopping unrelated workloads. Added regression coverage for image overrides,
  unverified runtime status and selected-node driver evidence.
  Validation: deployment-preflight regression, formatting and strict Clippy passed.
  Real image/driver/runtime compatibility probes remain an open lifecycle gate.
- Resumed work: Furiosa batch/chunk/block/activation-DQ non-default values now fail
  preflight instead of silently producing a build that ignores them. Help identifies
  legacy defaults as SDK-preset placeholders, not enforced settings. Regression tests
  require every unsupported override to fail without an apply plan. Full custom
  bucket adapters remain open; explicit refusal prevents false support claims.
- Resumed interactive work: Furiosa SDK-preset fields no longer accept digit/text/
  backspace edits or cycle through unsupported values. Cycling resets incompatible
  historical values to the legacy placeholder, preserving recovery from old forms.
  CLI non-default overrides remain explicit errors. Existing field labels/aliases
  are retained for compatibility; help explains SDK ownership. Added actual form
  input tests for all four fields. Validation: 300 unit + 11 CLI tests passed (one
  opt-in capture test ignored), strict Clippy passed. Full SDK bucket adapters remain
  open; this UI correction does not claim their implementation.
- Compile memory estimate audit: unknown FXB buckets or absent measured device memory
  no longer yield Fits/Oom certification from placeholder/default values. Heuristic
  assumptions are displayed; unsupported batch recommendations and definitive AOT
  driver-label failure claims removed. Quantization advice requires a compatible
  checkpoint. Regression coverage adds unknown-evidence cases; existing RBLN fit
  tests now supply measured capacity. Validation: 301 unit + 11 CLI tests passed,
  one opt-in capture ignored, formatting and strict Clippy passed. Actual FXB bucket
  inspection and runtime memory evidence remain required for full lifecycle readiness.
- Added streaming artifact-content manifests and aggregate SHA-256 before atomic
  publication, plus an explicit --verify command. Provenance fingerprints remain
  distinct from byte integrity. Tests cover byte/path changes, metadata exclusion
  and corrupted published artifacts. Python suite: 20 passed; compile goldens
  regenerated for the embedded helper. This is integrity evidence, not authenticity,
  runtime compatibility or proof of the remaining 1.0 gates.
- Integrity follow-up: fixed directory symlinks being skipped as directories before
  link validation. Publication stages links without dereferencing them and rejects
  them before rename; verification rejects linked metadata and special entries.
  Directory traversal errors propagate rather than silently omitting subtrees.
  Tests cover injected directory links, rejection cleanup and unreadable walks.
  Validation: 22 Python tests passed; embedded helper goldens regenerated and tested.
  Concurrent modification/authenticity remain outside the checksum contract.
- Added scripts/check-release.sh to reproduce Rust/Python/shell/policy parity and
  EPP Go test/replay/build gates together, failing explicitly on missing tools. CI
  now syntax-checks every embedded shell recipe and names Python coverage accurately.
  Local validation: 301 unit + 11 CLI + 22 Python passed; fmt/clippy passed. Go was
  absent from PATH and inspected standard/cache locations; EPP tests were not run.
  The combined gate correctly exited 2 for missing Go. This is an environment gap,
  not a release pass; next distribution work must provision/locate Go and execute it.
- Provisioned Go 1.25.11 in an isolated temporary directory from go.dev, verifying
  the archive SHA-256 against official release metadata. Ran check-release.sh to
  completion: Rust fmt/clippy, 301 unit + 11 CLI tests, 22 Python tests, shell syntax,
  Rust/EPP policy-render parity, Go adapter/observer tests, synthetic GPU/NPU replay
  and CGO-disabled EPP binary build all passed. One opt-in captured-cluster test was
  excluded from the default Rust suite as documented. No cluster writes occurred.
  This closes the missing-Go local verification gap; runtime/UX/lifecycle acceptance
  gates above still prevent declaring 1.0 complete. Log: /tmp/lmd-release-gate.log.
- Compile-form viewport now follows the selected field when the terminal is too
  short to show all options. Render regression exercises RBLN/Furiosa rejected-input
  forms at 20x8, 40x12, 80x24 and 140x45, asserting the last selected field remains
  visible and no NaN leaks. Test passed; formatting and strict Clippy passed.
  This is buffer-render evidence, not a completed PTY/interactive usability audit;
  preflight/help scrolling and other form viewports remain to review.
- Shared selected-field viewport renderer now serves compile, deploy, prefetch and
  objective forms. Deployment regression covers a retained invalid-input form and
  its last field at 20x8 through 140x45. This removes divergent clipping behavior
  without modifying form values or submission paths. Validation: 303 unit + 11 CLI
  tests, fmt and strict Clippy passed; one opt-in capture test ignored. Long help/
  preflight content and actual PTY interaction remain open UX audit items.
- PTY startup audit found run_tui awaited initial full collection before opening the
  terminal. Removed that dependency; full collection now starts on its first background
  tick with an initially unobserved snapshot. Rust suite and strict Clippy pass.
  PTY startup/resize/help/quit experiment against unreachable Prometheus still timed
  out after this change, including a controlling-terminal retry. Therefore terminal
  restoration and responsive quit are NOT verified; investigate remaining startup/
  shutdown blocking and PTY key handling next. Harness: /tmp/lmd-pty-check.py.
- PTY follow-up isolated the remaining timeout to the harness: q opens exit
  confirmation, and a second q does not confirm it. Sending y instead produced
  exit 0, alternate-screen cleanup and exact termios restoration, without panic.
  Added reproducible scripts/check-terminal.py with bounded startup/shutdown,
  resize/help/quit input, failure exit status and cleanup. Its stub kubectl and
  unreachable Prometheus avoid cluster dependence. The committed script passed
  against the current debug binary. This verifies terminal lifecycle under missing
  backends, not rendered help contents, vendor workflows or all 1.0 UX gates.
- Help now scrolls with arrows, PageUp/PageDown and Home/End, resetting on reopen.
  Actual wrapped paragraph line counts determine the viewport and scroll limit at
  the current width, including resize after End. Enabled Ratatui 0.29's opt-in
  rendered-line-info API for this calculation (review API on dependency upgrades).
  The short-screen regression now checks the final help entry is reachable at
  height 12, beyond the former no-panic-only assertion. All 303 unit + 11 CLI tests
  passed (one capture ignored), formatting and strict Clippy passed. Form preflight
  scrolling and vendor runtime validation remain open.
- Shared compile/deploy/prefetch/objective form viewports now support PgUp/PgDn
  and Home/End independently of field navigation. Editing resumes field-following
  mode. Viewport-only Cells retain the actual rendered position and clamp manual
  scroll after resize so moving back from End works immediately. Regression covers
  reaching the last check, paging back, resuming field tracking and growing the
  terminal. Full pre-change-state suite passed; added viewport regression passed.
  Horizontal clipping of long form details remains an open usability issue; this
  change addresses vertical access only. No cluster mutation was performed.
- Shared form paragraphs now wrap long diagnostics and field values, including CJK.
  Automatic field tracking counts wrapped rows before and within the selected field;
  manual scroll bounds count actual rendered rows. This resolves horizontal clipping
  in the shared compile/deploy/prefetch/objective form bodies. Added a 24x8 regression
  for reaching both the selected field and the final long Korean diagnostic. All
  305 unit + 11 CLI tests passed, with one opt-in capture ignored; strict Clippy and
  formatting passed. Other overlays and titles are not covered by this wrapping fix.
- Preflight presentation audit removed unconditional ready/apply-ready summaries:
  passing configuration/resource checks now explicitly leaves runtime unverified.
  Failed advisory checks request review rather than claim an enforced compile block.
  RBLN catalog-family and configured-image messages distinguish catalog/configuration
  evidence from installed SDK, image pull and node compatibility. Missing node labels
  no longer assert that the compiler is definitively absent. Rust suite: 305 unit +
  11 CLI passed, one opt-in capture ignored. These wording corrections do not add
  runtime validation; vendor workflow acceptance remains open.
- Cordon/uncordon no longer invokes a name-only kubectl command. Node collection
  retains UID/resourceVersion; confirmation captures that observed identity; execution
  requires it and submits the same conditional JSON patch through server dry-run and
  apply. Both operations test UID and resourceVersion before changing unschedulable,
  rejecting replacement/concurrent updates and missing observation. Tests assert
  both guarded payloads and no-I/O rejection when identity is absent. 306 unit +
  11 CLI tests passed (one capture ignored), formatting and strict Clippy passed.
  No live node scheduling state changed. End-to-end fake/server conflict traces and
  remaining delete/generic apply concurrency paths still require operation audit.
- Added subprocess-isolated fake-kubectl command traces for cordon, uncordon,
  server-dry-run conflict and apply-time conflict. Tests execute the actual kube
  command path, verify pinned kubeconfig, node target and identical UID/version
  conditions on both calls. Dry-run failure emits no apply; apply failure propagates
  without retry. All four scenarios passed without touching a cluster. This closes
  the fake-command-path gap from the prior entry, not real API-server acceptance.
- Extracted node-list parsing from Kubernetes I/O and reject missing/non-array items,
  unnamed nodes and duplicate names instead of treating malformed responses as empty
  successful observations. Valid empty lists remain valid. Tests cover malformed vs
  empty payloads and preservation of UID/version, readiness, cordon state and address
  mapping. Existing merge_fast logic retains prior successful nodes on source error.
  Strict Clippy and formatting passed; no cluster mutations performed.
- Common tracked Kubernetes list collection now rejects missing/non-array items,
  unnamed objects and duplicate namespace/name identities before publishing a
  successful source status. Applies to workloads, pods, HTTPRoutes and InferencePools;
  merge_full therefore preserves previous observations on malformed responses.
  Valid empty lists and equal names in different namespaces remain accepted.
  Validation: 310 unit + 11 CLI tests passed (one opt-in capture ignored), formatting
  and strict Clippy passed. Untracked event/job/EPP-config collection still requires
  independent freshness/failure coverage; this is not a complete collector audit.
- Events and Gateway now use independently tracked, validated list collection.
  Failures retain prior values and last-success timestamps; successful empty recovery
  clears them. Event table labels source quality; Gateway header/topology no longer
  present retained Programmed state as current health. Source states also appear in
  existing JSON/source diagnostics. Failure/recovery regression and full Rust suite
  passed: 311 unit + 11 CLI, one opt-in capture ignored. Job/EPP-config/store sources
  and downstream uses of Gateway readiness still need freshness audit.
- Job-list collection now records independent jobs quality and validates list payloads.
  Failed refresh preserves compile/prefetch executions and original success timestamp;
  valid empty recovery clears them (covered by merge regression). Activity rows mark
  retained observations stale and suppress live progress bars/animation. Missing pod
  or log observation no longer claims the compiler is starting. Job/pod/log evidence
  still needs deeper per-component quality tracking and real vendor runtime checks.
- Integrated revalidation after viewport, node-conflict and collector-quality changes:
  PATH=/tmp/lmd-go-toolchain-h2muq_2t/go/bin:$PATH scripts/check-release.sh exited 0.
  Rust fmt/strict Clippy, 311 unit + 11 CLI tests (one capture ignored), 22 Python
  tests, shell syntax, policy-render parity, EPP Go packages (cached), synthetic replay
  and CGO-disabled EPP build passed. Log: /tmp/lmd-integrated-check.log.
  python3 scripts/check-terminal.py also exited 0, with alternate-screen enter/leave,
  exact termios restoration and no panic. This is current-worktree integration
  evidence, not real vendor runtime or complete 1.0 acceptance. Remaining priorities:
  EPP/store source quality, remaining mutation conflicts and blocking UI reads,
  reproducible extracted source package build, and vendor lifecycle evidence.
- EPP ConfigMap fetch/key/parse errors now publish epp_config source failure and
  retain the last parsed configuration through merge_full. Scalar YAML, absent/empty
  profile lists, invalid plugin weights and non-finite weights no longer become a
  successful empty configuration. EPP profile/scorer views label source quality.
  Removed the unobserved max-score-picker default claim; picker remains explicitly
  unverified until plugin configuration is resolved. Parser regression and full Rust
  suite pass. Multi-profile/plugin-role resolution and live EPP compatibility remain
  open; retained configuration is not proof of current runtime configuration.
- Model-store ConfigMap collection now publishes model_store quality and preserves
  previous inventory/capacity on read, missing-key or malformed-row failure. Valid
  empty inventory clears prior entries; old six-column inventories remain accepted.
  Store legend exposes source state. Added strict inventory-row tests and extended
  recovery tests across store capacity/inventory and retained EPP configuration.
  Scan timestamps inside ConfigMap are still not tracked: successful fetch freshness
  does not prove the discovery scan itself is recent. This remains a lifecycle/data
  quality gap rather than claiming fully current artifact availability.
- Store capacity parser rejects trailing fields, overflow, negative values and
  used+available exceeding total instead of displaying impossible free-space data.
  Reserved blocks (used+available below total) remain valid. Invalid optional capacity
  is omitted while independently valid inventory remains usable. Both capacity tests,
  formatting and strict Clippy passed. Scanner timestamp/provenance remains open.
- Discovery manifest now publishes scanned_at (Unix seconds) at scan completion.
  Collection validates the timestamp, retains it with inventory on fetch failure,
  and clears it when a subsequent result lacks it. TUI shows scan age/unknown
  separately from fetch quality; JSON adds nullable store_scanned_at. Future/invalid
  times remain unknown. Full Rust suite and strict Clippy passed; scanner YAML parsed
  and extracted shell passed sh -n. No CronJob applied: live timestamp production is
  not yet verified. Existing deployments without timestamps correctly remain unknown.
- Source-package acceptance: cargo package --allow-dirty --locked --offline verified
  an extracted package build; cargo test --manifest-path
  target/package/lmd-top-0.36.0/Cargo.toml --locked --offline also passed (313 unit +
  11 CLI, one opt-in capture ignored). This used a separate package test target and
  cached dependencies, with no publication. Package contains embedded recipes,
  catalogs and EPP module source; local assistant workflow directories are now
  excluded in Cargo.toml. Verified 165-file crate SHA256:
  0231652115cdd9cf13d4508b64b7713f0ed9caf16b67640e30125b3714b8df0c.
  Logs: /tmp/lmd-package-verify.log and /tmp/lmd-package-tests.log. Version remains
  0.36.0; this dirty-worktree validation artifact is not a tagged 1.0 release.
- Scanner failure audit enabled bash pipefail, explicitly rejects unavailable root,
  and separates ConfigMap generation from apply. Required du failures stop publication;
  optional compiler provenance probes explicitly tolerate unavailable metadata.
  Executed extracted manifest script against temporary stores and fake kubectl:
  success emits create/apply, du failure emits neither, create failure never applies,
  missing root emits neither. YAML parsing and bash -n passed. No live apply occurred.
  Directory enumeration under restricted subdirectories and concurrent filesystem
  changes still require deeper scanner validation; scan timestamp alone is not an
  atomic snapshot guarantee.
- Added setup-repository scripts/test-model-store-discovery.py, extracting and
  executing the manifest's actual bash against temporary HF/RBLN/Furiosa trees.
  Six scenarios pass: normal publication, du failure, missing root, create failure,
  apply failure and optional df failure. Asserts no apply after failed create/scan,
  nonzero exit after failed apply, positive scanned_at and legacy provenance tolerance.
  Requires PyYAML; documented command in manifests/README-model-store.md. This is
  offline scanner evidence, not validation of the live container image or PVC.
- Read-only live JSON validation against llm-serving with Prometheus
  192.0.2.5:30090 completed successfully using the rebuilt binary. Source states
  fresh: nodes 8, events 54, jobs 1, gateway 1, epp_config 1, model_store 4,
  inventory 8, workloads 29. No collection warnings. store_scanned_at was null,
  correctly distinguishing the legacy scanner from a recently fetched ConfigMap.
  Local observation: /tmp/lmd-live-quality.json (not committed). Added
  docs/JSON-SOURCE-QUALITY.md documenting source mappings, retained values, sample
  counts and fetch-vs-scan freshness. No cluster configuration or workloads changed.
- EPP interpretation audit compared live default configuration (four scorer refs)
  with the repository base config containing max-score-picker. Parser now resolves
  configured aliases to known types, excludes picker/unknown roles from scorer
  weights, and displays unresolved plugins separately. Multiple profiles are listed
  with an explicit first-profile-only notice, rather than implying global coverage.
  Alias/picker/unknown/multiple-profile regression passes; 314 unit + 11 CLI tests
  and strict Clippy pass. Profile selection, exhaustive plugin role discovery and
  runtime selection semantics remain open; classification is a conservative known
  type list, not proof of arbitrary extension behavior.
- EPP parser now rejects ambiguous duplicate profile names, duplicate plugin aliases,
  repeated references in the displayed profile, missing plugin types and malformed
  plugin-definition lists. Previously a malformed definitions block could silently
  fall back to pluginRef-name classification, or conflicting aliases chose the first
  entry. Six invalid fixture cases and existing EPP tests passed (4 targeted tests),
  plus formatting and strict Clippy. Invalid configs follow the existing failed-source
  retention path. Complete multi-profile rendering/selection remains open.
- Extracted EPP configuration domain type, pure parser and its regression fixtures
  into src/epp_config.rs. collect.rs now only fetches/tracks configuration and calls
  that parser; existing type import path is re-exported for callers. This removes
  configuration interpretation from the network collector without changing runtime
  semantics. Full suite passed (315 unit + 11 CLI; one capture ignored), formatting
  and strict Clippy passed. Multi-profile selection remains implementation work.
- EPP parser now retains every validated scheduling profile. EPP view P cycles
  profiles, uses the selected profile for scorer rows/filter/order/weight editing,
  and clears local what-if weights on profile switch or configuration replacement.
  Snapshot configuration is not mutated by view selection. Removed-profile selection
  falls back to the first available profile. Regression verifies switching, row counts,
  override reset and invalid later-profile rejection. Existing overview consumers
  still summarize the first configured profile; runtime profile selection is not inferred.
- Added curated epp_profiles JSON entries for every parsed profile, named scorer
  weights, nullable configured picker and unclassified plugins. Actual picker refs
  are stored separately from display text so machine output does not classify by
  a label string. Contract test covers two profiles and unknown picker/role behavior.
  Added EPP P navigation to help and documented JSON semantics. Full Rust suite before
  typed picker-ref follow-up passed; targeted JSON test and strict Clippy passed after.
- Endpoint decision audit changed unobserved pick count/share to Option rather than
  fabricated zero. Explicit observed zero remains zero; zero-total share is unknown.
  Invalid/non-finite counts/queues/scores do not become measurements. Removed a causal
  cache-locality claim inferred only from differing pick counts and queue lengths.
  EPP view labels endpoint data as lacking profile attribution; profile navigation
  does not imply traffic was scoped to that profile. Full suite before final added
  regression passed; missing/nonfinite regression and strict Clippy passed afterward.
- Endpoint metric ingestion now rejects ambiguous duplicate pod keys and non-finite
  measurements instead of arbitrary map overwrite/duplicate rows. Explicit zero is
  retained; queues/counts must be nonnegative while finite scores may be signed.
  Collision regression and full suite pass (319 unit + 11 CLI, one capture ignored),
  formatting and strict Clippy pass. Current pod-only view still lacks namespace/pool/
  profile dimensions; scheduler attempt query is aggregated upstream and needs scoped
  metric design. Omitting ambiguous samples is not full multi-pool attribution support.
- Added independent epp_decisions/epp_queues/epp_scores quality from transport through
  duplicate/invalid-series validation and snapshot merging. Failed source values keep
  their original timestamp; independent successful empty responses clear normally.
  Endpoint panel exposes source states and suppresses decision hints on stale/error
  decision/queue observations. Expanded panel height to retain endpoint rows alongside
  quality labels. Regression covers query failure vs empty and independent retention.
- Added EPP render regression verifying profile name/picker changes after cycling,
  explicit unscoped endpoint attribution, and no NaN at 40x12/80x24/120x32.
  The unavailable-config view no longer incorrectly says ConfigMap not found for
  parse failures or transport errors; it shows source quality and diagnostic guidance.
  Render test passed. Narrow-screen assertions cover rendering/no-NaN, not complete
  readability of every help/detail line; live multi-profile traffic remains unverified.
- Unix subprocesses now run in isolated process groups. Timeout/I/O failure kills
  the group before reaping the direct child; cancellation drops a group guard and
  kills descendants as well. Normal completion disarms it. Added libc as a direct
  Unix dependency using the already locked version (no dependency upgrade).
  Timeout and cancellation regressions launch background writers and verify they
  cannot continue after the runner stops. This covers descendants that remain in
  the group, not processes explicitly daemonizing into a different session/group.
  API basis: https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.process_group
  and https://man7.org/linux/man-pages/man2/kill.2.html .
- Bounded subprocess stdout/stderr to 16 MiB each. A stream exceeding the limit
  returns an explicit error and follows process-group termination/reaping rather
  than retaining unlimited output or parsing truncated JSON as success. Real command
  tests exercise both stdout and stderr overflow, alongside timeout/cancellation.
  All five process tests and strict Clippy passed. Limit is documented; representative
  very-large-cluster response sizing remains to validate, and streaming large exports
  are not implemented by this in-memory runner.
- Prometheus raw HTTP reads now stop at 16 MiB and report an explicit non-retryable
  response-limit error. Chunk decoding requires valid hex lengths, data CRLF, final
  zero chunk and terminated trailers; incomplete frames cannot become successful
  parseable JSON prefixes. Extensions/trailers and split UTF-8 remain supported.
  Eleven Prometheus tests and strict Clippy passed; added local TCP oversize test
  also passed. Basis: RFC 9112 section 7.1. Content-Length/status/header validation
  still needs broader HTTP audit; this change does not claim full protocol coverage.
- Extracted HTTP response decoding from socket I/O. Validates HTTP/1.x status 200,
  header size/syntax, consistent Content-Length and exact body length, mutually
  exclusive length/chunked framing, supported transfer/content encoding and UTF-8.
  Header lookup is case-insensitive and accepts no-space colon syntax. Thirteen
  Prometheus tests and strict Clippy pass. Rebuilt live read-only JSON collection
  completed with zero warnings: epp_config fresh; endpoint/performance series empty,
  correctly distinct from HTTP failure. Log/data: /tmp/lmd-http-live.json.
  Client still uses Connection: close and does not implement redirects, TLS, compression
  or interim responses; these are not silently interpreted as metric success.
- Prometheus JSON parsing now requires an instant-vector result, array entries,
  string labels and a numeric timestamp/string-value pair. Missing result objects
  no longer become successful empty observations, and invalid numeric strings no
  longer silently become NaN. Genuine NaN/+Inf metrics remain representable for
  downstream unknown-state handling. Malformed-success/valid-empty fixtures and
  full suite passed: 328 unit + 11 CLI (one capture ignored), fmt/strict Clippy pass.
  This collector intentionally accepts instant vectors, not scalar/matrix queries.
- Post-network/process integration gate passed: 328 Rust unit + 11 CLI tests, 22
  Python tests, EPP Go tests/replay/build, policy parity, fmt/strict Clippy and shell
  syntax. Setup scanner regression (six scenarios) passed separately. PTY check
  passed all exit/alternate-screen/termios checks on the current binary.
  Rebuilt read-only live JSON had zero warnings, fresh GPU/RBLN/RNGD observations,
  one default EPP profile with four scorers and unknown configured picker, and empty
  endpoint/performance metrics (not errors). Logs: /tmp/lmd-final-integration.log,
  /tmp/lmd-validated-live.json. This establishes ingestion compatibility, not live
  traffic analysis/vendor lifecycle correctness: runtime traffic evidence remains
  unavailable in the observed state and those acceptance gates remain open.
- EPP coefficient-share visualization scales finite nonnegative weights before
  summation to avoid overflow. Zero-total, negative or non-finite coefficient sets
  show unavailable share rather than fabricated zero percentages. Local adjustment
  rejects non-finite results. Regression covers f64::MAX pairs, zero, signed and
  non-finite values; full suite passes (329 unit + 11 CLI, one capture ignored),
  formatting and strict Clippy pass. Shares remain coefficient ratios, not routing
  probabilities or simulated endpoint decisions.

- TUI log opening and refresh now run through one background worker. Requests for
  the same loading target are coalesced; a newer target replaces queued work.
  Closing/reopening invalidates old generations so delayed success/failure cannot
  overwrite the current overlay. Empty output and failures have explicit states;
  failed reads can be retried. Closing the overlay does not cancel the bounded
  kubectl request; a newer request waits for that worker to finish. Regression
  tests cover stale results, close/reopen, coalescing and error/retry. Full suite:
  331 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy passed.
- Pod and compile Job deletion now captures reviewed UID/resourceVersion from the
  collected row and uses atomic DeleteOptions preconditions for both dry-run and
  persisted raw DELETE. Missing identity blocks before kubectl; conflicts never
  retry against a replacement. Path validation restricts raw deletion to the two
  fixed resource endpoints. Notifications report deletion requested, not completed.
  Isolated subprocess command traces cover Pod/Job success, dry-run conflict
  (no persisted request), and persisted conflict (no retry), with identical reviewed
  identity and pinned kubeconfig. Current restricted-environment checks pass:
  332 unit + 11 CLI, strict Clippy, formatting. One opt-in capture is ignored;
  one existing HTTP loopback test was explicitly excluded after PermissionDenied
  on bind under the network sandbox. No live resource deletion was performed.
  Generic manifest application still requires its own concurrency audit.
- Prometheus label discovery now rejects missing/non-array data and non-string
  elements rather than silently reporting no metric names or dropping bad entries.
  Empty valid arrays remain successful. The production HTTP request/write/bounded
  read is extracted over AsyncRead + AsyncWrite; TCP connection and timeout remain
  in the caller. A small duplex buffer tests async backpressure without socket
  permissions: exactly 16 MiB succeeds after EOF, and 16 MiB + 1 fails before EOF
  with a non-retryable size error. This replaces the permission-dependent loopback
  size test, not the production transport. All Rust tests now run in the restricted
  environment without that exclusion: 335 unit + 11 CLI passed, one opt-in capture
  ignored; strict Clippy and formatting passed. Network reachability/DNS remain
  outside the scope of these in-memory transport tests.
- Model Zoo refresh no longer blocks key handling: one async refresh runs at a
  time, repeated refresh keys report an existing request, and completion is merged
  on the UI thread. Supplier failures and malformed response/id failures remain
  explicit; partial results preserve the existing catalog and report incomplete
  refresh rather than claiming up-to-date. Curl now uses the shared process timeout,
  output limit and process cleanup. Merge also deduplicates repeated new entries
  within one remote result. Regression covers malformed versus empty replies,
  partial failure messaging, unsupported roles and preservation/deduplication.
  Full Rust verification: 337 unit + 11 CLI passed, one opt-in capture ignored;
  strict Clippy and formatting passed. Live supplier connectivity is unverified
  in the restricted network environment. YAML and performance-detail on-demand
  reads still have synchronous UI paths requiring conversion.
- Performance histogram drill-down now queues async collection instead of calling
  block_on in the key handler. The detail view displays its loading target and
  remains navigable; one collector runs at a time, duplicate Enter is coalesced,
  and generation/model checks prevent closed or replaced views receiving old data.
  Navigation clears queued requests; a running request completes within existing
  Prometheus query budgets and its stale result is discarded. Regression covers
  close/reopen, changing models, duplicate keys and navigation away/back.
  Verification: 339 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy
  and formatting passed. YAML on-demand reads still require the same treatment.
- Both YAML inspection entry points now queue one background read instead of
  running kubectl in the input handler. A small app/reads module owns request
  generation and preview ownership checks: closing, reopening or replacing a
  preview invalidates older completions, including when a generated apply-ready
  manifest replaces it. Loading previews remain read-only and cannot be saved as
  YAML; failed reads show an error and retry instructions. Tests cover delayed
  results, close/reopen, generated-preview preservation and read failure/retry.
  Rust tests: 341 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy
  and formatting passed. There are no remaining block_on calls in main.rs; log,
  YAML, Zoo and performance-detail reads run off the input thread. Synchronous
  local file/editor operations and other UI module I/O still need final UX audit.
- Added a real PTY slow-read regression mode (`scripts/check-terminal.py --slow-zoo`).
  The current debug binary was rebuilt and run with fake kubectl/curl, with vendor
  requests deliberately delayed 20 seconds. The test waits for a started marker
  before exercising input, verifies repeated refresh does not launch more requests,
  and requires help/close/confirmed quit to exit within five seconds while the
  response remains pending. Observed all checks true: exit success, alternate
  screen enter/leave, exact termios restoration, no panic, pending-read quit and
  no completed fake response. Python syntax and diff whitespace checks pass.
  This supplies runtime evidence for Zoo responsiveness; it does not establish
  the same complete interaction coverage for every other on-demand view.
- Latency detail now keeps query failures/missing/non-finite scalar evidence as
  explicit warnings instead of silently reducing everything to NaN and labeling
  an empty distribution idle. Default percentiles are unknown, not zero; a scalar
  aggregate must have exactly one finite nonnegative sample. Histogram conversion
  validates boundaries/rates, unique ordered boundaries, nondecreasing cumulative
  values and a terminal +Inf bucket before differencing. Invalid/incomplete
  histograms are unavailable; valid zero-rate buckets remain distinguishable from
  absent evidence. Regression covers reordered valid buckets, malformed/duplicate/
  decreasing/missing-Inf/non-finite buckets, missing/ambiguous scalar results and
  real zero observations. Full suite: 343 unit + 11 CLI passed, one opt-in capture
  ignored; strict Clippy and formatting passed. Service/profile attribution and
  real traffic compatibility remain separate inference-analysis requirements.
- Latency drill-down now builds selectors from both configured namespace and
  selected Service, including the proxy metric family. Label strings are escaped
  before interpolation; detail headers show namespace/Service and the 5-minute
  rate window. Query-builder regression verifies both namespaces remain distinct,
  proxy/ordinary query sets are scoped, and quote/newline/backslash-containing
  labels cannot alter the selector. Missing namespace/service scrape labels now
  yield unavailable detail; there is no unscoped fallback. Full checks: 345 unit
  + 11 CLI passed, one opt-in capture ignored; strict Clippy and formatting passed.
  This closes the detail-query namespace gap only. Performance list/global/EPP
  aggregation scoping and evidence-based vendor-family selection remain open.
- Performance summary, per-Service percentile/throughput rows and model running/
  waiting/KV queries now bind the configured namespace before Prometheus aggregation.
  Both operands of the cache-hit ratio are scoped; existing abort filtering is
  preserved. The fixed ds4-proxy summary additionally uses the same Service filter
  as its drill-down. No unscoped fallback is used when scrape labels are missing.
  Tests exercise namespace binding, preserving other matchers, ratio operands and
  escaped label values. 346 unit + 11 CLI passed, one opt-in capture ignored;
  strict Clippy and formatting passed. Live multi-namespace Prometheus evaluation
  is not verified in this network-restricted environment; query string/unit checks
  do not establish exporter-label compatibility. EPP pool/endpoint aggregate
  scoping and cross-vendor collision handling still need implementation/audit.
- EPP queries now scope pool/queue/prefix-index metrics to the scrape namespace and
  endpoint decisions/scores to exporter endpoint namespace (exported_namespace
  preferred after Prometheus label collisions). Score ingestion restores
  exported_pod, avoiding accidental joins to the EPP scraper Pod. Inspected pinned
  GIE v1.5.0 local source and the score-observer implementation for metric labels;
  confirmed Prometheus honor_labels semantics in official configuration docs.
  Tests cover two model endpoints sharing an EPP scrape Pod and namespace selectors.
  348 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy passed.
- After network restrictions lifted, rebuilt and collected live read-only JSON:
  /tmp/lmd-scoped-live.json, zero warnings, all EPP/performance queries accepted by
  Prometheus. EPP decision/queue/score and performance sources are empty. Independent
  unscoped queries (/tmp/lmd-scope-live-labels.json) also found no serving request
  counters and only inference_pool_ready_pods among inspected EPP families, labeled
  namespace=llm-serving. This verifies current query syntax/collection and that
  observed emptiness is not introduced by namespace filtering; it does not prove
  multi-namespace traffic isolation or full vendor compatibility. Cross-namespace
  EPP/pool mappings and profile/pool attribution remain open.
- Per-Service performance fields now detect multiple observations instead of letting
  the later vendor query overwrite the earlier value. Repeated conflicts remain
  unavailable, including equal-valued samples. Running/waiting/token/KV/TTFT maps
  omit Services present in both engine metric families and report the collision.
  Conflicted or invalid per-Service request/token fields also invalidate their
  aggregate totals. The aggregate TTFT no longer picks one vendor's p95 when both
  have finite observations; it reports unavailable with guidance to per-Service
  values (quantiles cannot be combined by choosing one). Tests cover disjoint vs
  overlapping Services, repeated field observations, real zero and mixed p95.
  Verification: 351 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy
  and formatting passed. This does not establish complete engine attribution or
  duplicate-scrape detection inside Prometheus aggregation; those remain open.
- Release workflow now requires reusable CI quality checks for the validated
  version tag before platform builds and publishing. Both reusable jobs check out
  the explicit source_ref, including manual dispatch of a tag from another branch;
  Go plugin tests/replay/build and Rust policy parity can no longer be bypassed by
  the publish dependency graph. CI and the complete local release script now run
  both normal PTY and slow-Zoo pending-read checks after building the TUI binary.
  YAML parsing and job dependency references checked locally; no hosted workflow
  or publication was triggered.
- Current complete local gate passed after these edits (/tmp/lmd-integrated-gated.log):
  351 Rust unit + 11 CLI, 22 Python tests, normal/slow-read PTY checks, strict Clippy,
  format, shell syntax, Rust/Go policy-render parity, Go plugin tests/replay/build.
  One opt-in Rust capture remains ignored. This verifies integrated local behavior;
  multi-platform hosted builds, real vendor compile/deploy/traffic scenarios and
  remaining reviewed-operation audit are still required for 1.0 readiness.
- Performance drill-down now uses observed metric families and preserves the actual
  Service query target separately from a displayed Deployment alias. Collection
  records vLLM/Furiosa/proxy evidence; snapshot error retention preserves it with
  the corresponding performance rows. Requests capture that evidence before async
  work. Unknown or mixed families stop before network I/O; model names no longer
  select a metric family. Furiosa uses TTFT/inter-token histogram contracts already
  present in the collector, with E2E explicitly unsupported rather than guessed.
  JSON per_model_perf includes nullable service and metric_families. Tests cover
  family selection independent of names, supported/unsupported Furiosa queries,
  ambiguous/no-evidence handling and request target survival across snapshot changes.
  Full suite passed 353 unit + 11 CLI (one capture ignored), strict Clippy passed;
  the subsequently added request-target regression also passed independently.
  Actual vendor traffic/histogram compatibility and heuristic Service-to-Deployment
  ownership remain unproven and require further work.
- Removed substring-normalized Deployment-to-Service metric matching. New
  service_binding module requires same namespace, a nonempty Service selector
  matching the Deployment Pod template, no other matching Deployment, and exactly
  one observed Service candidate. Selectorless/external, shared and ambiguous
  bindings remain unavailable. Performance joins use the per-Service metric rows,
  so missing running-request gauges no longer prevent a latency-only row binding.
  Unbound observations are labeled service/<name> instead of masquerading as a
  Deployment. Tracked services source reports read failures explicitly.
  Regression covers unrelated names with real selectors, similar names without
  a relation, shared selectors, multiple Service candidates and namespace isolation.
  Full tests: 355 unit + 11 CLI passed, one capture ignored; Clippy/fmt passed.
  Rebuilt live read-only JSON (/tmp/lmd-service-binding-live.json) has zero warnings,
  31 observed Services and 22 performance rows. These bindings prove declared
  selector relations only; actual EndpointSlice membership and other workload
  kinds sharing selectors still require a deeper ownership audit.
- Service binding now validates actual EndpointSlice membership and owner UID chains:
  Slice→Service and endpoint Pod→ReplicaSet→Deployment. Stale/recreated UID targets,
  missing/foreign controllers, selector mismatch, empty endpoints and any foreign
  selector-matching Pod block attribution. Pods are fetched once and reused by the
  normal table; ReplicaSet and EndpointSlice lists have explicit source status.
  JSON service_bindings makes the evidence inspectable. Tests cover UID changes at
  each link, foreign Job/StatefulSet ownership, pending foreign Pods and selector
  drift. Full checks: 357 unit + 11 CLI passed, one opt-in capture ignored; Clippy
  and formatting passed.
- Rebuilt live read-only JSON (/tmp/lmd-endpoint-binding-live.json): zero warnings,
  31 Services, 164 ReplicaSets, 31 EndpointSlices, 24 Pods; six verified bindings
  (proxy plus EPP Deployments). No model-server binding was fabricated from name
  resemblance. Actual proxy Service is ds4-inject-proxy, revealing the remaining
  fixed ds4-proxy query target is incorrect for this deployment and needs removal.
  Cross-resource lists are not atomic; rollout races remain conservative unknowns.
- Removed the fixed ds4-proxy metric target/row. Proxy request/token/TTFT/E2E
  queries now aggregate by the observed service label within the configured
  namespace, through the same per-Service ingestion and collision guards as other
  engines. Distinct proxy Services remain independent; labels are preserved in
  PerformanceTarget and detail queries. Unknown Deployment association remains a
  service/<name> observation rather than a fabricated model. Regression covers
  ds4-inject-proxy plus a second proxy and conflicting engine-family evidence.
  Full checks: 358 unit + 11 CLI passed, one opt-in capture ignored; Clippy passed.
  Rebuilt live JSON (/tmp/lmd-proxy-service-live.json) has no warnings and valid
  empty performance evidence; request traffic correctness remains unverified.
- Strengthened subprocess descendant timeout/cancellation regressions. Tests now
  wait for a marker written inside the background descendant, verify the runner
  has not finished before startup, and only release its pending work after the
  timeout/cancellation has completed. A surviving process would then write an
  escape marker. This removes the earlier possibility of passing because the
  100 ms timer fired before a descendant ever started. A fixture guard kills the
  isolated test process group and removes files even on assertion failure.
  Targeted process tests: five passed (2.31 s); strict all-target Clippy and fmt
  passed. This validates ordinary inherited Unix process groups; deliberate
  setsid/group escape remains outside this cleanup contract.
- Created and verified a current Cargo source package (169 files before adding the
  inventory tooling); extracted source passed 358 unit + 11 CLI tests with one
  opt-in capture ignored. Logs: /tmp/lmd-current-package.log and
  /tmp/lmd-current-package-tests.log. This verifies source completeness for the
  current host; it is not a 1.0 release or multi-platform build claim.
- Added scripts/build-inventory.py and release archive DEPENDENCIES.json containing
  target-filtered locked Rust resolution, enabled features, declared licenses,
  toolchain versions and Cargo.lock hash. Local x86_64 output has 78 packages;
  repeated generation is byte-identical. Two offline regression tests verify
  target filtering/path omission and failure propagation. CI/local release gate
  run inventory generation; release bundles it for each platform. Workflow YAML
  parsing and whitespace checks passed; hosted runs remain unverified. Go/SDK/
  container dependencies are explicitly outside the Rust inventory's scope.
- Extracted Service-scoped latency query construction, metric-family evidence,
  scalar/histogram validation and cross-engine collision handling into latency.rs,
  together with its six focused regression modules. collect.rs retains orchestration
  and snapshot assembly, with existing call sites using explicit re-exports. The
  extraction removes approximately 530 lines of independent performance concerns
  from the broad collector; Kubernetes/UI state is not required for the module's
  observation-validation tests. Full suite: 358 unit + 11 CLI passed, one opt-in
  capture ignored; strict Clippy and formatting passed. This is a structural change,
  not evidence of additional vendor runtime compatibility.
- Latency detail records query completion time and displays checked-age plus
  Enter-refresh/Esc-back guidance. Values and histogram bars become dim once the
  detail exceeds the configured snapshot stale threshold (minimum 10 seconds);
  missing/future timestamps show query time unknown. This describes last check,
  not successful measurement: existing error/missing warnings remain explicit.
  Refresh now targets the open/pending detail model, rather than whatever row a
  background collection/re-sort has moved under the selection. Render regression
  checks stale/current/unknown timestamps and narrow sizes; state regression
  verifies refresh preserves the opened model when selection disappears.
  Full checks: 360 unit + 11 CLI passed, one capture ignored; Clippy/fmt passed.
- Manifest saves now exclusively create files and add a suffix on collision,
  preserving existing files and symlink targets. External editing uses a private
  per-session directory instead of a shared fixed temporary filename; normal
  return and launch failure clean up the session and editor auxiliary files.
  Unix permissions are 0600 for created files and 0700 for editor directories.
  Tests cover repeated saves, existing symlinks, permissions, session isolation
  and cleanup. Full checks: 362 unit + 11 CLI passed, one opt-in capture ignored;
  strict Clippy and formatting passed. This prevents overwrite collisions, but
  does not promise atomic visibility during writing or cleanup after SIGKILL.
- Removed implicit name-only deletion from compile Job manifest application.
  Applying a manifest now refuses any already-existing Job, including successful
  and failed executions, and directs the operator to review/delete it explicitly
  or choose a new name. This preserves execution history and removes the race
  where a replacement Job could be deleted after inspecting an older object.
  Explicit Job deletion retains UID/resourceVersion preconditions. Regression
  cases cover active, complete, failed and malformed inspection responses.
  Full checks: 362 unit + 11 CLI passed, one opt-in capture ignored; strict Clippy
  passed. General manifest apply still needs optimistic concurrency protection;
  the absence check alone is not an atomic create precondition.
- Replaced the preceding Job absence-check approach with create-only API requests.
  Both server validation and real creation use the exact previewed name; an
  execution created between those steps produces AlreadyExists rather than an
  update or deletion. YAML documents are parsed before I/O, including commented
  separators/CRLF and generic/typed Lists, so embedded Jobs receive the same
  behavior. Resources retain document order, all dry-runs precede mutation, and
  API failures report how many earlier resources were applied. A subprocess
  fake-kubectl regression covers validation-only, success, existing Job and a
  race after validation, asserting verbs, payload names and request order.
  General non-Job apply remains unchanged in concurrency semantics; multi-object
  application is not transactional. No cluster resources were modified to test
  this behavior.
  Verification: 363 unit + 11 CLI tests passed, one opt-in capture ignored;
  strict Clippy, formatting and whitespace checks passed.
- Extracted pure YAML planning into `manifest_plan.rs`, separating parsing,
  List flattening and create/apply selection from kubectl execution. Planning
  rejects repeated named targets by API group/kind/effective namespace/name,
  including version variants and duplicates inside Lists, before validation or
  mutation. Distinct groups/kinds/namespaces and generateName executions are
  preserved. This prevents predictable mid-application duplicate failures; it
  does not establish generic optimistic concurrency or discover API resource
  aliases/scopes. Regressions exercise both dry-run and real-apply entry points
  plus distinct-target cases. Verification: 365 unit + 11 CLI passed, one opt-in
  capture ignored; strict Clippy passed.
