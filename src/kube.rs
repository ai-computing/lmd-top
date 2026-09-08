//! kubectl shell-out helpers — receive JSON and parse into serde_json::Value.
//! Phase 1 starts with kubectl shell-out (fast). Consider promoting to native kube-rs later.

use anyhow::{anyhow, Result};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Turn a spawn failure into an actionable message. `--request-timeout` bounds the API call but
/// says nothing when the binary itself is missing — users saw a bare
/// "No such file or directory (os error 2)" instead (BUG-09).
fn spawn_err(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        anyhow!("kubectl not found in PATH — install kubectl or fix PATH")
    } else {
        anyhow!("cannot run kubectl: {}", e)
    }
}

/// Outer wall-clock bound on a kubectl shell-out. `--request-timeout` only covers the API
/// round-trip; an exec credential plugin or DNS stall can hang the process past it, and one
/// stuck call would stall the whole collect tick (REG-07 / BUG-01).
async fn run(mut cmd: Command, label: &str, budget: Duration) -> Result<std::process::Output> {
    match timeout(budget, cmd.output()).await {
        Ok(r) => r.map_err(spawn_err),
        Err(_) => Err(anyhow!(
            "kubectl {} timed out after {}s",
            label,
            budget.as_secs()
        )),
    }
}

/// Run `kubectl <args...> -o json` → Value. (caller includes -o json in args)
pub async fn get_json(args: &[&str]) -> Result<serde_json::Value> {
    let mut cmd = Command::new("kubectl");
    cmd.args(args).arg("--request-timeout=15s");
    let out = run(cmd, args.join(" ").as_str(), Duration::from_secs(20)).await?;
    if !out.status.success() {
        return Err(anyhow!(
            "kubectl {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    Ok(v)
}

/// Extract `.data["<key>"]` text (ConfigMap, etc.).
pub fn cm_data<'a>(cm: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    cm["data"][key].as_str()
}

/// Existence probe: `kubectl get <args> --ignore-not-found -o name`.
/// `Some(true)`=object present, `Some(false)`=absent, `None`=kubectl error (kind/CRD missing or cluster unreachable).
/// Read-only; used by the Setup(Doctor) view's prerequisite checks.
pub async fn get_exists(args: &[&str]) -> Option<bool> {
    let mut cmd = Command::new("kubectl");
    cmd.args(args)
        .args(["--ignore-not-found", "-o", "name", "--request-timeout=8s"]);
    let out = run(cmd, args.join(" ").as_str(), Duration::from_secs(12))
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

/// `kubectl get <args> -o jsonpath=<jp>` → trimmed stdout, or `None` on kubectl error (object/kind absent).
pub async fn get_jsonpath(args: &[&str], jp: &str) -> Option<String> {
    let mut cmd = Command::new("kubectl");
    cmd.args(args)
        .arg(format!("-o=jsonpath={}", jp))
        .arg("--request-timeout=8s");
    let out = run(cmd, args.join(" ").as_str(), Duration::from_secs(12))
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Apply an upstream release manifest by URL: `kubectl apply -f <url>` (server-side).
/// For Setup(Doctor) CRD installs (Gateway API / Inference Extension). Sync (worker thread).
pub fn apply_url(url: &str) -> Result<String> {
    let out = std::process::Command::new("kubectl")
        .args(["apply", "--server-side", "-f", url, "--request-timeout=60s"])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "apply -f {} failed: {}",
            url,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Tail pod logs (sync, called from UI thread). --all-containers, last `tail` lines.
pub fn logs(ns: &str, pod: &str, tail: u32) -> Result<Vec<String>> {
    let out = std::process::Command::new("kubectl")
        .args([
            "logs",
            pod,
            "-n",
            ns,
            "--all-containers=true",
            "--prefix=false",
            &format!("--tail={}", tail),
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect())
}

/// Last non-empty line of pod logs (for progress hints). async, short timeout. None on failure.
pub async fn last_log_line(ns: &str, pod: &str) -> Option<String> {
    let out = Command::new("kubectl")
        .args(["logs", pod, "-n", ns, "--tail=5", "--request-timeout=4s"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|l| {
            // Truncate if too long (protect panel width).
            if l.chars().count() > 60 {
                format!("{}…", l.chars().take(60).collect::<String>())
            } else {
                l.to_string()
            }
        })
}

/// Mutating action: deploy scale. (M5) — uses sync std (called from blocking UI thread).
pub fn scale_deploy(ns: &str, name: &str, replicas: i64) -> Result<()> {
    let out = std::process::Command::new("kubectl")
        .args([
            "scale",
            "deployment",
            name,
            "-n",
            ns,
            &format!("--replicas={}", replicas),
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "scale failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Collect metadata.name of `kind: Job` documents from a (multi-document) manifest.
/// Used to pre-delete existing Jobs before re-applying a compile Job. Unparseable documents are silently skipped.
fn job_names(yaml: &str) -> Vec<String> {
    let mut names = Vec::new();
    for doc in yaml.split("\n---\n") {
        if let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(doc) {
            if v.get("kind").and_then(|k| k.as_str()) == Some("Job") {
                if let Some(n) = v
                    .get("metadata")
                    .and_then(|m| m.get("name"))
                    .and_then(|n| n.as_str())
                {
                    names.push(n.to_string());
                }
            }
        }
    }
    names
}

fn dry_run_job_name(name: &str) -> String {
    const SUFFIX: &str = "-dryrun";
    if name.len() + SUFFIX.len() <= 63 {
        return format!("{}{}", name, SUFFIX);
    }
    let keep = 63usize.saturating_sub(SUFFIX.len());
    format!(
        "{}{}",
        name.chars()
            .take(keep)
            .collect::<String>()
            .trim_matches('-'),
        SUFFIX
    )
}

/// Server-side dry-run should validate the generated Job spec as a new Job. If a real Job
/// of the same name already exists, `kubectl apply --dry-run=server` still checks the update
/// path and fails on Job spec.template immutability. Rename only Job documents for dry-run
/// validation; real apply keeps the exact names and deletes/recreates Jobs above.
fn rename_jobs_for_dry_run(yaml: &str) -> String {
    let mut docs = Vec::new();
    for doc in yaml.split("\n---\n") {
        if doc.trim().is_empty() {
            continue;
        }
        let Ok(mut v) = serde_yaml::from_str::<serde_yaml::Value>(doc) else {
            docs.push(doc.to_string());
            continue;
        };
        if v.get("kind").and_then(|k| k.as_str()) == Some("Job") {
            if let Some(name) = v
                .get("metadata")
                .and_then(|m| m.get("name"))
                .and_then(|n| n.as_str())
                .map(dry_run_job_name)
            {
                if let Some(meta) = v.get_mut("metadata").and_then(|m| m.as_mapping_mut()) {
                    meta.insert(
                        serde_yaml::Value::String("name".into()),
                        serde_yaml::Value::String(name),
                    );
                }
            }
        }
        docs.push(serde_yaml::to_string(&v).unwrap_or_else(|_| doc.to_string()));
    }
    docs.join("---\n")
}

/// Apply a manifest via stdin with `kubectl apply -f -`. dry_run=true does a server dry-run (validate without changes).
/// On success, returns kubectl output (created/changed summary).
pub fn apply_manifest(ns: &str, yaml: &str, dry_run: bool) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;
    // A Job's spec.template is immutable → if a Job of the same name already exists, `apply`
    // rejects it with "field is immutable" (the cause of compile-retry failures). Compile Jobs
    // are one-shot, so re-apply = re-run is correct; delete the existing Job first (if any) before the real apply.
    if !dry_run {
        for name in job_names(yaml) {
            let _ = std::process::Command::new("kubectl")
                .args([
                    "delete",
                    "job",
                    &name,
                    "-n",
                    ns,
                    "--ignore-not-found",
                    "--wait=true",
                    "--request-timeout=8s",
                ])
                .output();
        }
    }
    let dry_run_yaml;
    let input_yaml = if dry_run && !job_names(yaml).is_empty() {
        dry_run_yaml = rename_jobs_for_dry_run(yaml);
        dry_run_yaml.as_str()
    } else {
        yaml
    };
    let mut cmd = std::process::Command::new("kubectl");
    cmd.args(["apply", "-n", ns, "-f", "-", "--request-timeout=8s"]);
    if dry_run {
        cmd.arg("--dry-run=server");
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no stdin"))?
        .write_all(input_yaml.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Delete pod (`kubectl delete pod <name> -n ns`) — triggers reschedule. admin action.
pub fn delete_pod(ns: &str, name: &str) -> Result<()> {
    let out = std::process::Command::new("kubectl")
        .args([
            "delete",
            "pod",
            name,
            "-n",
            ns,
            "--wait=false",
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// Delete compile Job (`kubectl delete job <name>`) — cancel/clean up in-progress work. Pods are cleaned up too.
pub fn delete_job(ns: &str, name: &str) -> Result<()> {
    let out = std::process::Command::new("kubectl")
        .args([
            "delete",
            "job",
            name,
            "-n",
            ns,
            "--ignore-not-found",
            "--wait=false",
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// Fetch live resource YAML (`kubectl get <kind> <name> [-n ns] -o yaml`) — for read-only preview.
pub fn resource_yaml(kind: &str, ns: Option<&str>, name: &str) -> Result<String> {
    let mut args: Vec<String> = vec![
        "get".into(),
        kind.into(),
        name.into(),
        "-o".into(),
        "yaml".into(),
    ];
    if let Some(n) = ns {
        args.push("-n".into());
        args.push(n.into());
    }
    args.push("--request-timeout=8s".into());
    let out = std::process::Command::new("kubectl").args(&args).output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

// ── HTTPRoute editing (route management) — get→modify JSON→server-side apply ──
fn route_load(ns: &str, name: &str) -> Result<serde_json::Value> {
    let out = std::process::Command::new("kubectl")
        .args([
            "get",
            "httproute",
            name,
            "-n",
            ns,
            "-o",
            "json",
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

fn route_save(ns: &str, mut v: serde_json::Value) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;
    // Remove server-managed fields for SSA (resourceVersion/managedFields/status, etc.).
    if let Some(m) = v.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        for k in [
            "managedFields",
            "resourceVersion",
            "uid",
            "creationTimestamp",
            "generation",
        ] {
            m.remove(k);
        }
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("status");
    }
    let body = serde_json::to_string(&v)?;
    let mut cmd = std::process::Command::new("kubectl");
    cmd.args([
        "apply",
        "--server-side",
        "--force-conflicts",
        "-n",
        ns,
        "-f",
        "-",
        "--request-timeout=8s",
    ]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no stdin"))?
        .write_all(body.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Change route path (rename) — set matches[].path.value == old to new within the HTTPRoute.
pub fn route_set_path(ns: &str, route: &str, old: &str, new: &str) -> Result<String> {
    let mut v = route_load(ns, route)?;
    let mut found = false;
    if let Some(rules) = v["spec"]["rules"].as_array_mut() {
        for rule in rules.iter_mut() {
            if let Some(ms) = rule["matches"].as_array_mut() {
                for m in ms.iter_mut() {
                    if m["path"]["value"].as_str() == Some(old) {
                        m["path"]["value"] = serde_json::Value::String(new.to_string());
                        found = true;
                    }
                }
            }
        }
    }
    if !found {
        return Err(anyhow!("path {} not found in httproute {}", old, route));
    }
    route_save(ns, v)
}

/// Delete route rule (delete) — remove the rule with the given path.
pub fn route_delete_rule(ns: &str, route: &str, path: &str) -> Result<String> {
    let mut v = route_load(ns, route)?;
    if let Some(rules) = v["spec"]["rules"].as_array_mut() {
        let before = rules.len();
        rules.retain(|rule| {
            rule["matches"]
                .as_array()
                .map(|ms| !ms.iter().any(|m| m["path"]["value"].as_str() == Some(path)))
                .unwrap_or(true)
        });
        if rules.len() == before {
            return Err(anyhow!("path {} not found in httproute {}", path, route));
        }
    }
    route_save(ns, v)
}

/// Change route backend (retarget) — set the given path's backendRefs to a new backend/kind.
pub fn route_retarget(
    ns: &str,
    route: &str,
    path: &str,
    backend: &str,
    kind: &str,
) -> Result<String> {
    let mut v = route_load(ns, route)?;
    let group = if kind == "InferencePool" {
        "inference.networking.k8s.io"
    } else {
        ""
    };
    let mut found = false;
    if let Some(rules) = v["spec"]["rules"].as_array_mut() {
        for rule in rules.iter_mut() {
            let hit = rule["matches"]
                .as_array()
                .map(|ms| ms.iter().any(|m| m["path"]["value"].as_str() == Some(path)))
                .unwrap_or(false);
            if hit {
                rule["backendRefs"] =
                    serde_json::json!([{ "group": group, "kind": kind, "name": backend }]);
                found = true;
            }
        }
    }
    if !found {
        return Err(anyhow!("path {} not found in httproute {}", path, route));
    }
    route_save(ns, v)
}

/// Node cordon/uncordon — block/unblock scheduling. admin action (namespace-agnostic).
pub fn cordon(node: &str, on: bool) -> Result<()> {
    let verb = if on { "cordon" } else { "uncordon" };
    let out = std::process::Command::new("kubectl")
        .args([verb, node, "--request-timeout=8s"])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "{} failed: {}",
            verb,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Rollout restart (`kubectl rollout restart deploy/<name>`) — rolling restart. admin action.
pub fn rollout_restart(ns: &str, name: &str) -> Result<()> {
    let out = std::process::Command::new("kubectl")
        .args([
            "rollout",
            "restart",
            "deployment",
            name,
            "-n",
            ns,
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "rollout restart failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Rollout undo (`kubectl rollout undo deploy/<name>`) — roll back to the previous ReplicaSet. admin action.
pub fn rollout_undo(ns: &str, name: &str) -> Result<()> {
    let out = std::process::Command::new("kubectl")
        .args([
            "rollout",
            "undo",
            "deployment",
            name,
            "-n",
            ns,
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "rollout undo failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Pick the routing/selector label to break for an endpoint drain, in priority order.
/// Changing this label's value evicts the pod from the InferencePool/Service endpoints and from
/// its owning ReplicaSet's selector (so a fresh replacement spins up), while the pod keeps serving
/// in-flight requests. Returns the key present in `labels`, or None if no known selector label exists.
pub fn drain_label_key(labels: &serde_json::Value) -> Option<String> {
    const CANDIDATES: &[&str] = &[
        "app",
        "app.kubernetes.io/name",
        "llm-d.ai/model",
        "llm-d.ai/inferenceServing",
    ];
    let obj = labels.as_object()?;
    CANDIDATES
        .iter()
        .find(|k| obj.get(**k).and_then(|v| v.as_str()).is_some())
        .map(|k| k.to_string())
}

/// Endpoint drain — relabel `<key>=<val>-drained` so the pod leaves routing (new requests stop)
/// while finishing in-flight streams. Reversible: relabel back to `<val>`. Returns "(key=old→new)".
/// Reads the pod's labels live to find the selector label; errors if the pod has none we recognize.
pub fn drain_pod(ns: &str, pod: &str) -> Result<String> {
    let get = std::process::Command::new("kubectl")
        .args([
            "get",
            "pod",
            pod,
            "-n",
            ns,
            "-o",
            "jsonpath={.metadata.labels}",
            "--request-timeout=8s",
        ])
        .output()?;
    if !get.status.success() {
        return Err(anyhow!(
            "read pod labels failed: {}",
            String::from_utf8_lossy(&get.stderr).trim()
        ));
    }
    let raw = String::from_utf8_lossy(&get.stdout);
    let labels: serde_json::Value = serde_json::from_str(raw.trim()).unwrap_or(serde_json::Value::Null);
    let key = drain_label_key(&labels)
        .ok_or_else(|| anyhow!("no routing label (app / llm-d.ai/model) to drain on {}", pod))?;
    let old = labels
        .get(&key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if old.ends_with("-drained") {
        return Err(anyhow!("{} already drained ({}={})", pod, key, old));
    }
    let new = format!("{}-drained", old);
    let out = std::process::Command::new("kubectl")
        .args([
            "label",
            "pod",
            pod,
            "-n",
            ns,
            &format!("{}={}", key, new),
            "--overwrite",
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "drain (relabel) failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(format!("({}={}→{})", key, old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_label_key_priority_and_absence() {
        // app 우선.
        let l = serde_json::json!({"app": "gemma4-rbln", "role": "decode"});
        assert_eq!(drain_label_key(&l).as_deref(), Some("app"));
        // app 없으면 llm-d.ai/model 로 폴백.
        let l2 = serde_json::json!({"llm-d.ai/model": "k-exaone-236b", "role": "decode"});
        assert_eq!(drain_label_key(&l2).as_deref(), Some("llm-d.ai/model"));
        // 알 수 없는 라벨만 있으면 None(드레인 대상 없음).
        let l3 = serde_json::json!({"pod-template-hash": "abc123"});
        assert_eq!(drain_label_key(&l3), None);
        // 라벨 객체 아님 → None.
        assert_eq!(drain_label_key(&serde_json::Value::Null), None);
    }

    #[test]
    fn job_names_extracts_only_jobs() {
        // Only Job names from a multi-document manifest (ConfigMap---Job). flow-style metadata parsing.
        let yaml = "apiVersion: v1\nkind: ConfigMap\nmetadata: { name: foo-script, namespace: llm-serving }\ndata: { a: b }\n---\napiVersion: batch/v1\nkind: Job\nmetadata: { name: compile-foo-rbln-tp4, namespace: llm-serving }\nspec: { backoffLimit: 0 }\n";
        assert_eq!(job_names(yaml), vec!["compile-foo-rbln-tp4".to_string()]);
        // Manifest with no Job (Deployment, etc.) → empty list (no pre-deletion).
        let dep = "apiVersion: apps/v1\nkind: Deployment\nmetadata: { name: srv }\n";
        assert!(job_names(dep).is_empty());
    }
}
