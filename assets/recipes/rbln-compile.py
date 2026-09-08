# RBLN ahead-of-time compile (optimum-rbln). Mounted from a ConfigMap; all input via env.
#
#   MODEL_ID                    HF repo id or local path        (required)
#   OUTPUT                      destination in the model store  (required)
#   RBLN_NPU                    target chip                     (default RBLN-CA22)
#   RBLN_TENSOR_PARALLEL_SIZE   chip count                      (default 1)
#   RBLN_MAX_SEQ_LEN            compile-time context length     (default 4096)
#   RBLN_BATCH_SIZE             static batch size               (default 1)
#   RBLN_ATTN_IMPL              flash_attn | eager              (default flash_attn)
#   RBLN_KVCACHE_PARTITION_LEN  flash_attn only                 (default 16384)
import os
import shutil

from optimum.rbln import RBLNAutoModelForCausalLM as M

get = os.environ.get
output = os.environ["OUTPUT"]
local = "/work/out"

cfg = dict(
    rbln_npu=get("RBLN_NPU", "RBLN-CA22"),
    rbln_num_devices=int(get("RBLN_TENSOR_PARALLEL_SIZE", "1")),
    rbln_max_seq_len=int(get("RBLN_MAX_SEQ_LEN", "4096")),
    rbln_batch_size=int(get("RBLN_BATCH_SIZE", "1")),
)
attn = get("RBLN_ATTN_IMPL", "flash_attn")
if attn:
    cfg["rbln_attn_impl"] = attn
if attn == "flash_attn":
    cfg["rbln_kvcache_partition_len"] = int(get("RBLN_KVCACHE_PARTITION_LEN", "16384"))

print("RBLN_CONFIG", cfg)
# create_runtimes=False: compile without claiming a device, so serving may hold the chips.
model = M.from_pretrained(
    os.environ["MODEL_ID"], export=True, rbln_create_runtimes=False, **cfg
)
# Build into local scratch first — the shared store is SMB and rejects the compiler's I/O.
model.save_pretrained(local)

os.makedirs(output, exist_ok=True)
for entry in os.listdir(local):
    src = os.path.join(local, entry)
    dst = os.path.join(output, entry)
    if os.path.isdir(src):
        shutil.copytree(src, dst, dirs_exist_ok=True)
    else:
        shutil.copy2(src, dst)
print("COMPILE_DONE", os.listdir(output))
