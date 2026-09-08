#!/bin/sh
# Install a matched RBLN toolchain from the shared model store, then run the compile recipe.
# Mounted from a ConfigMap; all input via env.
#
#   TOOLCHAIN_DIR  directory of wheels in the store, e.g. /mnt/store/rbln-toolchain/0.10.3
#
# This is the alternative to borrowing the node's rebel-compiler over hostPath. That fallback
# inherits whatever the host's Python environment happens to be, and on this cluster the host
# had moved to a version whose compiles fail. A vendor bundle staged in the store pins every
# version together — including rebel_compiler, which is licensed and not on public PyPI, so
# it cannot be installed any other way.
#
# --no-index: resolve only from the store. If the bundle is incomplete the install fails here
# with the missing name, which is a far better failure than compiling against a half-upgraded
# environment.
set -eu

: "${TOOLCHAIN_DIR:?TOOLCHAIN_DIR must point at a wheel directory in the store}"
echo "installing RBLN toolchain from $TOOLCHAIN_DIR"
ls "$TOOLCHAIN_DIR" | wc -l | xargs echo "  wheels available:"

python3 -m pip install -q --no-index --find-links="$TOOLCHAIN_DIR" \
  rebel-compiler optimum-rbln

python3 - <<'VERSIONS'
import importlib.metadata as md
parts = []
for pkg in ("optimum-rbln", "rebel-compiler", "transformers", "torch"):
    try:
        parts.append(f"{pkg}={md.version(pkg)}")
    except Exception:
        parts.append(f"{pkg}=absent")
print("LMD_TOOLCHAIN " + " ".join(parts), flush=True)
VERSIONS

exec python3 /scripts/compile.py
