#!/usr/bin/env bash
# lmd-top installer — installs the *system* prerequisites, then builds & installs lmd-top.
#
# Note on "libraries": lmd-top's Rust crate dependencies (ratatui, tokio, tachyonfx, …)
# are fetched and compiled automatically by `cargo` — there is nothing to install by hand.
# The binary links only glibc; there are NO native/C-library deps (no OpenSSL/pkg-config).
# So this script only needs to ensure: (1) the Rust toolchain, (2) a C linker (cc/gcc).
#
# Usage:
#   ./install.sh              # install prereqs (if missing) + `cargo install --path .`
*Demo recording omitted from the public export.*
#   ./install.sh --check      # only report what's present/missing, install nothing
set -euo pipefail

cyan()  { printf '\033[36m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
yellow(){ printf '\033[33m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
have()  { command -v "$1" >/dev/null 2>&1; }

WITH_DEMO=0; CHECK_ONLY=0
for a in "$@"; do
  case "$a" in
    --with-demo) WITH_DEMO=1 ;;
    --check)     CHECK_ONLY=1 ;;
    -h|--help)   grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) red "unknown flag: $a"; exit 2 ;;
  esac
done

# ── detect a package manager (for the C linker only) ───────────────────────
PM=""; PM_INSTALL=""
if   have apt-get; then PM=apt;    PM_INSTALL="sudo apt-get install -y build-essential"
elif have dnf;     then PM=dnf;    PM_INSTALL="sudo dnf install -y gcc"
elif have yum;     then PM=yum;    PM_INSTALL="sudo yum install -y gcc"
elif have pacman;  then PM=pacman; PM_INSTALL="sudo pacman -S --noconfirm base-devel"
elif have zypper;  then PM=zypper; PM_INSTALL="sudo zypper install -y gcc"
elif have brew;    then PM=brew;   PM_INSTALL="xcode-select --install"   # macOS ships cc via CLT
fi

cyan "== lmd-top install =="
echo  "package manager : ${PM:-none detected}"

# ── report current state ───────────────────────────────────────────────────
report() {
  have cargo   && green  "  ✓ rust/cargo   $(cargo --version 2>/dev/null)" || yellow "  · rust/cargo   MISSING (required to build)"
  have cc || have gcc \
               && green  "  ✓ C linker     $((cc --version 2>/dev/null || gcc --version) | head -1)" \
               || yellow "  · C linker     MISSING (required to link)"
  have kubectl && green  "  ✓ kubectl      $(kubectl version --client -o json 2>/dev/null | grep -o '\"gitVersion\":\"[^\"]*\"' | head -1 || echo present)" \
               || yellow "  · kubectl      MISSING (required at runtime for topology/status/scale)"
  have xdg-open && green "  ✓ xdg-open     present (optional — 'g' opens Grafana)" \
               || echo   "  · xdg-open     absent (optional — only affects the 'g' key)"
  have agg     && green  "  ✓ agg          present (optional — demo GIF generation)" \
               || echo   "  · agg          absent (optional — only for --with-demo)"
}
echo "current state:"; report

if [ "$CHECK_ONLY" = 1 ]; then exit 0; fi

# ── 1) C linker ────────────────────────────────────────────────────────────
if ! (have cc || have gcc); then
  cyan "→ installing C linker …"
  if [ -n "$PM_INSTALL" ]; then eval "$PM_INSTALL"; else
    red "no known package manager; install a C compiler (gcc/clang) manually, then re-run."; exit 1
  fi
fi

# ── 2) Rust toolchain ──────────────────────────────────────────────────────
if ! have cargo; then
  cyan "→ installing Rust via rustup (https://sh.rustup.rs) …"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

# ── 3) build & install lmd-top (cargo fetches all Rust crates automatically) ─
cyan "→ cargo install --path .  (compiles lmd-top + its Rust deps)"
cargo install --path . --force

# ── 4) optional: demo GIF tooling ──────────────────────────────────────────
if [ "$WITH_DEMO" = 1 ]; then
  if ! have agg; then
    cyan "→ installing agg (asciicast→gif) from github.com/asciinema/agg …"
    cargo install --git https://github.com/asciinema/agg
  fi
*Demo recording omitted from the public export.*
*Demo recording omitted from the public export.*
fi

echo
green "✓ installed: $(command -v lmd-top || echo "$HOME/.cargo/bin/lmd-top")"
case ":$PATH:" in *":$HOME/.cargo/bin:"*) ;; *) yellow "  add ~/.cargo/bin to PATH:  export PATH=\"\$HOME/.cargo/bin:\$PATH\"";; esac
if ! have kubectl; then yellow "  reminder: install kubectl for topology/status/scale (metrics still work via Prometheus)."; fi
echo  "  run:  lmd-top            (point elsewhere: LMD_PROM=host:port LMD_NS=ns lmd-top)"
