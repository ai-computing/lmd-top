//! kubectl shell-out helpers — receive JSON and parse into serde_json::Value.
//! Phase 1 starts with kubectl shell-out (fast). Consider promoting to native kube-rs later.

use crate::kube_session::{blocking_command, command};
use anyhow::{anyhow, Result};
use tokio::process::Command;
use tokio::time::Duration;

/// Bound the whole process, including credential plugins, and reap it on timeout.
async fn run(cmd: Command, label: &str, budget: Duration) -> Result<std::process::Output> {
    crate::process::run(cmd, None, budget)
        .await
        .map_err(|e| anyhow!("kubectl {label}: {e}"))
}

/// Run `kubectl <args...> -o json` → Value. (caller includes -o json in args)
pub async fn get_json(args: &[&str]) -> Result<serde_json::Value> {
    let mut cmd = command()?;
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
    let mut cmd = command().ok()?;
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
    let mut cmd = command().ok()?;
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

/// Minimal base64 (standard alphabet, padded). A Secret's data must be base64, and pulling in
/// a crate for 20 lines is not worth it in a build that deliberately keeps its dependency
/// list short.
fn base64(input: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(A[(n >> 18 & 63) as usize] as char);
        out.push(A[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Create or replace an opaque Secret with one key.
///
/// The value goes to kubectl over **stdin**, never as an argument: `kubectl create secret
/// --from-literal=token=…` would put the token in the process table for every user on the
/// host. `apply` makes this an upsert, so re-entering a rotated token just works.
/// Command output is withheld because Kubernetes validation errors can echo secret material.
pub fn apply_secret(ns: &str, name: &str, key: &str, value: &str) -> Result<String> {
    let manifest = format!(
        "apiVersion: v1\nkind: Secret\nmetadata:\n  name: {}\n  namespace: {}\ntype: Opaque\ndata:\n  {}: {}\n",
        name,
        ns,
        key,
        base64(value.as_bytes())
    );
    let out = blocking_command()?
        .args(["apply", "-n", ns, "-f", "-", "--request-timeout=20s"])
        .input(manifest.as_bytes())
        .output()?;
    secret_output(name, key, out)
}

fn secret_output(name: &str, key: &str, out: std::process::Output) -> Result<String> {
    // Both stdout and stderr may echo a Secret, even on the first line. Never forward them.
    if !out.status.success() {
        return Err(anyhow!("creating secret {}/{} failed (kubectl exit {:?}); inspect RBAC, namespace and Secret key validity; command output withheld to protect the value", name, key, out.status.code()));
    }
    Ok(format!("secret/{} applied", name))
}

/// Apply an upstream release manifest by URL: `kubectl apply -f <url>` (server-side).
/// For Setup(Doctor) CRD installs (Gateway API / Inference Extension). Sync (worker thread).
pub fn apply_url(url: &str) -> Result<String> {
    let out = blocking_command()?
        .args(["apply", "--server-side", "-f", url, "--request-timeout=60s"])
        .budget(Duration::from_secs(75))
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

/// Tail pod logs (blocking; TUI callers use a worker). --all-containers, last `tail` lines.
pub fn logs(ns: &str, pod: &str, tail: u32) -> Result<Vec<String>> {
    let out = blocking_command()?
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
/// Tail a pod's log (async, for the collect tick). Used to classify why a Job failed — the
/// reason has to be captured before `ttlSecondsAfterFinished` deletes the Job.
pub async fn log_tail(ns: &str, pod: &str, lines: u32) -> Option<String> {
    let mut cmd = command().ok()?;
    cmd.args([
        "logs",
        pod,
        "-n",
        ns,
        &format!("--tail={}", lines),
        "--request-timeout=6s",
    ]);
    let out = run(cmd, "logs", Duration::from_secs(10)).await.ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Container exit code of a Job's pod, when it has terminated.
pub async fn pod_exit_code(ns: &str, pod: &str) -> Option<i32> {
    let v = get_json(&["get", "pod", pod, "-n", ns, "-o", "json"])
        .await
        .ok()?;
    v["status"]["containerStatuses"]
        .as_array()?
        .iter()
        .find_map(|cs| cs["state"]["terminated"]["exitCode"].as_i64())
        .map(|c| c as i32)
}

pub async fn last_log_line(ns: &str, pod: &str) -> Option<String> {
    let mut cmd = command().ok()?;
    cmd.args(["logs", pod, "-n", ns, "--tail=5", "--request-timeout=4s"]);
    let out = run(cmd, "log progress", Duration::from_secs(8))
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

/// Scale exactly the deployment version reviewed by the operator.
pub fn scale_deploy(
    ns: &str,
    name: &str,
    replicas: i64,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<()> {
    patch_deployment(ns, name, &crate::resource_guard::scale(expected, replicas)?)
}

/// Both dry-run and apply carry the same atomic UID/version tests. Never retry a conflict.
fn patch_deployment(ns: &str, name: &str, patch: &serde_json::Value) -> Result<()> {
    for dry_run in [true, false] {
        let mut cmd = blocking_command()?;
        cmd.args([
            "patch",
            "deployment",
            name,
            "-n",
            ns,
            "--type=json",
            "--patch-file=/dev/stdin",
            "--request-timeout=8s",
        ]);
        if dry_run {
            cmd.arg("--dry-run=server");
        }
        let out = cmd.input(&serde_json::to_vec(patch)?).output()?;
        if !out.status.success() {
            return Err(anyhow!(
                "conditional deployment update failed; refresh and review again: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// Resolve apply semantics with a server dry-run, retaining the full merged
/// object. The real request then uses create or version-conditional replace;
/// never a name-only apply that can silently rebase over concurrent changes.
pub fn prepare_manifest(ns: &str, yaml: &str) -> Result<crate::manifest_plan::Reviewed> {
    let mut prepared = Vec::new();
    for (verb, body) in crate::manifest_plan::plan(ns, yaml)? {
        if verb == "create" {
            prepared.push((verb, body));
            continue;
        }
        let current = blocking_command()?
            .args([
                "get",
                "-n",
                ns,
                "-f",
                "-",
                "--ignore-not-found",
                "-o",
                "json",
                "--request-timeout=8s",
            ])
            .input(body.as_bytes())
            .output()?;
        if !current.status.success() {
            return Err(anyhow!(
                "cannot inspect manifest target: {}",
                String::from_utf8_lossy(&current.stderr).trim()
            ));
        }
        if current.stdout.iter().all(u8::is_ascii_whitespace) {
            prepared.push(("create", body));
            continue;
        }
        let current: serde_json::Value = serde_json::from_slice(&current.stdout)?;
        let identity = crate::resource_guard::Identity::read(&current).ok_or_else(|| {
            anyhow!("manifest target identity unavailable; refresh and review again")
        })?;
        let merged = blocking_command()?
            .args([
                "apply",
                "-n",
                ns,
                "-f",
                "-",
                "--dry-run=server",
                "-o",
                "json",
                "--request-timeout=8s",
            ])
            .input(body.as_bytes())
            .output()?;
        if !merged.status.success() {
            return Err(anyhow!(
                "cannot prepare manifest update: {}",
                String::from_utf8_lossy(&merged.stderr).trim()
            ));
        }
        let merged: serde_json::Value = serde_json::from_slice(&merged.stdout)?;
        identity.verify(&merged)?;
        prepared.push(("replace", serde_json::to_string(&merged)?));
    }
    Ok(crate::manifest_plan::Reviewed::new(ns, yaml, prepared))
}

/// Validate all resources first, then apply them in document order. Jobs use
/// create so the API atomically refuses an existing execution, including one
/// created after validation. Multi-resource changes are not transactional.
pub fn apply_manifest(ns: &str, yaml: &str, dry_run: bool) -> Result<String> {
    let reviewed = prepare_manifest(ns, yaml)?;
    apply_reviewed_manifest(ns, yaml, &reviewed, dry_run)
}

pub fn apply_reviewed_manifest(
    ns: &str,
    yaml: &str,
    reviewed: &crate::manifest_plan::Reviewed,
    dry_run: bool,
) -> Result<String> {
    let steps = reviewed.steps(ns, yaml)?;
    let mut output = Vec::new();
    for dry in if dry_run {
        &[true][..]
    } else {
        &[true, false][..]
    } {
        output.clear();
        for (index, (verb, body)) in steps.iter().enumerate() {
            let mut cmd = blocking_command()?;
            cmd.args([verb, "-n", ns, "-f", "-", "--request-timeout=8s"]);
            if *dry {
                cmd.arg("--dry-run=server");
            }
            let out = cmd.input(body.as_bytes()).output()?;
            if !out.status.success() {
                let completed = if *dry { 0 } else { index };
                return Err(anyhow!(
                    "manifest resource {} failed during {}; {} earlier resource(s) applied: {}{}",
                    index + 1,
                    if *dry { "validation" } else { "application" },
                    completed,
                    String::from_utf8_lossy(&out.stderr).trim(),
                    if *verb == "create" {
                        "; an existing resource requires a new name or a newly reviewed update (Jobs require explicit reviewed deletion)"
                    } else {
                        ""
                    }
                ));
            }
            output.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
    }
    Ok(output.join("\n"))
}

/// Delete exactly the reviewed object; the API checks preconditions atomically.
fn delete_reviewed(
    ns: &str,
    kind: &str,
    name: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<()> {
    let (uri, body) = crate::resource_guard::delete_request(ns, kind, name, expected)?;
    for dry in [true, false] {
        let mut payload = body.clone();
        if dry {
            payload["dryRun"] = serde_json::json!(["All"]);
        }
        let out = blocking_command()?
            .args(["delete", "--raw", &uri, "-f", "-", "--request-timeout=8s"])
            .input(payload.to_string().as_bytes())
            .output()?;
        if !out.status.success() {
            return Err(anyhow!(
                "delete failed; refresh and review again: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// Delete the reviewed Pod; replacement is controlled by its workload controller.
pub fn delete_pod(
    ns: &str,
    name: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<()> {
    delete_reviewed(ns, "pods", name, expected)
}

/// Kick the store discovery scan now, instead of waiting for its CronJob schedule.
///
/// After a store build is removed or moved, the `model-inventory` ConfigMap still describes the
/// old contents until the CronJob next runs — up to ten minutes of a row that is no longer
/// there. This creates a one-off Job *from that CronJob*, so the scan logic stays defined in
/// one place (the cluster's own manifest) rather than being reimplemented here.
///
/// Returns the Job name. A missing CronJob is reported as such: the discovery manifest is
/// optional, and the store view works without it.
pub fn refresh_inventory(ns: &str) -> Result<String> {
    let name = format!("store-refresh-{}", crate::collect::now_secs());
    let out = blocking_command()?
        .args([
            "create",
            "job",
            &name,
            "--from=cronjob/model-discovery",
            "-n",
            ns,
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if err.contains("not found") {
            return Err(anyhow!(
                "cronjob/model-discovery is not installed in {} — apply manifests/model-store-discovery.yaml",
                ns
            ));
        }
        return Err(anyhow!("{}", err));
    }
    Ok(name)
}

/// Apply a one-shot Job, wait for it to finish, and return its logs.
///
/// Synchronous by design: it runs on the mutation worker thread, so the UI keeps rendering and
/// the header shows the operation in flight. Bounded by `timeout_secs` — a probe that cannot
/// be scheduled (a node cordoned or gone) must report that rather than hang.
///
/// The Job is left in place; its `ttlSecondsAfterFinished` cleans it up, which also means the
/// logs are still readable afterwards if the caller wants a second look.
pub fn run_job_for_output(ns: &str, yaml: &str, job: &str, timeout_secs: u64) -> Result<String> {
    apply_manifest(ns, yaml, false)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut last_state;
    loop {
        let out = blocking_command()?
            .args([
                "get",
                "job",
                job,
                "-n",
                ns,
                "-o",
                "jsonpath={.status.succeeded}/{.status.failed}",
                "--request-timeout=8s",
            ])
            .output()?;
        last_state = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if last_state.starts_with('1') || last_state.ends_with('1') {
            break;
        }
        if std::time::Instant::now() >= deadline {
            // Say why nothing came back, and leave the Job for inspection.
            let why = pending_reason(ns, job).unwrap_or_default();
            return Err(anyhow!(
                "probe did not finish within {}s (job status {:?}){}",
                timeout_secs,
                last_state,
                if why.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", why)
                }
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
    }
    let out = blocking_command()?
        .args([
            "logs",
            &format!("job/{}", job),
            "-n",
            ns,
            "--tail=200",
            "--request-timeout=15s",
        ])
        .output()?;
    let body = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if body.is_empty() {
        return Err(anyhow!(
            "probe produced no output (job status {:?}): {}",
            last_state,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(body)
}

/// Why a Job's pod has not started — the scheduler's own message, when there is one.
fn pending_reason(ns: &str, job: &str) -> Option<String> {
    let out = blocking_command()
        .ok()?
        .args([
            "get",
            "pods",
            "-n",
            ns,
            "-l",
            &format!("job-name={}", job),
            "-o",
            "jsonpath={.items[0].status.conditions[0].message}",
            "--request-timeout=8s",
        ])
        .output()
        .ok()?;
    let m = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!m.is_empty()).then_some(m)
}

/// Delete the reviewed compile Job with background dependent cleanup.
pub fn delete_job(
    ns: &str,
    name: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<()> {
    delete_reviewed(ns, "jobs", name, expected)
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
    let out = blocking_command()?.args(&args).output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

// ── HTTPRoute editing (route management) — get→modify JSON→server-side apply ──
fn route_load(ns: &str, name: &str) -> Result<serde_json::Value> {
    let out = blocking_command()?
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

fn reviewed_route(
    ns: &str,
    route: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<serde_json::Value> {
    let identity = crate::resource_guard::required(expected)?;
    let current = route_load(ns, route)?;
    identity.verify(&current)?;
    Ok(current)
}

fn route_save(ns: &str, mut v: serde_json::Value) -> Result<String> {
    // Replace retains resourceVersion: concurrent edits cause a conflict, never a forced overwrite.
    if v["metadata"]["resourceVersion"].as_str().is_none() {
        return Err(anyhow!("HTTPRoute resourceVersion missing"));
    }
    v.as_object_mut().unwrap().remove("status");
    if let Some(m) = v["metadata"].as_object_mut() {
        m.remove("managedFields");
    }
    let body = serde_json::to_vec(&v)?;
    for dry_run in [true, false] {
        let mut cmd = blocking_command()?;
        cmd.args(["replace", "-n", ns, "-f", "-", "--request-timeout=8s"]);
        if dry_run {
            cmd.arg("--dry-run=server");
        }
        let out = cmd.input(&body).output()?;
        if !out.status.success() {
            return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        if !dry_run {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
    }
    unreachable!()
}

pub fn route_set_path(
    ns: &str,
    route: &str,
    old: &str,
    new: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<String> {
    let mut v = reviewed_route(ns, route, expected)?;
    crate::route_edit::rename(&mut v, old, new)?;
    route_save(ns, v)
}

pub fn route_delete_rule(
    ns: &str,
    route: &str,
    path: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<String> {
    let mut v = reviewed_route(ns, route, expected)?;
    crate::route_edit::delete(&mut v, path)?;
    route_save(ns, v)
}

fn get_resource(ns: &str, kind: &str, name: &str) -> Result<serde_json::Value> {
    let mut cmd = blocking_command()?;
    cmd.args(["get", kind, "-n", ns, "-o", "json", "--request-timeout=8s"]);
    if !name.is_empty() {
        cmd.arg(name);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

pub fn route_retarget(
    ns: &str,
    route: &str,
    path: &str,
    backend: &str,
    kind: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<String> {
    let mut v = reviewed_route(ns, route, expected)?;
    let target = match kind {
        "Service" => get_resource(ns, "service", backend)?,
        "InferencePool" => get_resource(ns, "inferencepools.inference.networking.k8s.io", backend)?,
        _ => return Err(anyhow!("unsupported backend kind")),
    };
    crate::route_edit::retarget(
        &mut v,
        path,
        backend,
        kind,
        (kind == "Service").then_some(&target),
    )?;
    route_save(ns, v)
}

/// Node cordon/uncordon — block/unblock scheduling. admin action (namespace-agnostic).
pub fn cordon(
    node: &str,
    on: bool,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<()> {
    let patch = crate::resource_guard::cordon(expected, on)?;
    for dry_run in [true, false] {
        let mut cmd = blocking_command()?;
        cmd.args([
            "patch",
            "node",
            node,
            "--type=json",
            "--patch-file=/dev/stdin",
            "--request-timeout=8s",
        ]);
        if dry_run {
            cmd.arg("--dry-run=server");
        }
        let out = cmd.input(&serde_json::to_vec(&patch)?).output()?;
        if !out.status.success() {
            return Err(anyhow!(
                "conditional node update failed; refresh and review again: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// Restart the reviewed deployment without losing existing template metadata.
pub fn rollout_restart(
    ns: &str,
    name: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<()> {
    crate::resource_guard::required(expected)?;
    let current = get_resource(ns, "deployment", name)?;
    let timestamp = crate::audit::iso_utc(crate::collect::now_secs());
    let patch = crate::resource_guard::restart(expected, &current, &timestamp)?;
    patch_deployment(ns, name, &patch)
}

/// Roll back the reviewed Deployment to the latest earlier revision owned by its UID.
pub fn rollout_undo(
    ns: &str,
    name: &str,
    expected: Option<&crate::resource_guard::Identity>,
) -> Result<String> {
    crate::resource_guard::required(expected)?;
    let current = get_resource(ns, "deployment", name)?;
    let history = get_resource(ns, "replicasets", "")?;
    let plan = crate::rollback::plan(expected, &current, &history)?;
    patch_deployment(ns, name, &plan.patch)?;
    Ok(format!(
        "revision {} from ReplicaSet {}",
        plan.revision, plan.replica_set
    ))
}

fn patch_pod(ns: &str, pod: &str, patch: &serde_json::Value) -> Result<()> {
    let out = blocking_command()?
        .args([
            "patch",
            "pod",
            pod,
            "-n",
            ns,
            "--type=json",
            "-p",
            &patch.to_string(),
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

/// Exclude a Pod using actual Service/InferencePool selectors. Verify Service endpoint
/// propagation and retain original labels for recovery. This does not delete the Pod or
/// claim that active streams finished or that an EPP's internal cache has converged.
pub fn drain_pod(ns: &str, pod: &str) -> Result<String> {
    let original = get_resource(ns, "pod", pod)?;
    let services = get_resource(ns, "services", "")?;
    let pools = match get_resource(ns, "inferencepools.inference.networking.k8s.io", "") {
        Ok(p) => p,
        Err(e) if e.to_string().contains("doesn't have a resource type") => {
            serde_json::json!({"items":[]})
        }
        Err(e) => return Err(e),
    };
    let plan = crate::drain::plan(&original, &services, &pools)?;
    // Ensure permissions to observe convergence before changing any labels.
    if !plan.services.is_empty() {
        get_resource(ns, "endpointslices.discovery.k8s.io", "")?;
    }
    let patch = plan.patch(&original)?;
    patch_pod(ns, pod, &patch)?;
    let verify = (|| -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let current = get_resource(ns, "pod", pod)?;
            if current["metadata"]["uid"] != plan.uid
                || !plan
                    .changed
                    .iter()
                    .all(|(k, v)| current["metadata"]["labels"][k].as_str() == Some(v))
            {
                return Err(anyhow!(
                    "pod identity or drain labels changed during verification"
                ));
            }
            if plan.services.is_empty()
                || plan.services_detached(&get_resource(
                    ns,
                    "endpointslices.discovery.k8s.io",
                    "",
                )?)?
            {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(anyhow!("Service endpoints did not converge within 20s"));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    })();
    if let Err(e) = verify {
        return match patch_pod(ns, pod, &plan.rollback()) {
            Ok(()) => Err(anyhow!("drain verification failed; original labels restored: {e}")),
            Err(rollback) => Err(anyhow!("drain verification failed: {e}; recovery also failed: {rollback}; original labels remain in annotation {}", crate::drain::ORIGINAL)),
        };
    }
    Ok(format!(
        "({} labels changed; original labels: annotation {}; EPP cache/active streams unverified)",
        plan.changed.len(),
        crate::drain::ORIGINAL
    ))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn secret_output_never_forwards_echoed_values() {
        use std::os::unix::process::ExitStatusExt;
        for code in [0, 256] {
            let output = std::process::Output {
                status: std::process::ExitStatus::from_raw(code),
                stdout: b"private-value base64-value".to_vec(),
                stderr: b"private-value base64-value".to_vec(),
            };
            let result = super::secret_output("hf-token", "token", output);
            let text = result.unwrap_or_else(|e| e.to_string());
            assert!(!text.contains("private-value") && !text.contains("base64-value"));
        }
    }
}

#[cfg(test)]
mod secret_tests {
    use super::base64;

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        // RFC 4648 test vectors.
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want, "base64({:?})", input);
        }
        // A realistic HF token shape round-trips through a decoder.
        let tok = "hf_EXAMPLEnotARealTokenXXXXXXXXXXXXXX";
        let enc = base64(tok.as_bytes());
        assert!(!enc.contains('\n'), "no wrapping — k8s wants one line");
        assert_eq!(enc.len() % 4, 0, "padded to a multiple of 4");
    }
}

#[cfg(all(test, unix))]
mod node_command_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn node_conflict_command_trace() {
        const CHILD: &str = "LMD_NODE_GUARD_TEST_CHILD";
        if let Ok(case) = std::env::var(CHILD) {
            let identity = crate::resource_guard::Identity {
                uid: "reviewed-node".into(),
                version: "42".into(),
            };
            let result = cordon("worker-reviewed", case != "uncordon", Some(&identity));
            assert_eq!(
                result.is_ok(),
                matches!(case.as_str(), "success" | "uncordon")
            );
            if let Err(error) = result {
                assert!(error.to_string().contains("refresh and review again"));
            }
            drop(crate::kube_session::Cleanup);
            return;
        }
        let root = std::env::temp_dir().join(format!("lmd-node-guard-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let fake = root.join("kubectl");
        std::fs::write(&fake, r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
if args[:2] == ['config', 'view']:
    print(json.dumps({'current-context':'test','contexts':[{'name':'test','context':{'cluster':'test'}}], 'clusters':[{'name':'test','cluster':{'server':'https://invalid.test'}}]}))
    sys.exit(0)
payload = json.load(sys.stdin)
with open(os.environ['LMD_NODE_GUARD_TRACE'], 'a') as output:
    output.write(json.dumps({'args':args,'patch':payload})+'\n')
dry = '--dry-run=server' in args
case = os.environ['LMD_NODE_GUARD_TEST_CHILD']
if (case == 'dry-conflict' and dry) or (case == 'apply-conflict' and not dry):
    print('JSON patch test failed: resourceVersion changed', file=sys.stderr)
    sys.exit(1)
print('node/worker-reviewed patched')
"#).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        for case in ["success", "uncordon", "dry-conflict", "apply-conflict"] {
            let trace = root.join(format!("{case}.jsonl"));
            let _ = std::fs::remove_file(&trace);
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "kube::node_command_tests::node_conflict_command_trace",
                    "--nocapture",
                ])
                .env(CHILD, case)
                .env("LMD_NODE_GUARD_TRACE", &trace)
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        root.display(),
                        std::env::var("PATH").unwrap_or_default()
                    ),
                )
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{case}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let calls: Vec<serde_json::Value> = std::fs::read_to_string(trace)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(calls.len(), if case == "dry-conflict" { 1 } else { 2 });
            for (index, call) in calls.iter().enumerate() {
                let args = call["args"].as_array().unwrap();
                assert!(args.iter().any(|a| a == "--kubeconfig"));
                assert!(args
                    .windows(3)
                    .any(|a| a == ["patch", "node", "worker-reviewed"]));
                assert_eq!(args.iter().any(|a| a == "--dry-run=server"), index == 0);
                assert_eq!(
                    call["patch"],
                    crate::resource_guard::cordon(
                        Some(&crate::resource_guard::Identity {
                            uid: "reviewed-node".into(),
                            version: "42".into()
                        }),
                        case != "uncordon"
                    )
                    .unwrap()
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, unix))]
mod delete_command_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn delete_conflict_command_trace() {
        const CHILD: &str = "LMD_DELETE_GUARD_TEST_CHILD";
        if let Ok(case) = std::env::var(CHILD) {
            let identity = crate::resource_guard::Identity {
                uid: "reviewed-object".into(),
                version: "42".into(),
            };
            let result = if case == "job" {
                delete_job("test-ns", "compile-reviewed", Some(&identity))
            } else {
                delete_pod("test-ns", "pod-reviewed", Some(&identity))
            };
            assert_eq!(result.is_ok(), matches!(case.as_str(), "success" | "job"));
            if let Err(error) = result {
                assert!(error.to_string().contains("refresh and review again"));
            }
            drop(crate::kube_session::Cleanup);
            return;
        }
        let root =
            std::env::temp_dir().join(format!("lmd-delete-guard-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let fake = root.join("kubectl");
        std::fs::write(&fake, r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
if args[:2] == ['config', 'view']:
    print(json.dumps({'current-context':'test','contexts':[{'name':'test','context':{'cluster':'test'}}], 'clusters':[{'name':'test','cluster':{'server':'https://invalid.test'}}]}))
    sys.exit(0)
payload = json.load(sys.stdin)
with open(os.environ['LMD_DELETE_GUARD_TRACE'], 'a') as output:
    output.write(json.dumps({'args':args,'patch':payload})+'\n')
dry = payload.get('dryRun') == ['All']
assert payload['preconditions'] == {'uid':'reviewed-object','resourceVersion':'42'}
assert payload['propagationPolicy'] == 'Background'
case = os.environ['LMD_DELETE_GUARD_TEST_CHILD']
if (case == 'dry-conflict' and dry) or (case == 'apply-conflict' and not dry):
    print('Conflict: preconditions failed', file=sys.stderr)
    sys.exit(1)
print('node/worker-reviewed patched')
"#).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        for case in ["success", "job", "dry-conflict", "apply-conflict"] {
            let trace = root.join(format!("{case}.jsonl"));
            let _ = std::fs::remove_file(&trace);
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "kube::delete_command_tests::delete_conflict_command_trace",
                    "--nocapture",
                ])
                .env(CHILD, case)
                .env("LMD_DELETE_GUARD_TRACE", &trace)
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        root.display(),
                        std::env::var("PATH").unwrap_or_default()
                    ),
                )
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{case}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let calls: Vec<serde_json::Value> = std::fs::read_to_string(trace)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(calls.len(), if case == "dry-conflict" { 1 } else { 2 });
            for (index, call) in calls.iter().enumerate() {
                let args = call["args"].as_array().unwrap();
                assert!(args.iter().any(|a| a == "--kubeconfig"));
                assert!(args.windows(2).any(|a| a == ["delete", "--raw"]));
                let (kind, name) = if case == "job" {
                    ("jobs", "compile-reviewed")
                } else {
                    ("pods", "pod-reviewed")
                };
                let (uri, mut expected) = crate::resource_guard::delete_request(
                    "test-ns",
                    kind,
                    name,
                    Some(&crate::resource_guard::Identity {
                        uid: "reviewed-object".into(),
                        version: "42".into(),
                    }),
                )
                .unwrap();
                assert!(args.iter().any(|a| a == &uri));
                assert!(args.windows(2).any(|a| a == ["-f", "-"]));
                if index == 0 {
                    expected["dryRun"] = serde_json::json!(["All"]);
                }
                assert_eq!(call["patch"], expected);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, unix))]
mod manifest_command_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn manifest_conflicts_never_delete_rebase_or_force() {
        const CHILD: &str = "LMD_MANIFEST_CREATE_CHILD";
        if let Ok(case) = std::env::var(CHILD) {
            let yaml = "apiVersion: v1\nkind: ConfigMap\nmetadata: {name: script}\n---\napiVersion: batch/v1\nkind: Job\nmetadata: {name: build}\n";
            let result = apply_manifest("test-ns", yaml, case == "validate");
            assert_eq!(
                result.is_ok(),
                matches!(case.as_str(), "success" | "validate" | "cm-existing")
            );
            if let Err(error) = result {
                let expected = match case.as_str() {
                    "race" => "1 earlier resource(s) applied",
                    "cm-merge-conflict" => "resource changed since confirmation",
                    _ => "0 earlier resource(s) applied",
                };
                assert!(error.to_string().contains(expected), "{error}");
            }
            drop(crate::kube_session::Cleanup);
            return;
        }
        let directory = crate::local_file::EditFile::new("").unwrap();
        let root = directory.path().parent().unwrap();
        let fake = root.join("kubectl");
        std::fs::write(&fake, r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
if args[:2] == ['config', 'view']:
    print(json.dumps({'current-context':'test','contexts':[{'name':'test','context':{'cluster':'test'}}], 'clusters':[{'name':'test','cluster':{'server':'https://invalid.test'}}]}))
    sys.exit(0)
assert args[0] == '--kubeconfig'
args = args[2:]
body = sys.stdin.read()
with open(os.environ['LMD_MANIFEST_CREATE_TRACE'], 'a') as output:
    output.write(json.dumps({'args':args,'body':body})+'\n')
verb = args[0]
case = os.environ['LMD_MANIFEST_CREATE_CHILD']
current = {'apiVersion':'v1','kind':'ConfigMap','metadata':{'name':'script','namespace':'test-ns','uid':'original','resourceVersion':'42'},'data':{'keep':'unmentioned'}}
exists = case.startswith('cm-') and case != 'cm-new-race'
assert args[1:3] == ['-n', 'test-ns']
if verb == 'get':
    if exists: print(json.dumps(current))
    sys.exit(0)
dry = '--dry-run=server' in args
if verb == 'apply':
    assert dry and exists
    if case == 'cm-merge-conflict': current['metadata']['resourceVersion'] = '43'
    print(json.dumps(current))
    sys.exit(0)
job = 'kind: Job' in body
assert verb == ('replace' if exists and not job else 'create')
if verb == 'replace':
    assert json.loads(body) == current
fail = (job and (case == 'existing' or (case == 'race' and not dry))) or (not job and ((case == 'cm-dry-conflict' and dry) or (case in ('cm-conflict','cm-new-race') and not dry)))
if fail:
    print('Conflict / AlreadyExists: object changed', file=sys.stderr)
    sys.exit(1)
print('job/build created' if job else 'configmap/script configured')
"#).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        for case in [
            "success",
            "validate",
            "existing",
            "race",
            "cm-existing",
            "cm-merge-conflict",
            "cm-dry-conflict",
            "cm-conflict",
            "cm-new-race",
        ] {
            let trace = root.join(format!("{case}.jsonl"));
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "kube::manifest_command_tests::manifest_conflicts_never_delete_rebase_or_force",
                    "--nocapture",
                ])
                .env(CHILD, case)
                .env("LMD_MANIFEST_CREATE_TRACE", &trace)
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        root.display(),
                        std::env::var("PATH").unwrap_or_default()
                    ),
                )
                .output()
                .unwrap();
            assert!(
                child.status.success(),
                "{case}: {} {}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
            let calls: Vec<serde_json::Value> = std::fs::read_to_string(trace)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let expected: Vec<&str> = match case {
                "success" | "race" => vec!["get", "create", "create", "create", "create"],
                "existing" | "validate" => vec!["get", "create", "create"],
                "cm-existing" => vec!["get", "apply", "replace", "create", "replace", "create"],
                "cm-merge-conflict" => vec!["get", "apply"],
                "cm-dry-conflict" => vec!["get", "apply", "replace"],
                "cm-conflict" => vec!["get", "apply", "replace", "create", "replace"],
                "cm-new-race" => vec!["get", "create", "create", "create"],
                _ => unreachable!(),
            };
            assert_eq!(
                calls
                    .iter()
                    .map(|c| c["args"][0].as_str().unwrap())
                    .collect::<Vec<_>>(),
                expected,
                "{case}"
            );
            let mut real_seen = false;
            for call in &calls {
                let args = call["args"].as_array().unwrap();
                assert!(!args.iter().any(|arg| arg == "--force"));
                if args[0] == "get" {
                    continue;
                }
                let dry = args.iter().any(|arg| arg == "--dry-run=server");
                assert!(!real_seen || !dry, "validation must finish before mutation");
                real_seen |= !dry;
                if args[0] == "apply" {
                    assert!(dry);
                }
            }
        }
    }
}

#[cfg(all(test, unix))]
mod route_command_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn reviewed_route_identity_is_enforced_before_edit_and_on_replace() {
        const CHILD: &str = "LMD_ROUTE_REVIEW_CHILD";
        if let Ok(case) = std::env::var(CHILD) {
            let id = crate::resource_guard::Identity {
                uid: "original".into(),
                version: "42".into(),
            };
            let result = match case.as_str() {
                "delete" => route_delete_rule("ns", "route", "/a", Some(&id)),
                "retarget" => route_retarget("ns", "route", "/a", "new", "Service", Some(&id)),
                _ => route_set_path("ns", "route", "/a", "/b", Some(&id)),
            };
            assert_eq!(
                result.is_ok(),
                matches!(case.as_str(), "rename" | "delete" | "retarget")
            );
            drop(crate::kube_session::Cleanup);
            return;
        }
        assert!(route_set_path("ns", "route", "/a", "/b", None).is_err());
        assert!(route_delete_rule("ns", "route", "/a", None).is_err());
        assert!(route_retarget("ns", "route", "/a", "new", "Service", None).is_err());
        let session = crate::local_file::EditFile::new("").unwrap();
        let root = session.path().parent().unwrap();
        let fake = root.join("kubectl");
        std::fs::write(&fake, r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
if args[:2] == ['config', 'view']:
    print(json.dumps({'current-context':'test','contexts':[{'name':'test','context':{'cluster':'test'}}], 'clusters':[{'name':'test','cluster':{'server':'https://invalid.test'}}]}))
    sys.exit(0)
assert args[0] == '--kubeconfig'
args = args[2:]
body = json.load(sys.stdin) if args[0] == 'replace' else None
with open(os.environ['LMD_ROUTE_REVIEW_TRACE'], 'a') as out:
    out.write(json.dumps({'args':args,'body':body})+'\n')
case = os.environ['LMD_ROUTE_REVIEW_CHILD']
if args[0] == 'get':
    if 'service' in args:
        print(json.dumps({'spec':{'ports':[{'port':8080,'protocol':'TCP'}]}}))
    else:
        print(json.dumps({'apiVersion':'gateway.networking.k8s.io/v1','kind':'HTTPRoute','metadata':{'name':'route','uid':'replacement' if case=='recreated' else 'original','resourceVersion':'43' if case=='changed' else '42'},'spec':{'hostnames':['keep.example'],'rules':[{'matches':[{'path':{'type':'PathPrefix','value':'/a'}}],'backendRefs':[{'name':'old','kind':'Service','port':8080}]}]}}))
    sys.exit(0)
assert args[0] == 'replace' and '--force' not in args
assert body['metadata']['uid'] == 'original' and body['metadata']['resourceVersion'] == '42'
assert body['spec']['hostnames'] == ['keep.example']
if case == 'delete': assert body['spec']['rules'] == []
elif case == 'retarget': assert body['spec']['rules'][0]['backendRefs'][0]['name'] == 'new'
else: assert body['spec']['rules'][0]['matches'][0]['path']['value'] == '/b'
dry = '--dry-run=server' in args
if (case == 'dry-conflict' and dry) or (case == 'write-conflict' and not dry):
    print('Conflict', file=sys.stderr)
    sys.exit(1)
print('httproute/route replaced')
"#).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        for case in [
            "rename",
            "delete",
            "retarget",
            "recreated",
            "changed",
            "dry-conflict",
            "write-conflict",
        ] {
            let trace = root.join(format!("{case}.jsonl"));
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "kube::route_command_tests::reviewed_route_identity_is_enforced_before_edit_and_on_replace", "--nocapture"])
                .env(CHILD, case).env("LMD_ROUTE_REVIEW_TRACE", &trace)
                .env("PATH", format!("{}:{}", root.display(), std::env::var("PATH").unwrap_or_default()))
                .output().unwrap();
            assert!(
                child.status.success(),
                "{case}: {} {}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
            let calls: Vec<serde_json::Value> = std::fs::read_to_string(trace)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let expected = match case {
                "recreated" | "changed" => 1,
                "dry-conflict" => 2,
                "retarget" => 4,
                _ => 3,
            };
            assert_eq!(calls.len(), expected, "{case}");
            let replacements: Vec<_> = calls.iter().filter(|c| c["args"][0] == "replace").collect();
            if let Some(first) = replacements.first() {
                assert!(first["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a == "--dry-run=server"));
            }
            if replacements.len() == 2 {
                assert_eq!(replacements[0]["body"], replacements[1]["body"]);
                assert!(!replacements[1]["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a == "--dry-run=server"));
            }
        }
    }
}
