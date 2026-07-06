//! Setup(Doctor) 뷰 — 새 llm-d 환경 부트스트랩 전제조건 점검 + 가이드된 조치.
//! lmd-top 은 플랫폼 부트스트래퍼가 아니라 "이미 깔린 플랫폼 위에 모델을 올리는" 도구이므로,
//! 새 클러스터에선 CRD/Gateway/EPP Role/PVC/Secret 이 먼저 있어야 한다. 이 뷰가 무엇이 있고
//! 무엇이 없는지 진단하고, lmd-top 이 "정확히 아는" 오브젝트(ns·Gateway·CRD-URL)만 동의 기반으로
//! apply 하며, 사이트 특화·helm 관리 항목(model-store PVC·EPP Role·secret)은 명령만 안내한다.
//! (선택한 안전 모델: "점검 + 가이드된 apply" — 위험 항목은 절대 자동 합성/적용하지 않는다.)

use super::*;

/// 상류 릴리스 매니페스트 URL — 이 클러스터의 실제 설치 레시피(scripts/02, GIE GA)와 일치.
/// (Gateway API v1.2.0 = Cilium 1.16 호환 standard 채널)
pub const GATEWAY_API_URL: &str =
    "https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.2.0/standard-install.yaml";
/// Gateway API Inference Extension(InferencePool GA v1) CRD + RBAC 번들.
pub const GIE_URL: &str =
    "https://github.com/kubernetes-sigs/gateway-api-inference-extension/releases/download/v1.0.0/manifests.yaml";

/// 전제조건 점검 상태.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckState {
    Ok,      // 충족
    Warn,    // 있으나 주의(예: PVC Pending) 또는 선택적 미충족
    Missing, // 필수인데 없음
}
impl CheckState {
    pub fn glyph(&self) -> &'static str {
        match self {
            CheckState::Ok => "✓",
            CheckState::Warn => "!",
            CheckState::Missing => "✗",
        }
    }
    /// UI 색상 매핑용 심각도(0=ok, 1=warn/info, 2=missing).
    pub fn sev(&self) -> u8 {
        match self {
            CheckState::Ok => 0,
            CheckState::Missing => 2,
            _ => 1,
        }
    }
}

/// 각 점검 항목의 조치 종류.
#[derive(Clone)]
pub enum SetupFix {
    None,                               // 조치 불필요(충족) 또는 정보성
    Apply { title: String, yaml: String }, // lmd-top 생성 매니페스트 → 미리보기 후 a(apply)
    ApplyUrl { title: String, url: String }, // 상류 릴리스 URL → 확인 후 kubectl apply -f <url>
    Command(String),                    // 사이트 특화/helm 관리 → 명령만 안내(직접 실행)
}
impl SetupFix {
    /// Setup 표의 ACTION 열에 보일 짧은 라벨.
    pub fn label(&self) -> &'static str {
        match self {
            SetupFix::None => "—",
            SetupFix::Apply { .. } => "⏎ review→apply",
            SetupFix::ApplyUrl { .. } => "⏎ apply (URL)",
            SetupFix::Command(_) => "⏎ show cmd",
        }
    }
}

/// 한 점검 행.
pub struct SetupCheck {
    pub category: &'static str,
    pub name: String,
    pub state: CheckState,
    pub detail: String,
    pub fix: SetupFix,
}
impl SetupCheck {
    fn new(
        category: &'static str,
        name: &str,
        state: CheckState,
        detail: String,
        fix: SetupFix,
    ) -> Self {
        SetupCheck {
            category,
            name: name.to_string(),
            state,
            detail,
            fix,
        }
    }
}

impl App {
    fn ns_manifest(&self) -> String {
        format!(
            "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {ns}\n",
            ns = self.ns
        )
    }

    /// 사용 가능한 GatewayClass 중 선택 — cilium 우선(이 클러스터 CNI), 없으면 첫 번째.
    fn pick_gateway_class(&self) -> Option<String> {
        let gcs = &self.snap.setup.gatewayclasses;
        gcs.iter()
            .find(|c| c.as_str() == "cilium")
            .or_else(|| gcs.first())
            .cloned()
    }

    fn gateway_manifest(&self, class: &str) -> String {
        format!(
            "apiVersion: gateway.networking.k8s.io/v1\n\
             kind: Gateway\n\
             metadata:\n  name: llm-d-gateway\n  namespace: {ns}\n\
             spec:\n  gatewayClassName: {class}\n\
             \x20 listeners:\n\
             \x20   - name: http\n\
             \x20     port: 80\n\
             \x20     protocol: HTTP\n\
             \x20     allowedRoutes:\n\
             \x20       namespaces:\n\
             \x20         from: Same\n",
            ns = self.ns,
            class = class
        )
    }

    /// 부트스트랩 전제조건 점검 목록(카테고리 순). Setup 뷰가 이걸 렌더하고, ⏎(setup_enter)로 조치.
    pub fn setup_checks(&self) -> Vec<SetupCheck> {
        let s = &self.snap.setup;
        // kubectl 미도달 → 단일 안내 행.
        if !s.probed {
            return vec![SetupCheck::new(
                "Cluster",
                "kubectl access",
                CheckState::Missing,
                "cannot reach the cluster — check your kubeconfig / current-context".into(),
                SetupFix::Command(
                    "kubectl config current-context\nkubectl cluster-info\n# ensure KUBECONFIG points at the target cluster".into(),
                ),
            )];
        }

        let mut v: Vec<SetupCheck> = Vec::new();

        // ── Cluster ──
        v.push(if self.snap.prom_ok {
            SetupCheck::new(
                "Cluster",
                "Prometheus",
                CheckState::Ok,
                "reachable — metrics/perf views live".into(),
                SetupFix::None,
            )
        } else {
            SetupCheck::new(
                "Cluster",
                "Prometheus",
                CheckState::Warn,
                "unreachable — perf/traffic views stay empty (deploy/compile still work)".into(),
                SetupFix::Command(
                    "# point lmd-top at your Prometheus (plain HTTP host:port):\nexport LMD_PROM=<host>:30090   # NodePort, or: kubectl -n monitoring port-forward svc/prometheus 9090".into(),
                ),
            )
        });
        v.push(if s.ns_exists {
            SetupCheck::new(
                "Cluster",
                &format!("namespace/{}", self.ns),
                CheckState::Ok,
                "target namespace exists".into(),
                SetupFix::None,
            )
        } else {
            SetupCheck::new(
                "Cluster",
                "namespace",
                CheckState::Missing,
                format!("namespace '{}' not found", self.ns),
                SetupFix::Apply {
                    title: format!("create namespace/{}", self.ns),
                    yaml: self.ns_manifest(),
                },
            )
        });

        // ── CRDs ──
        let gw_crd = s.crd_gateway && s.crd_httproute;
        v.push(if gw_crd {
            SetupCheck::new(
                "CRDs",
                "Gateway API",
                CheckState::Ok,
                "gateways + httproutes CRDs installed".into(),
                SetupFix::None,
            )
        } else {
            SetupCheck::new(
                "CRDs",
                "Gateway API",
                CheckState::Missing,
                "gateway.networking.k8s.io CRDs absent — Gateway/HTTPRoute won't apply".into(),
                SetupFix::ApplyUrl {
                    title: "Gateway API v1.2.0 (standard channel)".into(),
                    url: GATEWAY_API_URL.into(),
                },
            )
        });
        v.push(if s.crd_infpool {
            SetupCheck::new(
                "CRDs",
                "Inference Extension",
                CheckState::Ok,
                "inferencepools.inference.networking.k8s.io installed".into(),
                SetupFix::None,
            )
        } else {
            SetupCheck::new(
                "CRDs",
                "Inference Extension",
                CheckState::Missing,
                "InferencePool CRD absent — llm-d routing (EPP) can't apply".into(),
                SetupFix::ApplyUrl {
                    title: "Gateway API Inference Extension v1.0.0".into(),
                    url: GIE_URL.into(),
                },
            )
        });

        // ── Gateway ──
        v.push(if s.gateway {
            SetupCheck::new(
                "Gateway",
                "llm-d-gateway",
                CheckState::Ok,
                format!("present in {}", self.ns),
                SetupFix::None,
            )
        } else if !gw_crd {
            SetupCheck::new(
                "Gateway",
                "llm-d-gateway",
                CheckState::Missing,
                "install the Gateway API CRDs first (see CRDs above)".into(),
                SetupFix::None,
            )
        } else if let Some(class) = self.pick_gateway_class() {
            SetupCheck::new(
                "Gateway",
                "llm-d-gateway",
                CheckState::Missing,
                format!("absent — will use gatewayClassName '{}'", class),
                SetupFix::Apply {
                    title: format!("create Gateway llm-d-gateway (class {})", class),
                    yaml: self.gateway_manifest(&class),
                },
            )
        } else {
            SetupCheck::new(
                "Gateway",
                "llm-d-gateway",
                CheckState::Missing,
                "no GatewayClass in cluster — install a controller (Cilium/Istio/kgateway) first".into(),
                SetupFix::Command(
                    "# no GatewayClass found. Install a Gateway API controller that provides one, e.g. Cilium:\n#   cilium install --set gatewayAPI.enabled=true\n# then re-check — this row will offer to create llm-d-gateway.".into(),
                ),
            )
        });

        // ── RBAC / EPP ──
        v.push(if s.epp_role_sa && s.epp_role_non_sa {
            SetupCheck::new(
                "RBAC",
                "EPP shared Roles",
                CheckState::Ok,
                "llmd-router-epp-sa / -non-sa present".into(),
                SetupFix::None,
            )
        } else {
            SetupCheck::new(
                "RBAC",
                "EPP shared Roles",
                CheckState::Missing,
                "llmd-router-epp-sa/-non-sa absent — per-model deploy binds to them (helm-managed)".into(),
                SetupFix::Command(format!(
                    "# The shared EPP Roles are created by the llm-d router/modelservice Helm chart\n\
                     # (lmd-top's per-model Deploy only creates the SA + RoleBindings that reference them).\n\
                     # Install the router chart into {ns}, e.g.:\n\
                     #   helm install llmd-router <llm-d-modelservice-chart> -n {ns} -f manifests/llmd-router-values.yaml\n\
                     # Verify:  kubectl get role llmd-router-epp-sa llmd-router-epp-non-sa -n {ns}",
                    ns = self.ns
                )),
            )
        });

        // ── Storage / Secrets ──
        v.push(match s.pvc_model_store.as_deref() {
            Some("Bound") => SetupCheck::new(
                "Storage",
                "model-store PVC",
                CheckState::Ok,
                "Bound — shared HF cache + compiled artifacts".into(),
                SetupFix::None,
            ),
            Some(phase) => SetupCheck::new(
                "Storage",
                "model-store PVC",
                CheckState::Warn,
                format!("phase={} (not Bound) — check the backing PV/StorageClass", phase),
                SetupFix::Command(format!(
                    "kubectl -n {ns} describe pvc model-store   # inspect why it isn't Bound",
                    ns = self.ns
                )),
            ),
            None => SetupCheck::new(
                "Storage",
                "model-store PVC",
                CheckState::Missing,
                "absent — compile/deploy mount an RWX PVC named 'model-store'".into(),
                SetupFix::Command(format!(
                    "# 'model-store' is a site-specific RWX PVC (SMB/NFS/CSI) — lmd-top won't synthesize it.\n\
                     # Use your storage backend; this cluster's example:\n\
                     #   kubectl apply -f manifests/model-store.yaml   # SMB CSI → PV/PVC in {ns}\n\
                     # Verify:  kubectl -n {ns} get pvc model-store   # STATUS=Bound",
                    ns = self.ns
                )),
            ),
        });
        v.push(if s.secret_hf {
            SetupCheck::new(
                "Secrets",
                "hf-token",
                CheckState::Ok,
                "present — gated-model downloads authorized".into(),
                SetupFix::None,
            )
        } else {
            SetupCheck::new(
                "Secrets",
                "hf-token",
                CheckState::Warn,
                "absent — needed only for gated HF models (Llama etc.)".into(),
                SetupFix::Command(format!(
                    "# create the HF token secret (keep the token out of lmd-top — run this yourself):\n\
                     kubectl create secret generic hf-token -n {ns} --from-literal=token=$HF_TOKEN",
                    ns = self.ns
                )),
            )
        });

        // ── Accelerators ──
        let acc: Vec<String> = self
            .snap
            .inventory
            .iter()
            .filter(|(_, total, _)| *total > 0)
            .map(|(res, total, used)| {
                let short = res.rsplit('/').next().unwrap_or(res);
                format!("{} {}/{}", short, used, total)
            })
            .collect();
        v.push(if acc.is_empty() {
            SetupCheck::new(
                "Accelerators",
                "device plugins",
                CheckState::Warn,
                "no accelerator resources advertised — pods needing them stay Pending".into(),
                SetupFix::Command(
                    "# install the vendor device plugin(s) so nodes advertise accelerator resources:\n\
                     #   NVIDIA:     nvidia.com/gpu       (gpu-operator / k8s-device-plugin)\n\
                     #   Rebellions: rebellions.ai/ATOM   (rbln device plugin)\n\
                     #   Furiosa:    furiosa.ai/rngd      (furiosa device plugin)\n\
                     # Verify:  kubectl get nodes -o json | jq '.items[].status.allocatable'".into(),
                ),
            )
        } else {
            SetupCheck::new(
                "Accelerators",
                "device plugins",
                CheckState::Ok,
                format!("advertised: {}", acc.join(" · ")),
                SetupFix::None,
            )
        });

        v
    }

    /// Setup 뷰에서 ⏎ — 선택 행의 조치 실행.
    /// Apply → 미리보기(preview_apply, a 로 적용) · ApplyUrl → 확인 팝업(admin+) · Command → 읽기전용 미리보기.
    pub fn setup_enter(&mut self) {
        let checks = self.setup_checks();
        let Some(c) = checks.get(self.selected) else {
            return;
        };
        match &c.fix {
            SetupFix::None => {
                self.notify(format!("{}: {}", c.name, c.detail));
            }
            SetupFix::Command(cmd) => {
                let title = format!("Setup · {} — run this (read-only)", c.name);
                self.preview = Some((title, cmd.clone()));
                self.preview_scroll = 0;
                self.preview_apply = false;
            }
            SetupFix::Apply { title, yaml } => {
                self.preview = Some((title.clone(), yaml.clone()));
                self.preview_scroll = 0;
                self.preview_apply = true;
            }
            SetupFix::ApplyUrl { title, url } => {
                if !self.can(Mode::Admin) {
                    self.notify(format!(
                        "apply needs --mode admin+ (current: {})",
                        self.mode.name()
                    ));
                } else {
                    self.confirm = Some(Pending::ApplyUrl {
                        title: title.clone(),
                        url: url.clone(),
                    });
                    self.confirm_yes = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    fn find<'a>(v: &'a [SetupCheck], name: &str) -> &'a SetupCheck {
        v.iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no check named {name}"))
    }

    #[test]
    fn unreachable_cluster_yields_single_kubectl_check() {
        let a = App::new(); // 기본 snap: setup.probed=false
        let checks = a.setup_checks();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "kubectl access");
        assert_eq!(checks[0].state, CheckState::Missing);
        assert!(matches!(checks[0].fix, SetupFix::Command(_)));
    }

    #[test]
    fn bare_cluster_missing_prereqs_have_correct_fixes() {
        let mut a = App::new();
        a.snap.setup.probed = true; // 클러스터엔 닿지만 아무것도 안 깔림
        let v = a.setup_checks();

        // namespace 없음 → lmd-top 이 생성 가능(Apply)
        let ns = find(&v, "namespace");
        assert_eq!(ns.state, CheckState::Missing);
        assert!(matches!(ns.fix, SetupFix::Apply { .. }));

        // CRD 없음 → 상류 URL apply
        let gw = find(&v, "Gateway API");
        assert_eq!(gw.state, CheckState::Missing);
        match &gw.fix {
            SetupFix::ApplyUrl { url, .. } => assert!(url.contains("gateway-api/releases")),
            _ => panic!("Gateway API fix should be ApplyUrl"),
        }
        let gie = find(&v, "Inference Extension");
        assert!(matches!(gie.fix, SetupFix::ApplyUrl { .. }));

        // Gateway 오브젝트: CRD 없으니 조치 없음(먼저 CRD 설치 안내)
        let g = find(&v, "llm-d-gateway");
        assert_eq!(g.state, CheckState::Missing);
        assert!(matches!(g.fix, SetupFix::None));

        // 사이트 특화·helm 관리 → 절대 합성 apply 하지 않고 명령만
        assert!(matches!(find(&v, "EPP shared Roles").fix, SetupFix::Command(_)));
        assert!(matches!(find(&v, "model-store PVC").fix, SetupFix::Command(_)));
        assert!(matches!(find(&v, "hf-token").fix, SetupFix::Command(_)));
    }

    #[test]
    fn gateway_apply_prefers_cilium_class_when_crds_present() {
        let mut a = App::new();
        a.snap.setup.probed = true;
        a.snap.setup.crd_gateway = true;
        a.snap.setup.crd_httproute = true;
        a.snap.setup.gatewayclasses = vec!["istio".into(), "cilium".into()];
        a.snap.setup.gateway = false; // Gateway 만 없음
        let v = a.setup_checks();
        let g = find(&v, "llm-d-gateway");
        assert_eq!(g.state, CheckState::Missing);
        match &g.fix {
            SetupFix::Apply { yaml, .. } => {
                assert!(yaml.contains("gatewayClassName: cilium"), "cilium 우선 선택");
                assert!(yaml.contains("kind: Gateway"));
            }
            _ => panic!("Gateway fix should be Apply when a GatewayClass exists"),
        }
    }

    #[test]
    fn pvc_pending_is_warn_bound_is_ok() {
        let mut a = App::new();
        a.snap.setup.probed = true;
        a.snap.setup.pvc_model_store = Some("Pending".into());
        assert_eq!(find(&a.setup_checks(), "model-store PVC").state, CheckState::Warn);
        a.snap.setup.pvc_model_store = Some("Bound".into());
        let bound = a.setup_checks();
        let ok = find(&bound, "model-store PVC");
        assert_eq!(ok.state, CheckState::Ok);
        assert!(matches!(ok.fix, SetupFix::None));
    }
}
