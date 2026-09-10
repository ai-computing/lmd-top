//! Shared model store maintenance — removing and relocating build artifacts.
//!
//! The store is a PVC (SMB in this cluster), so lmd-top cannot touch it from the client side.
//! Every change runs as a Job that mounts the claim read-write, the same shape as compile and
//! prefetch. That also means each change is reviewable as a manifest before it runs.
//!
//! The paths involved come from an inventory the cluster produced, not from free text, and they
//! end up as arguments to `rm -rf` and `mv`. So the validation here is the load-bearing part of
//! the module: a path that reaches those commands must be inside the store, must name a real
//! inventory entry, and must not be able to escape via `..` or a shell metacharacter. Nothing
//! is passed through a shell at all — `command` is an argv array, so quoting never enters into
//! it.

use crate::collect::{ModelArtifact, StoredModel};
use crate::manifest::{args, mount, s, seq, Doc, Manifest};

/// Where the store PVC is mounted inside the maintenance Job.
pub const STORE_MOUNT: &str = "/mnt/store";

/// Top-level store directories the inventory publishes, and the only ones we will touch.
///
/// `hub/` holds HuggingFace snapshots, `compiled/` the per-vendor build artifacts. Anything
/// else in the store belongs to something that is not us.
const ALLOWED_ROOTS: [&str; 2] = ["hub/", "compiled/"];

/// Why a path was refused. Separate from a plain string so the caller can phrase it.
#[derive(Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Absolute,
    Traversal,
    BadChar(char),
    OutsideStore,
    TooShallow,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::Empty => write!(f, "empty path"),
            PathError::Absolute => write!(f, "must be relative to the store root"),
            PathError::Traversal => write!(f, "'.' and '..' are not allowed in a store path"),
            PathError::BadChar(c) => write!(f, "illegal character {:?} in a store path", c),
            PathError::OutsideStore => {
                write!(f, "must be under hub/ or compiled/")
            }
            PathError::TooShallow => write!(f, "refusing to operate on a store root directory"),
        }
    }
}

/// Characters permitted in a store path component.
///
/// HuggingFace ids are `[A-Za-z0-9._-]`, and the discovery scan joins them with `/` and `--`,
/// so this covers every path the inventory can produce while excluding every shell
/// metacharacter, whitespace and control byte.
fn char_ok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')
}

/// Validate a store-relative path well enough to hand it to `rm -rf`.
///
/// Rejects absolute paths, `.`/`..` components, anything outside the published roots, and any
/// character that is not part of an inventory path. Also rejects a bare root (`compiled/`),
/// which would otherwise delete every artifact in the store.
pub fn validate_path(path: &str) -> Result<String, PathError> {
    let p = path.trim().trim_end_matches('/');
    if p.is_empty() {
        return Err(PathError::Empty);
    }
    if p.starts_with('/') {
        return Err(PathError::Absolute);
    }
    if let Some(c) = p.chars().find(|c| !char_ok(*c)) {
        return Err(PathError::BadChar(c));
    }
    let parts: Vec<&str> = p.split('/').filter(|x| !x.is_empty()).collect();
    if parts.iter().any(|x| *x == "." || *x == "..") {
        return Err(PathError::Traversal);
    }
    if !ALLOWED_ROOTS.iter().any(|r| p.starts_with(r)) {
        return Err(PathError::OutsideStore);
    }
    // "compiled/x" is still a whole model's worth of builds; the inventory always publishes
    // deeper than that (compiled/<repo>/<fmt>/<target>, hub/models--x/snapshots/<rev>).
    if parts.len() < 3 {
        return Err(PathError::TooShallow);
    }
    Ok(p.to_string())
}

/// Find the inventory entry a path names, so a deletion can only target something the cluster
/// actually reported. Guards against acting on a stale or hand-typed path.
pub fn entry_for<'a>(path: &str, inventory: &'a [StoredModel]) -> Option<&'a StoredModel> {
    let want = path.trim().trim_end_matches('/');
    inventory
        .iter()
        .find(|s| s.path.trim().trim_end_matches('/') == want)
}

/// Deployments that appear to be serving out of this store path.
///
/// Best effort by construction: it compares the path against what each Deployment passes as its
/// model argument, which is the only place the mount is visible to us. A rename of the mount
/// inside the pod would hide the reference, so callers must treat an empty result as "no
/// evidence of use" rather than "proven unused" — and say so in the prompt.
pub fn in_use_by(path: &str, artifacts: &[ModelArtifact]) -> Vec<String> {
    let needle = path.trim().trim_end_matches('/');
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = artifacts
        .iter()
        .filter(|a| a.source.contains(needle) || a.mount.contains(needle))
        .map(|a| a.model.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Validate a move destination: the same rules as any store path, plus it must not already be
/// an inventory entry (a move must not silently swallow another build) and must differ from the
/// source.
pub fn validate_dest(
    dest: &str,
    src: &str,
    inventory: &[StoredModel],
) -> Result<String, String> {
    let d = validate_path(dest).map_err(|e| e.to_string())?;
    let s = src.trim().trim_end_matches('/');
    if d == s {
        return Err("destination is the same as the source".into());
    }
    if entry_for(&d, inventory).is_some() {
        return Err(format!("{} already exists in the store inventory", d));
    }
    // Moving a directory into itself ("a/b" -> "a/b/c") would recurse.
    if d.starts_with(&format!("{}/", s)) {
        return Err("destination is inside the source directory".into());
    }
    Ok(d)
}

/// A Job name that is a legal DNS-1123 label derived from an operation and path.
fn job_name(op: &str, path: &str) -> String {
    let mut slug: String = path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-').to_string();
    // Leave room for the "store-<op>-" prefix inside the 63-char label limit.
    let budget = 63usize.saturating_sub(op.len() + 7);
    let tail: String = if slug.len() > budget {
        slug[slug.len() - budget..].trim_matches('-').to_string()
    } else {
        slug
    };
    format!("store-{}-{}", op, tail)
}

/// Common Job scaffolding for a store maintenance operation.
fn store_job(
    ns: &str,
    name: &str,
    pvc: &str,
    op: &str,
    containers: serde_yaml::Value,
    init: Option<serde_yaml::Value>,
) -> serde_yaml::Value {
    let mut pod = crate::ymap! {
        "restartPolicy" => s("Never"),
        "volumes" => seq(vec![crate::ymap! {
            "name" => s("store"),
            "persistentVolumeClaim" => crate::ymap! { "claimName" => s(pvc) },
        }]),
        "containers" => containers,
    };
    if let Some(i) = init {
        if let serde_yaml::Value::Mapping(m) = &mut pod {
            m.insert(s("initContainers"), i);
        }
    }
    crate::ymap! {
        "apiVersion" => s("batch/v1"),
        "kind" => s("Job"),
        "metadata" => crate::ymap! {
            "name" => s(name),
            "namespace" => s(ns),
            "labels" => crate::ymap! {
                "app.kubernetes.io/component" => s("store-maintenance"),
                "lmd-top/store-op" => s(op),
            },
        },
        "spec" => crate::ymap! {
            "backoffLimit" => serde_yaml::Value::from(0),
            // Outlive the gap between two runs of the tool, so the outcome is still readable.
            "ttlSecondsAfterFinished" => serde_yaml::Value::from(86_400),
            "template" => crate::ymap! { "spec" => pod },
        },
    }
}

/// The container image used for store maintenance. Only needs `rm`, `mv` and `mkdir`.
const MAINT_IMAGE: &str = "busybox:1.36";

fn maint_resources() -> serde_yaml::Value {
    crate::ymap! {
        "requests" => crate::ymap! { "cpu" => s("100m"), "memory" => s("64Mi") },
        "limits" => crate::ymap! { "memory" => s("256Mi") },
    }
}

/// Job that deletes one store artifact.
///
/// `path` must have come through [`validate_path`] and matched an inventory entry. The path is
/// an argv element, never a shell word.
pub fn delete_manifest(ns: &str, pvc: &str, path: &str, size: &str) -> Manifest {
    let full = format!("{}/{}", STORE_MOUNT, path);
    let containers = seq(vec![crate::ymap! {
        "name" => s("remove"),
        "image" => s(MAINT_IMAGE),
        // argv, not `sh -c` — the path cannot be reinterpreted as shell syntax.
        "command" => args(["rm", "-rf", "--", full.as_str()]),
        "resources" => maint_resources(),
        "volumeMounts" => seq(vec![mount("store", STORE_MOUNT, false)]),
    }]);
    let job = store_job(ns, &job_name("rm", path), pvc, "delete", containers, None);
    Manifest::new()
        .note(format!("Store maintenance — delete {} ({})", path, size))
        .note("Irreversible: the artifact is removed from the shared store.".to_string())
        .note(
            "Inventory is stale until discovery re-runs — use Rescan (r) to do it now."
                .to_string(),
        )
        .push(Doc::new(job).note(format!("rm -rf {}/{}", STORE_MOUNT, path)))
}

/// Job that relocates one store artifact. Creates the destination's parent first.
pub fn move_manifest(ns: &str, pvc: &str, src: &str, dest: &str) -> Manifest {
    let from = format!("{}/{}", STORE_MOUNT, src);
    let to = format!("{}/{}", STORE_MOUNT, dest);
    let parent = match dest.rsplit_once('/') {
        Some((p, _)) => format!("{}/{}", STORE_MOUNT, p),
        None => STORE_MOUNT.to_string(),
    };
    let init = seq(vec![crate::ymap! {
        "name" => s("mkparent"),
        "image" => s(MAINT_IMAGE),
        "command" => args(["mkdir", "-p", "--", parent.as_str()]),
        "resources" => maint_resources(),
        "volumeMounts" => seq(vec![mount("store", STORE_MOUNT, false)]),
    }]);
    let containers = seq(vec![crate::ymap! {
        "name" => s("relocate"),
        "image" => s(MAINT_IMAGE),
        "command" => args(["mv", "--", from.as_str(), to.as_str()]),
        "resources" => maint_resources(),
        "volumeMounts" => seq(vec![mount("store", STORE_MOUNT, false)]),
    }]);
    let job = store_job(
        ns,
        &job_name("mv", dest),
        pvc,
        "move",
        containers,
        Some(init),
    );
    Manifest::new()
        .note(format!("Store maintenance — move {} → {}", src, dest))
        .note("Deployments that reference the old path will stop resolving it.".to_string())
        .note(
            "Inventory is stale until discovery re-runs — use Rescan (r) to do it now."
                .to_string(),
        )
        .push(Doc::new(job).note(format!("mv {} {}", from, to)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(path: &str) -> StoredModel {
        StoredModel {
            repo: "org/name".into(),
            family: "name".into(),
            revision: "-".into(),
            format: "rbln".into(),
            compiled_for: "RBLN-CA22-tp4".into(),
            size: "12G".into(),
            path: path.into(),
        }
    }

    #[test]
    fn accepts_the_paths_the_inventory_actually_publishes() {
        for p in [
            "compiled/Qwen--Qwen2.5-0.5B-Instruct/rbln/RBLN-CA22-tp4-s8192",
            "compiled/furiosa-ai--qwen3-4b-fp8/furiosa/tp4-s4096",
            "hub/models--meta-llama--Llama-3.1-8B-Instruct/snapshots/abc123def",
        ] {
            assert_eq!(validate_path(p).as_deref(), Ok(p), "{}", p);
        }
    }

    #[test]
    fn refuses_anything_that_could_escape_or_be_reinterpreted() {
        // Each of these would otherwise become an argument to `rm -rf`.
        let cases: [(&str, PathError); 12] = [
            ("", PathError::Empty),
            ("   ", PathError::Empty),
            ("/etc/passwd", PathError::Absolute),
            ("compiled/../../etc", PathError::Traversal),
            ("compiled/a/../../..", PathError::Traversal),
            ("compiled/./a/b", PathError::Traversal),
            ("etc/shadow/x", PathError::OutsideStore),
            ("rbln-toolchain/0.10.3/x", PathError::OutsideStore),
            ("compiled/a b/c", PathError::BadChar(' ')),
            ("compiled/a;rm -rf/c", PathError::BadChar(';')),
            ("compiled/a$(id)/c", PathError::BadChar('$')),
            ("compiled/a\nb/c", PathError::BadChar('\n')),
        ];
        for (input, want) in cases {
            assert_eq!(validate_path(input), Err(want), "input {:?}", input);
        }
    }

    #[test]
    fn refuses_a_store_root_so_one_keystroke_cannot_empty_the_store() {
        for p in ["compiled", "compiled/", "hub", "hub/models--x"] {
            assert!(
                matches!(
                    validate_path(p),
                    Err(PathError::TooShallow) | Err(PathError::OutsideStore)
                ),
                "{} should not be deletable, got {:?}",
                p,
                validate_path(p)
            );
        }
    }

    #[test]
    fn a_deletion_must_name_a_real_inventory_entry() {
        let inv = [stored("compiled/org--name/rbln/tp4")];
        assert!(entry_for("compiled/org--name/rbln/tp4", &inv).is_some());
        assert!(entry_for("compiled/org--name/rbln/tp4/", &inv).is_some());
        assert!(entry_for("compiled/org--name/rbln/tp8", &inv).is_none());
    }

    fn artifact(model: &str, source: &str, mount: &str) -> ModelArtifact {
        ModelArtifact {
            model: model.into(),
            family: "f".into(),
            engine: "vllm".into(),
            node: "n".into(),
            image: "i".into(),
            source: source.into(),
            mount: mount.into(),
            opts: Vec::new(),
        }
    }

    #[test]
    fn reports_deployments_serving_from_the_path() {
        let arts = [
            artifact(
                "vllm-rbln-koni",
                "/mnt/store/compiled/org--name/rbln/tp4",
                "/mnt/store ← pvc/model-store",
            ),
            artifact("other", "Qwen/Qwen2.5-0.5B", "emptyDir"),
        ];
        assert_eq!(
            in_use_by("compiled/org--name/rbln/tp4", &arts),
            vec!["vllm-rbln-koni".to_string()]
        );
        assert!(in_use_by("compiled/org--name/rbln/tp8", &arts).is_empty());
        // An empty needle must not match every deployment.
        assert!(in_use_by("", &arts).is_empty());
    }

    #[test]
    fn move_destination_rules() {
        let inv = [
            stored("compiled/org--name/rbln/tp4"),
            stored("compiled/org--name/rbln/tp8"),
        ];
        let src = "compiled/org--name/rbln/tp4";
        // Happy path.
        assert_eq!(
            validate_dest("compiled/org--name/rbln/tp4-archive", src, &inv).as_deref(),
            Ok("compiled/org--name/rbln/tp4-archive")
        );
        // Would clobber another build.
        assert!(validate_dest("compiled/org--name/rbln/tp8", src, &inv).is_err());
        // No-op.
        assert!(validate_dest(src, src, &inv).is_err());
        // Into itself.
        assert!(validate_dest(&format!("{}/inner", src), src, &inv).is_err());
        // Still subject to the path rules.
        assert!(validate_dest("../escape/x", src, &inv).is_err());
    }

    #[test]
    fn job_names_are_legal_dns_labels() {
        for p in [
            "compiled/Qwen--Qwen2.5-0.5B-Instruct/rbln/RBLN-CA22-tp4-s8192",
            "hub/models--meta-llama--Llama-3.1-8B-Instruct/snapshots/abcdef1234567890abcdef",
        ] {
            for op in ["rm", "mv"] {
                let n = job_name(op, p);
                assert!(n.len() <= 63, "{} is {} chars", n, n.len());
                assert!(
                    n.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                    "{}",
                    n
                );
                assert!(!n.starts_with('-') && !n.ends_with('-'), "{}", n);
                assert!(!n.contains("--"), "{}", n);
            }
        }
    }

    #[test]
    fn delete_job_passes_the_path_as_argv_not_shell() {
        let m = delete_manifest("llm-serving", "model-store", "compiled/org--n/rbln/tp4", "12G");
        let y = m.to_yaml();
        let v: serde_yaml::Value = serde_yaml::from_str(
            y.lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        )
        .expect("valid yaml");
        let cmd = &v["spec"]["template"]["spec"]["containers"][0]["command"];
        let got: Vec<&str> = cmd
            .as_sequence()
            .expect("argv sequence")
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert_eq!(
            got,
            vec!["rm", "-rf", "--", "/mnt/store/compiled/org--n/rbln/tp4"]
        );
        // No shell anywhere in the command.
        assert!(!got.iter().any(|a| *a == "sh" || *a == "bash" || *a == "-c"));
        // Read-write mount, since we are modifying the store. `mount` omits readOnly when
        // false, which is k8s's own default, so absent and `false` both mean writable — what
        // must never appear here is `true`.
        assert_ne!(
            v["spec"]["template"]["spec"]["containers"][0]["volumeMounts"][0]["readOnly"]
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn move_job_creates_the_parent_then_renames() {
        let m = move_manifest(
            "llm-serving",
            "model-store",
            "compiled/org--n/rbln/tp4",
            "compiled/archive/org--n/rbln/tp4",
        );
        let y = m.to_yaml();
        let v: serde_yaml::Value = serde_yaml::from_str(
            y.lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        )
        .expect("valid yaml");
        let pod = &v["spec"]["template"]["spec"];
        let init: Vec<&str> = pod["initContainers"][0]["command"]
            .as_sequence()
            .expect("argv")
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert_eq!(
            init,
            vec!["mkdir", "-p", "--", "/mnt/store/compiled/archive/org--n/rbln"]
        );
        let mv: Vec<&str> = pod["containers"][0]["command"]
            .as_sequence()
            .expect("argv")
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert_eq!(
            mv,
            vec![
                "mv",
                "--",
                "/mnt/store/compiled/org--n/rbln/tp4",
                "/mnt/store/compiled/archive/org--n/rbln/tp4"
            ]
        );
    }
}
