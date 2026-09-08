#!/usr/bin/env bash
# Server-side validate every golden manifest against the current cluster.
#
# `cargo test golden` pins what we generate; this checks the API server agrees it is applicable
# (CRD shapes, resource quantity forms, RBAC references). Read-only: --dry-run=server validates
# without persisting anything. Needs a reachable cluster, so it is not part of `cargo test`.
set -uo pipefail
cd "$(dirname "$0")/.."
pass=0 fail=0
for f in tests/golden/*.yaml; do
  if out=$(kubectl apply --dry-run=server -f "$f" 2>&1); then
    printf '  ok   %-28s %s objects\n' "$(basename "$f" .yaml)" "$(echo "$out" | wc -l)"
    pass=$((pass + 1))
  else
    printf '  FAIL %-28s\n' "$(basename "$f" .yaml)"
    echo "$out" | head -5 | sed 's/^/       /'
    fail=$((fail + 1))
  fi
done
echo "server-side validation: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
