# Contributing to lmd-top

Thanks for your interest! Issues and pull requests are welcome.

We follow the organization's [contributing workflow](https://github.com/ai-computing/.github/blob/main/CONTRIBUTING.md):
Issue (problem, evidence, scope, done-when, owner) → purpose branch → PR with verification evidence → review and
green checks → squash and merge → delete the branch. Small fixes can start at the PR. The sections below add what is
specific to lmd-top.

## Reporting issues

- For bugs, include the output of `lmd-top --doctor` (it summarizes which exporters and
  metrics your cluster exposes) and your terminal/font if the problem is visual.
- Feature ideas are welcome too — check [ROADMAP.md](ROADMAP.md) first to see if it's
  already planned.

## Development

```bash
cargo build            # debug build
cargo test --locked    # run tests
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

Useful while developing:

- `lmd-top --render` renders one frame headlessly (size via `LMD_W`/`LMD_H`) — handy for
  layout checks, but it cannot verify colors or font glyphs; check those in a real terminal.
- `lmd-top --snapshot` prints a plain-text state dump.
- Preview saves use `LMD_SAVE_DIR` (default: current directory). Existing names,
  including symlinks, are preserved; repeated saves add a numeric suffix. Files
  are created with owner-only permissions on Unix. External editor sessions use
  separate private temporary directories, removed when the editor returns.
- Reapplying a compile Job does not delete the previous execution. Use a new Job
  name, or review and explicitly delete the previous Job before retrying; save
  any logs you need before deleting it.
  Job manifests use `kubectl create`, including server dry-run, with the exact
  previewed name. All documents are validated before application starts. Other
  resources are read and merged using server dry-run apply. Existing objects
  are then updated with `replace` carrying their UID/resourceVersion; absent
  objects use `create`. This requires get/create/update permissions as applicable,
  in addition to patch permission for dry-run merge. Changes during preparation
  or execution fail without automatic retry or force. Multi-resource application
  is not a transaction and a later failure can leave earlier resources applied.
  TUI confirmation prepares targets in the background; `p` reviews identities,
  `e` edits and invalidates the plan, and `v` refreshes target preparation. Execution
  uses those exact prepared targets. Audit rows append a JSON target array after
  the existing scope columns, without including manifest resource data.
  Include each named resource once: repeated group/kind/namespace/name targets
  are rejected locally, including repetitions inside Lists or across API versions.

The local release gate and CI also build the extracted source package offline
with locked dependencies, using an isolated target directory. Local checks allow
uncommitted changes; CI requires a clean checkout. This checks packaging without
publishing to a registry.

Verify source packages manually with a separate target directory, for example
`cargo package --allow-dirty --locked --offline --target-dir target/package-check`.
Sharing the development target directory with extracted-package verification can
leave its executable/depfile pointing at the package copy; a later ordinary build
may report fresh. If this occurs, `cargo clean --package lmd-top` followed by
`cargo build --locked` regenerates this package's development artifacts.


## Pull requests

- Keep PRs focused — one change per PR.
- Make sure `cargo clippy --locked --all-targets -- -D warnings` and `cargo test` pass.
- The crate is **pure Rust by design**: no C-library dependencies (no OpenSSL, ring,
  pkg-config, or cmake). Please don't add crates that break this.
- `ratatui` is pinned to 0.29 and `tachyonfx` to `=0.16.0` (the last release compatible
  with monolithic ratatui 0.29) — bumping either is a coordinated change, not a routine
  dependency update.

## License

By contributing, you agree that your contributions will be licensed under the
[Apache-2.0](LICENSE) license.

## Release process

CI checks formatting, strict Clippy, Rust tests, and offline installer tests on PRs.
Run `python3 -m unittest discover -s tests -p 'test_*.py' -v` locally as well.
Existing sources were normalized with rustfmt when the formatting gate was added.

Before release, update Cargo.toml/Cargo.lock together and move Unreleased notes to
that version. Push a matching `vX.Y.Z` tag when authorized to publish. The release
workflow rejects version/tag mismatch, tests each target, then publishes the four
archives and mandatory SHA-256 files only when every target succeeds. A manual
run requires an existing tag; it never packages a branch name as a release.

Client CI does not verify real NPU compilation or inference. Use the scratch-cluster
QA plan for changes to cluster mutations, and record actual model/compiler/runtime
versions for vendor E2E validation. See [quick start](docs/QUICKSTART.md) for update
and recovery, and [analysis proposal](docs/HETEROGENEOUS-ANALYSIS.md) for future scope.

### Complete local release checks

Run `scripts/check-release.sh` with Rust (rustfmt/clippy), Python 3 and the Go
version required by `contrib/epp-score-observer/go.mod` available on PATH. The
script checks Rust, normal/pending-read Unix PTY behavior, Python recipes/installer,
shell syntax, policy-render parity,
EPP tests/replay and the EPP binary build. Missing tools fail with exit code 2;
there is no successful partial mode. Build tools may populate their dependency
caches. No cluster changes are made. Passing this script does not replace runtime
and release evidence tracked in `docs/V1-READINESS.md`.

To run the Unix terminal lifecycle check separately, run `cargo build --locked` followed
by `python3 scripts/check-terminal.py` from the repository root. It opens a PTY,
resizes the screen, sends help/close and quit/confirm keys, and checks successful
exit, alternate-screen cleanup and exact terminal attribute restoration. A stub
kubectl and unreachable Prometheus endpoint keep this check independent of a live
cluster. Use `--binary PATH` to check a release binary. This is a lifecycle smoke
check; it does not assert the visual contents of help or resized screens.

Subprocess output is bounded to 16 MiB per stdout/stderr stream. Exceeding either
limit fails the command and stops its Unix process group; callers must not interpret
that failure as an empty successful response. Large-cluster validation should include
representative list sizes. Commands intentionally exporting larger data need a
separate streaming design, not silent truncation.

Use `python3 scripts/check-terminal.py --slow-zoo` to also exercise input handling
while vendor discovery is pending. The harness supplies a local fake curl, waits
for evidence that it started, checks repeated refresh does not spawn more work,
then sends help/close/quit and requires normal termination within five seconds
before the fake response completes. No vendor network access is made.

Use `python3 scripts/check-terminal.py --manifest-review success` and
`--manifest-review conflict` to drive Setup → manifest preview → confirmation →
target review → execution with fake kubectl. These checks delay preparation and
attempt early confirmation, inspect the displayed UID, verify the exact conditional
update requests and audit identity, and exercise both success and conflict paths.
The local release gate and CI run both scenarios without contacting a cluster.

Release publishing depends on the reusable CI workflow for the validated version
tag, including its EPP job. Every reusable job checks out that source ref, rather
than implicitly testing the branch used to dispatch the release. Publication also
requires all platform builds to succeed; local checks do not publish artifacts.
