//! Context action menu — per-view Enter menu plus cross-layer "Go: …" pivots.
//! Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    /// "Go: …" pivot entries for the current view's action menu — makes cross-layer jumps discoverable.
    /// Menu accelerator keys are chosen to not collide with the view's own action keys; the last field
    /// is the pivot key passed to `pivot()` (which decides the destination from the current selection).
    pub(super) fn pivot_items(&self) -> Vec<ActionItem> {
        // (menu_key, label, pivot_key)
        let defs: &[(char, &'static str, char)] = match self.view {
            View::Overview | View::Serving => &[
                ('p', "Go: Pods", 'p'),
                ('v', "Go: Devices", 'i'),
                ('e', "Go: EPP", 'e'),
                ('g', "Go: Route", 'r'),
            ],
            View::Pods => &[('v', "Go: Devices", 'i'), ('m', "Go: Model", 'm')],
            View::Routing if self.panel_focus == 0 => &[
                ('p', "Go: Pods", 'p'),
                ('v', "Go: Devices", 'i'),
                ('e', "Go: EPP", 'e'),
            ],
            _ => &[],
        };
        defs.iter()
            .map(|(k, l, pk)| ActionItem {
                key: *k,
                label: l,
                desc: "pivot to the related layer",
                action: Action::Pivot(*pk),
            })
            .collect()
    }

    /// Enter — 선택 항목의 컨텍스트 액션 메뉴를 연다(단축키를 몰라도 되게).
    pub fn open_action_menu(&mut self) {
        let mut items: Vec<ActionItem> = Vec::new();
        let (title, subject) = match self.view {
            View::Serving if self.panel_focus == 0 => {
                // Serving = 돌아가는 배포의 *운영* 렌즈. 컴파일/신규 배포는 Deploy▸Model List 로.
                let Some(a) = self.selected_artifact() else {
                    return;
                };
                let running = self
                    .snap
                    .models
                    .iter()
                    .any(|m| m.name == a.model && m.desired > 0);
                items.push(ActionItem::info("show full deployment detail"));
                items.push(ActionItem::logs("tail serving pod logs"));
                items.push(ActionItem::yaml("live Deployment YAML (read-only)"));
                items.push(ActionItem::scale());
                items.push(ActionItem::restart());
                items.push(ActionItem::rollback());
                items.push(ActionItem::objective());
                if running {
                    items.push(ActionItem::stop("scale serving → 0 (frees devices)"));
                }
                (format!("actions · {}", a.model), a.model.clone())
            }
            View::Library
                if self.panel_focus == 0
                    && matches!(self.selected_lib_item(), Some(LibItem::Stored(_))) =>
            {
                // 통합 트리의 스토어 컴파일본 — 물리적으로 존재하는 배포 가능 빌드. Info + Deploy.
                let Some(s) = self.selected_stored().map(|s| {
                    (
                        s.repo.clone(),
                        s.format.clone(),
                        s.compiled_for.clone(),
                    )
                }) else {
                    return;
                };
                items.push(ActionItem::info("build detail — format · target · size · path"));
                items.push(ActionItem::deploy(if s.1 == "hf" {
                    "serve source weights (GPU); Rebellions/Furiosa 는 먼저 컴파일 필요"
                } else {
                    "serve this compiled build → Deployment"
                }));
                // Store housekeeping. A build that took hours occupies tens of GB, and until
                // now the only way to reclaim it was a shell on a pod with the PVC mounted.
                items.push(ActionItem::store_move());
                items.push(ActionItem::store_delete());
                // Right after a removal the row is still listed until discovery re-runs, so
                // the way to fix that belongs in the same menu.
                items.push(ActionItem::store_refresh());
                let label = if s.1 == "hf" {
                    format!("store · {} (source)", s.0)
                } else {
                    format!("store · {} [{}]", s.0, s.2)
                };
                (label, s.0)
            }
            View::Library if self.panel_focus == 0 => {
                // 통합 트리의 카탈로그(조직 제공) 행 — 가능성 설명 + 배포/컴파일 경로.
                let Some(m) = self.selected_catalog_model() else {
                    return;
                };
                items.push(ActionItem::info("why ready / needs artifact (feasibility)"));
                if let Some(p) = self.preferred_catalog_placement(m) {
                    let vendor = Self::placement_vendor(p);
                    let model_id = Self::placement_model_id(m, p);
                    // Offer a compile when the chosen placement's accelerator builds ahead of
                    // time, or when some other accelerator could build this model family.
                    let placement_compiles = crate::accel::by_id(vendor)
                        .is_some_and(|p| p.caps.compiles_ahead_of_time);
                    let alternatives = crate::compat::compilable_vendors(&model_id);
                    if let Some(cv) = if placement_compiles {
                        Some(vendor)
                    } else {
                        alternatives.first().copied()
                    } {
                        items.push(ActionItem::compile(cv));
                    }
                    items.push(ActionItem::deploy(if p.requires_artifact {
                        "generate Deployment; artifact path may need review"
                    } else {
                        "serving options → Deployment"
                    }));
                }
                (format!("catalog · {}", m.id), m.id.clone())
            }
            View::Zoo if self.panel_focus == 0 => {
                // 벤더 모델 zoo — prefetch(가중치 다운로드) + 컴파일 가능한 각 벤더.
                let Some(z) = self.selected_zoo() else {
                    return;
                };
                let (source, in_store) = (z.source.clone(), self.zoo_in_store(&z.source));
                items.push(ActionItem::info("model source / notes"));
                items.push(ActionItem::prefetch());
                for v in Self::zoo_vendors(&source) {
                    items.push(ActionItem::compile(v));
                }
                let note = if in_store {
                    "compiled build present — deploy from Deploy▸Library"
                } else {
                    "prefetch/compile, then deploy from Deploy▸Library"
                };
                (format!("zoo · {} ({})", source, note), source)
            }
            View::Library | View::Zoo if self.panel_focus == 1 => {
                // Deploy 하단 Activity 패널 — compile Job / deploy rollout. 로그 · (Job 이면) 삭제.
                let Some(row) = self.selected_activity() else {
                    return;
                };
                if row.pod.is_some() {
                    items.push(ActionItem::logs("tail the operation's pod logs"));
                }
                if row.job.is_some() {
                    items.push(ActionItem::delete_job());
                }
                // subject = 삭제 대상 Job 이름(있으면), 없으면 요약 라벨.
                let subject = row.job.clone().unwrap_or_else(|| row.label.clone());
                (format!("activity · {}", row.label), subject)
            }
            View::Nodes => {
                // 노드 관리 — 스케줄 차단/해제(예전 Deploy 타깃 패널에서 이동).
                let Some(node) = self.selected_node().map(|n| (n.name.clone(), n.cordoned)) else {
                    return;
                };
                items.push(ActionItem::info("node detail — devices, occupancy, capacity"));
                if node.1 {
                    items.push(ActionItem::new('u', "Uncordon", "allow scheduling on this node", Action::Uncordon));
                } else {
                    items.push(ActionItem::new('C', "Cordon", "block new scheduling on this node", Action::Cordon));
                }
                (format!("node · {}", node.0), node.0)
            }
            View::Overview => {
                let Some(m) = self.selected_model() else {
                    return;
                };
                let running = m.desired > 0;
                items.push(ActionItem::info("model detail"));
                items.push(ActionItem::logs("tail pod logs"));
                items.push(ActionItem::yaml("live Deployment YAML (read-only)"));
                items.push(ActionItem::scale());
                items.push(ActionItem::restart());
                items.push(ActionItem::rollback());
                items.push(ActionItem::objective());
                if running {
                    items.push(ActionItem::stop("scale → 0 (frees devices)"));
                }
                (format!("actions · {}", m.name), m.name.clone())
            }
            View::Pods => {
                let Some(p) = self.selected_pod() else { return };
                items.push(ActionItem::info("pod detail"));
                items.push(ActionItem::logs("tail pod logs"));
                items.push(ActionItem::yaml("live Pod YAML (read-only)"));
                items.push(ActionItem::new('r', "Drain", "relabel out of routing — stop new requests, finish in-flight streams", Action::Drain));
                items.push(ActionItem::delete("delete pod (reschedules)"));
                (format!("actions · {}", p.name), p.name.clone())
            }
            View::Routing if self.panel_focus == 0 => {
                // Flow 의 선택된 라우트 — 경로 관리.
                let Some(r) = self.selected_route() else {
                    return;
                };
                items.push(ActionItem::new('i', "Backend", "jump to backend model detail", Action::Info));
                items.push(ActionItem::new('r', "Rename", "change gateway path (/accel/model)", Action::RouteRename));
                items.push(ActionItem::new('t', "Retarget", "point path at another pool/service", Action::RouteRetarget));
                items.push(ActionItem::new('D', "Delete", "remove this route rule", Action::RouteDelete));
                (format!("route · {}", r.path), r.path.clone())
            }
            _ => return,
        };
        items.extend(self.pivot_items()); // append "Go: …" cross-layer jumps (empty for views without pivots)
        self.action_menu = Some(ActionMenu {
            title,
            subject,
            items,
            cursor: 0,
        });
    }
}
