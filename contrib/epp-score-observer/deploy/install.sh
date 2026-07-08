#!/usr/bin/env bash
# Attach the endpoint-score-observer to EXISTING llm-d EPP deployments in a namespace.
# For each target EPP it (1) injects the picker into its plugins ConfigMap and
# (2) swaps the container image to the score-observer build, then restarts it.
# Selection behavior is unchanged (the picker delegates to max-score); it only adds
# the epp_endpoint_score / epp_endpoint_picked_total metrics on the EPP's existing :9090.
#
# Prereqs: kubectl (context set), python3 + PyYAML, and IMAGE pullable by the cluster.
#
# Usage:
#   ./install.sh --image <registry>/epp-score-observer:<tag> [--ns llm-serving] [--epp d1,d2] [--dry-run]
# If --epp is omitted, every Deployment using the llm-d endpoint-picker image is targeted.
#
# Rollback: re-run with the original endpoint-picker image, and remove the picker lines
#   (kubectl edit the ConfigMap) — or keep them; without the custom image they are ignored
#   only if the type is unknown, so revert the ConfigMap too. See README.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

NS="llm-serving"; IMAGE=""; EPPS=""; DRY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ns) NS="$2"; shift 2;;
    --image) IMAGE="$2"; shift 2;;
    --epp) EPPS="$2"; shift 2;;
    --dry-run) DRY="1"; shift;;
    *) echo "unknown arg: $1"; exit 2;;
  esac
done
[ -n "$IMAGE" ] || { echo "ERROR: --image required"; exit 2; }

# Discover EPP deployments if not given: those running the llm-d endpoint-picker image.
if [ -z "$EPPS" ]; then
  EPPS=$(kubectl get deploy -n "$NS" -o json \
    | python3 -c "import json,sys;
d=json.load(sys.stdin)
for x in d['items']:
    img=x['spec']['template']['spec']['containers'][0].get('image','')
    if 'endpoint-picker' in img or 'llm-d-router' in img: print(x['metadata']['name'])" \
    | paste -sd, -)
fi
[ -n "$EPPS" ] || { echo "no EPP deployments found in ns=$NS"; exit 1; }
echo "namespace: $NS"; echo "image:     $IMAGE"; echo "targets:   $EPPS"; [ -n "$DRY" ] && echo "(dry-run)"

IFS=',' read -ra LIST <<< "$EPPS"
for d in "${LIST[@]}"; do
  d="$(echo "$d" | xargs)"; [ -n "$d" ] || continue
  echo "== $d =="
  # Resolve the config-file path -> mount -> ConfigMap name + key.
  cfgpath=$(kubectl get deploy "$d" -n "$NS" -o jsonpath='{range .spec.template.spec.containers[0].args[*]}{@}{"\n"}{end}' \
            | awk 'p{print;p=0} /^--config-file$/{p=1}')
  cfgpath="${cfgpath:-/config/default-plugins.yaml}"
  cfgdir="$(dirname "$cfgpath")"; cfgkey="$(basename "$cfgpath")"
  vol=$(kubectl get deploy "$d" -n "$NS" -o jsonpath="{range .spec.template.spec.containers[0].volumeMounts[?(@.mountPath=='$cfgdir')]}{.name}{end}")
  cm=$(kubectl get deploy "$d" -n "$NS" -o jsonpath="{range .spec.template.spec.volumes[?(@.name=='$vol')]}{.configMap.name}{end}")
  echo "   config: cm=$cm key=$cfgkey"
  # Inject the picker into the config.
  newcfg=$(kubectl get cm "$cm" -n "$NS" -o jsonpath="{.data.$(echo "$cfgkey" | sed 's/\./\\./g')}" | python3 "$HERE/inject_picker.py")
  if [ -n "$DRY" ]; then
    echo "--- would set $cm/$cfgkey to: ---"; echo "$newcfg" | sed 's/^/     /'
    echo "--- would set image of $d to $IMAGE ---"; continue
  fi
  # Patch the single ConfigMap key (preserve other keys), then image + restart.
  tmp="$(mktemp)"; printf '%s' "$newcfg" > "$tmp"
  patch="$(python3 -c "import json,sys; print(json.dumps({'data':{sys.argv[1]:open(sys.argv[2]).read()}}))" "$cfgkey" "$tmp")"
  rm -f "$tmp"
  kubectl patch cm "$cm" -n "$NS" --type merge -p "$patch"
  c=$(kubectl get deploy "$d" -n "$NS" -o jsonpath='{.spec.template.spec.containers[0].name}')
  kubectl set image "deploy/$d" -n "$NS" "$c=$IMAGE"
  kubectl rollout status "deploy/$d" -n "$NS" --timeout=120s
done
echo "done. verify: kubectl port-forward deploy/<epp> $NS 9090; curl -s localhost:9090/metrics | grep epp_endpoint_score"
