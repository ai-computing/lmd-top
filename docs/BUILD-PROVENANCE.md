# Build provenance

New compile publications write `lmd-build.json` with schema `lmd-top/build/v2`.
Existing artifacts remain untouched. The read-only provenance probe displays either
schema alongside vendor-native metadata.

The v2 record includes the model identifier; a commit hash when the saved config
reports one; requested image reference and compile form options; effective RBLN
kwargs (including the TP spelling accepted by the installed SDK), or the actual
Furiosa command options; observed Python package versions; recipe SHA-256 hashes;
Python/host architecture; completion time; and the execution Pod UID. Missing
packages/revision are null. The legacy `execution_uid` field is explicitly identified
as a Pod UID, not the Job UID.

`provenance_sha256` hashes the canonical recorded provenance, excluding publication
time and execution UID. It is not an artifact-content checksum or a guarantee that
all inputs were captured. Arbitrary environment overrides, external host libraries,
driver/firmware, and resolved container image identity are not fully measured.
Runtime compatibility stays `unknown`; a matching metadata fingerprint does not
establish safe reuse, binary reproducibility, or runtime compatibility.

Only known compile option keys are persisted. The environment is not dumped, and
HF tokens are not collected. Generated `LMD_BUILD_*` variables cannot be overridden
through the debug environment passthrough. Metadata generation must succeed before
atomic publication; any failure removes the staging directory, leaving no published
partial artifact. Existing build directories are never overwritten.

Requested and effective options are intentionally separate. RBLN `w8a8` and
`w4a16` now map to `rbln_quantization` dictionaries (format, weights, activations),
which the installed SDK validates. They describe compatible prequantized checkpoints;
they do not perform calibration or quantization of floating-point weights. `none`
leaves the SDK setting unspecified. SDK rejection is not retried without quantization.
See the [vendor implementation](https://github.com/RBLN-SW/optimum-rbln/blob/v0.10.2/src/optimum/rbln/transformers/utils/rbln_quantization.py).

Furiosa bucket/chunk/block/activation-DQ fields are not yet mapped into the current
fxb command. Non-default overrides are now explicitly rejected before manifest generation.
Legacy default values are placeholders, not effective SDK options. Full custom
bucket support still requires an SDK-specific adapter. Recording an option does not implement it.

Furiosa compile workers (`concurrency`, default 8) and bucket optimization (`optim`,
default O3) are forwarded to `fxb build` and recorded in effective provenance.
Workers consume host CPU/RAM and are not inference batch size. O0 is a minimal
bucket set intended for development; O3 uses the full bucket set. Non-default
selection changes the build target fingerprint. These options require an SDK with
[the documented FXB CLI](https://developer.furiosa.ai/latest/en/furiosa_llm/fxb.html);
unsupported CLI flags fail the job rather than being silently removed.

## Artifact integrity

New publications include `artifact_manifest` (`sha256-canonical-file-list-v1`) and
`artifact_content_sha256`. Each regular file is hashed in 1 MiB chunks from the
private staging copy. The sorted relative POSIX path, byte length and SHA-256 list
is encoded as JSON with sorted keys, compact separators and ASCII escaping; its
UTF-8 SHA-256 is the aggregate hash. Root `lmd-build.json` is excluded. Empty
folders, permissions and timestamps are not covered. Existing metadata with null
content hashes is not retroactively considered verified.

Run `python3 assets/recipes/publish-artifact.py --verify /path/to/build` to compare
both the file list and aggregate hash. This detects additions, removals, renames and
byte changes, but is not an authenticity signature: replacing both files and metadata
can bypass integrity checking. Keep trusted hashes separately when authenticity is
needed. Reading large artifacts adds I/O before atomic publication.

The manifest requires a real root directory and regular files. Symbolic links
(including directory and metadata links), special files and unreadable subtrees
fail verification rather than being skipped. Publication preserves links while
staging so this validation rejects them instead of following external targets.
The verifier expects a quiescent artifact directory; concurrent writers are outside
this integrity contract. Published directories should remain immutable.

## Rust release dependency inventory

Release archives include `DEPENDENCIES.json`, generated with:

```sh
python3 scripts/build-inventory.py --target x86_64-unknown-linux-gnu --output DEPENDENCIES.json
```

The script uses locked, offline Cargo metadata filtered to the requested target
(default: rustc host). It records resolved package versions, sources, enabled
features, declared license expressions, Rust/Cargo versions, and the Cargo.lock
SHA-256. Entries are sorted and no timestamp or absolute local manifest path is
included, so repeated runs with identical inputs/toolchains are byte-identical.
Metadata failure fails the command rather than emitting a partial inventory.

This describes Cargo's target-filtered resolution, including build/test dependencies;
it is not binary reachability analysis or a license audit. The separate Go plugin,
Python packages installed in recipes, external SDKs and runtime container contents
are outside this inventory and need their own provenance records.
