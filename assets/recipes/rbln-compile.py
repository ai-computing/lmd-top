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

# Report the toolchain before anything else. Compiles here failed for three different models
# across three parameter sets with the same opaque codegen error, and nothing surfaced that the
# host's transformers/torch had moved far ahead of optimum-rbln. lmd-top parses this line out of
# the log into its history, so a version skew shows up as a pattern instead of a mystery.
def _report_toolchain():
    import importlib.metadata as md

    parts = []
    for pkg in ("optimum-rbln", "rebel-compiler", "transformers", "torch"):
        try:
            parts.append(f"{pkg}={md.version(pkg)}")
        except Exception:
            parts.append(f"{pkg}=absent")
    print("LMD_TOOLCHAIN " + " ".join(parts), flush=True)

    # Compare what is installed against what the vendor SDK itself declares, rather than
    # against a compatibility matrix maintained by hand. optimum-rbln pins its dependencies
    # exactly, so this is a precise check — and it is the vendor's opinion, not ours.
    bad = []
    try:
        for req in md.requires("optimum-rbln") or []:
            spec = req.split(";")[0].strip()
            if "==" not in spec:
                continue
            name, want = (p.strip() for p in spec.split("==", 1))
            try:
                have = md.version(name)
            except Exception:
                continue
            # PEP 440: when the specifier carries no local label, the candidate's local
            # label is ignored — `torch 2.11.0+cpu` satisfies `==2.11.0`. Comparing the raw
            # strings reported three mismatches on a perfectly correct install.
            def _public(v):
                return v.split("+", 1)[0]

            if _public(have) != _public(want):
                bad.append(f"{name} {have} != required {want}")
    except Exception:
        pass
    if bad:
        print("LMD_DEPS_MISMATCH " + "; ".join(bad[:4]), flush=True)


_report_toolchain()

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
try:
    model = M.from_pretrained(
        os.environ["MODEL_ID"], export=True, rbln_create_runtimes=False, **cfg
    )
except Exception as exc:
    # rebel-compiler raises a bare RuntimeError from a frozen module, so the traceback alone
    # says only "Error occurred while compiling the model". Whatever detail exists lives in the
    # exception's args and cause chain; print it so the failure is reportable.
    import traceback

    print("LMD_COMPILE_ERROR", type(exc).__name__, repr(exc.args), flush=True)
    cause, depth = exc.__cause__ or exc.__context__, 0
    while cause is not None and depth < 5:
        print(
            "LMD_COMPILE_CAUSE",
            type(cause).__name__,
            repr(getattr(cause, "args", ())),
            flush=True,
        )
        cause, depth = cause.__cause__ or cause.__context__, depth + 1
    traceback.print_exc()
    raise
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
