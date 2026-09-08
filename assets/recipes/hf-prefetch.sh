#!/bin/sh
# Download HuggingFace weights into the shared model store, reporting progress as it goes.
# Mounted from a ConfigMap; all input via env.
#
#   SOURCE     HF repo id                                    (required)
#   HF_HOME    cache root inside the store, e.g. /mnt/store/hub (required)
#   MODEL_DIR  the snapshot directory to measure for progress (required)
#   REVISION   branch/tag/commit; empty means the default branch
#
# hf_transfer is opportunistic: much faster when it installs, skipped when it does not.
# Progress comes from du against the growing snapshot, because snapshot_download prints no
# machine-readable progress; the last line of these logs is what the Activity panel shows.
set -eu

pip install -q --no-cache-dir huggingface_hub
pip install -q --no-cache-dir hf_transfer && export HF_HUB_ENABLE_HF_TRANSFER=1 || true

TOTAL=$(python -c '
import os
from huggingface_hub import HfApi
info = HfApi().model_info(os.environ["SOURCE"], files_metadata=True)
print(sum((f.size or 0) for f in info.siblings))
' 2>/dev/null || echo 0)

python -c '
import os
from huggingface_hub import snapshot_download
snapshot_download(repo_id=os.environ["SOURCE"], revision=os.environ.get("REVISION") or None)
' &
DL=$!

while kill -0 "$DL" 2>/dev/null; do
  B=$(du -sb "$MODEL_DIR" 2>/dev/null | cut -f1) || B=0
  B=${B:-0}
  if [ "$TOTAL" -gt 0 ] 2>/dev/null; then
    echo "downloading $SOURCE: $((B * 100 / TOTAL))% ($((B / 1073741824))G/$((TOTAL / 1073741824))G)"
  else
    echo "downloading $SOURCE: $(du -sh "$MODEL_DIR" 2>/dev/null | cut -f1) on disk"
  fi
  sleep 15
done
wait "$DL"
echo "PREFETCH_DONE $SOURCE 100% -> $HF_HOME"
