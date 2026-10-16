# Operation correctness changes — 2026-09-28

These changes are in the working source tree, not a published release. They have
been checked offline; no cluster objects, NPU builds or routing policies were changed.

## Execution history

New compile/prefetch records use `job:<namespace>/<name>/<uid>` as the ID. The UID
identifies the Kubernetes execution across deletion/recreation and clusters. Old
name-only records remain readable. The first observation of a still-visible old
Job may produce one UID-based record as well. Logs are associated with owner UID,
not Pod-name prefixes. Missing UIDs are not persisted as ambiguous executions.

## Routes

Deleting a selected path preserves the other matches in its rule. Retargeting a
path splits a shared rule and retains rule filters and backend weight. Multiple
backends, duplicate paths across rules and ambiguous Service ports require an
explicit HTTPRoute edit. Target resources are read live. The final replacement
retains resourceVersion and is server-validated; a concurrent edit produces a
conflict. The operator must still check the route's Accepted/ResolvedRefs status
and traffic after a change: API acceptance is not traffic validation.

## Endpoint exclusion

The action changes labels used by matching Services and InferencePools, after
reading live selectors. It checks Service EndpointSlice removal and attempts to
restore labels if verification fails. Unsupported selector expressions and
unreadable routing resources stop the operation. Original values are retained in
`lmd-top.io/drain-original-labels`; restore those exact values before removing the
annotation when undoing a successful exclusion. Do not blindly undo after an
external label change: recovery is conditional on Pod UID and changed values.

This is routing-selector exclusion, not proof of finished streams. It does not
verify an EPP's internal cache, direct Pod-IP traffic or all existing connections,
and never automatically deletes a drained Pod. Confirm propagation and stream
completion independently before deletion. Changing an owning ReplicaSet selector
label can cause a replacement Pod to be created.

## Compilation and publication

New artifact paths append a stable fingerprint of the model/vendor and all compile
form fields. The suffix is not a content checksum or a full toolchain fingerprint:
externally supplied compiler images, passthrough environment and mutable model
revisions are not captured by it. Existing published paths cannot be overwritten;
move a build explicitly or use a different configuration before rebuilding it.
Old paths remain deployable from the store or via an explicit mount override. A
headless deployment with an inferred default path now targets the new layout.

Recipes copy finished outputs into a hidden sibling staging directory, then rename
on the store filesystem. A publication lock prevents cooperating publishers from
racing. On failure, no partial destination is exposed. An abrupt container kill can
leave hidden `.lmd-staging-*` / `.lmd-publish-*.lock` entries. Inspect the owning Job
and confirm no publisher is running before manually removing those leftovers.
Atomic rename semantics on the actual SMB/storage backend still need E2E validation.

Re-application performs server validation before deleting an old Job, refuses
pending/running Jobs, and stops on deletion failure. Multi-resource kubectl apply
is not transactional: a later API failure may still leave partial changes.

## Process/UI behavior

The shared runner bounds the complete kubectl process, writes stdin concurrently
with output reads and terminates/reaps the direct child on timeout. This does not
provide process-group termination for a detached credential-helper descendant.
Server dry-run uses the existing operation slot on a worker thread, leaving the UI
responsive and preventing another mutation until that operation completes.

## Execution modes

Confirmed operations execute through `mutation.rs`, with `Pending::authorize`
checked before any Kubernetes I/O. Observe/debug cannot mutate, including creating
inventory/provenance Jobs. Delete operations require danger; ordinary changes
require admin. Store-delete previews carry a typed danger requirement into the
confirmation and preserve it through editing. Negative replica targets are rejected
before execution. The confirmation displays current and required modes.

Headless `--plan ... --apply` requires `--mode admin` or `--mode danger` and rejects
insufficient modes before collection. This changes the previous CLI behavior where
`--apply` alone was enough. Plan generation and dry-run remain available separately.

Modes are accident-prevention controls, not a sandbox for arbitrary edited YAML.
They do not replace Kubernetes RBAC. Context pinning, resource identity at preview
versus execution, and end-to-end fake-kubectl traces still require further audit.

## Session target pinning

`kube_session.rs` snapshots the selected context with kubectl config view
(minified, flattened JSON). Every collector and mutation helper obtains kubectl
through the same factory and passes an explicit `--kubeconfig` path. Later changes
to current-context or the original cluster mapping do not redirect a running session.
Initialization errors are cached and fail closed; restart after correcting configuration.
Offline help/version/config/audit commands do not create a snapshot.

The snapshot contains kubeconfig credentials: creation is exclusive, permissions
are 0600 on Linux, raw output is never included in errors, and normal process return
removes it. Abrupt process termination can leave the private temporary file until
system/user cleanup. External credential plugins/token files remain dynamic.
Context pinning does not solve resource deletion/recreation under the same name;
preview-to-execution UID/resourceVersion protection remains a separate requirement.

## Reviewed deployment identity

Scale, stop, and restart capture the Deployment UID and resourceVersion from the
snapshot when their confirmation is created. Background collection does not rewrite
that target. Missing identity blocks execution. Both server dry-run and persisted
JSON Patch begin with UID and resourceVersion tests, so recreation under the same
name and updates between validation and application fail atomically. Conflicts are
not retried: refresh and review again, including when an earlier scale has not yet
appeared in the collector snapshot. Restart preserves existing template labels and
annotations. Other operation guards are described separately below.

## Audit delivery and scope

The operation worker records its result before sending it to the TUI; CLI plan/apply
uses the same executor. UI result handling does not duplicate an already-attempted
record. An audit write failure is surfaced separately and does not convert a
successful mutation into a failed/retriable operation. The six existing TSV columns
are retained, followed by context, request/default namespace, reviewed UID,
resourceVersion, elapsed milliseconds, and `manifest_targets_json`. Old records
may omit trailing columns. The manifest array records each target's operation,
API version, kind, name/generateName, explicit namespace, default request namespace,
and reviewed UID/resourceVersion. It omits resource data and the source manifest.
Missing explicit namespace is null, rather than guessing that a cluster-scoped
resource belongs to the default namespace. Create-only targets may have null
UID/version because no existing object is being updated.
For multi-document manifests, the namespace column is the executor's default, not
an assertion that every object is in that namespace; node cordon leaves it empty.

## Reviewed manifest application

Opening a generated-manifest confirmation prepares its targets asynchronously.
Existing targets are read, merged with server dry-run apply and bound to their
observed UID/resourceVersion. New targets and Jobs use create-only requests.
The confirmation cannot execute until preparation succeeds. `p` opens a scrollable
target/version review, `e` edits the source and invalidates preparation, and `v`
prepares again. Closing, editing or replacing the confirmation rejects late
preparation responses. Preparation errors are available through `p`; `v` retries.

Execution verifies the exact namespace and source match the prepared plan, then
server-validates all prepared requests before running any mutation. It uses the
prepared merged objects with version-conditional replace, without rereading and
rebasing onto newer state. Conflict and AlreadyExists errors stop execution;
there is no force, deletion fallback or automatic retry. Earlier resources in a
multi-resource operation can remain applied. Get/create/update and dry-run patch
RBAC permissions are required as applicable. The CLI's explicit `--apply` prepares
the same bound plan immediately before execution and records its target versions.

## Reviewed route edits

Each collected route row carries its owning HTTPRoute UID/resourceVersion. The
action menu freezes the complete selected row, including HTTPRoute name and path.
Rename/retarget forms and deletion confirmation inherit that row. They never
resolve a target by path alone or fall back to the current selection. Later
snapshot refreshes do not retarget the menu or form; menu titles include the
HTTPRoute name to distinguish identical paths on different objects.
Execution requires that identity before any lookup, verifies the loaded HTTPRoute
still matches, and retains its UID/version in both dry-run and real replace.
Recreation, concurrent edits and API conflicts stop the operation without retry.
The common confirmation and audit scope includes route, node, Pod and Job guarded
identities. Retargeting still checks destination existence/ports at execution;
this does not freeze a destination Service/InferencePool's identity or prove the
running Gateway/EPP has converged to the new route.

New log files use 0600 permissions. All TSV fields remove control characters. Secret
apply stdout/stderr is never forwarded because validation errors can echo secret
values, including on the first line. This loses raw Secret validation diagnostics;
the operator gets the exit status and a namespace/RBAC/key-validation hint instead.
Recording remains best effort: process termination during an operation can still
leave no completion record; this is not a durable transaction journal or an RBAC audit.

## Guarded rollback

Rollback now also captures the reviewed Deployment identity. It selects the largest
revision below the current revision from ReplicaSets whose controller owner UID is
that exact Deployment. Foreign owners, missing/malformed history, duplicate candidate
revisions, paused deployments, and identical pod templates are rejected. The selected
pod template retains its annotations/labels except the controller-generated
pod-template-hash. Replicas and other Deployment settings are not rolled back.

The template is applied through the same UID/resourceVersion-conditional dry-run and
patch as scale/restart; intervening changes fail without retry. Audit and result text
name the selected revision and ReplicaSet. This does not establish rollout readiness
or prove the old model image can still run; those remain observable after application.

## Reviewed Pod and Job deletion

Pod and compile Job confirmation captures the snapshot UID and resourceVersion.
The worker sends a `DeleteOptions` body through `kubectl delete --raw -f -`, with
both fields in `preconditions` and background garbage collection. The same target
is checked with server dry-run before the real request. Missing identity, a changed
version, recreation under the same name, or failed dry-run stops the operation;
there is no name-only fallback or automatic conflict retry. Refresh and review
again when an actively updating object conflicts. API acceptance does not prove
that finalizers or dependent cleanup have completed.

References: [DeleteOptions](https://kubernetes.io/docs/reference/kubernetes-api/definitions/delete-options-v1-meta/)
and [kubectl raw delete implementation](https://github.com/kubernetes/kubectl/blob/master/pkg/cmd/delete/delete.go).

Path-based route edits require exactly one matching condition in exactly one rule.
A repeated path with different path types, methods, headers or query parameters
is ambiguous in the current route table. Rename, delete and retarget reject it
without changing the object; edit the complete HTTPRoute explicitly to select the
intended condition. Distinct sibling paths remain supported and preserved.

Retargeting one of several match paths splits the rule. The guided editor rejects
splitting named rules: duplicating the name violates uniqueness, while removing
or renaming it could change policies attached by rule name. Review the complete
HTTPRoute and attached policies explicitly in that case. It also rejects a split
when 16 rules already exist. In-place updates of a single-match named rule keep
its name and remain supported, including routes already at the rule-count limit.
