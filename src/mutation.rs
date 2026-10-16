//! Authorized execution of confirmed operations, separate from UI dispatch.
use crate::app::{Mode, Pending};
use crate::kube;

/// Mutation execution result — kube is called on a worker thread, then this is passed to the main thread for audit/UI.
/// Moving kube calls off the UI thread keeps rendering from freezing (removes up-to-8s freezes).
pub(crate) struct MutationOutcome {
    pub(crate) audit_scope: crate::audit::Scope,
    pub(crate) audit_attempted: bool,
    pub(crate) audit_error: Option<String>,
    pub(crate) mode: Mode,
    pub(crate) audit_action: String,     // audit log action field
    pub(crate) audit_target: String,     // audit log target field
    pub(crate) fail_label: &'static str, // window title on failure (show_fail)
    pub(crate) result: Result<OkInfo, String>, // Ok=success display info / Err=failure reason (1+ lines)
}
/// Display info on success — audit detail, toast text, whether to close preview (Apply success).
pub(crate) struct OkInfo {
    pub(crate) audit_detail: String,
    pub(crate) notify: String,
    pub(crate) clear_preview: bool,
    /// A (title, body) to open in the scrollable window on success.
    ///
    /// The failure path has always done this; a read-only probe needs it too, since its whole
    /// output is the answer and a toast would truncate it.
    pub(crate) window: Option<(String, String)>,
    /// `(deployment, replicas)` to write into the local snapshot on success.
    ///
    /// The replica toggle reads `desired` from the snapshot, which is up to a full tick (~3s)
    /// old. Without this, pressing `s` twice in quick succession computes the second toggle
    /// from the pre-scale value and scales to 0 again — so the second press looks like it did
    /// nothing. Found by ACT-01. `ready` is deliberately left alone: the row should keep
    /// showing that it has not converged yet.
    pub(crate) desired_now: Option<(String, i64)>,
}

impl MutationOutcome {
    pub(crate) fn record_once(&mut self) {
        if self.audit_attempted {
            return;
        }
        self.audit_attempted = true;
        let result = self
            .result
            .as_ref()
            .map(|ok| ok.audit_detail.as_str())
            .map_err(String::as_str);
        self.audit_error = crate::audit::record(
            self.mode,
            &self.audit_action,
            &self.audit_target,
            result,
            &self.audit_scope,
        )
        .err()
        .map(|e| e.to_string());
    }
}

pub(crate) fn run_mutation(pending: Pending, ns: &str, mode: Mode) -> MutationOutcome {
    let started = std::time::Instant::now();
    let identity = pending.reviewed_identity().cloned();
    let manifest_targets = match &pending {
        Pending::Apply {
            reviewed: Some(plan),
            ..
        } => plan.targets(),
        _ => Vec::new(),
    };
    let namespace = if matches!(pending, Pending::Cordon { .. }) {
        ""
    } else {
        ns
    };
    let mut outcome = execute(pending, ns, mode);
    outcome.audit_scope = crate::audit::Scope {
        context: crate::kube_session::context()
            .unwrap_or("unavailable")
            .into(),
        namespace: namespace.into(),
        uid: identity.as_ref().map(|i| i.uid.clone()).unwrap_or_default(),
        resource_version: identity.map(|i| i.version).unwrap_or_default(),
        elapsed_ms: started.elapsed().as_millis(),
        manifest_targets,
    };
    outcome.record_once();
    outcome
}

/// Execute a confirmed mutation via actual kubectl and return a MutationOutcome. (for worker threads)
/// Authorization is checked before any Kubernetes operation, including probes that create Jobs.
fn execute(pending: Pending, ns: &str, mode: Mode) -> MutationOutcome {
    if let Err(reason) = pending.authorize(mode) {
        return MutationOutcome {
            audit_scope: Default::default(),
            audit_attempted: false,
            audit_error: None,
            mode,
            audit_action: "denied".into(),
            audit_target: pending.prompt(),
            fail_label: "permission",
            result: Err(reason),
        };
    }
    let mk = |audit_action: String,
              audit_target: String,
              fail_label: &'static str,
              result: Result<OkInfo, String>| MutationOutcome {
        audit_scope: Default::default(),
        audit_attempted: false,
        audit_error: None,
        mode,
        audit_action,
        audit_target,
        fail_label,
        result,
    };
    match pending {
        Pending::Scale {
            name,
            target,
            expected,
        } => {
            let applied = (name.clone(), target);
            let r = kube::scale_deploy(ns, &name, target, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "scaled".into(),
                    notify: format!("scaled {} → {}", name, target),
                    clear_preview: false,
                    window: None,
                    desired_now: Some(applied),
                })
                .map_err(|e| e.to_string());
            mk(format!("scale→{}", target), name, "scale", r)
        }
        Pending::Restart { name, expected } => {
            let r = kube::rollout_restart(ns, &name, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "restarted".into(),
                    notify: format!("rollout restart {}", name),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk("rollout-restart".into(), name, "restart", r)
        }
        Pending::Rollback { name, expected } => {
            let r = kube::rollout_undo(ns, &name, expected.as_ref())
                .map(|detail| OkInfo {
                    audit_detail: detail.clone(),
                    notify: format!("rollout undo {} — {}", name, detail),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk("rollout-undo".into(), name, "rollback", r)
        }
        Pending::Drain { pod } => {
            let r = kube::drain_pod(ns, &pod)
                .map(|detail| OkInfo {
                    audit_detail: format!("drained {}", detail),
                    notify: format!(
                        "routing labels excluded for {} {} — active streams not verified",
                        pod, detail
                    ),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk("drain".into(), pod, "drain", r)
        }
        Pending::Stop { name, expected } => {
            let stopped = (name.clone(), 0);
            let r = kube::scale_deploy(ns, &name, 0, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "stopped".into(),
                    notify: format!("stopped {} (scaled → 0)", name),
                    clear_preview: false,
                    window: None,
                    desired_now: Some(stopped),
                })
                .map_err(|e| e.to_string());
            mk("stop(scale→0)".into(), name, "stop", r)
        }
        Pending::SetSecret {
            name,
            key,
            value,
            purpose,
        } => {
            let r = kube::apply_secret(ns, &name, &key, &value)
                .map(|o| OkInfo {
                    // Neither the audit detail nor the toast may contain the value.
                    audit_detail: o.lines().next().unwrap_or("applied").to_string(),
                    notify: format!("{} stored in secret {}/{}", purpose, name, key),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk(
                "set-secret".into(),
                format!("{}/{}", name, key),
                "set secret",
                r,
            )
        }
        Pending::Cordon { node, on, expected } => {
            let act = if on { "cordon" } else { "uncordon" };
            let r = kube::cordon(&node, on, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "ok".into(),
                    notify: format!("{} {}", if on { "cordoned" } else { "uncordoned" }, node),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk(act.into(), node, "cordon", r)
        }
        Pending::DeletePod { name, expected } => {
            let r = kube::delete_pod(ns, &name, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "deletion requested".into(),
                    notify: format!("deletion requested for pod {}", name),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk("delete-pod".into(), name, "delete", r)
        }
        Pending::Provenance {
            deployment,
            node,
            path,
        } => {
            let (m, job) = crate::probe::provenance_manifest(ns, &deployment, &node, &path);
            let r = kube::run_job_for_output(ns, &m.to_yaml(), &job, 90)
                .map(|body| OkInfo {
                    audit_detail: format!("read {}", path),
                    notify: format!("provenance of {} — see window", deployment),
                    clear_preview: false,
                    window: Some((
                        format!("provenance · {} @ {} (q to close)", deployment, node),
                        body,
                    )),
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk("provenance".into(), deployment, "provenance", r)
        }
        Pending::StoreRefresh => {
            let r = kube::refresh_inventory(ns)
                .map(|job| OkInfo {
                    audit_detail: job.clone(),
                    notify: format!("store re-scan started ({})", job),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk(
                "store-refresh".into(),
                "model-inventory".into(),
                "store refresh",
                r,
            )
        }
        Pending::DeleteJob { name, expected } => {
            let r = kube::delete_job(ns, &name, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "deletion requested".into(),
                    notify: format!("deletion requested for job {}", name),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk("delete-job".into(), name, "delete job", r)
        }
        Pending::RouteRename {
            route,
            old,
            new,
            expected,
        } => {
            let r = kube::route_set_path(ns, &route, &old, &new, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "renamed".into(),
                    notify: format!("renamed route {} → {}", old, new),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk(
                format!("route-rename→{}", new),
                format!("{}:{}", route, old),
                "route rename",
                r,
            )
        }
        Pending::RouteRetarget {
            expected,
            route,
            path,
            backend,
            kind,
        } => {
            let r = kube::route_retarget(ns, &route, &path, &backend, &kind, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "retargeted".into(),
                    notify: format!("retargeted {} → {}", path, backend),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk(
                format!("route-retarget→{}:{}", kind, backend),
                format!("{}:{}", route, path),
                "route retarget",
                r,
            )
        }
        Pending::RouteDelete {
            route,
            path,
            expected,
        } => {
            let r = kube::route_delete_rule(ns, &route, &path, expected.as_ref())
                .map(|_| OkInfo {
                    audit_detail: "deleted".into(),
                    notify: format!("deleted route {}", path),
                    clear_preview: false,
                    window: None,
                    desired_now: None,
                })
                .map_err(|e| e.to_string());
            mk(
                "route-delete".into(),
                format!("{}:{}", route, path),
                "route delete",
                r,
            )
        }
        Pending::Apply {
            yaml,
            title,
            reviewed,
            ..
        } => {
            let r = reviewed
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!("manifest review not prepared; refresh and review again")
                })
                .and_then(|reviewed| kube::apply_reviewed_manifest(ns, &yaml, reviewed, false))
                .map(|o| {
                    let line = o.lines().next().unwrap_or("ok").to_string();
                    OkInfo {
                        audit_detail: line.clone(),
                        notify: format!("applied — {}", line),
                        clear_preview: true,
                        window: None,
                        desired_now: None,
                    }
                })
                .map_err(|e| e.to_string());
            mk("apply".into(), title, "apply", r)
        }
        Pending::ApplyUrl { url, title } => {
            let r = kube::apply_url(&url)
                .map(|o| {
                    let n = o.lines().count();
                    OkInfo {
                        audit_detail: format!("{} ({} objects)", url, n),
                        notify: format!("applied {} — {} object(s)", title, n),
                        clear_preview: true,
                        window: None,
                        desired_now: None,
                    }
                })
                .map_err(|e| e.to_string());
            mk("apply-url".into(), title, "apply", r)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn conditional_update_worker_child() {
        let Ok(scenario) = std::env::var("LMD_TEST_MUTATION_SCENARIO") else {
            return;
        };
        let _cleanup = crate::kube_session::Cleanup;
        let expected = Some(crate::resource_guard::Identity {
            uid: "original".into(),
            version: "12".into(),
        });
        let pending = if scenario == "restart" {
            Pending::Restart {
                name: "model".into(),
                expected,
            }
        } else if scenario.starts_with("rollback") {
            Pending::Rollback {
                name: "model".into(),
                expected,
            }
        } else {
            Pending::Scale {
                name: "model".into(),
                target: 0,
                expected,
            }
        };
        let outcome = run_mutation(pending, "serving", Mode::Admin);
        assert_eq!(
            outcome.result.is_ok(),
            matches!(scenario.as_str(), "scale" | "restart" | "rollback")
        );
    }
    #[test]
    fn conditional_updates_use_server_preconditions_and_do_not_retry_conflicts() {
        use std::os::unix::fs::PermissionsExt;
        for scenario in [
            "scale",
            "restart",
            "rollback",
            "recreated",
            "changed_after_dry_run",
            "rollback_changed_after_dry_run",
        ] {
            let dir = std::env::temp_dir()
                .join(format!("lmd-guard-test-{}-{scenario}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let object = json!({"metadata":{"uid":if scenario=="recreated" {"replacement"} else {"original"},"resourceVersion":"12","annotations":{"deployment.kubernetes.io/revision":"3"}},"spec":{"replicas":2,"template":{"metadata":{"labels":{"app":"model"},"annotations":{"keep":"yes"}},"spec":{"containers":[{"name":"server","image":"new"}]}}}});
            std::fs::write(
                dir.join("object.json"),
                serde_json::to_vec(&object).unwrap(),
            )
            .unwrap();
            let executable = dir.join("kubectl");
            std::fs::write(&executable,r#"#!/usr/bin/python3
import json,sys,os
from pathlib import Path
root=Path(os.environ['LMD_TEST_MUTATION_ROOT'])
args=sys.argv[1:]
if args[:2]==['config','view']:
    print(json.dumps({'current-context':'test','contexts':[{'name':'test','context':{'cluster':'test'}}], 'clusters':[{'name':'test','cluster':{'server':'https://test.invalid'}}]}))
    sys.exit(0)
assert args[0]=='--kubeconfig'
args=args[2:]
obj=json.loads((root/'object.json').read_text())
if args[0]=='get':
    if args[1]=='replicasets':
        import copy
        old=copy.deepcopy(obj)
        old['metadata']={'name':'previous-rs','annotations':{'deployment.kubernetes.io/revision':'2'},'ownerReferences':[{'kind':'Deployment','uid':'original','controller':True}]}
        old['spec']['template']['spec']['containers'][0]['image']='old'
        old['spec']['template']['metadata']['labels']['pod-template-hash']='generated'
        print(json.dumps({'items':[old]}));sys.exit(0)
    print(json.dumps(obj));sys.exit(0)
assert args[:3]==['patch','deployment','model'],args
assert '--type=json' in args and '--patch-file=/dev/stdin' in args
patch=json.load(sys.stdin)
with (root/'trace').open('a') as f: f.write(('dry-run' if '--dry-run=server' in args else 'apply')+'\n')
for op in patch[:2]:
    assert op['op']=='test'
    actual=obj
    for key in op['path'].strip('/').split('/'): actual=actual[key]
    if actual!=op['value']:
        print('resource precondition failed',file=sys.stderr);sys.exit(1)
if '--dry-run=server' in args:
    if os.environ['LMD_TEST_MUTATION_SCENARIO'].endswith('changed_after_dry_run'):
        obj['metadata']['resourceVersion']='13'
        (root/'object.json').write_text(json.dumps(obj))
else:
    for op in patch[2:]:
        assert op['op']=='add'
        keys=op['path'].strip('/').split('/')
        parent=obj
        for key in keys[:-1]: parent=parent[key]
        parent[keys[-1]]=op['value']
    obj['metadata']['resourceVersion']='13'
    (root/'object.json').write_text(json.dumps(obj))
print('deployment.apps/model patched')
"#).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "mutation::tests::conditional_update_worker_child",
                    "--nocapture",
                ])
                .env("PATH", &dir)
                .env("LMD_AUDIT", dir.join("audit.log"))
                .env("LMD_TEST_MUTATION_ROOT", &dir)
                .env("LMD_TEST_MUTATION_SCENARIO", scenario)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{scenario}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let after: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("object.json")).unwrap()).unwrap();
            let trace = std::fs::read_to_string(dir.join("trace")).unwrap();
            let audit = std::fs::read_to_string(dir.join("audit.log")).unwrap();
            assert_eq!(
                audit.lines().count(),
                1,
                "worker records before UI result handling"
            );
            let fields: Vec<_> = audit.trim_end_matches('\n').split('\t').collect();
            assert_eq!(&fields[6..10], ["test", "serving", "original", "12"]);
            assert_eq!(
                fields[4],
                if matches!(scenario, "scale" | "restart" | "rollback") {
                    "ok"
                } else {
                    "FAIL"
                }
            );

            assert_eq!(
                trace,
                if scenario == "recreated" {
                    "dry-run\n"
                } else {
                    "dry-run\napply\n"
                }
            );
            assert_eq!(
                after["spec"]["replicas"],
                if scenario == "scale" { 0 } else { 2 }
            );
            assert_eq!(
                after["spec"]["template"]["metadata"]["annotations"]["keep"],
                "yes"
            );
            assert_eq!(
                after["spec"]["template"]["metadata"]["annotations"]
                    ["kubectl.kubernetes.io/restartedAt"]
                    .is_string(),
                scenario == "restart"
            );
            assert_eq!(
                after["spec"]["template"]["spec"]["containers"][0]["image"],
                if scenario == "rollback" { "old" } else { "new" }
            );
            if scenario == "rollback" {
                assert!(after["spec"]["template"]["metadata"]["labels"]
                    .get("pod-template-hash")
                    .is_none());
                assert!(audit.contains("revision 2 from ReplicaSet previous-rs"));
            }
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
