//! Read an artifact's recorded provenance off the node that holds it.
//!
//! Store builds get their toolchain from the discovery scan (see the `built_with` column). The
//! artifacts actually serving on this cluster are hand-compiled `hostPath` directories under a
//! node's home, which no scan reaches — and those are the ones whose provenance matters most,
//! because they are what is running.
//!
//! A `hostPath` can only be read by a pod on that node, so this builds a short-lived read-only
//! Job pinned there. The script is a constant: the directory arrives through the environment
//! and is expanded quoted, so a path from a Deployment spec can never become shell syntax.

use crate::manifest::{args, env_val, mount, s, seq, Doc, Manifest};

/// Where the artifact directory is mounted inside the probe.
const PROBE_MOUNT: &str = "/artifact";

/// Only needs `ls`, `tr`, `awk`, `grep` and `head`.
const PROBE_IMAGE: &str = "busybox:1.36";

/// Reject a path that could not have come from a Deployment's `hostPath`.
///
/// The value is cluster-supplied rather than typed, and the pod mounts it read-only and only
/// reads from it, so this is a sanity check rather than the whole defence — the defence is that
/// the path never reaches a shell.
pub fn validate_host_path(p: &str) -> Result<String, String> {
    let t = p.trim();
    if t.is_empty() {
        return Err("no hostPath on this artifact's model volume".into());
    }
    if !t.starts_with('/') {
        return Err(format!("hostPath must be absolute: {}", t));
    }
    if t.split('/').any(|seg| seg == "..") {
        return Err(format!("refusing a path containing '..': {}", t));
    }
    // Newlines and NULs would corrupt the manifest itself; the rest is belt and braces.
    if let Some(c) = t.chars().find(|c| c.is_control()) {
        return Err(format!("control character {:?} in hostPath", c));
    }
    if t == "/" {
        return Err("refusing to probe the node's root filesystem".into());
    }
    Ok(t.to_string())
}

/// A DNS-1123 Job name for probing one deployment.
fn job_name(deployment: &str) -> String {
    let mut slug: String = deployment
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
    let slug = slug.trim_matches('-');
    let budget = 63usize.saturating_sub("probe-".len());
    let tail = if slug.len() > budget {
        slug[slug.len() - budget..].trim_matches('-')
    } else {
        slug
    };
    format!("probe-{}", tail)
}

/// The provenance reader. A constant script — `$DIR` is quoted, so the path is data.
///
/// Reads both vendors the same way the discovery scan does: RBLN records its version in a
/// plain-text `rbln_config.json`, Furiosa inside `model.fxb`, a zip whose manifest is stored
/// uncompressed at the *end* of the file (hence `tail`, not `head`).
const PROBE_SCRIPT: &str = r#"set -u
echo "path: $DIR"
if [ ! -d "$DIR" ]; then echo "MISSING: not a directory on this node"; exit 0; fi
echo "files:"
ls -la "$DIR" 2>/dev/null | head -20
echo

read_rbln() {
  f=$1
  v=$(grep -o '"optimum_rbln_version"[[:space:]]*:[[:space:]]*"[^"]*"' "$f" 2>/dev/null       | head -1 | sed 's/.*"\([^"]*\)"$//')
  echo "built_with: optimum-rbln=${v:-unknown}   ($f)"
  grep -o '"cls_name"[[:space:]]*:[[:space:]]*"[^"]*"' "$f" 2>/dev/null | head -1
  grep -o '"dtype"[[:space:]]*:[[:space:]]*"[^"]*"' "$f" 2>/dev/null | head -1
}

read_fxb() {
  f=$1
  v=$(tail -c 262144 "$f" 2>/dev/null | tr -c '[:print:]
' '
'       | awk '/"furiosa_compiler"/{f=1} f && /"version"/{ if (match($0,/[0-9][0-9.]*/)) { print substr($0,RSTART,RLENGTH); exit } }')
  echo "built_with: furiosa-compiler=${v:-unknown}   ($f)"
}

found=0
if [ -f "$DIR/rbln_config.json" ]; then read_rbln "$DIR/rbln_config.json"; found=1
elif [ -f "$DIR/model.fxb" ]; then read_fxb "$DIR/model.fxb"; found=1
fi

# Disaggregated builds keep one artifact per role (prefill/decode), so the config lives one
# level down. Report every role rather than the first, since they can differ.
if [ "$found" = 0 ]; then
  for d in "$DIR"/*/; do
    [ -d "$d" ] || continue
    if [ -f "$d/rbln_config.json" ]; then read_rbln "$d/rbln_config.json"; found=1
    elif [ -f "$d/model.fxb" ]; then read_fxb "$d/model.fxb"; found=1
    fi
  done
fi

[ "$found" = 0 ] && echo "built_with: unknown (no rbln_config.json or model.fxb here or one level down)"
exit 0
"#;

/// Job that reads one artifact's provenance on the node that holds it.
///
/// Returns the manifest and the Job's name together, so the caller waits on the same name the
/// manifest creates instead of re-deriving it and drifting.
///
/// `host_path` must have come through [`validate_host_path`].
pub fn provenance_manifest(
    ns: &str,
    deployment: &str,
    node: &str,
    host_path: &str,
) -> (Manifest, String) {
    let name = job_name(deployment);
    let job = crate::ymap! {
        "apiVersion" => s("batch/v1"),
        "kind" => s("Job"),
        "metadata" => crate::ymap! {
            "name" => s(name.clone()),
            "namespace" => s(ns),
            "labels" => crate::ymap! {
                "app.kubernetes.io/component" => s("provenance-probe"),
                "lmd-top/probe-of" => s(deployment),
            },
        },
        "spec" => crate::ymap! {
            "backoffLimit" => serde_yaml::Value::from(0),
            // Short-lived: the answer is in the logs and is read straight away.
            "ttlSecondsAfterFinished" => serde_yaml::Value::from(600),
            "template" => crate::ymap! {
                "spec" => crate::ymap! {
                    "restartPolicy" => s("Never"),
                    // A hostPath only exists on its own node.
                    "nodeSelector" => crate::ymap! { "kubernetes.io/hostname" => s(node) },
                    // The artifact node may be tainted for accelerator work; this pod does no
                    // accelerator work, but it must still be allowed to land there.
                    "tolerations" => seq(vec![crate::ymap! { "operator" => s("Exists") }]),
                    "volumes" => seq(vec![crate::ymap! {
                        "name" => s("artifact"),
                        "hostPath" => crate::ymap! {
                            "path" => s(host_path),
                            "type" => s("Directory"),
                        },
                    }]),
                    "containers" => seq(vec![crate::ymap! {
                        "name" => s("read"),
                        "image" => s(PROBE_IMAGE),
                        // Constant script; the path arrives as env and is expanded quoted.
                        "command" => args(["sh", "-c", PROBE_SCRIPT]),
                        "env" => seq(vec![env_val("DIR", PROBE_MOUNT)]),
                        "resources" => crate::ymap! {
                            "requests" => crate::ymap! { "cpu" => s("50m"), "memory" => s("32Mi") },
                            "limits" => crate::ymap! { "memory" => s("128Mi") },
                        },
                        // Read-only: a probe must not be able to change what it inspects.
                        "volumeMounts" => seq(vec![mount("artifact", PROBE_MOUNT, true)]),
                    }]),
                },
            },
        },
    };
    let m = Manifest::new()
        .note(format!(
            "Provenance probe — read {} on node {} (read-only)",
            host_path, node
        ))
        .note("Reads the toolchain the artifact recorded; changes nothing.".to_string())
        .push(Doc::new(job).note(format!("for deployment {}", deployment)));
    (m, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_of(m: &Manifest) -> serde_yaml::Value {
        let y = m.to_yaml();
        let body: String = y
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        serde_yaml::from_str(&body).expect("valid yaml")
    }

    #[test]
    fn accepts_the_host_paths_this_cluster_actually_uses() {
        for p in [
            "/home/gspark/rbln-gemma4-26b-a4b-tp4-s8192",
            "/home/gspark/rbln-llama8b-pd",
            "/home/gspark/model-cache/ds4-gguf",
            "/home/gspark/.cache/huggingface",
        ] {
            assert_eq!(validate_host_path(p).as_deref(), Ok(p), "{}", p);
        }
    }

    #[test]
    fn refuses_paths_that_are_not_a_deployments_hostpath() {
        assert!(validate_host_path("").is_err(), "empty");
        assert!(validate_host_path("relative/path").is_err(), "not absolute");
        assert!(validate_host_path("/a/../../etc").is_err(), "traversal");
        assert!(validate_host_path("/").is_err(), "node root");
        assert!(validate_host_path("/a\nb").is_err(), "newline");
    }

    #[test]
    fn probe_is_pinned_read_only_and_never_interpolates_the_path() {
        let (m, name) = provenance_manifest(
            "llm-serving",
            "gemma4-rbln",
            "etri-001",
            "/home/gspark/rbln-gemma4-26b-a4b-tp4-s8192",
        );
        let v = job_of(&m);
        assert_eq!(
            v["metadata"]["name"].as_str(),
            Some(name.as_str()),
            "the returned name must be the one the manifest creates"
        );
        let pod = &v["spec"]["template"]["spec"];
        assert_eq!(
            pod["nodeSelector"]["kubernetes.io/hostname"].as_str(),
            Some("etri-001"),
            "a hostPath only exists on its own node"
        );
        assert_eq!(
            pod["volumes"][0]["hostPath"]["path"].as_str(),
            Some("/home/gspark/rbln-gemma4-26b-a4b-tp4-s8192")
        );
        assert_eq!(
            pod["containers"][0]["volumeMounts"][0]["readOnly"].as_bool(),
            Some(true),
            "a probe must not be able to change what it inspects"
        );
        // The script must be the constant, with the path supplied as env instead.
        let cmd: Vec<&str> = pod["containers"][0]["command"]
            .as_sequence()
            .expect("argv")
            .iter()
            .map(|x| x.as_str().unwrap_or_default())
            .collect();
        assert_eq!(cmd[0], "sh");
        assert_eq!(cmd[1], "-c");
        assert!(
            !cmd[2].contains("/home/gspark"),
            "the path must not be baked into the script"
        );
        assert!(cmd[2].contains("\"$DIR\""), "and must be expanded quoted");
        assert_eq!(pod["containers"][0]["env"][0]["name"].as_str(), Some("DIR"));
        assert_eq!(
            pod["containers"][0]["env"][0]["value"].as_str(),
            Some(PROBE_MOUNT)
        );
    }

    #[test]
    fn job_names_are_legal_dns_labels() {
        for d in [
            "gemma4-rbln",
            "sw-atom-decode-b1",
            "a-very-long-deployment-name-that-goes-well-past-the-sixty-three-character-limit",
        ] {
            let n = job_name(d);
            assert!(n.len() <= 63, "{} is {}", n, n.len());
            assert!(
                n.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{}",
                n
            );
            assert!(!n.starts_with('-') && !n.ends_with('-'), "{}", n);
        }
    }

    /// The script must read both vendors the way the discovery scan does — in particular the
    /// Furiosa manifest is at the end of the zip, so a head read would silently find nothing.
    #[test]
    fn script_reads_both_vendors_from_the_right_end() {
        assert!(PROBE_SCRIPT.contains("rbln_config.json"));
        assert!(PROBE_SCRIPT.contains("optimum_rbln_version"));
        assert!(PROBE_SCRIPT.contains("model.fxb"));
        assert!(
            PROBE_SCRIPT.contains("tail -c"),
            "the fxb manifest is at the end of the file"
        );
        assert!(
            !PROBE_SCRIPT.contains("head -c"),
            "a bounded head read returns nothing for an fxb"
        );
        // Disaggregated builds keep the config one level down, so a top-level-only probe
        // reports "unknown" for exactly the artifacts that are most interesting.
        assert!(
            PROBE_SCRIPT.contains(r#"for d in "$DIR"/*/"#),
            "must also look one level down"
        );
    }
}
