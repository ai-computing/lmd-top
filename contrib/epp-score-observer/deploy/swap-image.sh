#!/usr/bin/env bash
# Point EPP deployment(s) at the custom epp-score-observer image.
# The image is a drop-in for the stock endpoint-picker (same flags/ports), plus the
# endpoint-score-observer picker. Reversible: re-run with the original image to roll back.
#
# Usage:
#   ./swap-image.sh <registry>/epp-score-observer:<tag> [ns] [deploy1 deploy2 ...]
# Example:
#   ./swap-image.sh registry.local/epp-score-observer:v1 llm-serving llmd-router-epp
set -euo pipefail

IMAGE="${1:?usage: swap-image.sh <image> [ns] [deploy...]}"
NS="${2:-llm-serving}"
shift || true; shift || true
DEPLOYS=("$@")
if [ "${#DEPLOYS[@]}" -eq 0 ]; then
  # default: every EPP deployment in the namespace
  mapfile -t DEPLOYS < <(kubectl get deploy -n "$NS" -o name 2>/dev/null | grep -E 'epp$' | sed 's#deployment.apps/##')
fi

echo "namespace: $NS"
echo "image:     $IMAGE"
echo "targets:   ${DEPLOYS[*]:-<none>}"
for d in "${DEPLOYS[@]}"; do
  # container name varies; patch the first container's image.
  c=$(kubectl get deploy "$d" -n "$NS" -o jsonpath='{.spec.template.spec.containers[0].name}')
  echo "  -> $d (container $c)"
  kubectl set image deploy/"$d" -n "$NS" "$c=$IMAGE"
  kubectl rollout status deploy/"$d" -n "$NS" --timeout=120s
done
echo "done. verify: kubectl port-forward deploy/<epp> $NS 9090; curl -k https://localhost:9090/metrics | grep epp_endpoint_score"
