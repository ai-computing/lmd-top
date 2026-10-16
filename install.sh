#!/usr/bin/env bash
# lmd-top installer.
#
# Default (easy): download the prebuilt binary from GitHub Releases and
# put it on PATH — no Rust toolchain, no compile. The binary links only glibc
# and shells out to `kubectl`; all features (incl. compile/deploy) work as-is.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/gspark-etri/lmd-top/main/install.sh | bash
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

usage() {
  cat <<'EOF'
Usage: install.sh [--version vX.Y.Z] [--bin-dir DIR] [--from-source] [--with-demo] [--check]
Default: install latest verified release into ~/.local/bin (no Rust required).
--from-source builds the current checkout using Cargo and its locked dependencies.
EOF
}
while [ "$#" -gt 0 ]; do
  case "$1" in
    --from-source) FROM_SOURCE=1; shift ;;
    --with-demo) WITH_DEMO=1; shift ;;
    --check) CHECK_ONLY=1; shift ;;
    --version|--bin-dir)
      option="$1"
      [ "$#" -ge 2 ] && [ -n "$2" ] && [[ "$2" != --* ]] || {
        red "$option requires a value"; exit 2;
      }
      if [ "$option" = --version ]; then VERSION="$2"; else BIN_DIR="$2"; fi
      shift 2 ;;
    --version=*) VERSION="${1#*=}"; [ -n "$VERSION" ] || exit 2; shift ;;
    --bin-dir=*) BIN_DIR="${1#*=}"; [ -n "$BIN_DIR" ] || exit 2; shift ;;
    -h|--help) usage; exit 0 ;;
    *) red "unknown argument: $1"; usage >&2; exit 2 ;;
  esac
done
if [ -n "$VERSION" ] && [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]]; then
  red "invalid release version: $VERSION (expected vX.Y.Z)"; exit 2
fi
if [ "$WITH_DEMO" = 1 ] && [ "$FROM_SOURCE" != 1 ]; then
  red "--with-demo requires --from-source"; exit 2
fi
if [ "$FROM_SOURCE" = 1 ] && [ -n "$VERSION" ]; then
  red "--version cannot be combined with --from-source (builds current checkout)"; exit 2
fi

cyan "== lmd-top install =="
have kubectl && green "  ✓ kubectl present (required at runtime for status/scale/compile/deploy)" \
             || yellow "  · kubectl MISSING — install it; lmd-top shells out to kubectl"
if [ "$CHECK_ONLY" = 1 ]; then
  missing=0
  for tool in kubectl curl tar install; do
    have "$tool" || { red "missing: $tool"; missing=1; }
  done
  have sha256sum || have shasum || { red "missing: sha256sum or shasum"; missing=1; }
  if [ "$FROM_SOURCE" = 1 ]; then
    have cargo || { red "missing: cargo"; missing=1; }
    have cc || have gcc || { red "missing: C linker"; missing=1; }
  fi
  exit "$missing"
fi

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
  tmp_root="$(mktemp -d)"
  trap 'rm -rf "$tmp_root"' EXIT
  cyan "→ building locked checkout into temporary staging directory"
  cargo install --locked --path . --force --root "$tmp_root"
  mkdir -p "$BIN_DIR"
  staged="$(mktemp "$BIN_DIR/.lmd-top.XXXXXX")"
  trap 'rm -rf "$tmp_root"; rm -f "$staged"' EXIT
  install -m 0755 "$tmp_root/bin/lmd-top" "$staged"
  mv -f "$staged" "$BIN_DIR/lmd-top"
  if [ "$WITH_DEMO" = 1 ]; then
    have agg || cargo install --git https://github.com/asciinema/agg
*Demo recording omitted from the public export.*
  fi
  green "✓ installed: $BIN_DIR/lmd-top"
  echo "  next: $BIN_DIR/lmd-top --init"
  exit 0
fi

for tool in curl tar install; do
  have "$tool" || { red "required tool missing: $tool"; exit 1; }
done
have sha256sum || have shasum || { red "sha256sum or shasum is required"; exit 1; }

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

[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]] || {
  red "invalid release tag: $VERSION"; exit 1;
}
asset="lmd-top-${VERSION}-${arch}-${os}"
url="https://github.com/$REPO/releases/download/${VERSION}/${asset}.tar.gz"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
cyan "→ downloading ${asset}.tar.gz"
curl -fsSL "$url" -o "$tmp/a.tgz" || { red "download failed: $url  (no prebuilt for $arch-$os? use --from-source)"; exit 1; }
# Verification is mandatory: never replace an existing binary with unverified bytes.
curl -fsSL "$url.sha256" -o "$tmp/a.sha256" || { red "checksum download failed"; exit 1; }
sum="$(awk 'NR == 1 {print $1}' "$tmp/a.sha256")"
[[ "$sum" =~ ^[[:xdigit:]]{64}$ ]] || { red "invalid sha256 file"; exit 1; }
if have sha256sum; then actual="$(sha256sum "$tmp/a.tgz")"; else actual="$(shasum -a 256 "$tmp/a.tgz")"; fi
[ "$sum" = "${actual%% *}" ] || { red "sha256 mismatch"; exit 1; }
green "  ✓ sha256 verified"
# Extract only the expected binary; do not unpack arbitrary archive paths.
tar -xOzf "$tmp/a.tgz" "$asset/lmd-top" > "$tmp/lmd-top"
[ -s "$tmp/lmd-top" ] || { red "release binary is empty"; exit 1; }
mkdir -p "$BIN_DIR"
staged="$(mktemp "$BIN_DIR/.lmd-top.XXXXXX")"
trap 'rm -rf "$tmp"; rm -f "$staged"' EXIT
install -m 0755 "$tmp/lmd-top" "$staged"
mv -f "$staged" "$BIN_DIR/lmd-top"

green "✓ installed: $BIN_DIR/lmd-top  ($VERSION, $arch-$os)"
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) yellow "  add to PATH:  export PATH=\"$BIN_DIR:\$PATH\"";; esac
have kubectl || yellow "  reminder: install kubectl (required for status/scale/compile/deploy)"
# Older published versions may not have onboarding commands yet.
if "$BIN_DIR/lmd-top" --help 2>/dev/null | grep -q -- '--init'; then
  echo "  next: $BIN_DIR/lmd-top --init"
  echo "  then: $BIN_DIR/lmd-top --check-setup"
else
  echo "  run: $BIN_DIR/lmd-top (LMD_PROM=host:9090 LMD_NS=llm-serving)"
fi
