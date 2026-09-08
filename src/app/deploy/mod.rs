//! Serving deploy flow — deploy-spec derivation, form construction, capacity fit and
//! preflight. Manifest rendering lives in the submodules:
//!
//! - [`serving`]: the serving Deployment (and any recipe ConfigMap it needs).
//! - [`routing`]: the llm-d gateway path — EPP, InferencePool, HTTPRoute.

pub mod routing;
pub mod serving;

use super::*;
use serving::{Images, ServePlan};

/// Serving spec derived from the current selection:
/// (model, model_id, engine, vendor, mount, devices_default, serve_tp_default).
type DeploySpec = (String, String, String, &'static str, String, String, Option<String>);

impl App {    pub(super) fn selected_deploy_spec(&self) -> Option<DeploySpec> {
        if let Some(a) = self.selected_artifact() {
            let model_id = Self::artifact_model_id(a);
            let repo_dir = model_id.replace('/', "--");
            let vendor = if a.engine.contains("RBLN") {
                "rbln"
            } else if a.engine.contains("Furiosa") {
                "furiosa"
            } else {
                "gpu"
            };
            let mount = if a.mount.is_empty() {
                format!("/mnt/store/compiled/{}", repo_dir)
            } else {
                a.mount
                    .split(" ← ")
                    .next()
                    .unwrap_or("/mnt/store")
                    .to_string()
            };
            let tp = Self::opt_or(a, "tp", if vendor == "furiosa" { "8" } else { "1" });
            let dev_default = if vendor == "furiosa" {
                let pe = tp.parse::<i64>().unwrap_or(8).max(1);
                ((pe as f64 / 8.0).ceil() as i64).max(1).to_string()
            } else {
                tp.clone()
            };
            return Some((
                a.model.clone(),
                model_id,
                a.engine.clone(),
                vendor,
                mount,
                dev_default,
                if vendor == "furiosa" { Some(tp) } else { None },
            ));
        }
        // Library 패널0: 스토어 컴파일본을 바로 배포 — repo/포맷/타깃(compiled_for)에서 spec 유도.
        if let Some(s) = self.selected_stored() {
            let vendor = match s.format.as_str() {
                "rbln" => "rbln",
                "furiosa" => "furiosa",
                _ => "gpu",
            };
            let engine = match vendor {
                "rbln" => "vLLM-RBLN",
                "furiosa" => "Furiosa-LLM",
                _ => "vLLM",
            };
            // compiled_for(예: RBLN-CA22-tp4-s8192) 에서 tp 추출.
            let tp = s
                .compiled_for
                .split(['-', '_', ' '])
                .find_map(|t| {
                    let t = t.to_lowercase();
                    t.strip_prefix("tp").and_then(|n| n.parse::<i64>().ok())
                })
                .unwrap_or(if vendor == "furiosa" { 8 } else { 1 });
            let dev_default = if vendor == "furiosa" {
                ((tp as f64 / 8.0).ceil() as i64).max(1).to_string()
            } else {
                tp.to_string()
            };
            let mount = if s.path.is_empty() {
                "/mnt/store".to_string()
            } else if s.path.starts_with('/') {
                s.path.clone()
            } else {
                format!("/mnt/store/{}", s.path)
            };
            let model = s.repo.rsplit('/').next().unwrap_or(&s.repo).to_string();
            return Some((
                model,
                s.repo.clone(),
                engine.to_string(),
                vendor,
                mount,
                dev_default,
                if vendor == "furiosa" {
                    Some(tp.to_string())
                } else {
                    None
                },
            ));
        }
        let m = self.selected_catalog_model()?;
        let p = self.preferred_catalog_placement(m)?;
        let model_id = Self::placement_model_id(m, p);
        let model = if m.display.is_empty() {
            m.id.clone()
        } else {
            m.display.clone()
        };
        let vendor = Self::placement_vendor(p);
        Some((
            model,
            model_id,
            Self::placement_engine(p).to_string(),
            vendor,
            Self::placement_mount(m, p),
            p.count.max(1).to_string(),
            if vendor == "furiosa" {
                Some("8".to_string())
            } else {
                None
            },
        ))
    }

    /// `[d] deploy` — 선택 모델의 배포(서빙) 옵션 편집 폼을 연다. replicas·디바이스·노드 배치.
    pub fn open_deploy_form(&mut self) {
        let Some((model, model_id, engine, vendor, mount, dev_default, serve_tp_default)) =
            self.selected_deploy_spec()
        else {
            return;
        };
        // placement 는 폼 필드가 아니라 제출 직전 placement 화면에서 고른다(open_place_picker).
        let mut fields = vec![CompileField {
            key: "replicas".into(),
            label: "replicas".into(),
            value: "1".into(),
            choices: ["1", "2", "3", "4", "6", "8"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            numeric: true,
            help: "Number of serving instances. Total device demand = replicas × devices.".into(),
        }];
        if vendor == "furiosa" {
            fields.push(CompileField {
                key: "tp".into(),
                label: "serve TP (PE)".into(),
                value: serve_tp_default.unwrap_or_else(|| "8".to_string()),
                choices: ["4", "8", "16"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Furiosa serving tensor parallel size in PE units. Device request stays separate.".into(),
            });
        }
        fields.extend(vec![
            CompileField {
                key: "devices".into(),
                label: "devices/replica".into(),
                value: dev_default,
                choices: ["1", "2", "4", "8"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Accelerators requested per replica (resources.limits). For Furiosa, this is RNGD count, not PE TP.".into(),
            },
            CompileField {
                key: "port".into(),
                label: "port".into(),
                value: "8000".into(),
                choices: ["8000", "8080"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Serving container port.".into(),
            },
            CompileField {
                key: "routing".into(),
                label: "routing".into(),
                value: "llm-d".into(),
                choices: ["llm-d", "direct"].iter().map(|s| s.to_string()).collect(),
                numeric: false,
                help: "llm-d also creates InferencePool, EPP, and HTTPRoute; direct creates only Deployment.".into(),
            },
        ]);
        self.deploy_form = Some(DeployForm {
            model,
            model_id,
            engine,
            vendor,
            mount,
            fields,
            place: "any".into(), // placement 화면에서 선택; 기본은 제약 없음.
            cursor: 0,
            editing: false,
        });
    }

    /// deploy 옵션 확정(Enter) 후 열리는 placement 선택 화면 — 후보 노드 상태 목록
    /// (유휴/전체 디바이스·util·mem·스케줄가능)을 만든다. 선택하면 배치 확정 + 매니페스트 생성.
    pub fn open_place_picker(&mut self) {
        let Some(form) = self.deploy_form.as_ref() else {
            return;
        };
        let (kind, drv, res) = match form.vendor {
            "rbln" => (crate::collect::AccelKind::Rbln, "RBLN", "rebellions.ai/ATOM"),
            "furiosa" => (crate::collect::AccelKind::Rngd, "RNGD", "furiosa.ai/rngd"),
            _ => (crate::collect::AccelKind::Gpu, "", "nvidia.com/gpu"),
        };
        // 노드별 집계: (total, util_sum, mem_used, mem_total). free 는 아래에서 총−할당으로.
        let mut agg: std::collections::BTreeMap<String, (i64, f64, f64, f64)> =
            std::collections::BTreeMap::new();
        for a in self
            .snap
            .accel
            .iter()
            .filter(|a| a.kind == kind && !a.node.is_empty())
        {
            let e = agg.entry(a.node.clone()).or_default();
            e.0 += 1;
            e.1 += a.util;
            e.2 += a.mem_used_gb;
            e.3 += a.mem_total_gb;
        }
        let pseudo = |value: &str, note: &str| PlaceRow {
            value: value.into(),
            label: value.into(),
            free: 0,
            total: 0,
            util: f64::NAN,
            mem_used: 0.0,
            mem_total: 0.0,
            schedulable: true,
            note: note.into(),
            info_only: false,
        };
        let mut rows = vec![
            pseudo("any", "제약 없음 — 스케줄러가 유휴 리소스로 배치"),
            pseudo("spread", "replica 를 여러 노드에 고르게 분산"),
        ];
        for (node, (total, util_sum, mu, mt)) in agg {
            let nd = self.snap.nodes.iter().find(|n| n.name == node);
            let ready = nd.map(|n| n.ready && !n.cordoned).unwrap_or(true);
            let has_drv =
                drv.is_empty() || nd.map(|n| n.npu.to_uppercase().contains(drv)).unwrap_or(false);
            // 할당(파드 requests) 기반 유휴 = 총 − 할당. 이미 배포된 파드가 잡은 디바이스는 free 에서 제외.
            let alloc = self
                .snap
                .node_alloc
                .get(&node)
                .and_then(|m| m.get(res))
                .copied()
                .unwrap_or(0);
            let free = (total - alloc).max(0);
            let note = if !ready {
                "NotReady/cordon".into()
            } else if !has_drv {
                format!("{} 드라이버 없음", drv)
            } else if free == 0 {
                format!("{} allocated", alloc)
            } else {
                String::new()
            };
            rows.push(PlaceRow {
                util: if total > 0 {
                    util_sum / total as f64
                } else {
                    f64::NAN
                },
                value: node.clone(),
                label: node,
                free,
                total,
                mem_used: mu,
                mem_total: mt,
                schedulable: ready && has_drv,
                note,
                info_only: false,
            });
        }
        self.place_picker = Some(PlacePick { cursor: 0, rows });
    }

    /// placement 피커 커서 이동(순환).
    pub fn place_pick_move(&mut self, delta: i64) {
        if let Some(p) = self.place_picker.as_mut() {
            let n = p.rows.len() as i64;
            if n > 0 {
                p.cursor = ((p.cursor as i64 + delta).rem_euclid(n)) as usize;
            }
        }
    }

    /// 선택한 목적지를 확정 → 매니페스트 생성(제출)까지 진행. deploy/compile/prefetch 공용 2단계 마지막.
    /// 활성 폼에 따라 라우팅: deploy→place, compile→실행 노드, prefetch→저장 PVC.
    pub fn place_pick_apply(&mut self) {
        let Some(p) = self.place_picker.take() else {
            return;
        };
        let Some(value) = p.rows.get(p.cursor).map(|r| r.value.clone()) else {
            return;
        };
        if self.compile_form.is_some() {
            if let Some(form) = self.compile_form.as_mut() {
                form.dest = value;
            }
            self.compile_form_submit();
        } else if self.prefetch_form.is_some() {
            if let Some(form) = self.prefetch_form.as_mut() {
                form.dest = value;
            }
            self.prefetch_form_submit();
        } else {
            if let Some(form) = self.deploy_form.as_mut() {
                form.place = value;
            }
            self.deploy_form_submit(); // placement 확정 후 바로 매니페스트 미리보기
        }
    }

    /// Compile 목적지(실행 노드) picker — 2단계. RBLN=rebel-compiler 노드, Furiosa=아무 노드(AOT).
    pub fn open_compile_dest_picker(&mut self) {
        let Some(form) = self.compile_form.as_ref() else {
            return;
        };
        let vendor = form.vendor;
        let want = match vendor {
            "rbln" => Some("RBLN"),
            "furiosa" => Some("RNGD"),
            _ => None,
        };
        let kind = match vendor {
            "rbln" => crate::collect::AccelKind::Rbln,
            "furiosa" => crate::collect::AccelKind::Rngd,
            _ => crate::collect::AccelKind::Gpu,
        };
        let mut per_node: std::collections::BTreeMap<String, i64> = Default::default();
        for a in self.snap.accel.iter().filter(|a| a.kind == kind && !a.node.is_empty()) {
            *per_node.entry(a.node.clone()).or_insert(0) += 1;
        }
        let note_auto = if vendor == "rbln" {
            "자동 — rebel-compiler 설치 노드에 고정(hostPath)"
        } else {
            "자동 — AOT, 아무 노드(CPU 포함)에서 실행"
        };
        let mut rows = vec![PlaceRow {
            value: "any".into(),
            label: "any".into(),
            free: 0,
            total: 0,
            util: f64::NAN,
            mem_used: 0.0,
            mem_total: 0.0,
            schedulable: true,
            note: note_auto.into(),
            info_only: true,
        }];
        // 후보 노드: RBLN 은 드라이버 보유 노드만 실행 가능(호스트 스택), Furiosa 는 Ready 아무 노드.
        let mut nodes: Vec<&crate::collect::NodeInfo> = self.snap.nodes.iter().collect();
        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        for nd in nodes {
            let ready = nd.ready && !nd.cordoned;
            let has_drv = want
                .map(|w| nd.npu.to_uppercase().contains(w))
                .unwrap_or(true);
            // Furiosa(AOT)는 드라이버 불필요 → Ready 면 가능. RBLN 은 드라이버(=호스트 스택) 필요.
            let ok = ready && (vendor != "rbln" || has_drv);
            let dev = per_node.get(&nd.name).copied().unwrap_or(0);
            let note = if !ready {
                "NotReady/cordon".to_string()
            } else if vendor == "rbln" && !has_drv {
                "no rebel-compiler (RBLN 드라이버 없음)".to_string()
            } else if !nd.npu.is_empty() {
                nd.npu.clone()
            } else {
                "CPU only".to_string()
            };
            rows.push(PlaceRow {
                value: nd.name.clone(),
                label: nd.name.clone(),
                free: dev,
                total: dev,
                util: f64::NAN,
                mem_used: 0.0,
                mem_total: 0.0,
                schedulable: ok,
                note,
                info_only: true, // 컴파일은 디바이스 예약 안 함 → 노드+드라이버만 보여줌
            });
        }
        self.place_picker = Some(PlacePick { cursor: 0, rows });
    }

    /// 배포 용량 판정 — 총 디바이스 수요 대 클러스터 동종 가속기(총/유휴).
    pub fn deploy_fit(&self, form: &DeployForm) -> DeployFit {
        let want_kind = match form.vendor {
            "rbln" => crate::collect::AccelKind::Rbln,
            "furiosa" => crate::collect::AccelKind::Rngd,
            _ => crate::collect::AccelKind::Gpu,
        };
        let devs: Vec<&crate::collect::Accel> = self
            .snap
            .accel
            .iter()
            .filter(|x| x.kind == want_kind)
            .collect();
        let total = devs.len() as i64;
        let res_key = match form.vendor {
            "rbln" => "rebellions.ai/ATOM",
            "furiosa" => "furiosa.ai/rngd",
            _ => "nvidia.com/gpu",
        };
        // 노드별 총 디바이스(살아있는) 수.
        let mut total_by_node: std::collections::BTreeMap<&str, i64> =
            std::collections::BTreeMap::new();
        // 노드별 metric 유휴(busy_model 미점유) — 정보용(idle 이지만 예약됐을 수 있음).
        let mut metric_free_by_node: std::collections::BTreeMap<&str, i64> =
            std::collections::BTreeMap::new();
        for d in &devs {
            if !d.node.is_empty() && d.alive {
                *total_by_node.entry(d.node.as_str()).or_insert(0) += 1;
                if d.busy_model.is_empty() {
                    *metric_free_by_node.entry(d.node.as_str()).or_insert(0) += 1;
                }
            }
        }
        // 노드별 스케줄가능 유휴 = 노드 디바이스 수 − 리소스 예약(node_alloc requests).
        // 스케줄러가 실제로 보는 값 — metric 유휴여도 예약(request)돼 있으면 배치 불가.
        // (node_alloc 없으면 metric 유휴로 폴백.)
        let free_by_node: std::collections::BTreeMap<&str, i64> = total_by_node
            .iter()
            .map(|(&node, &tot)| {
                let req = self
                    .snap
                    .node_alloc
                    .get(node)
                    .and_then(|m| m.get(res_key))
                    .copied();
                let free = match req {
                    Some(r) => (tot - r).max(0),
                    None => metric_free_by_node.get(node).copied().unwrap_or(0),
                };
                (node, free)
            })
            .collect();
        let free: i64 = free_by_node.values().sum();
        let metric_free: i64 = metric_free_by_node.values().sum();
        let nodes = free_by_node.len() as i64;
        let max_node_free = free_by_node.values().copied().max().unwrap_or(0);
        let replicas = form.get("replicas").parse::<i64>().unwrap_or(1).max(1);
        let per = form.get("devices").parse::<i64>().unwrap_or(1).max(1);
        let demand = replicas * per;
        // 클러스터 리소스 관점 유휴 = allocatable - requested (인벤토리 집계, 위 노드별 합과 일치해야 함).
        let resource_free = self
            .snap
            .inventory
            .iter()
            .find(|(k, _, _)| k == res_key)
            .map(|(_, alloc, req)| (alloc - req).max(0))
            .unwrap_or(free); // inventory 없으면 노드별 합으로 폴백
                              // 실제 배치 가능 replica 수 = Σ floor(node_free / per) (한 노드 안에 per 개가 모여야).
        let placeable: i64 = free_by_node.values().map(|f| f / per).sum();
        let verdict = if total == 0 {
            FitVerdict::Unknown
        } else if demand > resource_free {
            // 스케줄러 관점 리소스 부족 — metric 유휴여도 예약돼 있으면 못 뜸(우선).
            FitVerdict::Oom
        } else if per > max_node_free {
            FitVerdict::Oom // replica 하나도 어느 노드에도 안 들어감(조각난 여유)
        } else if placeable < replicas {
            FitVerdict::Tight // 총량은 되지만 노드 패킹으로 일부만 배치
        } else {
            FitVerdict::Fits
        };
        let mut tips: Vec<String> = Vec::new();
        if demand > resource_free {
            tips.push(format!(
                "리소스 예약 기준 유휴 {} < 수요 {} — 다른 배포가 {} 를 점유(request)함. 그 서빙을 stop 하거나 replicas/devices↓",
                resource_free, demand, res_key
            ));
        } else if matches!(verdict, FitVerdict::Oom) {
            tips.push(format!("replica당 {}개가 단일 노드에 안 들어감(최대 유휴 {}/노드) — devices/replica↓ 또는 서빙 정리", per, max_node_free));
        } else if matches!(verdict, FitVerdict::Tight) {
            tips.push(format!("노드 패킹상 {}/{} replica 만 배치 가능(유휴 {}, 노드별 조각) — replicas↓ 또는 노드 확보", placeable, replicas, free));
        }
        // metric 유휴(idle)와 리소스 유휴(예약)가 어긋나면 명시(오해 방지).
        if metric_free != free {
            tips.push(format!(
                "(metric idle {} ≠ 스케줄가능 {} — 예약됐지만 idle 인 디바이스 있음)",
                metric_free, free
            ));
        }
        if form.place == "spread" && replicas > nodes && nodes > 0 {
            tips.push(format!(
                "⚠ spread: replicas {} > 노드 {} — 일부는 같은 노드로",
                replicas, nodes
            ));
        }
        if per > 1 && form.vendor == "rbln" {
            tips.push("replica당 다중 칩은 컴파일 TP 와 일치해야 함".into());
        }
        DeployFit {
            demand,
            total,
            free,
            metric_free,
            resource_free,
            nodes,
            verdict,
            tips,
        }
    }

    /// 배포 사전 점검(preflight) — apply 전에 서빙 전제조건 확인(사전 방어).
    pub fn deploy_preflight(&self, form: &DeployForm) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        // 이미지 — deploy_form_submit 의 벤더별 기본값과 동일 판정(불일치로 오탐 방지).
        //   furiosa=furiosaai/furiosa-llm:latest, gpu=vllm/vllm-openai:latest 기본 존재 → OK.
        //   rbln 은 vllm_rbln 런타임이 든 이미지가 필요(기본 없음) → LMD_SERVING_IMAGE 미지정이면 차단.
        let (img_ok, img_msg) = match form.vendor {
            "furiosa" => (
                true,
                "image ready: furiosaai/furiosa-llm:latest (furiosa-llm serve)".to_string(),
            ),
            "gpu" => {
                let img = self
                    .img_serving
                    .clone()
                    .unwrap_or_else(|| "vllm/vllm-openai:latest".into());
                (true, format!("image ready: {} (vLLM serve)", img))
            }
            _ => match &self.img_serving {
                Some(img) => (true, format!("image ready: {} (vllm_rbln runtime)", img)),
                None => (
                    true,
                    "image fallback: ubuntu:22.04 with host RBLN stack on the target node".into(),
                ),
            },
        };
        out.push((img_ok, format!("1. serving image: {}", img_msg)));
        // 2. Model artifact path — serving must load an HF id or compiled store path.
        out.push((
            !form.mount.is_empty(),
            format!(
                "2. model location: {}",
                if form.mount.is_empty() {
                    "unknown path: compile first or choose a store artifact".into()
                } else {
                    form.mount.clone()
                }
            ),
        ));
        // 3. NPU vendors need a node with the corresponding driver/resource plugin.
        if form.vendor != "gpu" {
            let want = if form.vendor == "rbln" {
                "RBLN"
            } else {
                "RNGD"
            };
            let any = self
                .snap
                .nodes
                .iter()
                .any(|n| n.npu.to_uppercase().contains(want));
            out.push((
                any,
                format!(
                    "3. accelerator driver: {} node {}",
                    want,
                    if any {
                        "exists: schedulable"
                    } else {
                        "missing: pods will stay Pending"
                    }
                ),
            ));
        }
        // 4. Capacity — requested devices must fit scheduler-visible free resources.
        let fit = self.deploy_fit(form);
        let cap_ok = matches!(fit.verdict, FitVerdict::Fits);
        out.push((
            cap_ok,
            format!(
                "4. capacity: needs {} device(s), {} free -> {}{}",
                fit.demand,
                fit.resource_free,
                fit.verdict.label(),
                if cap_ok {
                    ""
                } else {
                    " (stop another serving workload or lower replicas/devices)"
                }
            ),
        ));
        out
    }

    /// 배포 폼 → Deployment 매니페스트 미리보기(dry-run). Enter 시 호출.
    pub fn deploy_form_submit(&mut self) {
        let Some(form) = self.deploy_form.take() else {
            return;
        };
        // model_id/mount 는 Deployment 의 args·env 로 들어간다. 문법이 아닌 값은 거부한다 —
        // 직렬화가 YAML 탈출은 막지만, 애초에 모델 id 가 아닌 문자열은 배포할 대상이 아니다.
        let bad_field = [
            ("model", form.model_id.clone()),
            ("mount", form.mount.clone()),
        ]
        .into_iter()
        .find(|(_, v)| !v.is_empty() && !crate::quote::valid_model_id(v));
        if let Some((label, bad)) = bad_field {
            self.notify(format!(
                "deploy blocked — {} '{}' is not a valid HF repo id or store path",
                label, bad
            ));
            self.deploy_form = Some(form); // 폼 유지: 값만 고쳐 다시 Enter
            return;
        }

        let name = format!("serve-{}", form.model_id.replace(['/', '.'], "-").to_lowercase());
        let replicas = form.get("replicas");
        let devices = form.get("devices");
        let or = |key: &str, def: &str| {
            let v = form.get(key);
            if v.is_empty() {
                def.to_string()
            } else {
                v
            }
        };
        // Furiosa serving TP is a PE count, distinct from the RNGD card count it requests.
        let serve_tp = if form.vendor == "furiosa" {
            or("tp", "8")
        } else {
            devices.clone()
        };
        let (res_key, product_label) = match form.vendor {
            "rbln" => (
                "rebellions.ai/ATOM",
                Some(("rebellions.ai/npu.product", "RBLN-CA22")),
            ),
            "furiosa" => (
                "furiosa.ai/rngd",
                Some(("furiosa.ai/npu.product", "rngd")),
            ),
            _ => ("nvidia.com/gpu", None),
        };
        // A store-backed Furiosa artifact is served by path; everything else by HF id.
        let served = if form.vendor == "furiosa" && form.mount.starts_with("/mnt/store/") {
            form.mount.clone()
        } else {
            form.model_id.clone()
        };
        let place = form.place.clone();
        let plan = ServePlan {
            name: &name,
            ns: &self.ns,
            replicas: replicas.clone(),
            devices: devices.clone(),
            serve_tp,
            port: or("port", "8000"),
            served: served.clone(),
            res_key,
            product_label,
            place_host: place.split('(').next().unwrap_or("any").trim().to_string(),
            place_label: place.clone(),
        };
        let images = Images {
            furiosa: self.img_furiosa.as_deref(),
            serving: self.img_serving.as_deref(),
        };

        let mut manifest = serving::serving_manifest(&form, &plan, &images);
        // routing=llm-d → 게이트웨이 라우팅 리소스(EPP+InferencePool+HTTPRoute)를 뒤에 동봉.
        if form.get("routing") == "llm-d" {
            let routes = routing::routing_docs(&self.ns, &name, form.vendor, &served);
            manifest.docs.extend(routes.docs);
        }

        self.confirm = Some(Pending::Apply {
            title: format!("deploy {} ×{}", form.model, replicas),
            yaml: manifest.to_yaml(),
        });
        self.confirm_yes = false;
    }
}

#[cfg(test)]
mod deploy_routing_tests {
    use super::*;

    /// Parse the routing documents into objects, keyed by (kind, name) — assertions can then be
    /// about the wiring rather than about substrings that happen to appear in the file.
    fn objects(ns: &str, name: &str, vendor: &str, served: &str) -> Vec<serde_yaml::Value> {
        let yaml = routing::routing_docs(ns, name, vendor, served).to_yaml();
        serde_yaml::Deserializer::from_str(&yaml)
            .map(|d| {
                <serde_yaml::Value as serde::Deserialize>::deserialize(d)
                    .expect("routing docs are valid YAML")
            })
            .filter(|v: &serde_yaml::Value| !v.is_null())
            .collect()
    }

    fn find<'a>(docs: &'a [serde_yaml::Value], kind: &str) -> &'a serde_yaml::Value {
        docs.iter()
            .find(|d| d["kind"].as_str() == Some(kind))
            .unwrap_or_else(|| panic!("routing docs should contain a {}", kind))
    }

    /// The generated EPP must match the wiring proven in-cluster (`manifests/epp/*`): without the
    /// metrics source and extractor plugins the scorers read nothing and routing is inert.
    #[test]
    fn routing_docs_has_metric_source_plugins_and_auth() {
        let docs = objects("llm-serving", "myserve", "rbln", "meta-llama/Llama-3.1-8B-Instruct");

        // The EPP config is a YAML document embedded in the ConfigMap — parse it too.
        let cm = docs
            .iter()
            .find(|d| d["kind"].as_str() == Some("ConfigMap"))
            .expect("EPP ConfigMap");
        let cfg: serde_yaml::Value =
            serde_yaml::from_str(cm["data"]["default-plugins.yaml"].as_str().expect("plugins"))
                .expect("embedded EPP config parses as YAML");
        let types: Vec<&str> = cfg["plugins"]
            .as_sequence()
            .expect("plugins list")
            .iter()
            .filter_map(|p| p["type"].as_str())
            .collect();
        for want in [
            "metrics-data-source",
            "core-metrics-extractor",
            "no-hit-lru-scorer",
            "queue-scorer",
            "kv-cache-utilization-scorer",
            "prefix-cache-scorer",
        ] {
            assert!(types.contains(&want), "EPP plugins missing {}: {:?}", want, types);
        }
        // Every scorer in the profile refers to a declared plugin, with a weight.
        for entry in cfg["schedulingProfiles"][0]["plugins"]
            .as_sequence()
            .expect("profile plugins")
        {
            let r = entry["pluginRef"].as_str().expect("pluginRef");
            assert!(types.contains(&r), "profile references undeclared plugin {}", r);
            assert!(entry["weight"].as_u64().is_some(), "{} needs a weight", r);
        }

        // EPP verifies pool membership through TokenReview, so it needs auth delegation.
        let crb = find(&docs, "ClusterRoleBinding");
        assert_eq!(crb["roleRef"]["name"].as_str(), Some("system:auth-delegator"));
        assert_eq!(
            crb["subjects"][0]["name"].as_str(),
            Some("myserve-epp"),
            "the binding must name this deployment's own SA"
        );

        // Health probes on the EPP container.
        let epp = docs
            .iter()
            .find(|d| {
                d["kind"].as_str() == Some("Deployment")
                    && d["metadata"]["name"].as_str() == Some("myserve-epp")
            })
            .expect("EPP Deployment");
        let c = &epp["spec"]["template"]["spec"]["containers"][0];
        assert!(!c["livenessProbe"].is_null() && !c["readinessProbe"].is_null());

        // The pool selects the serving pods and points at the EPP service.
        let pool = find(&docs, "InferencePool");
        assert_eq!(pool["spec"]["selector"]["matchLabels"]["app"].as_str(), Some("myserve"));
        assert_eq!(pool["spec"]["endpointPickerRef"]["name"].as_str(), Some("myserve-epp"));

        // And the route sends traffic to the pool, not straight to a Service — which is exactly
        // the misconfiguration that used to bypass the EPP entirely.
        let route = find(&docs, "HTTPRoute");
        let backend = &route["spec"]["rules"][0]["backendRefs"][0];
        assert_eq!(backend["kind"].as_str(), Some("InferencePool"));
        assert_eq!(backend["name"].as_str(), Some("myserve-pool"));
    }

    /// Furiosa exposes `furiosa_llm_*`, so the extractor must be told the metric names;
    /// vLLM/RBLN expose `vllm:*`, which the defaults already read.
    #[test]
    fn furiosa_extractor_specifies_furiosa_metric_names() {
        let extractor = |vendor: &str| -> serde_yaml::Value {
            let docs = objects("llm-serving", "svc", vendor, "Qwen/Qwen3-Embedding-8B");
            let cm = docs
                .iter()
                .find(|d| d["kind"].as_str() == Some("ConfigMap"))
                .expect("EPP ConfigMap");
            let cfg: serde_yaml::Value =
                serde_yaml::from_str(cm["data"]["default-plugins.yaml"].as_str().unwrap()).unwrap();
            cfg["plugins"]
                .as_sequence()
                .unwrap()
                .iter()
                .find(|p| p["type"].as_str() == Some("core-metrics-extractor"))
                .cloned()
                .expect("core-metrics-extractor")
        };

        let f = extractor("furiosa");
        let engine = &f["parameters"]["engineConfigs"][0];
        assert_eq!(
            engine["queuedRequestsSpec"].as_str(),
            Some("furiosa_llm_num_requests_waiting")
        );
        assert_eq!(
            engine["kvUsageSpec"].as_str(),
            Some("furiosa_llm_kv_cache_usage_percent")
        );
        assert_eq!(
            engine["cacheInfoSpec"].as_str(),
            Some("furiosa_llm_cache_config_info")
        );

        let r = extractor("rbln");
        assert!(
            r["parameters"].is_null(),
            "vLLM/RBLN should take the extractor defaults, got {:?}",
            r["parameters"]
        );
    }

    /// The route path is derived from the accelerator family and the model slug.
    #[test]
    fn route_path_names_accelerator_and_model() {
        for (vendor, want) in [
            ("furiosa", "/rngd/qwen3-4b-fp8"),
            ("rbln", "/atom/qwen3-4b-fp8"),
            ("gpu", "/gpu/qwen3-4b-fp8"),
        ] {
            let docs = objects("llm-serving", "svc", vendor, "furiosa-ai/Qwen3.4B_FP8");
            let route = find(&docs, "HTTPRoute");
            assert_eq!(
                route["spec"]["rules"][0]["matches"][0]["path"]["value"].as_str(),
                Some(want),
                "{} route path",
                vendor
            );
        }
    }
}
