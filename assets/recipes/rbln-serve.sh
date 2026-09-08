#!/bin/sh
# RBLN serving on the host RBLN stack (no vllm_rbln registry image configured).
# Mounted from a ConfigMap; all input via env.
#
#   MOUNT   compiled artifact path in the model store  (required)
#   SERVED  --served-model-name value                  (required)
#   PORT    listen port                                (required)
#
# A bare ubuntu image plus the node's own python3.10 / rebel-compiler install (hostPath).
# prometheus-fastapi-instrumentator is upgraded into an overlay dir because the host copy is
# too old for vLLM's metrics endpoint, and PYTHONPATH must find the overlay first.
set -eux

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  python3.10 python3.10-dev python3-pip libdrm2 libnuma1 libgomp1 \
  ca-certificates tzdata g++ libc6-dev >/dev/null
ln -sf /usr/bin/python3.10 /usr/local/bin/python3

python3 -m pip install -q --target=/opt/py-overrides --upgrade prometheus-fastapi-instrumentator
export PYTHONPATH="/opt/py-overrides:${PYTHONPATH:-}"

exec python3 -m vllm.entrypoints.openai.api_server \
  --model="$MOUNT" --served-model-name="$SERVED" \
  --enforce-eager --max-num-seqs 1 --host=0.0.0.0 --port="$PORT"
