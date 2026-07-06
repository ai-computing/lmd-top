//! Navigation — cross-layer pivots, breadcrumb back, section/sub-tab movement,
//! panel focus, list selection/scroll, and detail cursor. Split out of `app.rs`
//! (see `impl App`).

use super::*;
use std::sync::atomic::Ordering;

impl App {
    /// Cross-layer drill: jump from the selected entity to a related layer (view switch + correlation filter).
    /// Pushes the current position onto the breadcrumb so esc can retrace it. The collector is already wired.
    pub fn pivot(&mut self, key: char) {
        // Avoid mutable-borrow conflicts — extract the selected entity's values first.
        let model = self.selected_model().map(|m| m.name.clone());
        let serving_model = self.selected_artifact().map(|a| a.model.clone());
        let accel = self
            .selected_accel()
            .map(|a| (a.busy_model.clone(), a.node.clone()));
        let pod = self.selected_pod().map(|p| p.name.clone());
        let node = self.selected_node().map(|n| n.name.clone());
        let perf_model = self.selected_perf_model();
        let route_backend = self.selected_route_backend();
        let target: Option<(View, String)> = match self.view {
            View::Routing => route_backend.and_then(|b| match key {
                'p' => Some((View::Pods, b)),
                'i' => Some((View::Accel, b)),
                'm' => Some((View::Serving, b)),
                'e' => Some((View::Epp, String::new())),
                _ => None,
            }),
            View::Perf => perf_model.and_then(|name| match key {
                'p' => Some((View::Pods, name)),
                'i' => Some((View::Accel, name)),
                'e' => Some((View::Epp, String::new())),
                _ => None,
            }),
            View::Overview => model.and_then(|name| match key {
                'p' => Some((View::Pods, name)),
                'i' => Some((View::Accel, name)),
                'e' => Some((View::Epp, String::new())),
                'r' => Some((View::Routing, String::new())),
                _ => None,
            }),
            View::Serving => serving_model.and_then(|name| match key {
                'p' => Some((View::Pods, name)),
                'i' => Some((View::Accel, name)),
                'e' => Some((View::Epp, String::new())),
                'r' => Some((View::Routing, String::new())),
                _ => None,
            }),
            View::Accel => accel
                .filter(|(b, _)| !b.is_empty())
                .and_then(|(bm, nd)| match key {
                    'p' => Some((View::Pods, bm)),
                    'm' => Some((View::Serving, self.model_of_pod(&bm))),
                    'n' => Some((View::Nodes, nd)),
                    _ => None,
                }),
            View::Pods => pod.and_then(|pn| match key {
                'i' => Some((View::Accel, pn.clone())),
                'm' => Some((View::Serving, self.model_of_pod(&pn))),
                _ => None,
            }),
            View::Nodes => node.and_then(|nn| match key {
                'i' => Some((View::Accel, nn)),
                _ => None,
            }),
            _ => None,
        };
        match target {
            Some((v, filter)) => {
                self.nav_stack.push(NavState {
                    view: self.view,
                    selected: self.selected,
                    filter: self.filter.clone(),
                    detail: self.detail,
                });
                self.view = v;
                self.filter = filter;
                self.selected = 0;
                self.reset_sort();
                self.detail = false;
                self.epp_weights.clear();
                // Avoid landing on an empty screen: if 0 matches, retrace and notify (avoid a dead-end screen).
                if self.list_len() == 0 {
                    self.nav_back();
                    self.notify(format!("no related items to pivot to ('{}')", key));
                }
            }
            None => {
                // Unsupported pivot key in a pivot-source view → a hint instead of a dead keypress.
                if "piremn".contains(key) {
                    let hint = match self.view {
                        View::Overview | View::Serving => Some("p/i/r/e"),
                        View::Accel => Some("p/m/n"),
                        View::Pods => Some("i/m"),
                        View::Nodes => Some("i"),
                        View::Perf => Some("p/i/e"),
                        View::Routing => Some("p/i/m/e"),
                        _ => None,
                    };
                    if let Some(h) = hint {
                        self.notify(format!("pivot here: {}", h));
                    }
                }
            }
        }
    }

    /// Retrace the breadcrumb (esc). True if it retraced.
    pub fn nav_back(&mut self) -> bool {
        if let Some(st) = self.nav_stack.pop() {
            self.view = st.view;
            self.selected = st.selected;
            self.filter = st.filter;
            self.detail = st.detail;
            self.reset_sort();
            self.epp_weights.clear();
            true
        } else {
            false
        }
    }

    /// Is the current mode at least `required` privilege (gate for mutating operations).
    pub fn can(&self, required: Mode) -> bool {
        self.mode >= required
    }

    /// 만료(toast_until)를 가진 토스트 알림 설정 — 액션 피드백/알림 공용.
    pub fn notify(&mut self, msg: String) {
        self.toast = Some(msg);
        self.toast_until = crate::collect::now_secs() + 5;
        self.toast_bad = false;
    }

    /// 뷰의 표시 컬럼 순서(설정 없으면 default 반환). 설정의 미지 키는 무시(default에서 교집합).
    pub fn columns<'a>(&'a self, view: &str, default: &'a [&'a str]) -> Vec<&'a str> {
        match self.cols.get(view) {
            Some(cfg) => cfg
                .iter()
                .filter(|c| default.contains(&c.as_str()))
                .map(|c| default.iter().find(|d| **d == c.as_str()).copied().unwrap())
                .collect(),
            None => default.to_vec(),
        }
    }

    pub fn toggle_help(&mut self) {
        self.help = !self.help;
    }
    pub fn toggle_alerts(&mut self) {
        self.alerts_panel = !self.alerts_panel;
    }
    pub fn cycle_theme(&mut self) {
        let n = (theme() + 1) % N_THEMES;
        THEME.store(n, Ordering::Relaxed);
        self.notify(format!("theme: {}", theme_name(n)));
    }

    /// 상세 패널을 가진 뷰인지(detail=true 가 실제로 렌더에 반영되는 뷰).
    /// 없는 뷰(Routing/Epp/Launch/Events)에서 detail=true 로 두면 ↑↓ 가 스크롤로 빠져 네비가 잠김.
    pub fn view_has_detail(&self) -> bool {
        matches!(
            self.view,
            View::Accel
                | View::Overview
                | View::Pods
                | View::Nodes
                | View::Events
                | View::Serving
                | View::Library
        )
    }

    pub fn toggle_detail(&mut self) {
        if self.view_has_detail() && self.list_len() > 0 {
            self.detail = !self.detail;
            self.dev_sel = 0; // 상세 진입 시 노드 요약부터
        }
    }

    /// Flow(route) 에서 Enter → 백엔드 모델 상세로 드릴(브레드크럼 쌓음, esc 로 복귀).
    pub fn drill_route(&mut self) {
        if self.view != View::Routing {
            return;
        }
        self.pivot('m'); // → Serving, filter=backend (매칭 0건이면 pivot 이 되짚음)
        if self.view == View::Serving && self.list_len() > 0 {
            self.detail = true;
        }
    }

    /// Jump to a top-level section by number key (0-5) — lands on its first sub-tab.
    pub fn goto_section(&mut self, i: usize) {
        if let Some(sec) = Section::ALL.get(i) {
            if let Some(v) = sec.members().first() {
                self.goto_view(*v);
            }
        }
    }

    /// 임의 뷰로 직접 점프(섹션·서브탭 무관) — 팔레트/pivot/섹션 착지 공용.
    /// 선택/정렬/패널포커스/브레드크럼 초기화.
    pub fn goto_view(&mut self, v: View) {
        self.view = v;
        self.selected = 0;
        self.reset_sort();
        self.detail = false;
        self.panel_focus = 0;
        self.panel_move = false;
        self.dev_sel = 0;
        self.nav_stack.clear();
        self.epp_weights.clear();
    }

    /// 커맨드 팔레트 열기(`:`). 다른 오버레이가 없을 때만.
    pub fn open_palette(&mut self) {
        self.palette = Some(crate::palette::Palette::global());
    }

    /// Tab / Shift+Tab — cycle top-level sections (lands on each section's first sub-tab).
    pub fn next_tab(&mut self) {
        let n = Section::ALL.len();
        self.goto_section((self.view.section().idx() + 1) % n);
    }
    pub fn prev_tab(&mut self) {
        let n = Section::ALL.len();
        self.goto_section((self.view.section().idx() + n - 1) % n);
    }
    /// 현재 뷰의 포커스 가능한 패널 수(멀티패널 뷰만 >1).
    pub fn panel_count(&self) -> usize {
        match self.view {
            View::Library => 1, // Deploy▸Model List: 배포 가능 트리(단일 패널)
            View::Serving => 1, // Serving: 라이브 배포 트리(단일 패널)
            View::Epp => 2,     // scorers / InferencePool
            View::Routing => 2, // routes / InferencePool
            _ => 1,
        }
    }
    /// `←` / `→` / `[` / `]` — cycle the current section's sub-tabs (views). No-op for single-member sections.
    pub fn cycle_subtab(&mut self, delta: i64) {
        let members = self.view.section().members();
        let n = members.len() as i64;
        if n <= 1 {
            return;
        }
        let cur = members.iter().position(|v| *v == self.view).unwrap_or(0);
        let next = members[((cur as i64 + delta).rem_euclid(n)) as usize];
        self.view = next;
        self.selected = 0;
        self.detail = false;
        self.panel_focus = 0;
        self.panel_move = false;
        self.dev_sel = 0;
        // 서브탭 전환은 "새 내비게이션" — 이전 pivot 브레드크럼을 비워, 이후 Esc 가 엉뚱한 곳(옛 pivot 출발점)
        // 으로 되짚지 않게 한다. (goto_view 와 동일한 브레드크럼 리셋)
        self.nav_stack.clear();
        self.epp_weights.clear();
    }
    /// Ctrl-w — arm vi/tmux panel-focus mode (hjkl/arrows then move focus). No-op unless multi-panel.
    pub fn arm_panel_move(&mut self) {
        if self.panel_count() > 1 {
            self.panel_move = true;
        }
    }
    /// Move panel focus by delta, staying in panel-move mode (repeatable). Used while `panel_move` is armed.
    pub fn cycle_panel_dir(&mut self, delta: i64) {
        let n = self.panel_count();
        if n > 1 {
            self.panel_focus = ((self.panel_focus as i64 + delta).rem_euclid(n as i64)) as usize;
            self.selected = 0;
        }
    }

    /// Perf per-model 리스트(서빙 중=active 만 + 정렬). 패널 포커스와 무관 — 뷰/order 공용.
    pub fn perf_rows_order(&self) -> Vec<usize> {
        use crate::collect::PerfRow;
        use std::cmp::Ordering::Equal;
        let v = &self.snap.perf_rows;
        let active = |r: &PerfRow| {
            (!r.req.is_nan() && r.req > 0.0)
                || (!r.tps.is_nan() && r.tps > 0.0)
                || !r.e2e_p95.is_nan()
                || !r.ttft_p95.is_nan()
                || !r.queue_p95.is_nan()
        };
        let mut idx: Vec<usize> = (0..v.len()).filter(|&i| active(&v[i])).collect();
        let key = |x: f64| if x.is_nan() { f64::MIN } else { x };
        idx.sort_by(|&a, &b| {
            let (x, y): (&PerfRow, &PerfRow) = (&v[a], &v[b]);
            match self.sort {
                1 => key(y.e2e_p95).partial_cmp(&key(x.e2e_p95)).unwrap_or(Equal),
                2 => key(y.ttft_p95)
                    .partial_cmp(&key(x.ttft_p95))
                    .unwrap_or(Equal),
                3 => key(y.queue_p95)
                    .partial_cmp(&key(x.queue_p95))
                    .unwrap_or(Equal),
                4 => x.model.cmp(&y.model),
                _ => key(y.tps).partial_cmp(&key(x.tps)).unwrap_or(Equal),
            }
        });
        idx
    }

    /// 현재 뷰에서 선택 가능한 행 수(필터 반영).
    pub fn list_len(&self) -> usize {
        self.order().len()
    }

    pub fn move_sel(&mut self, delta: i64) {
        let n = self.list_len();
        if n == 0 {
            return;
        }
        let cur = self.selected as i64 + delta;
        self.selected = cur.rem_euclid(n as i64) as usize;
        self.detail_scroll = 0; // 항목 바뀌면 스크롤 리셋
        self.dev_sel = 0; // 다른 노드로 이동 → device 커서 요약으로
    }
    /// g / G — jump the selection to the first / last row (less/vim convention).
    pub fn sel_edge(&mut self, last: bool) {
        let n = self.list_len();
        if n == 0 {
            return;
        }
        self.selected = if last { n - 1 } else { 0 };
        self.detail_scroll = 0;
        self.dev_sel = 0;
    }
    pub fn scroll_detail(&mut self, delta: i64) {
        self.detail_scroll = (self.detail_scroll as i64 + delta).max(0) as u16;
    }
    /// 현재 선택 노드가 가진 가속기 수(Node 상세 device 커서 범위).
    pub fn node_dev_count(&self) -> usize {
        match self.selected_node() {
            Some(n) => self.snap.accel.iter().filter(|a| a.node == n.name).count(),
            None => 0,
        }
    }
    /// Node 상세 device 커서 이동: 0(요약) ↔ 1..=n(개별 device) 순환.
    pub fn dev_cursor(&mut self, delta: i64) {
        let n = self.node_dev_count();
        if n == 0 {
            return;
        }
        let cur = self.dev_sel as i64 + delta;
        self.dev_sel = cur.rem_euclid((n + 1) as i64) as usize; // 0..=n
    }
    /// detail 위치(현재/전체) — "◂ prev  i/n  next ▸" 표시용.
    pub fn detail_pos(&self) -> (usize, usize) {
        (self.selected + 1, self.list_len())
    }
}
