#!/bin/sh
# Furiosa RNGD ahead-of-time compile (furiosa-llm). Mounted from a ConfigMap; all input via env.
#
#   MODEL_ID        HF repo id                              (required)
#   OUTPUT          destination in the model store          (required)
#   PREFETCHED_DIR  HF cache copy in the store, if present  (optional)
#   TP, PP          tensor / pipeline parallel size
#   MAX_LEN         max model length
set -eu

# RNGD's control processor is ARM64: the final EDF codegen needs an aarch64 cross-compiler,
# which the furiosa-llm serve image does not ship.
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq gcc-aarch64-linux-gnu build-essential >/dev/null 2>&1

# HF_HOME is local scratch — the downloader fails against the SMB-backed store (os error 95).
mkdir -p /work/hub/hub
if [ -n "${PREFETCHED_DIR:-}" ] && [ -d "$PREFETCHED_DIR" ]; then
  echo "reuse prefetched weights from store"
  cp -r "$PREFETCHED_DIR" /work/hub/hub/
  export HF_HUB_OFFLINE=1
fi

mkdir -p /work/out
fxb build "$MODEL_ID" /work/out/model \
  -tp "$TP" -pp "$PP" --max-model-len "$MAX_LEN" --concurrency 8

# Only the finished artifact goes back to the store.
mkdir -p "$OUTPUT"
cp -r /work/out/. "$OUTPUT"/
echo COMPILE_DONE
ls -la "$OUTPUT"
