//! Current-selection helpers — map the selected row to the underlying entity
//! per view, plus neighbor names, YAML target, and log target resolution.
//! Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    fn entity_name(&self, i: usize) -> String {
        match self.view {
            View::Accel => self
                .snap
                .accel
                .get(i)
                .map(|a| format!("{} {}", a.kind.label(), a.id))
                .unwrap_or_default(),
            View::Overview => self
                .snap
                .models
                .get(i)
                .map(|m| m.name.clone())
                .unwrap_or_default(),
            View::Pods => self
                .snap
                .pods
                .get(i)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            View::Nodes => self
                .snap
                .nodes
                .get(i)
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// 이전/다음 항목 이름(detail 네비 힌트용).
    pub fn neighbor_names(&self) -> (String, String) {
        let ord = self.order();
        let n = ord.len();
        if n <= 1 {
            return (String::new(), String::new());
        }
        let prev = self.entity_name(ord[(self.selected + n - 1) % n]);
        let next = self.entity_name(ord[(self.selected + 1) % n]);
        (prev, next)
    }

    pub fn selected_model(&self) -> Option<&crate::collect::ModelRow> {
        match self.view {
            View::Overview => self.sel_orig().and_then(|i| self.snap.models.get(i)),
            _ => None,
        }
    }
    pub fn selected_accel(&self) -> Option<&crate::collect::Accel> {
        match self.view {
            View::Accel => self.sel_orig().and_then(|i| self.snap.accel.get(i)),
            _ => None,
        }
    }
    pub fn selected_pod(&self) -> Option<&crate::collect::PodRow> {
        match self.view {
            View::Pods => self.sel_orig().and_then(|i| self.snap.pods.get(i)),
            _ => None,
        }
    }
    pub fn selected_node(&self) -> Option<&crate::collect::NodeInfo> {
        match self.view {
            View::Nodes => self.sel_orig().and_then(|i| self.snap.nodes.get(i)),
            _ => None,
        }
    }
    pub fn selected_event(&self) -> Option<&crate::collect::EventRow> {
        match self.view {
            View::Events => self.sel_orig().and_then(|i| self.snap.events.get(i)),
            _ => None,
        }
    }
    /// `y` — 현재 선택의 live YAML 조회 대상 (kind, namespaced?, name). 없으면 None.
    pub fn yaml_target(&self) -> Option<(&'static str, bool, String)> {
        match self.view {
            View::Overview => self
                .selected_model()
                .map(|m| ("deployment", true, m.name.clone())),
            View::Pods => self.selected_pod().map(|p| ("pod", true, p.name.clone())),
            View::Nodes => self
                .selected_node()
                .map(|n| ("node", false, n.name.clone())),
            View::Serving if self.panel_focus == 0 => self
                .selected_artifact()
                .map(|a| ("deployment", true, a.model.clone())),
            _ => None,
        }
    }

    /// Serving 뷰에서 선택된 아티팩트(라이브 배포).
    pub fn selected_artifact(&self) -> Option<&crate::collect::ModelArtifact> {
        if self.view == View::Serving && self.panel_focus == 0 {
            self.sel_orig().and_then(|i| self.snap.artifacts.get(i))
        } else {
            None
        }
    }

    /// 로그 대상 pod 이름(현재 선택 기준).
    pub fn logs_target_pod(&self) -> Option<String> {
        match self.view {
            View::Pods => self.selected_pod().map(|p| p.name.clone()),
            View::Overview => self.selected_model().and_then(|m| {
                self.snap
                    .pods
                    .iter()
                    .find(|p| p.name.starts_with(&m.name))
                    .map(|p| p.name.clone())
            }),
            View::Accel => self
                .selected_accel()
                .filter(|a| !a.busy_model.is_empty())
                .map(|a| a.busy_model.clone()),
            // Deploy 하단 Activity 패널 — 선택 작업(compile Job / deploy rollout)의 파드 로그.
            View::Library | View::Zoo if self.panel_focus == 1 => {
                self.selected_activity().and_then(|r| r.pod)
            }
            _ => None,
        }
    }
}
