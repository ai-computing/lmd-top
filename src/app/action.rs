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
                items.push(ActionItem {
                    key: 'i',
                    label: "Info",
                    desc: "show full deployment detail",
                    action: Action::Info,
                });
                items.push(ActionItem {
                    key: 'l',
                    label: "Logs",
                    desc: "tail serving pod logs",
                    action: Action::Logs,
                });
                items.push(ActionItem {
                    key: 'y',
                    label: "YAML",
                    desc: "live Deployment YAML (read-only)",
                    action: Action::Yaml,
                });
                items.push(ActionItem {
                    key: 's',
                    label: "Scale",
                    desc: "toggle replicas 0/1",
                    action: Action::Scale,
                });
                items.push(ActionItem {
                    key: 'S',
                    label: "Restart",
                    desc: "rollout restart (rolling)",
                    action: Action::Restart,
                });
                items.push(ActionItem {
                    key: 'O',
                    label: "Objective",
                    desc: "set SLO target (TTFT/TPOT/E2E/tok·s) — drives advisor",
                    action: Action::Objective,
                });
                if running {
                    items.push(ActionItem {
                        key: 'x',
                        label: "Stop",
                        desc: "scale serving → 0 (frees devices)",
                        action: Action::Stop,
                    });
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
                items.push(ActionItem {
                    key: 'i',
                    label: "Info",
                    desc: "build detail — format · target · size · path",
                    action: Action::Info,
                });
                items.push(ActionItem {
                    key: 'd',
                    label: "Deploy",
                    desc: if s.1 == "hf" {
                        "serve source weights (GPU); NPU 는 먼저 컴파일 필요"
                    } else {
                        "serve this compiled build → Deployment"
                    },
                    action: Action::Deploy,
                });
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
                items.push(ActionItem {
                    key: 'i',
                    label: "Info",
                    desc: "why ready / needs artifact (feasibility)",
                    action: Action::Info,
                });
                if let Some(p) = self.preferred_catalog_placement(m) {
                    let vendor = Self::placement_vendor(p);
                    let model_id = Self::placement_model_id(m, p);
                    if matches!(vendor, "rbln" | "furiosa")
                        || !crate::compat::compilable_vendors(&model_id).is_empty()
                    {
                        let cv = if matches!(vendor, "rbln" | "furiosa") {
                            vendor
                        } else {
                            crate::compat::compilable_vendors(&model_id)
                                .first()
                                .copied()
                                .unwrap_or("rbln")
                        };
                        let (key, label, desc) = if cv == "furiosa" {
                            (
                                'f',
                                "Compile→Furiosa",
                                "furiosa-llm build → artifact in store",
                            )
                        } else {
                            ('c', "Compile→RBLN", "optimum-rbln compile → .rbln in store")
                        };
                        items.push(ActionItem {
                            key,
                            label,
                            desc,
                            action: Action::Compile(cv),
                        });
                    }
                    items.push(ActionItem {
                        key: 'd',
                        label: "Deploy",
                        desc: if p.requires_artifact {
                            "generate Deployment; artifact path may need review"
                        } else {
                            "serving options → Deployment"
                        },
                        action: Action::Deploy,
                    });
                }
                (format!("catalog · {}", m.id), m.id.clone())
            }
            View::Activity => {
                // 통합 작업 피드 — compile Job / deploy rollout. 로그 · (Job 이면) 삭제.
                let Some(row) = self.selected_activity() else {
                    return;
                };
                if row.pod.is_some() {
                    items.push(ActionItem {
                        key: 'l',
                        label: "Logs",
                        desc: "tail the operation's pod logs",
                        action: Action::Logs,
                    });
                }
                if row.job.is_some() {
                    items.push(ActionItem {
                        key: 'D',
                        label: "Delete",
                        desc: "delete compile job (cancel / clean up)",
                        action: Action::DeleteJob,
                    });
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
                items.push(ActionItem {
                    key: 'i',
                    label: "Info",
                    desc: "node detail — devices, occupancy, capacity",
                    action: Action::Info,
                });
                if node.1 {
                    items.push(ActionItem {
                        key: 'u',
                        label: "Uncordon",
                        desc: "allow scheduling on this node",
                        action: Action::Uncordon,
                    });
                } else {
                    items.push(ActionItem {
                        key: 'C',
                        label: "Cordon",
                        desc: "block new scheduling on this node",
                        action: Action::Cordon,
                    });
                }
                (format!("node · {}", node.0), node.0)
            }
            View::Overview => {
                let Some(m) = self.selected_model() else {
                    return;
                };
                let running = m.desired > 0;
                items.push(ActionItem {
                    key: 'i',
                    label: "Info",
                    desc: "model detail",
                    action: Action::Info,
                });
                items.push(ActionItem {
                    key: 'l',
                    label: "Logs",
                    desc: "tail pod logs",
                    action: Action::Logs,
                });
                items.push(ActionItem {
                    key: 'y',
                    label: "YAML",
                    desc: "live Deployment YAML (read-only)",
                    action: Action::Yaml,
                });
                items.push(ActionItem {
                    key: 's',
                    label: "Scale",
                    desc: "toggle replicas 0/1",
                    action: Action::Scale,
                });
                items.push(ActionItem {
                    key: 'S',
                    label: "Restart",
                    desc: "rollout restart (rolling)",
                    action: Action::Restart,
                });
                items.push(ActionItem {
                    key: 'O',
                    label: "Objective",
                    desc: "set SLO target (TTFT/TPOT/E2E/tok·s) — drives advisor",
                    action: Action::Objective,
                });
                if running {
                    items.push(ActionItem {
                        key: 'x',
                        label: "Stop",
                        desc: "scale → 0 (frees devices)",
                        action: Action::Stop,
                    });
                }
                (format!("actions · {}", m.name), m.name.clone())
            }
            View::Pods => {
                let Some(p) = self.selected_pod() else { return };
                items.push(ActionItem {
                    key: 'i',
                    label: "Info",
                    desc: "pod detail",
                    action: Action::Info,
                });
                items.push(ActionItem {
                    key: 'l',
                    label: "Logs",
                    desc: "tail pod logs",
                    action: Action::Logs,
                });
                items.push(ActionItem {
                    key: 'y',
                    label: "YAML",
                    desc: "live Pod YAML (read-only)",
                    action: Action::Yaml,
                });
                items.push(ActionItem {
                    key: 'D',
                    label: "Delete",
                    desc: "delete pod (reschedules)",
                    action: Action::Delete,
                });
                (format!("actions · {}", p.name), p.name.clone())
            }
            View::Routing if self.panel_focus == 0 => {
                // Flow 의 선택된 라우트 — 경로 관리.
                let Some(r) = self.selected_route() else {
                    return;
                };
                items.push(ActionItem {
                    key: 'i',
                    label: "Backend",
                    desc: "jump to backend model detail",
                    action: Action::Info,
                });
                items.push(ActionItem {
                    key: 'r',
                    label: "Rename",
                    desc: "change gateway path (/accel/model)",
                    action: Action::RouteRename,
                });
                items.push(ActionItem {
                    key: 't',
                    label: "Retarget",
                    desc: "point path at another pool/service",
                    action: Action::RouteRetarget,
                });
                items.push(ActionItem {
                    key: 'D',
                    label: "Delete",
                    desc: "remove this route rule",
                    action: Action::RouteDelete,
                });
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
