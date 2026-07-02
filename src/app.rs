//! UI 상태머신 — 현재 뷰, 선택, 스파크라인 히스토리. 데이터(Snapshot)와 분리.

use crate::collect::Snapshot;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};

/// 알림 심각도.
#[derive(Clone, Copy, PartialEq)]
pub enum Sev {
    Warn,
    Bad,
}

/// 권한 모드(운영 사고 방지) — 선언 순서 = 권한 레벨(Observe < … < Danger).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Observe, // 보기만
    Debug,   // + logs / dry-run
    Admin,   // + scale / rollout
    Danger,  // + delete / force
}
impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_lowercase().as_str() {
            "observe" | "obs" | "ro" => Some(Mode::Observe),
            "debug" | "dbg" => Some(Mode::Debug),
            "admin" => Some(Mode::Admin),
            "danger" => Some(Mode::Danger),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Observe => "observe",
            Mode::Debug => "debug",
            Mode::Admin => "admin",
            Mode::Danger => "danger",
        }
    }
}

/// 크로스레이어 드릴 브레드크럼 — pivot 시 현재 위치를 쌓고 esc 로 되짚음.
#[derive(Clone)]
pub struct NavState {
    pub view: View,
    pub selected: usize,
    pub filter: String,
    pub detail: bool,
}

/// 확인(y/n) 대기 중인 변경 작업. 실행은 이벤트 루프(main)에서.
#[derive(Clone)]
pub enum Pending {
    Scale { name: String, target: i64 },
    Restart { name: String },
}
impl Pending {
    /// 확인 프롬프트 문구.
    pub fn prompt(&self) -> String {
        match self {
            Pending::Scale { name, target } => format!("scale {} → {} replica(s)?", name, target),
            Pending::Restart { name } => format!("rollout restart {} (rolling)?", name),
        }
    }
}

/// 임계치 초과/상태 이상 이벤트 하나. key=중복/엣지검출용 안정 식별자.
#[derive(Clone)]
pub struct Alert {
    pub ts: u64,
    pub sev: Sev,
    pub key: String,
    pub msg: String,
}

// 알림 임계치(ui.rs 의 색 임계치와 개념 일치 — 여기선 "경보" 발생선).
const ALERT_TEMP_BAD: f64 = 80.0;

/// 전역 테마 인덱스 (0=default, 1=고대비, 2=색맹친화). ui 색 함수가 읽음.
pub static THEME: AtomicUsize = AtomicUsize::new(0);
pub const N_THEMES: usize = 4;
pub fn theme() -> usize {
    THEME.load(Ordering::Relaxed)
}
/// 시작 테마 지정(config 값 반영). 범위 밖은 무시.
pub fn set_theme(i: usize) {
    if i < N_THEMES {
        THEME.store(i, Ordering::Relaxed);
    }
}
pub fn theme_name(i: usize) -> &'static str {
    match i {
        1 => "high-contrast",
        2 => "colorblind",
        3 => "soft (catppuccin)",
        _ => "default",
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Overview,
    Accel,
    Models,
    Epp,
    Routing,
    Pods,
    Perf,
    Launch,
    Events,
    Nodes,
}

impl View {
    pub const ALL: [View; 10] = [
        View::Overview,
        View::Accel,
        View::Models,
        View::Epp,
        View::Routing,
        View::Pods,
        View::Perf,
        View::Launch,
        View::Events,
        View::Nodes,
    ];
    pub fn idx(&self) -> usize {
        View::ALL.iter().position(|v| v == self).unwrap_or(0)
    }
    pub fn title(&self) -> &'static str {
        match self {
            View::Overview => "Overview",
            View::Accel => "Accel",
            View::Models => "Models",
            View::Epp => "EPP",
            View::Routing => "Flow",
            View::Pods => "Pods",
            View::Perf => "Perf",
            View::Launch => "Launch",
            View::Events => "Events",
            View::Nodes => "Nodes",
        }
    }
}

pub const HIST: usize = 40;

pub struct App {
    pub view: View,
    pub selected: usize,
    pub snap: Snapshot,
    pub hist: HashMap<String, VecDeque<u64>>, // accel util 히스토리
    pub toast: Option<String>,
    pub detail: bool,  // 선택 행 상세(drill-down) 표시 여부
    pub sort: usize,   // 현재 뷰의 정렬 모드(뷰별로 의미 다름, 순환)
    pub tick: u64,     // 렌더 틱(마퀴/스피너 애니메이션용)
    pub filter: String,   // 행 필터(부분일치)
    pub filtering: bool,  // 필터 입력 모드
    pub help: bool,       // 도움말/범례 오버레이
    pub zoom: bool,       // 포커스(줌) — 헤더/탭 숨기고 본문 최대화
    pub paused: bool,     // 화면 갱신 일시정지(데이터 고정, 읽기용)
    pub detail_scroll: u16, // detail 내부 세로 스크롤
    pub dev_sel: usize,     // Node 상세 내 device 커서: 0=노드요약, 1..=n=해당 device 히스토리
    pub models_persp: usize, // Models 뷰 관점: 0=serving(런타임) · 1=artifacts(모델 정체성/저장/컴파일)
    pub logs_mode: bool,      // 로그 오버레이
    pub logs_target: String,  // 로그 대상 pod
    pub logs: Vec<String>,    // 로그 줄
    pub logs_scroll: u16,
    pub cols: HashMap<String, Vec<String>>, // 뷰별 표시 컬럼(순서) — 설정파일
    pub catalog: Vec<crate::catalog::CatModel>, // 모델 카탈로그(런처)
    // ── 능동 알림 ──
    pub alerts: VecDeque<Alert>,        // 히스토리(최신 앞), cap 50
    pub active_alerts: HashSet<String>, // 현재 활성 키(엣지 검출용)
    pub alerts_panel: bool,             // 알림 히스토리 오버레이(A)
    pub flash_until: u64,               // epoch초 — 이 시각 전까지 요약바 플래시
    pub toast_until: u64,               // epoch초 — 토스트 만료
    pub toast_bad: bool,                // 토스트 배경색(빨강=심각)
    prev_restarts: HashMap<String, i64>, // pod 재시작 델타 추적
    // ── 권한 모드 ──
    pub mode: Mode,                 // observe(기본)/debug/admin/danger — 기동 시 --mode
    pub confirm: Option<Pending>,   // y/n 확인 대기 중인 변경 작업
    // ── 크로스레이어 드릴 ──
    pub nav_stack: Vec<NavState>,   // pivot 브레드크럼(esc 로 되짚음)
    // ── Perf 드릴 ──
    pub perf_detail: Option<crate::collect::PerfDetail>, // 선택 모델 지연 분포(Enter 시 온디맨드)
    // ── EPP scorer 가중치 what-if(로컬 시뮬, 클러스터 무변경) ──
    pub epp_weights: HashMap<String, f64>, // scorer 이름 → 조정 가중치 오버라이드
    // ── 세션 에너지(누적 mJ 기준선) ──
    pub energy_base: HashMap<String, f64>, // 디바이스 key → 세션 시작 시 누적 에너지(mJ)
    pub energy_since: u64,                 // 세션 시작 epoch초
}

/// ~/.config/lmd-top/lmd-top.yaml 의 columns: {view: [col,...]} 로드. 없으면 빈 맵(=기본 전체).
fn load_columns() -> HashMap<String, Vec<String>> {
    let v = match crate::config::load_yaml() {
        Some(v) => v,
        None => return HashMap::new(),
    };
    let mut out = HashMap::new();
    if let Some(m) = v.get("columns").and_then(|c| c.as_mapping()) {
        for (k, val) in m {
            if let (Some(view), Some(seq)) = (k.as_str(), val.as_sequence()) {
                let cols: Vec<String> = seq.iter().filter_map(|s| s.as_str().map(|x| x.to_string())).collect();
                if !cols.is_empty() {
                    out.insert(view.to_string(), cols);
                }
            }
        }
    }
    out
}

impl App {
    pub fn new() -> Self {
        App {
            view: View::Overview,
            selected: 0,
            snap: Snapshot::default(),
            hist: HashMap::new(),
            toast: None,
            detail: false,
            sort: 0,
            tick: 0,
            filter: String::new(),
            filtering: false,
            help: false,
            zoom: false,
            paused: false,
            detail_scroll: 0,
            dev_sel: 0,
            models_persp: 0,
            logs_mode: false,
            logs_target: String::new(),
            logs: Vec::new(),
            logs_scroll: 0,
            cols: load_columns(),
            catalog: crate::catalog::load(),
            alerts: VecDeque::new(),
            active_alerts: HashSet::new(),
            alerts_panel: false,
            flash_until: 0,
            toast_until: 0,
            toast_bad: false,
            prev_restarts: HashMap::new(),
            mode: Mode::Observe,
            confirm: None,
            nav_stack: Vec::new(),
            perf_detail: None,
            epp_weights: HashMap::new(),
            energy_base: HashMap::new(),
            energy_since: 0,
        }
    }

    /// 디바이스 안정 key(에너지/히스토리 공용).
    pub fn accel_key(a: &crate::collect::Accel) -> String {
        format!("{}:{}:{}", a.kind.label(), a.node, a.id)
    }
    /// 세션 에너지(Wh) = (현재 누적 − 기준선) mJ / 3.6e6. 데이터 없으면 NaN.
    pub fn energy_session_wh(&self, a: &crate::collect::Accel) -> f64 {
        if a.energy_mj.is_nan() {
            return f64::NAN;
        }
        let base = self.energy_base.get(&Self::accel_key(a)).copied().unwrap_or(a.energy_mj);
        (a.energy_mj - base).max(0.0) / 3.6e6
    }
    /// 세션 에너지 리셋(R) — 현재 누적을 새 기준선으로.
    pub fn reset_energy(&mut self) {
        self.energy_base.clear();
        for a in &self.snap.accel {
            if !a.energy_mj.is_nan() {
                self.energy_base.insert(Self::accel_key(a), a.energy_mj);
            }
        }
        self.energy_since = crate::collect::now_secs();
        self.notify("energy session reset".to_string());
    }

    /// EPP what-if: 선택 scorer 가중치를 delta 만큼 조정(로컬 오버라이드, ≥0).
    pub fn epp_adjust(&mut self, delta: f64) {
        if self.view != View::Epp {
            return;
        }
        let ord = self.order();
        if let (Some(cfg), Some(&i)) = (&self.snap.epp, ord.get(self.selected)) {
            if let Some((name, base)) = cfg.scorers.get(i) {
                let cur = *self.epp_weights.get(name).unwrap_or(base);
                self.epp_weights.insert(name.clone(), (cur + delta).max(0.0));
            }
        }
    }
    /// scorer 유효 가중치(오버라이드 있으면 그것, 없으면 base).
    pub fn epp_weight(&self, name: &str, base: f64) -> f64 {
        *self.epp_weights.get(name).unwrap_or(&base)
    }

    /// Flow(Topo)에서 선택된 route 의 backend(모델)명 — 경로에서 레이어 pivot 용.
    pub fn selected_route_backend(&self) -> Option<String> {
        if self.view == View::Routing {
            self.sel_orig().and_then(|i| self.snap.routes.get(i)).map(|r| r.backend.clone())
        } else {
            None
        }
    }

    /// 선택된 per-model perf 행의 모델(서비스)명 — Perf 드릴용. sel_orig 경유(정렬/필터 안전).
    pub fn selected_perf_model(&self) -> Option<String> {
        if self.view == View::Perf {
            self.sel_orig().and_then(|i| self.snap.perf_rows.get(i)).map(|r| r.model.clone())
        } else {
            None
        }
    }

    /// 파드명 → 소속 모델(배포)명. prefix 매칭, 없으면 파드명 그대로.
    fn model_of_pod(&self, pod: &str) -> String {
        self.snap
            .models
            .iter()
            .find(|m| pod.starts_with(&m.name))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| pod.to_string())
    }

    /// 크로스레이어 드릴: 선택 엔티티에서 관련 레이어로 점프(뷰 전환 + 상관 필터).
    /// 현재 위치를 브레드크럼에 쌓아 esc 로 되짚을 수 있게 함. collector 는 이미 연결돼 있음.
    pub fn pivot(&mut self, key: char) {
        // 뮤터블 차용 충돌 회피 — 선택 엔티티 값을 먼저 뽑는다.
        let model = self.selected_model().map(|m| m.name.clone());
        let accel = self.selected_accel().map(|a| (a.busy_model.clone(), a.node.clone()));
        let pod = self.selected_pod().map(|p| p.name.clone());
        let node = self.selected_node().map(|n| n.name.clone());
        let perf_model = self.selected_perf_model();
        let route_backend = self.selected_route_backend();
        let target: Option<(View, String)> = match self.view {
            View::Routing => route_backend.and_then(|b| match key {
                'p' => Some((View::Pods, b)),
                'i' => Some((View::Accel, b)),
                'm' => Some((View::Models, b)),
                'e' => Some((View::Epp, String::new())),
                _ => None,
            }),
            View::Perf => perf_model.and_then(|name| match key {
                'p' => Some((View::Pods, name)),
                'i' => Some((View::Accel, name)),
                'e' => Some((View::Epp, String::new())),
                _ => None,
            }),
            View::Models | View::Overview => model.and_then(|name| match key {
                'p' => Some((View::Pods, name)),
                'i' => Some((View::Accel, name)),
                'e' => Some((View::Epp, String::new())),
                'r' => Some((View::Routing, String::new())),
                _ => None,
            }),
            View::Accel => accel.filter(|(b, _)| !b.is_empty()).and_then(|(bm, nd)| match key {
                'p' => Some((View::Pods, bm)),
                'm' => Some((View::Models, self.model_of_pod(&bm))),
                'n' => Some((View::Nodes, nd)),
                _ => None,
            }),
            View::Pods => pod.and_then(|pn| match key {
                'i' => Some((View::Accel, pn.clone())),
                'm' => Some((View::Models, self.model_of_pod(&pn))),
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
                self.nav_stack.push(NavState { view: self.view, selected: self.selected, filter: self.filter.clone(), detail: self.detail });
                self.view = v;
                self.filter = filter;
                self.selected = 0;
                self.sort = 0;
                self.detail = false;
                self.epp_weights.clear();
                // 빈 화면 착지 방지: 매칭 0건이면 되짚고 안내(막다른 화면 회피).
                if self.list_len() == 0 {
                    self.nav_back();
                    self.notify(format!("no related items to pivot to ('{}')", key));
                }
            }
            None => {
                // pivot-source 뷰에서 미지원 pivot 키 → 죽은 입력 대신 힌트.
                if "piremn".contains(key) {
                    let hint = match self.view {
                        View::Models | View::Overview => Some("p/i/r/e"),
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

    /// 브레드크럼 되짚기(esc). 되짚었으면 true.
    pub fn nav_back(&mut self) -> bool {
        if let Some(st) = self.nav_stack.pop() {
            self.view = st.view;
            self.selected = st.selected;
            self.filter = st.filter;
            self.detail = st.detail;
            self.sort = 0;
            self.epp_weights.clear();
            true
        } else {
            false
        }
    }

    /// 현재 모드가 required 이상 권한인가(변경 작업 게이트).
    pub fn can(&self, required: Mode) -> bool {
        self.mode >= required
    }

    /// 만료(toast_until)를 가진 토스트 알림 설정 — 액션 피드백/알림 공용.
    pub fn notify(&mut self, msg: String) {
        self.toast = Some(msg);
        self.toast_until = crate::collect::now_secs() + 5;
        self.toast_bad = false;
    }

    pub fn selected_cat(&self) -> Option<&crate::catalog::CatModel> {
        if self.view == View::Launch {
            self.sel_orig().and_then(|i| self.catalog.get(i))
        } else {
            None
        }
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
    fn search_text(&self, i: usize) -> String {
        match self.view {
            View::Accel => self
                .snap
                .accel
                .get(i)
                .map(|a| format!("{} {} {} {}", a.kind.label(), a.id, a.node, a.busy_model))
                .unwrap_or_default(),
            View::Models | View::Overview => self.snap.models.get(i).map(|m| format!("{} {}", m.name, m.accel)).unwrap_or_default(),
            View::Pods => self.snap.pods.get(i).map(|p| format!("{} {}", p.name, p.node)).unwrap_or_default(),
            View::Launch => self.catalog.get(i).map(|m| format!("{} {}", m.id, m.display)).unwrap_or_default(),
            View::Epp => self.snap.epp.as_ref().and_then(|e| e.scorers.get(i)).map(|(n, _)| n.clone()).unwrap_or_default(),
            View::Events => self.snap.events.get(i).map(|e| format!("{} {} {}", e.reason, e.object, e.message)).unwrap_or_default(),
            View::Nodes => self.snap.nodes.get(i).map(|n| n.name.clone()).unwrap_or_default(),
            View::Routing => self.snap.routes.get(i).map(|r| format!("{} {}", r.path, r.backend)).unwrap_or_default(),
            View::Perf => self.snap.perf_rows.get(i).map(|r| r.model.clone()).unwrap_or_default(),
        }
    }

    /// 상세 패널을 가진 뷰인지(detail=true 가 실제로 렌더에 반영되는 뷰).
    /// 없는 뷰(Routing/Epp/Launch/Events)에서 detail=true 로 두면 ↑↓ 가 스크롤로 빠져 네비가 잠김.
    pub fn view_has_detail(&self) -> bool {
        matches!(self.view, View::Accel | View::Models | View::Overview | View::Pods | View::Nodes | View::Events)
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
        self.pivot('m'); // → Models, filter=backend (매칭 0건이면 pivot 이 되짚음)
        if self.view == View::Models && self.list_len() > 0 {
            self.detail = true;
        }
    }

    /// 현재 뷰의 정렬 모드 수(순환용).
    pub fn sort_modes(&self) -> usize {
        match self.view {
            View::Accel => 4,  // util / temp / mem / name
            View::Models => 3, // name / status / ready
            View::Pods => 3,   // name / phase / restarts
            View::Perf => 5,   // tok/s / E2E / TTFT / queue / name
            _ => 1,
        }
    }
    pub fn cycle_sort(&mut self) {
        let n = self.sort_modes();
        self.sort = (self.sort + 1) % n.max(1);
    }
    pub fn sort_label(&self) -> &'static str {
        match (self.view, self.sort) {
            (View::Accel, 0) => "util",
            (View::Accel, 1) => "temp",
            (View::Accel, 2) => "mem",
            (View::Accel, 3) => "name",
            (View::Models, 0) => "name",
            (View::Models, 1) => "status",
            (View::Models, 2) => "ready",
            (View::Pods, 0) => "name",
            (View::Pods, 1) => "phase",
            (View::Pods, 2) => "restarts",
            (View::Perf, 0) => "tok/s",
            (View::Perf, 1) => "E2E",
            (View::Perf, 2) => "TTFT",
            (View::Perf, 3) => "queue",
            (View::Perf, 4) => "name",
            _ => "—",
        }
    }

    fn push_hist(&mut self, key: &str, val: u64) {
        let buf = self.hist.entry(key.to_string()).or_default();
        buf.push_back(val);
        while buf.len() > HIST {
            buf.pop_front();
        }
    }

    /// 새 스냅샷 반영 + ts 가 바뀌었으면 히스토리 append.
    pub fn apply(&mut self, snap: Snapshot) {
        if snap.ts != self.snap.ts {
            // per-accelerator: util / mem% / temp 타임라인
            for a in &snap.accel {
                let k = format!("acc:{}:{}:{}", a.kind.label(), a.node, a.id);
                self.push_hist(&format!("{}:util", k), a.util.round().clamp(0.0, 100.0) as u64);
                let memp = if a.mem_total_gb > 0.0 { a.mem_used_gb / a.mem_total_gb * 100.0 } else { 0.0 };
                self.push_hist(&format!("{}:mem", k), memp.round().clamp(0.0, 100.0) as u64);
                self.push_hist(&format!("{}:temp", k), a.temp.round().max(0.0) as u64);
            }
            // per-node: cpu% / mem% / load
            for n in &snap.nodes {
                let k = format!("nod:{}", n.name);
                if !n.cpu_pct.is_nan() {
                    self.push_hist(&format!("{}:cpu", k), n.cpu_pct.round().clamp(0.0, 100.0) as u64);
                }
                let memp = if n.mem_total_gb > 0.0 { n.mem_used_gb / n.mem_total_gb * 100.0 } else { 0.0 };
                self.push_hist(&format!("{}:mem", k), memp.round().clamp(0.0, 100.0) as u64);
                if n.disk_total_gb > 0.0 {
                    let dp = n.disk_used_gb / n.disk_total_gb * 100.0;
                    self.push_hist(&format!("{}:disk", k), dp.round().clamp(0.0, 100.0) as u64);
                }
                if !n.load1.is_nan() {
                    self.push_hist(&format!("{}:load", k), (n.load1 * 10.0).round().max(0.0) as u64);
                }
            }
            // 클러스터 추이 — 실제 존재하는 가속기 종류만 집계(GPU/RBLN/RNGD 각각)
            let mean = |v: &[f64]| if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 };
            let pct = |u: f64, t: f64| if t > 0.0 { u / t * 100.0 } else { 0.0 };
            let mut byk: std::collections::BTreeMap<&str, (Vec<f64>, f64, f64)> = std::collections::BTreeMap::new();
            for a in &snap.accel {
                let e = byk.entry(a.kind.label()).or_default();
                e.0.push(a.util);
                e.1 += a.mem_used_gb;
                e.2 += a.mem_total_gb;
            }
            for (k, (u, mu, mt)) in &byk {
                self.push_hist(&format!("sys:{}_util", k), mean(u).round().clamp(0.0, 100.0) as u64);
                self.push_hist(&format!("sys:{}_mem", k), pct(*mu, *mt).round().clamp(0.0, 100.0) as u64);
            }
            let cpus: Vec<f64> = snap.nodes.iter().filter(|n| !n.cpu_pct.is_nan()).map(|n| n.cpu_pct).collect();
            if !cpus.is_empty() {
                self.push_hist("sys:cpu", mean(&cpus).round().clamp(0.0, 100.0) as u64);
            }
            let (hmu, hmt): (f64, f64) = snap.nodes.iter().fold((0.0, 0.0), |(u, t), n| (u + n.mem_used_gb, t + n.mem_total_gb));
            self.push_hist("sys:host_mem", pct(hmu, hmt).round().clamp(0.0, 100.0) as u64);
            let tps = snap.perf.tps;
            if !tps.is_nan() {
                self.push_hist("sys:tps", tps.round().max(0.0) as u64);
            }
            // per-model perf 시계열 — Perf/Model 상세의 지표별 타임라인용.
            // 지연은 ms(정수), 처리량은 rate(반올림)로 저장. 값 없으면(NaN) 스킵.
            for r in &snap.perf_rows {
                let k = format!("mperf:{}", r.model);
                let push_ms = |s: &mut Self, sub: &str, v: f64| {
                    if !v.is_nan() {
                        s.push_hist(&format!("{}:{}", k, sub), (v * 1000.0).round().max(0.0) as u64);
                    }
                };
                push_ms(self, "ttft", r.ttft_p95);
                push_ms(self, "tpot", r.tpot_p95);
                push_ms(self, "e2e", r.e2e_p95);
                push_ms(self, "queue", r.queue_p95);
                push_ms(self, "prefill", r.prefill_p95);
                push_ms(self, "decode", r.decode_p95);
                if !r.tps.is_nan() {
                    self.push_hist(&format!("{}:tps", k), r.tps.round().max(0.0) as u64);
                }
                if !r.req.is_nan() {
                    self.push_hist(&format!("{}:req", k), r.req.round().max(0.0) as u64);
                }
            }
            self.detect_alerts(&snap);
            // 세션 에너지 기준선(디바이스 최초 관측 시 캡처).
            for a in &snap.accel {
                if !a.energy_mj.is_nan() {
                    self.energy_base.entry(Self::accel_key(a)).or_insert(a.energy_mj);
                }
            }
            if self.energy_since == 0 {
                self.energy_since = snap.ts;
            }
        }
        self.snap = snap;
        let n = self.list_len();
        if n > 0 && self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// 스냅샷에서 임계 조건을 뽑아 신규 발생분만 히스토리에 쌓고 토스트+플래시 트리거.
    /// key 안정성으로 엣지(비활성→활성)만 알림 — 지속 조건은 반복 토스트하지 않음.
    fn detect_alerts(&mut self, snap: &Snapshot) {
        let now = snap.ts;
        // 상태 없는 조건(가속기/노드/pod-Failed) — JSON 출력과 공유.
        let mut current = snapshot_alerts(snap);
        // pod 재시작 증가(델타) — 이전 스냅샷 필요(stateful).
        for p in &snap.pods {
            let prev = self.prev_restarts.get(&p.name).copied().unwrap_or(p.restarts);
            if p.restarts > prev {
                current.push(Alert { ts: now, sev: Sev::Warn, key: format!("restart:{}:{}", p.name, p.restarts), msg: format!("pod {} restarted (x{})", p.name, p.restarts) });
            }
        }
        self.prev_restarts = snap.pods.iter().map(|p| (p.name.clone(), p.restarts)).collect();

        // 엣지 검출: active_alerts 에 없던 key = 신규.
        let mut new_alerts: Vec<Alert> = Vec::new();
        for a in &current {
            if !self.active_alerts.contains(&a.key) {
                new_alerts.push(a.clone());
            }
        }
        self.active_alerts = current.iter().map(|a| a.key.clone()).collect();
        if new_alerts.is_empty() {
            return;
        }
        // 히스토리 적재(최신 앞, cap 50)
        for a in &new_alerts {
            self.alerts.push_front(a.clone());
        }
        while self.alerts.len() > 50 {
            self.alerts.pop_back();
        }
        // 토스트: 1건이면 메시지, 여러건이면 요약. 하나라도 Bad 면 빨강.
        let any_bad = new_alerts.iter().any(|a| a.sev == Sev::Bad);
        let msg = if new_alerts.len() == 1 {
            let a = &new_alerts[0];
            format!("{} {}", if a.sev == Sev::Bad { "✗" } else { "⚠" }, a.msg)
        } else {
            format!("⚠ {} new alerts — press A", new_alerts.len())
        };
        self.toast = Some(msg);
        self.toast_until = now + 5;
        self.toast_bad = any_bad;
        self.flash_until = now + 3; // 3초 플래시
    }

    pub fn hist_for(&self, key: &str) -> Vec<u64> {
        self.hist
            .get(key)
            .map(|d| d.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn set_view_idx(&mut self, i: usize) {
        if i < View::ALL.len() {
            self.view = View::ALL[i];
            self.selected = 0;
            self.sort = 0;
            self.detail = false;
            self.nav_stack.clear(); // 수동 뷰 전환 → 브레드크럼 초기화
            self.epp_weights.clear(); // what-if 오버라이드는 EPP 떠나면 리셋
        }
    }

    pub fn next_tab(&mut self) {
        self.set_view_idx((self.view.idx() + 1) % View::ALL.len());
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
        self.dev_sel = 0;       // 다른 노드로 이동 → device 커서 요약으로
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

    fn entity_name(&self, i: usize) -> String {
        match self.view {
            View::Accel => self.snap.accel.get(i).map(|a| format!("{} {}", a.kind.label(), a.id)).unwrap_or_default(),
            View::Models | View::Overview => self.snap.models.get(i).map(|m| m.name.clone()).unwrap_or_default(),
            View::Pods => self.snap.pods.get(i).map(|p| p.name.clone()).unwrap_or_default(),
            View::Nodes => self.snap.nodes.get(i).map(|n| n.name.clone()).unwrap_or_default(),
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

    /// 현재 뷰의 표시 순서(정렬 적용된 원본 인덱스 목록). 렌더와 액션이 공유.
    pub fn order(&self) -> Vec<usize> {
        use crate::collect::{Accel, ModelRow, PodRow};
        use std::cmp::Ordering::Equal;
        let mut idx = match self.view {
            View::Accel => {
                let v = &self.snap.accel;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&Accel, &Accel) = (&v[a], &v[b]);
                    match self.sort {
                        0 => y.util.partial_cmp(&x.util).unwrap_or(Equal),
                        1 => y.temp.partial_cmp(&x.temp).unwrap_or(Equal),
                        2 => y.mem_used_gb.partial_cmp(&x.mem_used_gb).unwrap_or(Equal),
                        _ => (x.kind as u8, &x.node, &x.id).cmp(&(y.kind as u8, &y.node, &y.id)),
                    }
                });
                idx
            }
            View::Models | View::Overview => {
                let v = &self.snap.models;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&ModelRow, &ModelRow) = (&v[a], &v[b]);
                    match self.sort {
                        1 => y.status.cmp(&x.status).then(x.name.cmp(&y.name)),
                        2 => y.ready.cmp(&x.ready).then(x.name.cmp(&y.name)),
                        _ => x.name.cmp(&y.name),
                    }
                });
                idx
            }
            View::Pods => {
                let v = &self.snap.pods;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&PodRow, &PodRow) = (&v[a], &v[b]);
                    match self.sort {
                        1 => x.phase.cmp(&y.phase).then(x.name.cmp(&y.name)),
                        2 => y.restarts.cmp(&x.restarts).then(x.name.cmp(&y.name)),
                        _ => x.name.cmp(&y.name),
                    }
                });
                idx
            }
            View::Launch => (0..self.catalog.len()).collect(),
            View::Epp => (0..self.snap.epp.as_ref().map(|e| e.scorers.len()).unwrap_or(0)).collect(),
            View::Events => (0..self.snap.events.len()).collect(),
            View::Nodes => (0..self.snap.nodes.len()).collect(),
            View::Perf => {
                use crate::collect::PerfRow;
                // "지금 켜진 것"만: 서빙 신호(req/tps/지연 중 하나라도 유효)가 있는 모델.
                let v = &self.snap.perf_rows;
                let active = |r: &PerfRow| {
                    (!r.req.is_nan() && r.req > 0.0)
                        || (!r.tps.is_nan() && r.tps > 0.0)
                        || !r.e2e_p95.is_nan()
                        || !r.ttft_p95.is_nan()
                        || !r.queue_p95.is_nan()
                };
                let mut idx: Vec<usize> = (0..v.len()).filter(|&i| active(&v[i])).collect();
                let key = |x: f64| if x.is_nan() { f64::MIN } else { x }; // 값 없는 건 뒤로
                idx.sort_by(|&a, &b| {
                    let (x, y): (&PerfRow, &PerfRow) = (&v[a], &v[b]);
                    match self.sort {
                        1 => key(y.e2e_p95).partial_cmp(&key(x.e2e_p95)).unwrap_or(Equal),
                        2 => key(y.ttft_p95).partial_cmp(&key(x.ttft_p95)).unwrap_or(Equal),
                        3 => key(y.queue_p95).partial_cmp(&key(x.queue_p95)).unwrap_or(Equal),
                        4 => x.model.cmp(&y.model),
                        _ => key(y.tps).partial_cmp(&key(x.tps)).unwrap_or(Equal), // 0=tok/s desc
                    }
                });
                idx
            }
            View::Routing => (0..self.snap.routes.len()).collect(),
        };
        if !self.filter.is_empty() {
            let fl = self.filter.to_lowercase();
            idx.retain(|&i| self.search_text(i).to_lowercase().contains(&fl));
        }
        idx
    }

    /// 표시 순서상 selected 위치 → 원본 인덱스.
    pub fn sel_orig(&self) -> Option<usize> {
        self.order().get(self.selected).copied()
    }

    pub fn selected_model(&self) -> Option<&crate::collect::ModelRow> {
        match self.view {
            View::Models | View::Overview => self.sel_orig().and_then(|i| self.snap.models.get(i)),
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
    /// Models 뷰의 artifacts 관점에서만 — 선택 모델명에 대응하는 아티팩트.
    pub fn selected_artifact(&self) -> Option<&crate::collect::ModelArtifact> {
        if self.view != View::Models || self.models_persp != 1 {
            return None;
        }
        let i = self.sel_orig()?;
        let name = self.snap.models.get(i)?.name.clone();
        self.snap.artifacts.iter().find(|a| a.model == name)
    }
    /// Models 뷰 관점 토글(serving ⇄ artifacts).
    pub fn toggle_models_persp(&mut self) {
        if self.view == View::Models {
            self.models_persp ^= 1;
            self.detail = false;
        }
    }

    /// 로그 대상 pod 이름(현재 선택 기준).
    pub fn logs_target_pod(&self) -> Option<String> {
        match self.view {
            View::Pods => self.selected_pod().map(|p| p.name.clone()),
            View::Models | View::Overview => self
                .selected_model()
                .and_then(|m| self.snap.pods.iter().find(|p| p.name.starts_with(&m.name)).map(|p| p.name.clone())),
            View::Accel => self.selected_accel().filter(|a| !a.busy_model.is_empty()).map(|a| a.busy_model.clone()),
            _ => None,
        }
    }
}

/// 상태 없는 임계/헬스 조건 → 알림 목록(엣지 검출·재시작 델타 제외). UI 알림과 agent JSON 공유.
pub fn snapshot_alerts(snap: &Snapshot) -> Vec<Alert> {
    let now = snap.ts;
    let mut out: Vec<Alert> = Vec::new();
    for a in &snap.accel {
        let base = format!("{}/{}/{}", a.disp(), a.node, a.id);
        if !a.alive {
            out.push(Alert { ts: now, sev: Sev::Bad, key: format!("dead:{}", base), msg: format!("{} {} not alive @{}", a.disp(), a.id, a.node) });
        } else if a.throttle > 0.0 {
            out.push(Alert { ts: now, sev: Sev::Warn, key: format!("thr:{}", base), msg: format!("{} {} throttling @{}", a.disp(), a.id, a.node) });
        }
        if a.temp > ALERT_TEMP_BAD {
            out.push(Alert { ts: now, sev: Sev::Warn, key: format!("temp:{}", base), msg: format!("{} {} hot {:.0}°C @{}", a.disp(), a.id, a.temp, a.node) });
        }
    }
    for n in &snap.nodes {
        if n.cordoned {
            out.push(Alert { ts: now, sev: Sev::Warn, key: format!("cordon:{}", n.name), msg: format!("node {} cordoned", n.name) });
        } else if !n.ready {
            out.push(Alert { ts: now, sev: Sev::Bad, key: format!("notready:{}", n.name), msg: format!("node {} NotReady", n.name) });
        } else if n.pressure {
            out.push(Alert { ts: now, sev: Sev::Warn, key: format!("pressure:{}", n.name), msg: format!("node {} under pressure", n.name) });
        }
        // 루트 디스크 고갈 경보(90% 초과).
        if n.disk_total_gb > 0.0 {
            let dp = n.disk_used_gb / n.disk_total_gb * 100.0;
            if dp > 90.0 {
                out.push(Alert { ts: now, sev: Sev::Warn, key: format!("disk:{}", n.name), msg: format!("node {} disk {:.0}% full", n.name, dp) });
            }
        }
    }
    for p in &snap.pods {
        if p.phase == "Failed" {
            out.push(Alert { ts: now, sev: Sev::Bad, key: format!("failed:{}", p.name), msg: format!("pod {} Failed", p.name) });
        }
    }
    out
}

/// 크로스레이어 1줄 진단 → (문구, 심각도). None = 정상(healthy). UI diagnosis 와 agent JSON 공유.
pub fn diagnose(s: &Snapshot) -> (String, Option<Sev>) {
    let serving = s.models.iter().filter(|m| m.ready > 0).count();
    if s.accel.is_empty() && serving == 0 {
        return ("no accelerator metrics + no serving models — check Prometheus / model state".into(), Some(Sev::Bad));
    }
    if serving == 0 {
        return ("0 models serving — press 's' in Models to start one (no backend)".into(), Some(Sev::Warn));
    }
    let warns = s.events.iter().filter(|e| e.typ == "Warning").count();
    if warns > 0 {
        let top = s.events.iter().find(|e| e.typ == "Warning").map(|e| e.reason.clone()).unwrap_or_default();
        return (format!("{} model(s) serving · {} warning event(s) (top: {}) — see Events", serving, warns, top), Some(Sev::Warn));
    }
    let busy = s.accel.iter().filter(|a| a.util > 80.0).count();
    if busy > 0 {
        return (format!("{} model(s) serving, {} accelerator(s) hot (>80%)", serving, busy), None);
    }
    (format!("{} model(s) serving, accelerators have headroom", serving), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{ModelRow, PodRow, Snapshot};

    fn model(name: &str) -> ModelRow {
        ModelRow {
            name: name.into(), ready: 1, desired: 1, status: "● Running".into(), route: "/x".into(),
            engine: "vllm".into(), accel: "-".into(), running: None, waiting: None, tps: None, kv: None, ttft: None,
        }
    }
    fn pod(name: &str) -> PodRow {
        PodRow { name: name.into(), phase: "Running".into(), ready: "1/1".into(), node: "n1".into(), restarts: 0 }
    }
    fn app_with(models: Vec<ModelRow>, pods: Vec<PodRow>) -> App {
        let mut a = App::new();
        a.snap = Snapshot { models, pods, ..Default::default() };
        a.view = View::Models;
        a
    }

    #[test]
    fn pivot_roundtrip_restores_state() {
        let mut a = app_with(vec![model("m1")], vec![pod("m1-abc")]);
        a.pivot('p'); // Models → Pods filtered by m1
        assert_eq!(a.view, View::Pods);
        assert_eq!(a.filter, "m1");
        assert!(a.nav_back());
        assert_eq!(a.view, View::Models);
        assert_eq!(a.filter, "");
        assert_eq!(a.selected, 0);
        assert!(a.nav_stack.is_empty());
    }

    #[test]
    fn pivot_empty_landing_reverts() {
        // 매칭되는 pod 없음 → 막다른 빈 화면 대신 되짚어야 함
        let mut a = app_with(vec![model("lonely")], vec![]);
        a.pivot('p');
        assert_eq!(a.view, View::Models);
        assert_eq!(a.filter, "");
        assert!(a.nav_stack.is_empty());
    }

    #[test]
    fn unsupported_pivot_key_is_noop() {
        let mut a = app_with(vec![model("m1")], vec![pod("m1-abc")]);
        a.pivot('x'); // pivot 키 아님
        assert_eq!(a.view, View::Models);
        assert!(a.nav_stack.is_empty());
    }

    #[test]
    fn flow_route_enter_does_not_panic() {
        use crate::collect::Route;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut a = App::new();
        a.snap = Snapshot {
            models: vec![model("m1")],
            pods: vec![pod("m1-abc")],
            routes: vec![Route { path: "/v1".into(), backend: "m1".into(), kind: "InferencePool".into() }],
            ..Default::default()
        };
        a.view = View::Routing;
        a.selected = 0;
        // toggle_detail 은 상세 없는 Routing 에선 detail 을 켜지 않음(↑↓ 네비 잠김 방지).
        a.toggle_detail();
        assert!(!a.detail, "Routing has no detail panel; detail must stay off so nav is not trapped");
        // Enter 의 실제 동작: 백엔드 모델 상세로 드릴.
        a.drill_route();
        assert_eq!(a.view, View::Models);
        assert_eq!(a.filter, "m1");
        assert!(a.detail);
        // esc: 상세 닫기 → nav_back 으로 Flow 복귀.
        a.detail = false;
        assert!(a.nav_back());
        assert_eq!(a.view, View::Routing);
        let mut fx = crate::ui::FxState::disabled();
        for (w, h) in [(80u16, 24u16), (120, 48), (40, 16)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        }
    }
}
