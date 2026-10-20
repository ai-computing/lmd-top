#!/usr/bin/env bash
# Local release gate; no deployment, downloads or dependency updates requested.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
for tool in cargo python3 go; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing required tool: $tool. Install the repository's required toolchain before running the complete release gate." >&2
    exit 2
  fi
done
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 scripts/check-terminal.py
python3 scripts/check-terminal.py --slow-zoo
python3 scripts/check-terminal.py --manifest-review success
python3 scripts/check-terminal.py --manifest-review conflict
python3 -m unittest discover -s tests -p 'test_*.py'
bash -n install.sh scripts/validate-golden.sh scripts/check-release.sh scripts/record-demo.sh
for recipe in assets/recipes/*.sh; do
  sh -n "$recipe"
done
check_tmp=$(mktemp -d)
trap 'rm -rf "$check_tmp"' EXIT
python3 scripts/build-inventory.py --output "$check_tmp/dependencies.json"
cargo run --locked -- --policy render \
  --file contrib/epp-score-observer/policy.yaml \
  --base contrib/epp-score-observer/examples/base-epp.yaml > "$check_tmp/rendered-epp.yaml"
diff -u contrib/epp-score-observer/examples/rendered-epp.yaml "$check_tmp/rendered-epp.yaml"
(
  cd contrib/epp-score-observer
  go test -mod=readonly ./...
  go run -mod=readonly ./cmd/replay examples/parameters.json examples/requests.json
  CGO_ENABLED=0 go build -mod=readonly -trimpath -buildvcs=false -o "$check_tmp/epp-policy" .
)
# Verify the extracted source archive without replacing the development binary.
cargo package --allow-dirty --locked --offline --target-dir target/package-check
echo 'Local release checks passed. Consult docs/V1-READINESS.md for remaining runtime and release gates.'
