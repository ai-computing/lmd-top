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
            View::Models | View::Overview => &[
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
                let Some(a) = self.selected_artifact() else {
                    return;
                };
                let model_id = Self::artifact_model_id(a);
                let deployed = self
                    .snap
                    .models
                    .iter()
                    .any(|m| m.name == a.model && m.desired > 0);
                items.push(ActionItem {
                    key: 'i',
                    label: "Info",
                    desc: "show full build detail",
                    action: Action::Info,
                });
                // 컴파일 대상 벤더 — 엔진이 NPU 면 그 벤더, 아니면 지원 목록(GPU/HF→NPU)에서.
                let rbln_ok = a.engine.contains("RBLN")
                    || crate::compat::compilable_vendors(&model_id).contains(&"rbln");
                let furiosa_ok = a.engine.contains("Furiosa")
                    || crate::compat::compilable_vendors(&model_id).contains(&"furiosa");
                if rbln_ok {
                    items.push(ActionItem {
                        key: 'c',
                        label: "Compile→RBLN",
                        desc: "optimum-rbln compile → .rbln in store",
                        action: Action::Compile("rbln"),
                    });
                }
                if furiosa_ok {
                    items.push(ActionItem {
                        key: 'f',
                        label: "Compile→Furiosa",
                        desc: "furiosa-llm build → artifact in store",
                        action: Action::Compile("furiosa"),
                    });
                }
                items.push(ActionItem {
                    key: 'd',
                    label: "Deploy",
                    desc: "serving options → Deployment",
                    action: Action::Deploy,
                });
                items.push(ActionItem {
                    key: 'y',
                    label: "YAML",
                    desc: "live Deployment YAML (read-only)",
                    action: Action::Yaml,
                });
                if deployed {
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
            View::Library if self.panel_focus == 1 => {
                // 진행 중 컴파일 패널 — 로그 확인 / Job 삭제(취소·정리).
                let Some(c) = self.sel_orig().and_then(|i| self.snap.compiles.get(i)) else {
                    return;
                };
                items.push(ActionItem {
                    key: 'l',
                    label: "Logs",
                    desc: "tail compile pod logs",
                    action: Action::Logs,
                });
                items.push(ActionItem {
                    key: 'D',
                    label: "Delete",
                    desc: "delete job (cancel / clean up)",
                    action: Action::DeleteJob,
                });
                (format!("compile · {}", c.name), c.name.clone())
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
            View::Models | View::Overview => {
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
