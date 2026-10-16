#!/bin/sh
# Furiosa RNGD ahead-of-time compile (furiosa-llm). Mounted from a ConfigMap; all input via env.
#
#   MODEL_ID        HF repo id                              (required)
#   OUTPUT          destination in the model store          (required)
#   PREFETCHED_DIR  HF cache copy in the store, if present  (optional)
#   TP, PP          tensor / pipeline parallel size
#   MAX_LEN         max model length
#   FXB_CONCURRENCY host-side parallel compiler workers (default 8)
#   FXB_OPTIM_LEVEL bucket filtering level O0/O1/O2/O3 (default O3)
set -eu

# Report the toolchain first — lmd-top parses this out of the log into its history, so a
# version skew between the vendor SDK and its dependencies shows up as a pattern.
python3 - <<'TOOLCHAIN' || true
import importlib.metadata as md
parts = []
for pkg in ("furiosa-llm", "furiosa-smi", "transformers", "torch"):
    try:
        parts.append(f"{pkg}={md.version(pkg)}")
    except Exception:
        parts.append(f"{pkg}=absent")
print("LMD_TOOLCHAIN " + " ".join(parts), flush=True)
TOOLCHAIN

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
  -tp "$TP" -pp "$PP" --max-model-len "$MAX_LEN" --concurrency "${FXB_CONCURRENCY:-8}" \
  --optim-level "${FXB_OPTIM_LEVEL:-O3}"

# Publish atomically on the store filesystem, without merging an existing build.
python3 /scripts/publish-artifact.py /work/out "$OUTPUT"
