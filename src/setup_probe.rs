//! Read-only platform prerequisites, independent from model/metric collection.
use crate::{collect::now_secs, kube};

/// 플랫폼 부트스트랩 전제조건 프로브 — 새 llm-d 환경 셋업 시 "무엇이 이미 있고 무엇이 없나".
/// Setup 뷰(Doctor)가 이걸 읽어 present/missing + 가이드된 조치를 렌더한다. 전부 read-only kubectl.
#[derive(Clone, Default)]
pub struct SetupProbe {
    pub checked_at: Option<u64>,
    pub unknown: std::collections::BTreeSet<&'static str>,
    pub probed: bool,    // namespace query completed; false means access is unknown
    pub ns_exists: bool, // 대상 namespace 존재
    pub crd_gateway: bool, // gateways.gateway.networking.k8s.io CRD
    pub crd_httproute: bool, // httproutes.gateway.networking.k8s.io CRD
    pub crd_infpool: bool, // inferencepools.inference.networking.k8s.io CRD (GIE GA)
    pub gatewayclasses: Vec<String>, // 사용 가능한 GatewayClass 이름들(Gateway 생성 시 선택)
    pub gateway: bool,   // llm-d-gateway (대상 ns)
    pub gateway_class: Option<String>, // configured class of this Gateway, not installed classes
    pub epp_role_sa: bool, // 공유 Role llmd-router-epp-sa (helm 생성)
    pub epp_role_non_sa: bool, // 공유 Role llmd-router-epp-non-sa
    pub secret_hf: bool, // hf-token secret (gated 모델 가중치 다운로드용)
    pub pvc_model_store: Option<String>, // model-store PVC phase(Bound/Pending) 또는 None(없음)
}

/// 플랫폼 전제조건 프로브 — read-only kubectl 몇 번으로 CRD/Gateway/Role/PVC/Secret 존재 확인.
/// Failed namespace access leaves probed=false; absence is only a successful empty query.
fn observed(probe: &mut SetupProbe, row: &'static str, result: Option<bool>) -> bool {
    match result {
        Some(exists) => exists,
        None => {
            probe.unknown.insert(row);
            false
        }
    }
}

pub async fn collect(ns: &str) -> SetupProbe {
    let mut p = SetupProbe::default();
    let ns_ref = format!("namespace/{}", ns);
    // A failed namespace query does not establish namespace absence or connectivity.
    match kube::get_exists(&["get", &ns_ref]).await {
        Some(exists) => {
            p.probed = true;
            p.ns_exists = exists;
        }
        None => return p,
    }
    // These seven reads have no dependencies on each other. Bound concurrency
    // to this fixed group; a slow/failed item must not serialize every probe.
    let (gateway_crd, route_crd, pool_crd, role_sa, role_non_sa, secret, pvc) = tokio::join!(
        async { kube::get_exists(&["get", "crd", "gateways.gateway.networking.k8s.io"]).await },
        async { kube::get_exists(&["get", "crd", "httproutes.gateway.networking.k8s.io"]).await },
        async {
            kube::get_exists(&["get", "crd", "inferencepools.inference.networking.k8s.io"]).await
        },
        async { kube::get_exists(&["get", "role", "llmd-router-epp-sa", "-n", ns]).await },
        async { kube::get_exists(&["get", "role", "llmd-router-epp-non-sa", "-n", ns]).await },
        async { kube::get_exists(&["get", "secret", "hf-token", "-n", ns]).await },
        async {
            kube::get_jsonpath(
                &["get", "pvc", "model-store", "-n", ns, "--ignore-not-found"],
                "{.status.phase}",
            )
            .await
        },
    );
    p.crd_gateway = observed(&mut p, "Gateway API", gateway_crd);
    p.crd_httproute = observed(&mut p, "Gateway API", route_crd);
    p.crd_infpool = observed(&mut p, "Inference Extension", pool_crd);
    p.epp_role_sa = observed(&mut p, "EPP shared Roles", role_sa);
    p.epp_role_non_sa = observed(&mut p, "EPP shared Roles", role_non_sa);
    p.secret_hf = observed(&mut p, "hf-token", secret);
    match pvc {
        Some(phase) => p.pvc_model_store = (!phase.is_empty()).then_some(phase),
        None => {
            p.unknown.insert("model-store PVC");
        }
    }
    if p.crd_gateway {
        let (classes, gateway) = tokio::join!(
            async {
                kube::get_jsonpath(&["get", "gatewayclass"], "{.items[*].metadata.name}").await
            },
            async {
                kube::get_jsonpath(
                    &[
                        "get",
                        "gateway",
                        "llm-d-gateway",
                        "-n",
                        ns,
                        "--ignore-not-found",
                    ],
                    "{.metadata.name}{\"\\t\"}{.spec.gatewayClassName}",
                )
                .await
            },
        );
        if let Some(classes) = classes {
            p.gatewayclasses = classes.split_whitespace().map(str::to_owned).collect();
        } else {
            p.unknown.insert("llm-d-gateway");
        }
        match gateway {
            Some(value) => {
                let mut fields = value.split_whitespace();
                p.gateway = fields.next().is_some();
                p.gateway_class = fields.next().map(str::to_owned);
            }
            None => {
                p.unknown.insert("llm-d-gateway");
            }
        }
    }
    if p.unknown.contains("Gateway API") {
        p.unknown.insert("llm-d-gateway");
    }

    p.checked_at = Some(now_secs());
    p
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn independent_queries_overlap_and_keep_individual_failure_states() {
        const CHILD: &str = "LMD_SETUP_PARALLEL_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let result = runtime.block_on(collect("ns"));
            assert!(result.probed && result.ns_exists && result.crd_gateway && result.crd_infpool);
            assert!(!result.crd_httproute); // successful empty is observed absence
            assert!(!result.epp_role_sa && result.epp_role_non_sa && result.secret_hf);
            assert_eq!(result.unknown, ["EPP shared Roles"].into());
            assert_eq!(result.pvc_model_store.as_deref(), Some("Bound"));
            assert_eq!(result.gatewayclasses, ["test-class"]);
            assert!(result.gateway && result.checked_at.is_some());
            assert_eq!(result.gateway_class.as_deref(), Some("selected-class"));
            drop(crate::kube_session::Cleanup);
            return;
        }
        let session = crate::local_file::EditFile::new("").unwrap();
        let root = session.path().parent().unwrap();
        let fake = root.join("kubectl");
        std::fs::write(&fake, r#"#!/usr/bin/env python3
import json, os, pathlib, sys, time
args = sys.argv[1:]
if args[:2] == ['config', 'view']:
    print(json.dumps({'current-context':'test','contexts':[{'name':'test','context':{'cluster':'test'}}], 'clusters':[{'name':'test','cluster':{'server':'https://invalid.test'}}]}))
    sys.exit(0)
assert args[0] == '--kubeconfig'
args = args[2:]
assert args[0] == 'get'
if args[1].startswith('namespace/'):
    print('namespace/ns')
    sys.exit(0)
root = pathlib.Path(os.environ['LMD_SETUP_PARALLEL_ROOT'])
if args[1] in ['crd', 'role', 'secret', 'pvc']:
    (root / ('probe-'+args[1]+'-'+args[2])).touch()
    deadline = time.monotonic()+5
    while len(list(root.glob('probe-*'))) != 7:
        if time.monotonic() > deadline:
            print('independent queries did not overlap', file=sys.stderr)
            sys.exit(1)
        time.sleep(.02)
if 'llmd-router-epp-sa' in args:
    print('Forbidden', file=sys.stderr)
    sys.exit(1)
if 'httproutes.gateway.networking.k8s.io' in args:
    sys.exit(0)
if args[1]=='gateway':
    assert any('{.spec.gatewayClassName}' in arg for arg in args)
    print('llm-d-gateway\tselected-class')
    sys.exit(0)
print('Bound' if args[1]=='pvc' else 'test-class' if args[1]=='gatewayclass' else 'present')
"#).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "setup_probe::tests::independent_queries_overlap_and_keep_individual_failure_states", "--nocapture"])
            .env(CHILD, "1").env("LMD_SETUP_PARALLEL_ROOT", root)
            .env("PATH", format!("{}:{}", root.display(), std::env::var("PATH").unwrap_or_default()))
            .output().unwrap();
        assert!(
            child.status.success(),
            "{} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
    }
}
