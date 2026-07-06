//! Live filter state (`/`) and per-view search text used by row ordering.
//! Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    pub fn start_filter(&mut self) {
        self.filtering = true;
    }
    pub fn stop_filter(&mut self) {
        self.filtering = false;
    }
    pub fn filter_push(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }
    pub fn filter_pop(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.filtering = false;
        self.selected = 0;
    }

    /// 인덱스 i 의 검색 대상 문자열(뷰별).
    pub(super) fn search_text(&self, i: usize) -> String {
        match self.view {
            View::Accel => self
                .snap
                .accel
                .get(i)
                .map(|a| format!("{} {} {} {}", a.kind.label(), a.id, a.node, a.busy_model))
                .unwrap_or_default(),
            View::Overview => self
                .snap
                .models
                .get(i)
                .map(|m| format!("{} {}", m.name, m.accel))
                .unwrap_or_default(),
            View::Pods => self
                .snap
                .pods
                .get(i)
                .map(|p| format!("{} {}", p.name, p.node))
                .unwrap_or_default(),
            View::Serving => self
                .snap
                .artifacts
                .get(i)
                .map(|a| format!("{} {} {}", a.model, a.family, a.source))
                .unwrap_or_default(),
            View::Library if self.panel_focus == 1 => self
                .activity_rows()
                .get(i)
                .map(|r| r.label.clone())
                .unwrap_or_default(),
            View::Library => match self.library_items().get(i) {
                Some(LibItem::Catalog(k)) => self
                    .catalog
                    .get(*k)
                    .map(|m| format!("{} {}", m.id, m.role))
                    .unwrap_or_default(),
                Some(LibItem::Stored(k)) => self
                    .snap
                    .stored
                    .get(*k)
                    .map(|s| format!("{} {} {}", s.repo, s.format, s.compiled_for))
                    .unwrap_or_default(),
                None => String::new(),
            },
            View::Epp if self.panel_focus == 1 => self
                .snap
                .pools
                .get(i)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            View::Epp => self
                .snap
                .epp
                .as_ref()
                .and_then(|e| e.scorers.get(i))
                .map(|(n, _)| n.clone())
                .unwrap_or_default(),
            View::Events => self
                .snap
                .events
                .get(i)
                .map(|e| format!("{} {} {}", e.reason, e.object, e.message))
                .unwrap_or_default(),
            View::Nodes => self
                .snap
                .nodes
                .get(i)
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            View::Routing if self.panel_focus == 1 => self
                .snap
                .pools
                .get(i)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            View::Routing => self
                .snap
                .routes
                .get(i)
                .map(|r| format!("{} {}", r.path, r.backend))
                .unwrap_or_default(),
            View::Perf => self
                .snap
                .perf_rows
                .get(i)
                .map(|r| r.model.clone())
                .unwrap_or_default(),
            View::Topo => String::new(), // 맵 뷰 — 리스트 선택 없음
            View::Setup => self
                .setup_checks()
                .get(i)
                .map(|c| format!("{} {} {}", c.category, c.name, c.detail))
                .unwrap_or_default(),
        }
    }
}
