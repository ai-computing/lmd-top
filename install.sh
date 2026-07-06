#!/usr/bin/env bash
# lmd-top installer.
#
# Default (easy): download the prebuilt static binary from GitHub Releases and
# put it on PATH — no Rust toolchain, no compile. The binary links only glibc
# and shells out to `kubectl`; all features (incl. compile/deploy) work as-is.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/gspark-etri/lmd-top/main/install.sh | sh
#   ./install.sh                     # download latest prebuilt binary → BIN_DIR
#   ./install.sh --version v0.34.0   # pin a version
#   ./install.sh --bin-dir /usr/local/bin
#   ./install.sh --from-source       # build with cargo instead (needs Rust + cc)
#   ./install.sh --from-source --with-demo   # also install agg + regen demo gif
#   ./install.sh --check             # report prereqs, install nothing
set -euo pipefail

REPO="gspark-etri/lmd-top"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
VERSION=""
FROM_SOURCE=0; WITH_DEMO=0; CHECK_ONLY=0

cyan()  { printf '\033[36m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
yellow(){ printf '\033[33m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*" >&2; }
have()  { command -v "$1" >/dev/null 2>&1; }

for a in "$@"; do
  case "$a" in
    --from-source) FROM_SOURCE=1 ;;
    --with-demo)   WITH_DEMO=1 ;;
    --check)       CHECK_ONLY=1 ;;
    --version=*)   VERSION="${a#*=}" ;;
    --version)     shift; VERSION="${1:-}" ;;
    --bin-dir=*)   BIN_DIR="${a#*=}" ;;
    -h|--help)     grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) : ;;  # allow positional (e.g. from `--version vX` handled above)
  esac
done

cyan "== lmd-top install =="
have kubectl && green "  ✓ kubectl present (required at runtime for status/scale/compile/deploy)" \
             || yellow "  · kubectl MISSING — install it; lmd-top shells out to kubectl"
[ "$CHECK_ONLY" = 1 ] && exit 0

# ── source build path (devs / unsupported platforms) ───────────────────────
if [ "$FROM_SOURCE" = 1 ]; then
  if ! (have cc || have gcc); then
    red "C linker (cc/gcc) required for --from-source"; exit 1
  fi
  if ! have cargo; then
    cyan "→ installing Rust via rustup …"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
  cyan "→ cargo install --path . --force"
  cargo install --path . --force
  if [ "$WITH_DEMO" = 1 ]; then
    have agg || cargo install --git https://github.com/asciinema/agg
*Demo recording omitted from the public export.*
  fi
  green "✓ installed: $(command -v lmd-top || echo "$HOME/.cargo/bin/lmd-top")"
  exit 0
fi

# ── prebuilt binary path (default) ─────────────────────────────────────────
os="$(uname -s)"; arch="$(uname -m)"
case "$os" in Linux) os=linux ;; Darwin) os=darwin ;; *) red "unsupported OS: $os (try --from-source)"; exit 1 ;; esac
case "$arch" in x86_64|amd64) arch=x86_64 ;; aarch64|arm64) arch=aarch64 ;; *) red "unsupported arch: $arch (try --from-source)"; exit 1 ;; esac

if [ -z "$VERSION" ]; then
  cyan "→ resolving latest release …"
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
             | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"
  [ -n "$VERSION" ] || { red "could not resolve latest version; pass --version vX.Y.Z"; exit 1; }
fi

asset="lmd-top-${VERSION}-${arch}-${os}"
url="https://github.com/$REPO/releases/download/${VERSION}/${asset}.tar.gz"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
cyan "→ downloading ${asset}.tar.gz"
curl -fsSL "$url" -o "$tmp/a.tgz" || { red "download failed: $url  (no prebuilt for $arch-$os? use --from-source)"; exit 1; }
# checksum (best-effort — skip if the .sha256 asset is absent)
if curl -fsSL "$url.sha256" -o "$tmp/a.sha256" 2>/dev/null; then
  ( cd "$tmp" && sum="$(awk '{print $1}' a.sha256)" && echo "$sum  a.tgz" | shasum -a 256 -c - >/dev/null ) \
    && green "  ✓ sha256 verified" || { red "sha256 mismatch"; exit 1; }
fi
tar -C "$tmp" -xzf "$tmp/a.tgz"
mkdir -p "$BIN_DIR"
install -m 0755 "$tmp/$asset/lmd-top" "$BIN_DIR/lmd-top"

green "✓ installed: $BIN_DIR/lmd-top  ($VERSION, $arch-$os)"
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) yellow "  add to PATH:  export PATH=\"$BIN_DIR:\$PATH\"";; esac
have kubectl || yellow "  reminder: install kubectl (required for status/scale/compile/deploy)"
echo  "  run:  lmd-top          (point at a cluster: LMD_PROM=host:30090 LMD_NS=llm-serving lmd-top)"
echo  "  tip:  as a kubectl plugin →  kubectl krew install lmd-top   then  kubectl lmd-top"
