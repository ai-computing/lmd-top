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
    Stop { name: String }, // 서빙 중지 = replicas 0 으로(디바이스 반환, 되돌릴 수 있음)
    Apply { title: String, yaml: String }, // 미리보기 매니페스트를 실제 kubectl apply
    Cordon { node: String, on: bool }, // 노드 스케줄 차단/해제
    DeletePod { name: String },        // 파드 삭제
}
impl Pending {
    /// 확인 프롬프트 문구.
    pub fn prompt(&self) -> String {
        match self {
            Pending::Scale { name, target } => format!("scale {} → {} replica(s)?", name, target),
            Pending::Restart { name } => format!("rollout restart {} (rolling)?", name),
            Pending::Stop { name } => format!("stop serving {} (scale → 0, frees devices)?", name),
            Pending::Apply { title, .. } => format!("apply manifest to cluster — {}?", title),
            Pending::Cordon { node, on } => format!("{} node {}?", if *on { "cordon (block scheduling on)" } else { "uncordon (allow scheduling on)" }, node),
            Pending::DeletePod { name } => format!("delete pod {} (reschedules)?", name),
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

// 기능 타입(CompileForm/DeployForm/Action/Objective/Fit 등)은 crate::ops 로 분리, 재수출.
pub use crate::ops::*;


/// 알림 임계치(ui.rs 의 색 임계치와 개념 일치 — 여기선 "경보" 발생선).
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
    Topo, // Nodes 허브의 토폴로지/디바이스 pressure 맵(Canvas)
}

impl View {
    /// 탭 순서(숫자키 0-7). Accel·Perf·Topo 는 Nodes 허브의 하위 뷰라 여기 없음 — 슬롯 1(Nodes)에 통합.
    pub const ALL: [View; 8] = [
        View::Overview,
        View::Nodes,
        View::Models,
        View::Epp,
        View::Routing,
        View::Pods,
        View::Launch,
        View::Events,
    ];
    /// Nodes 허브의 하위 뷰 순환 순서(w 로 전환) — 노드 → 디바이스 → 서빙 성능 → pressure 맵.
    pub const HUB: [View; 4] = [View::Nodes, View::Accel, View::Perf, View::Topo];
    pub fn is_hub(&self) -> bool {
        matches!(self, View::Nodes | View::Accel | View::Perf | View::Topo)
    }
    pub fn idx(&self) -> usize {
        // 허브 하위 뷰(Accel/Perf)는 Nodes 탭 슬롯으로 취급.
        let anchor = if self.is_hub() { &View::Nodes } else { self };
        View::ALL.iter().position(|v| v == anchor).unwrap_or(0)
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
            View::Launch => "Deploy", // 모델 라이프사이클(컴파일 변형·저장 노드·배치 타깃)
            View::Events => "Events",
            View::Nodes => "Nodes",
            View::Topo => "Topology",
        }
    }
}

pub const HIST: usize = 40;

pub struct App {
    pub view: View,
    pub selected: usize,
    pub snap: Snapshot,
    // ── 배포/컴파일 대상 컨텍스트(매니페스트 생성·apply 에 사용) ──
    pub ns: String,                  // 대상 네임스페이스(cfg.ns) — 하드코딩 대신 주입
    pub img_rbln: Option<String>,    // LMD_COMPILE_IMAGE_RBLN — 없으면 placeholder
    pub img_furiosa: Option<String>, // LMD_COMPILE_IMAGE_FURIOSA — 기본 furiosaai/furiosa-llm:latest
    pub img_serving: Option<String>, // LMD_SERVING_IMAGE — 없으면 placeholder
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
    pub panel_focus: usize, // 멀티패널 뷰에서 활성(포커스) 패널 인덱스(w 로 순환)
    pub preview: Option<(String, String)>, // (제목, YAML) 미리보기 오버레이(생성 매니페스트 또는 읽기전용 YAML)
    pub preview_scroll: u16,
    pub preview_apply: bool, // true=생성 매니페스트(v 검증·a 적용 가능), false=읽기전용(describe/yaml)
    pub compile_form: Option<CompileForm>, // NPU 컴파일 옵션 편집 폼(c → 편집 → Enter → preview)
    pub deploy_form: Option<DeployForm>,   // 배포(서빙) 옵션 편집 폼(d → 편집 → Enter → preview)
    pub action_menu: Option<ActionMenu>,   // Enter 컨텍스트 액션 메뉴(Info/Compile/Deploy/Stop…)
    pub objectives: HashMap<String, Objective>, // 모델별 서빙 목표(SLO) — 사용자 입력
    pub objective_form: Option<ObjectiveForm>,  // 목표 편집 폼
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
        let env = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
        App {
            view: View::Overview,
            selected: 0,
            snap: Snapshot::default(),
            ns: "llm-serving".into(),
            img_rbln: env("LMD_COMPILE_IMAGE_RBLN"),
            img_furiosa: env("LMD_COMPILE_IMAGE_FURIOSA").or_else(|| Some("furiosaai/furiosa-llm:latest".into())),
            img_serving: env("LMD_SERVING_IMAGE"),
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
            panel_focus: 0,
            preview: None,
            preview_scroll: 0,
            preview_apply: false,
            compile_form: None,
            deploy_form: None,
            action_menu: None,
            objectives: HashMap::new(),
            objective_form: None,
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
        if self.view != View::Epp || self.panel_focus != 0 {
            return; // scorers 패널에 포커스일 때만
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
        if self.view == View::Perf && self.panel_focus == 0 {
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
            View::Launch => match self.panel_focus {
                1 => self.target_nodes().get(i).cloned().unwrap_or_default(),
                2 => self.catalog.get(i).map(|m| format!("{} {}", m.id, m.role)).unwrap_or_default(),
                _ => self.snap.artifacts.get(i).map(|a| format!("{} {} {}", a.model, a.family, a.source)).unwrap_or_default(),
            },
            View::Epp if self.panel_focus == 1 => self.snap.pools.get(i).map(|p| p.name.clone()).unwrap_or_default(),
            View::Epp => self.snap.epp.as_ref().and_then(|e| e.scorers.get(i)).map(|(n, _)| n.clone()).unwrap_or_default(),
            View::Events => self.snap.events.get(i).map(|e| format!("{} {} {}", e.reason, e.object, e.message)).unwrap_or_default(),
            View::Nodes => self.snap.nodes.get(i).map(|n| n.name.clone()).unwrap_or_default(),
            View::Routing if self.panel_focus == 1 => self.snap.pools.get(i).map(|p| p.name.clone()).unwrap_or_default(),
            View::Routing => self.snap.routes.get(i).map(|r| format!("{} {}", r.path, r.backend)).unwrap_or_default(),
            View::Perf => self.snap.perf_rows.get(i).map(|r| r.model.clone()).unwrap_or_default(),
            View::Topo => String::new(), // 맵 뷰 — 리스트 선택 없음
        }
    }

    /// 상세 패널을 가진 뷰인지(detail=true 가 실제로 렌더에 반영되는 뷰).
    /// 없는 뷰(Routing/Epp/Launch/Events)에서 detail=true 로 두면 ↑↓ 가 스크롤로 빠져 네비가 잠김.
    pub fn view_has_detail(&self) -> bool {
        matches!(self.view, View::Accel | View::Models | View::Overview | View::Pods | View::Nodes | View::Events | View::Launch)
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
            self.panel_focus = 0; // 뷰 바뀌면 첫 패널로
            self.nav_stack.clear(); // 수동 뷰 전환 → 브레드크럼 초기화
            self.epp_weights.clear(); // what-if 오버라이드는 EPP 떠나면 리셋
        }
    }

    pub fn next_tab(&mut self) {
        self.set_view_idx((self.view.idx() + 1) % View::ALL.len());
    }
    pub fn prev_tab(&mut self) {
        let n = View::ALL.len();
        self.set_view_idx((self.view.idx() + n - 1) % n);
    }
    /// 현재 뷰의 포커스 가능한 패널 수(멀티패널 뷰만 >1).
    pub fn panel_count(&self) -> usize {
        match self.view {
            View::Launch => 3,  // Deploy: 컴파일 변형 / 배치 타깃 / 카탈로그
            View::Epp => 2,     // scorers / InferencePool
            View::Routing => 2, // routes / InferencePool
            _ => 1,
        }
    }
    /// w 키 — 허브(Nodes/Accel/Perf)에선 하위 뷰 전환, 그 외엔 멀티패널 포커스 순환.
    pub fn cycle_panel(&mut self) {
        if self.view.is_hub() {
            let cur = View::HUB.iter().position(|v| *v == self.view).unwrap_or(0);
            self.view = View::HUB[(cur + 1) % View::HUB.len()];
            self.selected = 0;
            self.detail = false;
            self.panel_focus = 0;
            self.dev_sel = 0;
            return;
        }
        let n = self.panel_count();
        if n > 1 {
            self.panel_focus = (self.panel_focus + 1) % n;
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
                2 => key(y.ttft_p95).partial_cmp(&key(x.ttft_p95)).unwrap_or(Equal),
                3 => key(y.queue_p95).partial_cmp(&key(x.queue_p95)).unwrap_or(Equal),
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
            View::Launch => {
                // Deploy: 활성 패널에 따라 선택 리스트가 다름(0 변형 / 1 타깃노드 / 2 카탈로그).
                let n = match self.panel_focus {
                    1 => self.target_nodes().len(),
                    2 => self.catalog.len(),
                    _ => self.snap.artifacts.len(),
                };
                (0..n).collect()
            }
            View::Epp if self.panel_focus == 1 => (0..self.snap.pools.len()).collect(),
            View::Epp => (0..self.snap.epp.as_ref().map(|e| e.scorers.len()).unwrap_or(0)).collect(),
            View::Events => (0..self.snap.events.len()).collect(),
            View::Nodes => (0..self.snap.nodes.len()).collect(),
            View::Perf if self.panel_focus == 1 => (0..self.snap.pod_queues.len()).collect(),
            View::Perf => self.perf_rows_order(),
            View::Routing if self.panel_focus == 1 => (0..self.snap.pools.len()).collect(),
            View::Routing => (0..self.snap.routes.len()).collect(),
            View::Topo => Vec::new(), // 맵 뷰 — 리스트 선택 없음
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

    /// 현재 뷰의 (필터된/전체) 행 집계 요약 — Overview 처럼 통합 값을 함께 보이려는 용도.
    /// 필터가 있으면 보이는 행만, 없으면 전체. 없으면 None.
    pub fn agg_summary(&self) -> Option<String> {
        use crate::collect::{Accel, ModelRow, NodeInfo, PerfRow};
        let order = self.order();
        if order.is_empty() {
            return None;
        }
        let scope = if self.filter.is_empty() { "all" } else { "filt" };
        let n = order.len();
        match self.view {
            View::Accel => {
                let d: Vec<&Accel> = order.iter().filter_map(|&i| self.snap.accel.get(i)).collect();
                if d.is_empty() {
                    return None;
                }
                let util = d.iter().map(|x| x.util).sum::<f64>() / d.len() as f64;
                let mu: f64 = d.iter().map(|x| x.mem_used_gb).sum();
                let mt: f64 = d.iter().map(|x| x.mem_total_gb).sum();
                let pw: f64 = d.iter().map(|x| x.power).sum();
                let busy = d.iter().filter(|x| !x.busy_model.is_empty()).count();
                Some(format!("Σ{} {}dev · {}busy · util {:.0}% · VRAM {:.0}/{:.0}G · {:.0}W", scope, n, busy, util, mu, mt, pw))
            }
            View::Models | View::Overview => {
                let m: Vec<&ModelRow> = order.iter().filter_map(|&i| self.snap.models.get(i)).collect();
                if m.is_empty() {
                    return None;
                }
                let ready: i64 = m.iter().map(|x| x.ready).sum();
                let desired: i64 = m.iter().map(|x| x.desired).sum();
                let run: f64 = m.iter().filter_map(|x| x.running).sum();
                let wait: f64 = m.iter().filter_map(|x| x.waiting).sum();
                let tps: f64 = m.iter().filter_map(|x| x.tps).sum();
                Some(format!("Σ{} {}mdl · {}/{}ready · run {:.0} wait {:.0} · {:.0}tok/s", scope, n, ready, desired, run, wait, tps))
            }
            View::Nodes => {
                let nn: Vec<&NodeInfo> = order.iter().filter_map(|&i| self.snap.nodes.get(i)).collect();
                if nn.is_empty() {
                    return None;
                }
                let cpu = nn.iter().map(|x| x.cpu_pct).sum::<f64>() / nn.len() as f64;
                let mu: f64 = nn.iter().map(|x| x.mem_used_gb).sum();
                let mt: f64 = nn.iter().map(|x| x.mem_total_gb).sum();
                let du: f64 = nn.iter().map(|x| x.disk_used_gb).sum();
                let dt: f64 = nn.iter().map(|x| x.disk_total_gb).sum();
                let ready = nn.iter().filter(|x| x.ready).count();
                Some(format!("Σ{} {}node · {}ready · CPU {:.0}% · mem {:.0}/{:.0}G · disk {:.1}/{:.1}T", scope, n, ready, cpu, mu, mt, du / 1024.0, dt / 1024.0))
            }
            View::Perf => {
                let p: Vec<&PerfRow> = order.iter().filter_map(|&i| self.snap.perf_rows.get(i)).collect();
                if p.is_empty() {
                    return None;
                }
                let tps: f64 = p.iter().filter_map(|x| if x.tps.is_nan() { None } else { Some(x.tps) }).sum();
                let e2e: Vec<f64> = p.iter().map(|x| x.e2e_p95).filter(|v| !v.is_nan()).collect();
                let e2e_avg = if e2e.is_empty() { f64::NAN } else { e2e.iter().sum::<f64>() / e2e.len() as f64 };
                let e2e_s = if e2e_avg.is_nan() { "–".to_string() } else { format!("{:.0}ms", e2e_avg * 1000.0) };
                Some(format!("Σ{} {}active · E2E p95 {} · {:.0}tok/s", scope, n, e2e_s, tps))
            }
            _ => None,
        }
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
    /// `y` — 현재 선택의 live YAML 조회 대상 (kind, namespaced?, name). 없으면 None.
    pub fn yaml_target(&self) -> Option<(&'static str, bool, String)> {
        match self.view {
            View::Models | View::Overview => self.selected_model().map(|m| ("deployment", true, m.name.clone())),
            View::Pods => self.selected_pod().map(|p| ("pod", true, p.name.clone())),
            View::Nodes => self.selected_node().map(|n| ("node", false, n.name.clone())),
            View::Launch if self.panel_focus == 0 => self.selected_artifact().map(|a| ("deployment", true, a.model.clone())),
            View::Launch if self.panel_focus == 1 => {
                let nodes = self.target_nodes();
                self.sel_orig().and_then(|i| nodes.get(i).cloned()).map(|n| ("node", false, n))
            }
            _ => None,
        }
    }

    /// 노드 상세로 피벗 — Nodes 뷰의 해당 노드 detail 로 점프(브레드크럼 push, esc 로 복귀).
    pub fn pivot_to_node(&mut self, node: &str) {
        if let Some(pos) = self.snap.nodes.iter().position(|n| n.name == node) {
            self.nav_stack.push(NavState { view: self.view, selected: self.selected, filter: self.filter.clone(), detail: self.detail });
            self.view = View::Nodes;
            self.filter.clear();
            self.panel_focus = 0;
            self.sort = 0;
            self.selected = pos;
            self.detail = true;
            self.dev_sel = 0;
        } else {
            self.notify(format!("node {} not found in node list", node));
        }
    }

    /// 카탈로그 모델 feasibility 요약(placement 별 ready/needs-artifact/미지원).
    pub fn catalog_feasibility(&self, id: &str) -> String {
        let Some(m) = self.catalog.iter().find(|c| c.id == id) else { return format!("{}: not in catalog", id) };
        let mut ready = 0;
        let mut needs = 0;
        let mut no = 0;
        for p in &m.placements {
            match crate::catalog::solve(p, &self.snap.inventory).0 {
                crate::catalog::Ready::Ready => ready += 1,
                crate::catalog::Ready::NeedsArtifact => needs += 1,
                _ => no += 1,
            }
        }
        format!("{}: {} ready · {} needs-compile · {} unsupported placement(s)", id, ready, needs, no)
    }

    /// Deploy 뷰의 배치 타깃 후보 노드(가속기를 가진 노드, 정렬·중복제거). order()/뷰 공용.
    pub fn target_nodes(&self) -> Vec<String> {
        let mut v: Vec<String> = self.snap.accel.iter().map(|a| a.node.clone()).collect();
        v.sort();
        v.dedup();
        v
    }
    /// Deploy 뷰 '변형' 패널(focus 0)에서 선택된 아티팩트.
    pub fn selected_artifact(&self) -> Option<&crate::collect::ModelArtifact> {
        if self.view == View::Launch && self.panel_focus == 0 {
            self.sel_orig().and_then(|i| self.snap.artifacts.get(i))
        } else {
            None
        }
    }

    /// 아티팩트의 모델 식별자 — HF id(source) 우선, 없으면 family.
    fn artifact_model_id(a: &crate::collect::ModelArtifact) -> String {
        if a.source.contains('/') && !a.source.starts_with('/') {
            a.source.clone()
        } else {
            a.family.clone()
        }
    }
    fn opt_or<'a>(a: &'a crate::collect::ModelArtifact, k: &str, def: &'a str) -> String {
        a.opts.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone()).unwrap_or_else(|| def.to_string())
    }

    /// `[c] compile` — 선택 빌드의 NPU 컴파일 폼. 엔진이 NPU 면 그 벤더로,
    /// GPU/HF 라도 [[npu-compat]] 지원 목록에 있으면 해당 벤더로 컴파일 가능(GPU→NPU 경로).
    pub fn compile_preview(&mut self) {
        let Some(a) = self.selected_artifact() else { return };
        let model_id = Self::artifact_model_id(a);
        let vendor: Option<&'static str> = if a.engine.contains("RBLN") {
            Some("rbln")
        } else if a.engine.contains("Furiosa") {
            Some("furiosa")
        } else {
            crate::compat::compilable_vendors(&model_id).first().copied()
        };
        match vendor {
            Some(v) => {
                let form = self.build_compile_form(a, v);
                self.compile_form = Some(form);
            }
            None => {
                self.preview = Some((
                    format!("compile · {}", a.model),
                    format!(
                        "# {}\n# 이 모델 계열은 NPU 컴파일 지원 목록에 없습니다(RBLN/Furiosa).\n# 지원 계열: Llama·Qwen2/3·Gemma·Mistral·EXAONE·Phi·OPT·GPT2·SOLAR·DeepSeek·T5 …\n# 목록: src/npu-compat.json (벤더 공식 문서 기반)\n",
                        model_id
                    ),
                ));
                self.preview_scroll = 0;
                self.preview_apply = false;
            }
        }
    }

    /// 특정 벤더로 컴파일 폼 열기(액션 메뉴의 'Compile → RBLN/Furiosa'용).
    pub fn compile_form_for(&mut self, vendor: &'static str) {
        let Some(a) = self.selected_artifact() else { return };
        let form = self.build_compile_form(a, vendor);
        self.compile_form = Some(form);
    }

    /// 선택 아티팩트를 주어진 벤더로 컴파일하는 옵션 폼 구성(순수). 초기값은 관측 opts.
    fn build_compile_form(&self, a: &crate::collect::ModelArtifact, vendor: &'static str) -> CompileForm {
        let rbln = vendor == "rbln";
        let model_id = Self::artifact_model_id(a);
        let mkf = |key: &str, label: &str, def: &str, choices: &[&str], numeric: bool, help: &str| CompileField {
            key: key.into(),
            label: label.into(),
            value: Self::opt_or(a, key, def),
            choices: choices.iter().map(|s| s.to_string()).collect(),
            numeric,
            help: help.into(),
        };
        // 파라미터는 컴파일 타임 고정 — RBLN=optimum-rbln config, Furiosa=furiosa-llm build.
        // (참고: 심층 탐색/자동튜닝은 npu_aitune 프레임워크가 담당. 여기선 단발 컴파일 Job.)
        let mut fields = if rbln {
            // RBLNDecoderOnlyModelForCausalLMConfig — RBLN-CA22 는 칩 4장이라 TP≤4.
            vec![
                mkf("tp", "tensor-parallel", "4", &["1", "2", "4"], true, "텐서 병렬 = 사용할 RBLN 칩 수 (rbln_tensor_parallel_size). CA22 최대 4"),
                mkf("max-len", "max-seq-len", "8192", &["2048", "4096", "8192", "16384", "32768"], true, "컴파일 최대 컨텍스트 길이 (rbln_max_seq_len) — 크면 메모리·컴파일시간↑"),
                mkf("batch", "batch-size", "1", &["1", "2", "4", "8", "16"], true, "정적 배치 크기 (rbln_batch_size) — RBLN 은 컴파일 시 고정"),
                mkf("attn", "attn-impl", "flash_attn", &["flash_attn", "eager"], false, "어텐션 구현 — flash_attn(SRAM 최적) / eager(PagedAttention)"),
                mkf("kvpart", "kvcache-partition", "16384", &["4096", "8192", "16384", "32768"], true, "flash_attn 전용 SRAM 파티션당 KV 토큰(2의 거듭제곱). 크면 처리량↑·SRAM 압박↑"),
                mkf("quant", "quantization", "none", &["none", "w8a8", "w4a16"], false, "가중치/활성 양자화 포맷(RBLNQuantizationConfig) — 지원 모델 한정"),
                mkf("npu", "npu-chip", "RBLN-CA22", &["RBLN-CA22"], false, "대상 RBLN 칩 (rbln_npu) — 클러스터 감지값"),
            ]
        } else {
            // furiosa-llm ArtifactBuilder — RNGD 단일 칩 = 8 PE(full=8, half=4).
            vec![
                mkf("tp", "tensor-parallel", "8", &["4", "8"], true, "텐서 병렬 크기(PE 수). RNGD full=8, half=4"),
                mkf("pp", "pipeline-parallel", "1", &["1", "2"], true, "파이프라인 병렬 스테이지 수 (ParallelConfig)"),
                mkf("max-len", "max-seq-len", "8192", &["2048", "4096", "8192", "16384"], true, "max_seq_len_to_capture — 이 이상 버킷 제외"),
                mkf("batch", "batch-size", "1", &["1", "2", "4", "8"], true, "prefill/decode 버킷 배치 크기(BucketConfig)"),
                mkf("chunk", "prefill-chunk", "none", &["none", "512", "1024", "2048"], true, "chunked prefill 청크 크기(prefill_chunk_size)"),
                mkf("block", "kv-block-size", "16", &["16", "32"], true, "PagedAttention 블록당 토큰(paged_attention_block_size)"),
                mkf("quant", "activation-dq", "none", &["none", "on"], false, "use_activation_dq — 활성 동적 양자화(메모리↓·처리량↑)"),
            ]
        };
        // ── 인프라 배치: 실제 디바이스 수·노드는 스냅샷에서 후보를 뽑아 선택 가능하게 ──
        let want_kind = if rbln { crate::collect::AccelKind::Rbln } else { crate::collect::AccelKind::Rngd };
        // 노드별 동종 디바이스 개수.
        let mut per_node: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for ac in self.snap.accel.iter().filter(|x| x.kind == want_kind && !x.node.is_empty()) {
            *per_node.entry(ac.node.clone()).or_insert(0) += 1;
        }
        let mut cand_nodes: Vec<String> = per_node.keys().cloned().collect();
        cand_nodes.sort();
        let max_dev = per_node.values().copied().max().unwrap_or(4).max(1);
        // 컴파일에 필요한 디바이스 기본값: RBLN=TP, Furiosa=chips≈ceil(TP/8)*PP.
        let tp_v = fields.iter().find(|f| f.key == "tp").and_then(|f| f.value.parse::<i64>().ok()).unwrap_or(1).max(1);
        let pp_v = fields.iter().find(|f| f.key == "pp").and_then(|f| f.value.parse::<i64>().ok()).unwrap_or(1).max(1);
        let dev_default = if rbln { tp_v } else { ((tp_v as f64 / 8.0).ceil() as i64).max(1) * pp_v };
        let dev_choices: Vec<String> = (1..=max_dev.max(dev_default)).map(|i| i.to_string()).collect();
        fields.push(CompileField {
            key: "devices".into(),
            label: "devices".into(),
            value: dev_default.to_string(),
            choices: dev_choices,
            numeric: true,
            help: "요청할 NPU 디바이스 수(resources.limits). 보통 = TP(RBLN) / ceil(TP/8)×PP(Furiosa)".into(),
        });
        // 노드 선택지에 그 노드의 NPU 드라이버/SDK 요약을 붙임(컴파일은 드라이버 설치 노드에서만 가능).
        let node_drv = |n: &str| -> String {
            self.snap.nodes.iter().find(|x| x.name == n).map(|x| x.npu.clone()).filter(|s| !s.is_empty()).map(|s| format!(" {}", s)).unwrap_or_default()
        };
        let mut node_choices = vec!["any".to_string()];
        node_choices.extend(cand_nodes.iter().map(|n| format!("{}({}){}", n, per_node[n], node_drv(n))));
        fields.push(CompileField {
            key: "node".into(),
            label: "target-node".into(),
            value: "any".into(),
            choices: node_choices,
            numeric: false,
            help: "컴파일 실행 노드(NPU 드라이버 설치 노드만). any=제품 라벨 매칭. 괄호=디바이스 수, 뒤=드라이버 버전".into(),
        });
        CompileForm {
            model: a.model.clone(),
            model_id,
            vendor,
            engine: a.engine.clone(),
            fields,
            cursor: 0,
            editing: false,
        }
    }

    /// 컴파일 폼 → 매니페스트 미리보기(dry-run) 생성. Enter 시 호출. 폼 값을 env·OUTPUT 에 반영.
    pub fn compile_form_submit(&mut self) {
        let Some(form) = self.compile_form.take() else { return };
        let model_id = &form.model_id;
        let vendor = form.vendor;
        let target = form.target();
        let repo_dir = model_id.replace('/', "--");
        let name = format!("compile-{}", repo_dir.to_lowercase().replace(['.', '_'], "-"));
        let tp = form.get("tp");
        // 디바이스 수·노드는 폼에서 선택한 값. 노드 라벨의 "(N)" 접미는 제거.
        let devices = {
            let d = form.get("devices");
            if d.is_empty() { tp.clone() } else { d }
        };
        let node_pick = form.get("node");
        let node_host = node_pick.split('(').next().unwrap_or("any").trim().to_string();
        // 클러스터 감지값 기반 nodeSelector·디바이스 resource. 이미지는 env 설정값(없으면 placeholder).
        let (res_key, product_label, image) = if vendor == "rbln" {
            let img = self.img_rbln.clone().unwrap_or_else(|| "TODO-rbln-compiler-image".into());
            ("rebellions.ai/ATOM", "rebellions.ai/npu.product: RBLN-CA22", img)
        } else {
            let img = self.img_furiosa.clone().unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into());
            ("furiosa.ai/rngd", "furiosa.ai/npu.product: rngd", img)
        };
        let res_qty = devices;
        // 노드 지정: any=제품 라벨 매칭, 특정=hostname 고정.
        let node_label = if node_host == "any" || node_host.is_empty() {
            product_label.to_string()
        } else {
            format!("kubernetes.io/hostname: {}", node_host)
        };
        // 폼 값을 스크립트가 읽는 env 로 — 벤더별 파라미터 이름 대응.
        let mut envs: Vec<(String, String)> = vec![
            ("MODEL_STORE".into(), "/mnt/store".into()),
            ("MODEL_ID".into(), model_id.clone()),
            ("OUTPUT".into(), format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target)),
        ];
        for f in &form.fields {
            if f.value.is_empty() || f.value == "none" {
                continue;
            }
            let ek = match (vendor, f.key.as_str()) {
                ("rbln", "tp") => "RBLN_TENSOR_PARALLEL_SIZE",
                ("rbln", "max-len") => "RBLN_MAX_SEQ_LEN",
                ("rbln", "batch") => "RBLN_BATCH_SIZE",
                ("rbln", "attn") => "RBLN_ATTN_IMPL",
                ("rbln", "kvpart") => "RBLN_KVCACHE_PARTITION_LEN",
                ("rbln", "npu") => "RBLN_NPU",
                ("rbln", "quant") => "RBLN_QUANTIZATION",
                (_, "tp") => "TENSOR_PARALLEL_SIZE",
                (_, "pp") => "PIPELINE_PARALLEL_SIZE",
                (_, "max-len") => "MAX_SEQ_LEN_TO_CAPTURE",
                (_, "batch") => "BUCKET_BATCH_SIZE",
                (_, "chunk") => "PREFILL_CHUNK_SIZE",
                (_, "block") => "PAGED_ATTENTION_BLOCK_SIZE",
                (_, "quant") => "USE_ACTIVATION_DQ",
                _ => continue,
            };
            envs.push((ek.into(), f.value.clone()));
        }
        let opts_summary: String = form
            .fields
            .iter()
            .map(|f| format!("{}={}", f.key, f.value))
            .collect::<Vec<_>>()
            .join("  ");
        let outdir = format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target);
        // 벤더별 컨테이너: Furiosa 는 이미지에 든 `fxb build` 를 직접 호출(스크립트 불필요, 바로 실행 가능).
        // RBLN 은 optimum-rbln 커스텀 스크립트(compile-script ConfigMap)를 env 로 구동.
        // 벤더별 컨테이너: (volumes_extra, env_block, mounts_extra, command, note, extra_doc).
        // extra_doc = Job 앞에 붙는 별도 YAML 문서(RBLN 은 인라인 컴파일 스크립트 ConfigMap).
        let (volumes_extra, env_block, mounts_extra, command, note, extra_doc) = if vendor == "furiosa" {
            let pp = { let p = form.get("pp"); if p.is_empty() { "1".into() } else { p } };
            let ml = { let m = form.get("max-len"); if m.is_empty() { "8192".into() } else { m } };
            // 실기 검증 반영:
            //  - RNGD 는 ARM64 제어 프로세서라 EDF 최종 코드젠에 aarch64 크로스컴파일러 필요(gcc-aarch64-linux-gnu).
            //    furiosa-llm serve 이미지엔 없어 apt 로 설치(또는 build-complete 이미지 사용).
            //  - SMB 스토어는 컴파일 작업 I/O(mmap 등) 미지원(os error 95) → 로컬 emptyDir 에 빌드 후 스토어로 복사.
            //  - fxb 는 .fxb 아카이브 → {target}/model.fxb 로 디렉터리 안에 두어 discovery 레이아웃 유지.
            let cmd = format!(
                "set -e; apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq gcc-aarch64-linux-gnu build-essential >/dev/null 2>&1; \
                 mkdir -p /work/out; fxb build {model_id} /work/out/model -tp {tp} -pp {pp} --max-model-len {ml} --concurrency 8; \
                 mkdir -p {outdir}; cp -r /work/out/. {outdir}/; echo COMPILE_DONE; ls -la {outdir}",
                outdir = outdir, model_id = model_id, tp = tp, pp = pp, ml = ml
            );
            (
                "        - { name: work, emptyDir: {} }\n".to_string(),
                "             \x20           - { name: HF_HOME, value: /work/hub }\n".to_string(),
                "            - { name: work, mountPath: /work }\n".to_string(),
                format!("[\"sh\", \"-c\", \"{}\"]", cmd),
                "# Furiosa: fxb build 직접(레지스트리=furiosa-ai 양자화 체크포인트). aarch64 xcc 설치+로컬빌드→스토어복사.",
                String::new(),
            )
        } else {
            // RBLN: optimum-rbln 인라인 스크립트를 ConfigMap 으로 동봉(외부 의존 없음).
            //  - rbln_create_runtimes=False: 디바이스에 런타임을 올리지 않고 컴파일만 → 서빙이 칩을
            //    점유 중이어도 성공(실기 확인: create_runtimes=True 면 "Device 0 is not a valid NPU device").
            //  - SMB 스토어 I/O(os error 95) 회피: 로컬 /work 에 save 후 스토어로 복사.
            let env_lines: String = envs
                .iter()
                .map(|(k, v)| format!("             \x20           - {{ name: {}, value: \"{}\" }}\n", k, v))
                .collect();
            let script_doc = format!(
                "# RBLN 컴파일 스크립트(인라인) — create_runtimes=False + 로컬빌드→스토어복사(실기 검증).\n\
                 apiVersion: v1\n\
                 kind: ConfigMap\n\
                 metadata: {{ name: {name}-script, namespace: {ns} }}\n\
                 data:\n\
                 \x20 compile.py: |\n\
                 \x20\x20\x20 import os, shutil\n\
                 \x20\x20\x20 from optimum.rbln import RBLNAutoModelForCausalLM as M\n\
                 \x20\x20\x20 g = os.environ.get; o = os.environ[\"OUTPUT\"]; loc = \"/work/out\"\n\
                 \x20\x20\x20 m = M.from_pretrained(os.environ[\"MODEL_ID\"], export=True, rbln_create_runtimes=False,\n\
                 \x20\x20\x20\x20\x20 rbln_npu=g(\"RBLN_NPU\", \"RBLN-CA22\"),\n\
                 \x20\x20\x20\x20\x20 rbln_tensor_parallel_size=int(g(\"RBLN_TENSOR_PARALLEL_SIZE\", \"1\")),\n\
                 \x20\x20\x20\x20\x20 rbln_max_seq_len=int(g(\"RBLN_MAX_SEQ_LEN\", \"4096\")),\n\
                 \x20\x20\x20\x20\x20 rbln_batch_size=int(g(\"RBLN_BATCH_SIZE\", \"1\")))\n\
                 \x20\x20\x20 m.save_pretrained(loc)\n\
                 \x20\x20\x20 os.makedirs(o, exist_ok=True)\n\
                 \x20\x20\x20 for f in os.listdir(loc):\n\
                 \x20\x20\x20\x20\x20 s = os.path.join(loc, f); d = os.path.join(o, f)\n\
                 \x20\x20\x20\x20\x20 shutil.copytree(s, d, dirs_exist_ok=True) if os.path.isdir(s) else shutil.copy2(s, d)\n\
                 \x20\x20\x20 print(\"COMPILE_DONE\", os.listdir(o))\n\
                 ---\n",
                name = name, ns = self.ns
            );
            (
                // 들여쓰기는 형제 항목(store)과 정확히 일치해야 함(volumes 8칸, mounts 12칸).
                "        - { name: script, configMap: { name: SCRIPT_CM } }\n        - { name: work, emptyDir: {} }\n"
                    .replace("SCRIPT_CM", &format!("{}-script", name)),
                env_lines,
                "            - { name: script, mountPath: /scripts, readOnly: true }\n            - { name: work, mountPath: /work }\n".to_string(),
                "[\"python3\", \"/scripts/compile.py\"]".to_string(),
                "# RBLN: optimum-rbln 인라인 스크립트(create_runtimes=False). 이미지 placeholder(TODO-)면 LMD_COMPILE_IMAGE_RBLN 필요.",
                script_doc,
            )
        };
        let yaml = format!(
            "# 컴파일 Job (dry-run 미리보기) — 검토 후 `kubectl apply -f -` 로 적용.\n\
             # 모델 {model_id} → {vendor} 컴파일 → 공유 스토어(compiled/{repo_dir}/{vendor}/{target}).\n\
             # 옵션(컴파일 타임 고정): {opts}\n\
             {extra_doc}\
             {note}\n\
             apiVersion: batch/v1\n\
             kind: Job\n\
             metadata: {{ name: {name}, namespace: {ns} }}\n\
             spec:\n\
             \x20 backoffLimit: 0\n\
             \x20 template:\n\
             \x20   spec:\n\
             \x20     restartPolicy: Never\n\
             \x20     nodeSelector: {{ {node_label} }}\n\
             \x20     volumes:\n\
             \x20       - {{ name: store, persistentVolumeClaim: {{ claimName: model-store }} }}\n\
             {volumes_extra}\
             \x20     containers:\n\
             \x20       - name: compile\n\
             \x20         image: {image}\n\
             \x20         resources: {{ limits: {{ {res_key}: {res_qty} }} }}\n\
             \x20         env:\n\
             {env_block}\
             \x20         volumeMounts:\n\
             \x20           - {{ name: store, mountPath: /mnt/store }}\n\
             {mounts_extra}\
             \x20         command: {command}\n",
            model_id = model_id,
            vendor = vendor,
            repo_dir = repo_dir,
            target = target,
            opts = opts_summary,
            extra_doc = extra_doc,
            note = note,
            name = name,
            ns = self.ns,
            node_label = node_label,
            image = image,
            res_key = res_key,
            res_qty = res_qty,
            volumes_extra = volumes_extra,
            env_block = env_block,
            mounts_extra = mounts_extra,
            command = command,
        );
        self.preview = Some((format!("compile · {} → {}", form.model, target), yaml));
        self.preview_scroll = 0;
        self.preview_apply = true;
    }

    /// 모델 이름에서 파라미터 수(B) 추정 — "8B", "1.5b", "0.5B", "32b" 등 첫 매치.
    fn est_params_b(name: &str) -> Option<f64> {
        let lower = name.to_lowercase();
        let bytes = lower.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                // 숫자 뒤 바로 'b' 이고, 앞이 알파벳이 아니어야(예: fp"8b" 방지 위해 직전 문자 체크)
                if i < bytes.len() && (bytes[i] == b'b') {
                    let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphabetic();
                    if before_ok {
                        if let Ok(v) = lower[start..i].parse::<f64>() {
                            if (0.1..=2000.0).contains(&v) {
                                return Some(v);
                            }
                        }
                    }
                }
            } else {
                i += 1;
            }
        }
        None
    }

    /// 선택 인프라(NPU 메모리) 대비 컴파일 옵션 적합성 추정 + 조정 제안. 대략치.
    pub fn compile_fit(&self, form: &CompileForm) -> FitEstimate {
        let rbln = form.vendor == "rbln";
        let params_b = Self::est_params_b(&form.model_id).or_else(|| Self::est_params_b(&form.model));
        let tp = form.get("tp").parse::<f64>().unwrap_or(1.0).max(1.0);
        let pp = form.get("pp").parse::<f64>().unwrap_or(1.0).max(1.0);
        let seq = form.get("max-len").parse::<f64>().unwrap_or(8192.0).max(1.0);
        let batch = form.get("batch").parse::<f64>().unwrap_or(1.0).max(1.0);
        // dtype 바이트/파라미터 — 양자화·모델명 반영.
        let q = form.get("quant").to_lowercase();
        let name_l = form.model_id.to_lowercase();
        let dtype_bytes = if rbln {
            if q.contains("w4") {
                0.5
            } else if q.contains("w8") {
                1.0
            } else {
                2.0
            }
        } else if name_l.contains("fp8") || name_l.contains("-w8") {
            1.0
        } else if name_l.contains("int4") || name_l.contains("awq") || name_l.contains("gptq") {
            0.5
        } else {
            2.0
        };
        // 칩당 가용 메모리 — 스냅샷의 동종 디바이스 mem_total 평균, 없으면 표준값.
        let want_kind = if rbln { crate::collect::AccelKind::Rbln } else { crate::collect::AccelKind::Rngd };
        let mems: Vec<f64> = self
            .snap
            .accel
            .iter()
            .filter(|a| a.kind == want_kind && a.mem_total_gb > 0.0)
            .map(|a| a.mem_total_gb)
            .collect();
        let avail_gb = if mems.is_empty() {
            if rbln {
                15.7
            } else {
                48.0
            }
        } else {
            mems.iter().sum::<f64>() / mems.len() as f64
        };
        // 메모리 분산 칩 수: RBLN=TP, Furiosa≈ceil(tp/8)*pp.
        let chips = if rbln { tp } else { (tp / 8.0).ceil().max(1.0) * pp };
        let weight_gb = params_b.map(|p| p * dtype_bytes).unwrap_or(0.0);
        // KV: Llama-8B bf16 ≈ 0.25 MB/token 기준 스케일. (KV 양자화는 미반영 — 보수적)
        let kv_per_tok_mb = 0.25 * (params_b.unwrap_or(8.0) / 8.0);
        let kv_gb = batch * seq * kv_per_tok_mb / 1024.0;
        let overhead_gb = 2.0;
        let per_chip_gb = (weight_gb + kv_gb) / chips + overhead_gb;
        let ratio = per_chip_gb / avail_gb;
        let verdict = if params_b.is_none() {
            FitVerdict::Unknown
        } else if ratio > 1.0 {
            FitVerdict::Oom
        } else if ratio > 0.85 {
            FitVerdict::Tight
        } else {
            FitVerdict::Fits
        };
        // 조정 제안.
        let mut tips: Vec<String> = Vec::new();
        let max_chips = if rbln { 4.0 } else { 8.0 };
        if matches!(verdict, FitVerdict::Oom | FitVerdict::Tight) {
            if tp < max_chips {
                tips.push(format!("TP↑ {}→{} (칩 추가로 칩당 부담↓)", tp as i64, (tp * 2.0).min(max_chips) as i64));
            }
            if seq > 2048.0 {
                tips.push(format!("max-seq-len↓ {}→{} (KV {:.1}GiB↓)", seq as i64, (seq / 2.0) as i64, kv_gb / 2.0));
            }
            if batch > 1.0 {
                tips.push(format!("batch↓ {}→{} (KV 절반)", batch as i64, (batch / 2.0) as i64));
            }
            if dtype_bytes >= 2.0 {
                tips.push(if rbln { "양자화 w4a16/w8a8 로 가중치↓".into() } else { "FP8 체크포인트 사용 시 가중치 절반".into() });
            }
        } else if matches!(verdict, FitVerdict::Fits) && ratio < 0.4 {
            if batch < 8.0 {
                tips.push(format!("여유 있음 — batch↑ {}→{} 로 처리량 확보 여지", batch as i64, (batch * 2.0) as i64));
            }
            if tp > 1.0 && rbln {
                tips.push(format!("TP↓ {}→{} 로 칩 절약(가능 시)", tp as i64, (tp / 2.0) as i64));
            }
        }
        // 요청 디바이스 수가 물리 분산 칩 수보다 적으면 컴파일 불가.
        if let Ok(dev) = form.get("devices").parse::<f64>() {
            if dev > 0.0 && dev < chips {
                tips.push(format!("⚠ devices {} < 필요 {} 칩 — 이 TP/PP 로는 컴파일 불가", dev as i64, chips as i64));
            }
        }
        // RBLN kvcache_partition_len 은 2의 거듭제곱이어야.
        if rbln {
            if let Ok(k) = form.get("kvpart").parse::<u64>() {
                if k == 0 || (k & (k - 1)) != 0 {
                    tips.push(format!("⚠ kvcache-partition {} 는 2의 거듭제곱이 아님", k));
                }
            }
        }
        // 선택 노드가 이 NPU 드라이버를 갖고 있는지 — 컴파일은 드라이버 설치 노드에서만 가능.
        let node_host = form.get("node");
        let node_host = node_host.split('(').next().unwrap_or("any").trim();
        if node_host != "any" && !node_host.is_empty() {
            if let Some(nd) = self.snap.nodes.iter().find(|n| n.name == node_host) {
                let want = if rbln { "RBLN" } else { "RNGD" };
                if !nd.npu.to_uppercase().contains(want) {
                    tips.push(format!("⚠ 노드 {} 에 {} 드라이버 없음(npu: {}) — 컴파일 실패", node_host, want, if nd.npu.is_empty() { "none" } else { &nd.npu }));
                }
            }
        }
        FitEstimate { params_b, weight_gb, kv_gb, overhead_gb, chips, per_chip_gb, avail_gb, verdict, tips }
    }

    /// 컴파일 사전 점검(preflight) — 긴 컴파일 전에 전제조건 충족 여부를 미리 검사.
    /// 실기에서 발견한 함정(레지스트리 미등록·aarch64 툴체인·노드 드라이버·컴파일러 이미지)을 사전 방어.
    /// 반환: (충족?, 메시지). 하나라도 false 면 컴파일 실패 가능 → 폼에서 경고 표시.
    pub fn compile_preflight(&self, form: &CompileForm) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        let mid = form.model_id.to_lowercase();
        if form.vendor == "furiosa" {
            // fxb build 는 furiosa-ai 조직의 양자화 체크포인트만 컴파일(실기 확인).
            let quant = ["fp8", "nvfp4", "-w8", "-w4", "awq", "gptq", "int4", "int8"].iter().any(|q| mid.contains(q));
            let org = mid.starts_with("furiosa-ai/");
            out.push((
                org && quant,
                if org && quant {
                    "registry: furiosa-ai 양자화 체크포인트 — fxb 등록 대상".into()
                } else {
                    format!("registry: fxb 는 furiosa-ai 양자화 모델만 빌드(예: furiosa-ai/Qwen3-4B-FP8) — '{}' 미등록 가능성", form.model_id)
                },
            ));
            // RNGD 는 ARM64 제어 프로세서 → EDF 최종 코드젠에 aarch64 크로스컴파일러 필요(매니페스트가 자동 설치).
            out.push((true, "toolchain: aarch64 크로스컴파일러 매니페스트가 자동 설치".into()));
            out.push((true, "build I/O: 로컬 emptyDir 빌드→스토어 복사(SMB os error 95 회피)".into()));
        } else {
            let has_img = self.img_rbln.is_some();
            out.push((
                has_img,
                if has_img {
                    "image: LMD_COMPILE_IMAGE_RBLN 지정됨".into()
                } else {
                    "image: rebel-compiler 는 공개 PyPI 부재(pypi.rbln.ai 승인계정 전용) — 그게 든 이미지를 LMD_COMPILE_IMAGE_RBLN 로 지정 필요".into()
                },
            ));
            let compat_ok = crate::compat::compilable_vendors(&form.model_id).contains(&"rbln");
            out.push((compat_ok, format!("registry: RBLN 지원 계열 {}", if compat_ok { "확인됨(npu-compat)" } else { "미확인" })));
        }
        // 공통: 타깃 노드에 해당 NPU 드라이버가 설치돼 있는지(컴파일은 드라이버 노드에서만).
        let node = form.get("node");
        let node = node.split('(').next().unwrap_or("any").trim();
        let want = if form.vendor == "rbln" { "RBLN" } else { "RNGD" };
        if node != "any" && !node.is_empty() {
            let has = self.snap.nodes.iter().find(|n| n.name == node).map(|n| n.npu.to_uppercase().contains(want)).unwrap_or(false);
            out.push((has, format!("node {}: {} 드라이버 {}", node, want, if has { "설치됨" } else { "없음 — 컴파일 실패" })));
        } else {
            let any_node = self.snap.nodes.iter().any(|n| n.npu.to_uppercase().contains(want));
            out.push((any_node, format!("node: any — {} 드라이버 노드 {}", want, if any_node { "있음" } else { "없음(클러스터에 미설치)" })));
        }
        out
    }

    /// Enter — 선택 항목의 컨텍스트 액션 메뉴를 연다(단축키를 몰라도 되게).
    pub fn open_action_menu(&mut self) {
        let mut items: Vec<ActionItem> = Vec::new();
        let (title, subject) = match self.view {
            View::Launch if self.panel_focus == 0 => {
                let Some(a) = self.selected_artifact() else { return };
                let model_id = Self::artifact_model_id(a);
                let deployed = self.snap.models.iter().any(|m| m.name == a.model && m.desired > 0);
                items.push(ActionItem { key: 'i', label: "Info", desc: "show full build detail", action: Action::Info });
                // 컴파일 대상 벤더 — 엔진이 NPU 면 그 벤더, 아니면 지원 목록(GPU/HF→NPU)에서.
                let rbln_ok = a.engine.contains("RBLN") || crate::compat::compilable_vendors(&model_id).contains(&"rbln");
                let furiosa_ok = a.engine.contains("Furiosa") || crate::compat::compilable_vendors(&model_id).contains(&"furiosa");
                if rbln_ok {
                    items.push(ActionItem { key: 'c', label: "Compile→RBLN", desc: "optimum-rbln compile → .rbln in store", action: Action::Compile("rbln") });
                }
                if furiosa_ok {
                    items.push(ActionItem { key: 'f', label: "Compile→Furiosa", desc: "furiosa-llm build → artifact in store", action: Action::Compile("furiosa") });
                }
                items.push(ActionItem { key: 'd', label: "Deploy", desc: "serving options → Deployment", action: Action::Deploy });
                items.push(ActionItem { key: 'y', label: "YAML", desc: "live Deployment YAML (read-only)", action: Action::Yaml });
                if deployed {
                    items.push(ActionItem { key: 'x', label: "Stop", desc: "scale serving → 0 (frees devices)", action: Action::Stop });
                }
                (format!("actions · {}", a.model), a.model.clone())
            }
            View::Launch if self.panel_focus == 1 => {
                // 배치 타깃(노드) 패널 — 노드 액션.
                let nodes = self.target_nodes();
                let Some(node) = self.sel_orig().and_then(|i| nodes.get(i).cloned()) else { return };
                let cordoned = self.snap.nodes.iter().find(|n| n.name == node).map(|n| n.cordoned).unwrap_or(false);
                items.push(ActionItem { key: 'i', label: "Info", desc: "node detail — devices, occupancy, capacity", action: Action::Info });
                if cordoned {
                    items.push(ActionItem { key: 'u', label: "Uncordon", desc: "allow scheduling on this node", action: Action::Uncordon });
                } else {
                    items.push(ActionItem { key: 'C', label: "Cordon", desc: "block new scheduling on this node", action: Action::Cordon });
                }
                (format!("node · {}", node), node)
            }
            View::Launch if self.panel_focus == 2 => {
                // 카탈로그(배포 가능 모델) 패널 — feasibility 안내.
                let Some(m) = self.sel_orig().and_then(|i| self.catalog.get(i)) else { return };
                items.push(ActionItem { key: 'i', label: "Info", desc: "why ready / needs artifact (feasibility)", action: Action::Info });
                (format!("catalog · {}", m.id), m.id.clone())
            }
            View::Models | View::Overview => {
                let Some(m) = self.selected_model() else { return };
                let running = m.desired > 0;
                items.push(ActionItem { key: 'i', label: "Info", desc: "model detail", action: Action::Info });
                items.push(ActionItem { key: 'l', label: "Logs", desc: "tail pod logs", action: Action::Logs });
                items.push(ActionItem { key: 'y', label: "YAML", desc: "live Deployment YAML (read-only)", action: Action::Yaml });
                items.push(ActionItem { key: 's', label: "Scale", desc: "toggle replicas 0/1", action: Action::Scale });
                items.push(ActionItem { key: 'S', label: "Restart", desc: "rollout restart (rolling)", action: Action::Restart });
                items.push(ActionItem { key: 'O', label: "Objective", desc: "set SLO target (TTFT/TPOT/E2E/tok·s) — drives advisor", action: Action::Objective });
                if running {
                    items.push(ActionItem { key: 'x', label: "Stop", desc: "scale → 0 (frees devices)", action: Action::Stop });
                }
                (format!("actions · {}", m.name), m.name.clone())
            }
            View::Pods => {
                let Some(p) = self.selected_pod() else { return };
                items.push(ActionItem { key: 'i', label: "Info", desc: "pod detail", action: Action::Info });
                items.push(ActionItem { key: 'l', label: "Logs", desc: "tail pod logs", action: Action::Logs });
                items.push(ActionItem { key: 'y', label: "YAML", desc: "live Pod YAML (read-only)", action: Action::Yaml });
                items.push(ActionItem { key: 'D', label: "Delete", desc: "delete pod (reschedules)", action: Action::Delete });
                (format!("actions · {}", p.name), p.name.clone())
            }
            _ => return,
        };
        self.action_menu = Some(ActionMenu { title, subject, items, cursor: 0 });
    }

    /// 모델 목표(SLO) 조회 — 정확 매칭 우선, 없으면 느슨한 부분일치(서빙명 ≠ deploy명 대비).
    pub fn objective_for(&self, model: &str) -> Option<&Objective> {
        if let Some(o) = self.objectives.get(model) {
            return Some(o);
        }
        self.objectives.iter().find(|(k, _)| model.contains(k.as_str()) || k.contains(model)).map(|(_, o)| o)
    }

    /// Models 액션 메뉴 → Objective: 목표 편집 폼(기존 값 프리필).
    pub fn open_objective_form(&mut self) {
        let Some(m) = self.selected_model() else { return };
        let name = m.name.clone();
        let cur = self.objectives.get(&name).cloned().unwrap_or_default();
        let numf = |key: &str, label: &str, cur: Option<f64>, choices: &[&str], help: &str| CompileField {
            key: key.into(),
            label: label.into(),
            value: cur.map(|v| format!("{}", v as i64)).unwrap_or_else(|| "none".into()),
            choices: std::iter::once("none").chain(choices.iter().copied()).map(|s| s.to_string()).collect(),
            numeric: true,
            help: help.into(),
        };
        let fields = vec![
            numf("ttft", "TTFT p95 ≤ms", cur.ttft_ms, &["500", "1000", "2000", "4000"], "첫 토큰까지 목표 상한(ms) — 대화형 응답성"),
            numf("tpot", "TPOT p95 ≤ms", cur.tpot_ms, &["20", "50", "100", "200"], "토큰당 생성 시간 상한(ms) — 스트리밍 속도"),
            numf("e2e", "E2E p95 ≤ms", cur.e2e_ms, &["1000", "2000", "5000", "10000"], "요청 완료까지 상한(ms)"),
            numf("tps", "min tok/s ≥", cur.min_tps, &["10", "50", "100", "500"], "최소 처리량(tok/s) — 낮으면 처리량 부족"),
        ];
        self.objective_form = Some(ObjectiveForm { model: name, fields, cursor: 0, editing: false });
    }

    /// 목표 폼 제출 → objectives 에 반영(모두 none 이면 삭제).
    pub fn objective_form_submit(&mut self) {
        let Some(form) = self.objective_form.take() else { return };
        let p = |s: String| -> Option<f64> {
            if s.is_empty() || s == "none" {
                None
            } else {
                s.parse::<f64>().ok()
            }
        };
        let obj = Objective {
            ttft_ms: p(form.get("ttft")),
            tpot_ms: p(form.get("tpot")),
            e2e_ms: p(form.get("e2e")),
            min_tps: p(form.get("tps")),
        };
        if obj.is_empty() {
            self.objectives.remove(&form.model);
            self.notify(format!("objective cleared for {}", form.model));
        } else {
            self.objectives.insert(form.model.clone(), obj);
            self.notify(format!("objective set for {}", form.model));
        }
    }

    /// 관측(PerfRow) vs 목표 판정 + 병목 기반 조정 제안(값싼 런타임 노브 중심).
    pub fn perf_advice(&self, row: &crate::collect::PerfRow) -> PerfAdvice {
        let Some(o) = self.objective_for(&row.model) else {
            return PerfAdvice { has_obj: false, checks: Vec::new(), tips: Vec::new() };
        };
        let ms = |s: f64| s * 1000.0;
        let mut checks: Vec<(&'static str, bool)> = Vec::new();
        if let Some(t) = o.ttft_ms {
            if !row.ttft_p95.is_nan() {
                checks.push(("TTFT", ms(row.ttft_p95) <= t));
            }
        }
        if let Some(t) = o.tpot_ms {
            if !row.tpot_p95.is_nan() {
                checks.push(("TPOT", ms(row.tpot_p95) <= t));
            }
        }
        if let Some(t) = o.e2e_ms {
            if !row.e2e_p95.is_nan() {
                checks.push(("E2E", ms(row.e2e_p95) <= t));
            }
        }
        if let Some(t) = o.min_tps {
            if !row.tps.is_nan() {
                checks.push(("tok/s", row.tps >= t));
            }
        }
        let mut tips: Vec<String> = Vec::new();
        let violated = checks.iter().any(|(_, ok)| !ok);
        if violated {
            let q = row.queue_p95;
            let pf = row.prefill_p95;
            let dc = row.decode_p95;
            if row.preempt > 0.0 {
                tips.push("KV/메모리 스래싱(preemption↑) — batch↓ 또는 max-seq-len↓, KV 여유 확보".into());
            }
            if !q.is_nan() && q > 0.0 && q >= pf.max(dc) {
                tips.push(format!("스케줄 대기 지배적({:.0}ms) — replicas↑ 또는 배치 버킷↑", ms(q)));
            } else if !pf.is_nan() && pf >= dc && pf > 0.0 {
                tips.push(format!("prefill 지배적({:.0}ms) — max-seq-len↓ 또는 chunked prefill", ms(pf)));
            } else if !dc.is_nan() && dc > 0.0 {
                tips.push(format!("decode 지배적({:.0}ms) — TPOT 개선: 동시성 조정, 필요시 TP↑", ms(dc)));
            }
            if let Some(mt) = o.min_tps {
                if !row.tps.is_nan() && row.tps < mt {
                    // 이 모델을 점유한 디바이스 평균 util 로 방향 제시.
                    let ut: Vec<f64> = self.snap.accel.iter().filter(|a| !a.busy_model.is_empty() && row.model.contains(&a.busy_model) || a.busy_model == row.model).map(|a| a.util).collect();
                    let avg = if ut.is_empty() { f64::NAN } else { ut.iter().sum::<f64>() / ut.len() as f64 };
                    if !avg.is_nan() && avg > 70.0 {
                        tips.push("tok/s 미달 · util 높음 — compute-bound: TP↑ 또는 replica↑".into());
                    } else {
                        tips.push("tok/s 미달 · util 여유 — 동시성/배치↑ 로 처리량 확보".into());
                    }
                }
            }
        }
        PerfAdvice { has_obj: true, checks, tips }
    }

    /// `[d] deploy` — 선택 모델의 배포(서빙) 옵션 편집 폼을 연다. replicas·디바이스·노드 배치.
    pub fn open_deploy_form(&mut self) {
        let Some(a) = self.selected_artifact() else { return };
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
            a.mount.split(" ← ").next().unwrap_or("/mnt/store").to_string()
        };
        // replica당 디바이스 기본값: 아티팩트 TP(없으면 1).
        let dev_default = Self::opt_or(a, "tp", "1");
        let want_kind = match vendor {
            "rbln" => crate::collect::AccelKind::Rbln,
            "furiosa" => crate::collect::AccelKind::Rngd,
            _ => crate::collect::AccelKind::Gpu,
        };
        let mut per_node: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for ac in self.snap.accel.iter().filter(|x| x.kind == want_kind && !x.node.is_empty()) {
            *per_node.entry(ac.node.clone()).or_insert(0) += 1;
        }
        let mut cand_nodes: Vec<String> = per_node.keys().cloned().collect();
        cand_nodes.sort();
        let mut place_choices = vec!["any".to_string(), "spread".to_string()];
        place_choices.extend(cand_nodes.iter().map(|n| format!("{}({})", n, per_node[n])));
        let fields = vec![
            CompileField {
                key: "replicas".into(),
                label: "replicas".into(),
                value: "1".into(),
                choices: ["1", "2", "3", "4", "6", "8"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "서빙 인스턴스 수(Deployment replicas). 총 디바이스 수요 = replicas × devices".into(),
            },
            CompileField {
                key: "devices".into(),
                label: "devices/replica".into(),
                value: dev_default,
                choices: ["1", "2", "4", "8"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "replica 당 가속기 수(resources.limits). 보통 = 컴파일 TP".into(),
            },
            CompileField {
                key: "port".into(),
                label: "port".into(),
                value: "8000".into(),
                choices: ["8000", "8080"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "서빙 컨테이너 포트".into(),
            },
            CompileField {
                key: "place".into(),
                label: "placement".into(),
                value: "any".into(),
                choices: place_choices,
                numeric: false,
                help: "노드 배치 — any=제약 없음, spread=여러 노드 분산(topologySpread), 특정=hostname 고정".into(),
            },
            CompileField {
                key: "routing".into(),
                label: "routing".into(),
                value: "llm-d".into(),
                choices: ["llm-d", "direct"].iter().map(|s| s.to_string()).collect(),
                numeric: false,
                help: "llm-d=게이트웨이 라우팅(InferencePool+EPP+HTTPRoute /accel/model)까지 생성 · direct=Deployment 만".into(),
            },
        ];
        self.deploy_form = Some(DeployForm {
            model: a.model.clone(),
            model_id,
            engine: a.engine.clone(),
            vendor,
            mount,
            fields,
            cursor: 0,
            editing: false,
        });
    }

    /// 배포 용량 판정 — 총 디바이스 수요 대 클러스터 동종 가속기(총/유휴).
    pub fn deploy_fit(&self, form: &DeployForm) -> DeployFit {
        let want_kind = match form.vendor {
            "rbln" => crate::collect::AccelKind::Rbln,
            "furiosa" => crate::collect::AccelKind::Rngd,
            _ => crate::collect::AccelKind::Gpu,
        };
        let devs: Vec<&crate::collect::Accel> = self.snap.accel.iter().filter(|x| x.kind == want_kind).collect();
        let total = devs.len() as i64;
        // 노드별 유휴(살아있고 미점유) — replica 는 한 노드에 per 개가 모여야 배치 가능(패킹).
        let mut free_by_node: std::collections::BTreeMap<&str, i64> = std::collections::BTreeMap::new();
        for d in &devs {
            if !d.node.is_empty() {
                let e = free_by_node.entry(d.node.as_str()).or_insert(0);
                if d.alive && d.busy_model.is_empty() {
                    *e += 1;
                }
            }
        }
        let free: i64 = free_by_node.values().sum();
        let nodes = free_by_node.len() as i64;
        let max_node_free = free_by_node.values().copied().max().unwrap_or(0);
        let replicas = form.get("replicas").parse::<i64>().unwrap_or(1).max(1);
        let per = form.get("devices").parse::<i64>().unwrap_or(1).max(1);
        let demand = replicas * per;
        // k8s 리소스 관점 유휴 = allocatable - requested (스케줄러가 실제로 보는 값).
        // metric busy_model 로는 유휴여도, 다른 배포가 리소스를 예약(request)했으면 스케줄 불가.
        let res_key = match form.vendor {
            "rbln" => "rebellions.ai/ATOM",
            "furiosa" => "furiosa.ai/rngd",
            _ => "nvidia.com/gpu",
        };
        let resource_free = self
            .snap
            .inventory
            .iter()
            .find(|(k, _, _)| k == res_key)
            .map(|(_, alloc, req)| (alloc - req).max(0))
            .unwrap_or(free); // inventory 없으면 metric 값으로 폴백
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
        // metric 유휴와 리소스 유휴가 어긋나면 명시(오해 방지).
        if resource_free != free {
            tips.push(format!("(metric 유휴 {} ≠ 리소스 유휴 {} — 예약됐지만 idle 인 디바이스 있음)", free, resource_free));
        }
        if form.get("place") == "spread" && replicas > nodes && nodes > 0 {
            tips.push(format!("⚠ spread: replicas {} > 노드 {} — 일부는 같은 노드로", replicas, nodes));
        }
        if per > 1 && form.vendor == "rbln" {
            tips.push("replica당 다중 칩은 컴파일 TP 와 일치해야 함".into());
        }
        DeployFit { demand, total, free, resource_free, nodes, verdict, tips }
    }

    /// 배포 사전 점검(preflight) — apply 전에 서빙 전제조건 확인(사전 방어).
    pub fn deploy_preflight(&self, form: &DeployForm) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        // 이미지 — deploy_form_submit 의 벤더별 기본값과 동일 판정(불일치로 오탐 방지).
        //   furiosa=furiosaai/furiosa-llm:latest, gpu=vllm/vllm-openai:latest 기본 존재 → OK.
        //   rbln 은 vllm_rbln 런타임이 든 이미지가 필요(기본 없음) → LMD_SERVING_IMAGE 미지정이면 차단.
        let (img_ok, img_msg) = match form.vendor {
            "furiosa" => (true, "이미지 준비됨 — furiosaai/furiosa-llm:latest (furiosa-llm serve)".to_string()),
            "gpu" => {
                let img = self.img_serving.clone().unwrap_or_else(|| "vllm/vllm-openai:latest".into());
                (true, format!("이미지 준비됨 — {} (vLLM serve)", img))
            }
            _ => match &self.img_serving {
                Some(img) => (true, format!("이미지 준비됨 — {} (vllm_rbln 런타임)", img)),
                None => (false, "이미지 없음 — RBLN 서빙엔 vllm_rbln 런타임 이미지가 필요. LMD_SERVING_IMAGE 로 지정(또는 호스트 RBLN 스택 hostPath 방식)".into()),
            },
        };
        out.push((img_ok, format!("① 서빙 이미지: {}", img_msg)));
        // ② 모델 아티팩트 경로 — 스토어/컴파일본이 있어야 서빙이 로드.
        out.push((
            !form.mount.is_empty(),
            format!(
                "② 모델 위치: {}",
                if form.mount.is_empty() { "경로 미상 — 먼저 [c]로 컴파일하거나 스토어 경로 확인".into() } else { form.mount.clone() }
            ),
        ));
        // ③ NPU 벤더면 타깃 노드에 드라이버가 깔려 있어야 스케줄됨.
        if form.vendor != "gpu" {
            let want = if form.vendor == "rbln" { "RBLN" } else { "RNGD" };
            let any = self.snap.nodes.iter().any(|n| n.npu.to_uppercase().contains(want));
            out.push((
                any,
                format!(
                    "③ 가속기 드라이버: {} 드라이버가 있는 노드 {}",
                    want,
                    if any { "있음 — 스케줄 가능" } else { "없음 — 해당 노드가 클러스터에 없어 Pending 됨" }
                ),
            ));
        }
        // ④ 용량 — 요청 디바이스 수가 실제 유휴(리소스 기준)에 들어가야 함.
        let fit = self.deploy_fit(form);
        let cap_ok = matches!(fit.verdict, FitVerdict::Fits);
        out.push((
            cap_ok,
            format!(
                "④ 용량: 디바이스 {}개 필요 · 유휴 {}개 → {}{}",
                fit.demand,
                fit.resource_free,
                fit.verdict.label(),
                if cap_ok { "" } else { " (서빙 stop 으로 확보하거나 replicas/devices↓)" }
            ),
        ));
        out
    }

    /// 배포 폼 → Deployment 매니페스트 미리보기(dry-run). Enter 시 호출.
    pub fn deploy_form_submit(&mut self) {
        let Some(form) = self.deploy_form.take() else { return };
        let name = form.model_id.replace(['/', '.'], "-").to_lowercase();
        let name = format!("serve-{}", name);
        let replicas = form.get("replicas");
        let devices = form.get("devices");
        let port = {
            let p = form.get("port");
            if p.is_empty() { "8000".to_string() } else { p }
        };
        let (res_key, product_label) = match form.vendor {
            "rbln" => ("rebellions.ai/ATOM", "rebellions.ai/npu.product: RBLN-CA22"),
            "furiosa" => ("furiosa.ai/rngd", "furiosa.ai/npu.product: rngd"),
            _ => ("nvidia.com/gpu", ""),
        };
        let place = form.get("place");
        let place_host = place.split('(').next().unwrap_or("any").trim().to_string();
        // 배치 스펙 — spread=topologySpread, 특정=nodeSelector hostname, any=제약 없음(디바이스 resource 로 스케줄).
        let placement_yaml = if place_host == "spread" {
            format!(
                "\x20     topologySpreadConstraints:\n\
                 \x20       - {{ maxSkew: 1, topologyKey: kubernetes.io/hostname, whenUnsatisfiable: DoNotSchedule, labelSelector: {{ matchLabels: {{ app: {name} }} }} }}\n",
                name = name
            )
        } else if place_host != "any" && !place_host.is_empty() {
            format!("\x20     nodeSelector: {{ kubernetes.io/hostname: {} }}\n", place_host)
        } else if !product_label.is_empty() {
            format!("\x20     nodeSelector: {{ {} }}\n", product_label)
        } else {
            String::new()
        };
        // 벤더별 서빙 스펙 — 실기 검증된 명령/이미지/env (제네릭 `--model` 은 NPU 에서 안 뜸).
        //   Furiosa: `furiosa-llm serve <model> -tp N` (entrypoint=furiosa-llm, 첫 인자 serve 필수;
        //            published furiosa-ai 모델은 HF 에서 받아 서빙, 아티팩트 내장 tp 가 -tp 보다 우선).
        //   RBLN   : vllm_rbln 런타임 이미지에 `vllm serve <path> -tp N` + VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK.
        //   GPU    : `vllm serve <path> -tp N`.
        let served = form.model_id.clone();
        let (image, note, container_spec, volumes_block) = match form.vendor {
            "furiosa" => {
                let img = self.img_furiosa.clone().unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into());
                let spec = format!(
                    "\x20         args: [\"serve\", \"{served}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{devices}\"]\n\
                     \x20         ports: [{{ containerPort: {port} }}]\n\
                     \x20         env:\n\
                     \x20           - {{ name: HF_HOME, value: /model-cache }}\n\
                     \x20           - {{ name: HF_TOKEN, valueFrom: {{ secretKeyRef: {{ name: hf-token, key: HF_TOKEN }} }} }}\n\
                     \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"4\", memory: \"16Gi\", {res_key}: {devices} }} }}\n\
                     \x20         volumeMounts:\n\
                     \x20           - {{ name: cache, mountPath: /model-cache }}\n",
                    served = served, port = port, devices = devices, res_key = res_key
                );
                let vols = "\x20     volumes:\n\x20       - { name: cache, emptyDir: {} }\n".to_string();
                (img, "# Furiosa: furiosa-llm serve. HF_TOKEN=secret hf-token 필요. 아티팩트 내장 tp 가 -tp 보다 우선.".to_string(), spec, vols)
            }
            "rbln" => {
                // vllm_rbln 런타임 이미지 필요(LMD_SERVING_IMAGE). 없으면 apply 차단 placeholder.
                // (이 클러스터처럼 레지스트리 이미지가 없으면 호스트 RBLN 스택을 hostPath 로 마운트하는
                //  ubuntu 베이스 패턴이 대안 — deploy preflight 참고. sympy 는 /usr/local dist-packages 에.)
                let img = self.img_serving.clone().unwrap_or_else(|| "TODO-rbln-serving-image".into());
                let spec = format!(
                    "\x20         args: [\"serve\", \"{mount}\", \"--served-model-name\", \"{served}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{devices}\", \"--max-num-seqs\", \"1\"]\n\
                     \x20         ports: [{{ containerPort: {port} }}]\n\
                     \x20         env:\n\
                     \x20           - {{ name: VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK, value: \"{devices}\" }}\n\
                     \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"8\", memory: \"32Gi\", {res_key}: {devices} }} }}\n\
                     \x20         volumeMounts:\n\
                     \x20           - {{ name: store, mountPath: /mnt/store, readOnly: true }}\n",
                    mount = form.mount, served = served, port = port, devices = devices, res_key = res_key
                );
                let vols = "\x20     volumes:\n\x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n".to_string();
                (img, "# RBLN: vllm_rbln 런타임 이미지 필요(LMD_SERVING_IMAGE). 사전컴파일 .rbln 을 스토어에서 로드.".to_string(), spec, vols)
            }
            _ => {
                let img = self.img_serving.clone().unwrap_or_else(|| "vllm/vllm-openai:latest".into());
                let spec = format!(
                    "\x20         args: [\"serve\", \"{mount}\", \"--served-model-name\", \"{served}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{devices}\"]\n\
                     \x20         ports: [{{ containerPort: {port} }}]\n\
                     \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"4\", memory: \"16Gi\", {res_key}: {devices} }} }}\n\
                     \x20         volumeMounts:\n\
                     \x20           - {{ name: store, mountPath: /mnt/store, readOnly: true }}\n",
                    mount = form.mount, served = served, port = port, devices = devices, res_key = res_key
                );
                let vols = "\x20     volumes:\n\x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n".to_string();
                (img, "# GPU: vLLM 이 스토어 경로에서 로드(별도 컴파일 불필요).".to_string(), spec, vols)
            }
        };
        let yaml = format!(
            "# 배포 매니페스트 (dry-run 미리보기) — 검토 후 `kubectl apply -f -`.\n\
             # 모델 {model_id} 서빙. 엔진: {engine}.\n\
             # 배치: {place}  ·  총 디바이스 수요 = {replicas} × {devices}\n\
             # 이미지 placeholder(TODO-)면 LMD_SERVING_IMAGE 로 지정해야 apply 가능.\n\
             {note}\n\
             apiVersion: apps/v1\n\
             kind: Deployment\n\
             metadata: {{ name: {name}, namespace: {ns} }}\n\
             spec:\n\
             \x20 replicas: {replicas}\n\
             \x20 selector: {{ matchLabels: {{ app: {name} }} }}\n\
             \x20 template:\n\
             \x20   metadata: {{ labels: {{ app: {name} }} }}\n\
             \x20   spec:\n\
             {placement}\
             {volumes_block}\
             \x20     containers:\n\
             \x20       - name: server\n\
             \x20         image: {image}   # {engine}\n\
             {container_spec}",
            model_id = form.model_id,
            engine = form.engine,
            note = note,
            place = place,
            name = name,
            ns = self.ns,
            replicas = replicas,
            devices = devices,
            placement = placement_yaml,
            volumes_block = volumes_block,
            container_spec = container_spec,
            image = image,
        );
        // routing=llm-d → 게이트웨이 라우팅 리소스(InferencePool+EPP+HTTPRoute)를 뒤에 동봉.
        let yaml = if form.get("routing") == "llm-d" {
            format!("{}{}", yaml, self.routing_docs(&name, form.vendor, &served))
        } else {
            yaml
        };
        self.preview = Some((format!("deploy · {} ×{}", form.model, replicas), yaml));
        self.preview_scroll = 0;
        self.preview_apply = true;
    }

    /// llm-d 게이트웨이 라우팅 리소스 문서들(서빙 Deployment 뒤에 붙임).
    /// 실기 배선 그대로: SA, RoleBinding×2(공유 Role llmd-router-epp-sa/-non-sa 참조),
    /// plugins ConfigMap, EPP Deployment/Service, InferencePool(app={name} 선택), HTTPRoute.
    /// 경로 = /accel/model. EPP 는 pool 멤버 파드로 model/부하 인지 라우팅.
    fn routing_docs(&self, name: &str, vendor: &str, served: &str) -> String {
        let accel = match vendor {
            "furiosa" => "rngd",
            "rbln" => "atom",
            _ => "gpu",
        };
        let slug = served.rsplit('/').next().unwrap_or(served).to_lowercase().replace(['.', '_'], "-");
        let path = format!("/{}/{}", accel, slug);
        let ns = &self.ns;
        format!(
            "---\n\
             # ── llm-d 라우팅: 게이트웨이 {path} → InferencePool({name}-pool) → EPP → 이 서빙 ──\n\
             # (공유 Role llmd-router-epp-sa/-non-sa 는 클러스터에 이미 존재한다고 가정)\n\
             apiVersion: v1\n\
             kind: ServiceAccount\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             ---\n\
             apiVersion: rbac.authorization.k8s.io/v1\n\
             kind: RoleBinding\n\
             metadata: {{ name: {name}-epp-sa, namespace: {ns} }}\n\
             roleRef: {{ apiGroup: rbac.authorization.k8s.io, kind: Role, name: llmd-router-epp-sa }}\n\
             subjects:\n\
             \x20 - {{ kind: ServiceAccount, name: {name}-epp, namespace: {ns} }}\n\
             ---\n\
             apiVersion: rbac.authorization.k8s.io/v1\n\
             kind: RoleBinding\n\
             metadata: {{ name: {name}-epp-non-sa, namespace: {ns} }}\n\
             roleRef: {{ apiGroup: rbac.authorization.k8s.io, kind: Role, name: llmd-router-epp-non-sa }}\n\
             subjects:\n\
             \x20 - {{ kind: ServiceAccount, name: {name}-epp, namespace: {ns} }}\n\
             ---\n\
             apiVersion: v1\n\
             kind: ConfigMap\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             data:\n\
             \x20 default-plugins.yaml: |\n\
             \x20\x20\x20 apiVersion: inference.networking.x-k8s.io/v1alpha1\n\
             \x20\x20\x20 kind: EndpointPickerConfig\n\
             \x20\x20\x20 plugins:\n\
             \x20\x20\x20 - type: queue-scorer\n\
             \x20\x20\x20 - type: kv-cache-utilization-scorer\n\
             \x20\x20\x20 - type: prefix-cache-scorer\n\
             \x20\x20\x20 schedulingProfiles:\n\
             \x20\x20\x20 - name: default\n\
             \x20\x20\x20\x20\x20 plugins:\n\
             \x20\x20\x20\x20\x20 - {{ pluginRef: queue-scorer, weight: 2 }}\n\
             \x20\x20\x20\x20\x20 - {{ pluginRef: kv-cache-utilization-scorer, weight: 2 }}\n\
             \x20\x20\x20\x20\x20 - {{ pluginRef: prefix-cache-scorer, weight: 3 }}\n\
             ---\n\
             apiVersion: apps/v1\n\
             kind: Deployment\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             spec:\n\
             \x20 replicas: 1\n\
             \x20 selector: {{ matchLabels: {{ app: {name}-epp }} }}\n\
             \x20 template:\n\
             \x20   metadata: {{ labels: {{ app: {name}-epp }} }}\n\
             \x20   spec:\n\
             \x20     serviceAccountName: {name}-epp\n\
             \x20     containers:\n\
             \x20       - name: epp\n\
             \x20         image: ghcr.io/llm-d/llm-d-router-endpoint-picker-dev:main\n\
             \x20         args: [\"--pool-name\", \"{name}-pool\", \"--pool-namespace\", \"{ns}\", \"--pool-group\", \"inference.networking.k8s.io\", \"--config-file\", \"/config/default-plugins.yaml\", \"--zap-encoder\", \"json\", \"--tracing=false\"]\n\
             \x20         ports:\n\
             \x20           - {{ name: grpc, containerPort: 9002 }}\n\
             \x20           - {{ name: grpc-health, containerPort: 9003 }}\n\
             \x20           - {{ name: metrics, containerPort: 9090 }}\n\
             \x20         env:\n\
             \x20           - {{ name: NAMESPACE, valueFrom: {{ fieldRef: {{ fieldPath: metadata.namespace }} }} }}\n\
             \x20           - {{ name: POD_NAME, valueFrom: {{ fieldRef: {{ fieldPath: metadata.name }} }} }}\n\
             \x20         volumeMounts:\n\
             \x20           - {{ name: plugins, mountPath: /config }}\n\
             \x20     volumes:\n\
             \x20       - {{ name: plugins, configMap: {{ name: {name}-epp }} }}\n\
             ---\n\
             apiVersion: v1\n\
             kind: Service\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             spec:\n\
             \x20 selector: {{ app: {name}-epp }}\n\
             \x20 ports:\n\
             \x20   - {{ name: grpc-ext-proc, port: 9002, targetPort: 9002 }}\n\
             \x20   - {{ name: http-metrics, port: 9090, targetPort: 9090 }}\n\
             ---\n\
             apiVersion: inference.networking.k8s.io/v1\n\
             kind: InferencePool\n\
             metadata: {{ name: {name}-pool, namespace: {ns} }}\n\
             spec:\n\
             \x20 selector: {{ matchLabels: {{ app: {name} }} }}\n\
             \x20 targetPorts:\n\
             \x20   - {{ number: 8000 }}\n\
             \x20 endpointPickerRef: {{ group: \"\", kind: Service, name: {name}-epp, port: {{ number: 9002 }}, failureMode: FailClose }}\n\
             ---\n\
             apiVersion: gateway.networking.k8s.io/v1\n\
             kind: HTTPRoute\n\
             metadata: {{ name: {name}-route, namespace: {ns} }}\n\
             spec:\n\
             \x20 parentRefs:\n\
             \x20   - {{ name: llm-d-gateway }}\n\
             \x20 rules:\n\
             \x20   - matches:\n\
             \x20       - {{ path: {{ type: PathPrefix, value: {path} }} }}\n\
             \x20     filters:\n\
             \x20       - {{ type: URLRewrite, urlRewrite: {{ path: {{ type: ReplacePrefixMatch, replacePrefixMatch: /v1 }} }} }}\n\
             \x20     backendRefs:\n\
             \x20       - {{ group: inference.networking.k8s.io, kind: InferencePool, name: {name}-pool }}\n",
            name = name, ns = ns, path = path
        )
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
    fn panel_cycle_and_reverse_tab() {
        let mut a = App::new();
        a.set_view_idx(View::Launch.idx()); // Deploy: 3 패널
        assert_eq!(a.panel_count(), 3);
        a.selected = 2;
        a.cycle_panel();
        assert_eq!(a.panel_focus, 1);
        assert_eq!(a.selected, 0, "패널 전환 시 선택 리셋");
        a.cycle_panel();
        a.cycle_panel();
        assert_eq!(a.panel_focus, 0, "3패널 순환");
        // 단일 패널 뷰는 순환 무시.
        a.set_view_idx(View::Nodes.idx());
        assert_eq!(a.panel_count(), 1);
        a.cycle_panel();
        assert_eq!(a.panel_focus, 0);
        // 뷰 전환하면 포커스 리셋.
        a.set_view_idx(View::Epp.idx());
        assert_eq!(a.panel_focus, 0);
        // Shift+Tab = 이전 뷰.
        let before = a.view.idx();
        a.prev_tab();
        assert_eq!(a.view.idx(), (before + View::ALL.len() - 1) % View::ALL.len());
    }

    #[test]
    fn compile_deploy_preview_generates_manifests() {
        use crate::collect::ModelArtifact;
        let mut a = App::new();
        a.snap = Snapshot {
            artifacts: vec![ModelArtifact {
                model: "koni-rbln".into(),
                family: "kisti-koni/koni-llama3.1-8b".into(),
                engine: "vLLM-RBLN".into(),
                node: "etri-001".into(),
                image: String::new(),
                source: "KISTI-KONI/KONI-Llama3.1-8B-Instruct".into(),
                mount: "/mnt/store ← PVC:model-store".into(),
                opts: vec![("tp".into(), "4".into()), ("max-len".into(), "8192".into())],
            }],
            ..Default::default()
        };
        a.view = View::Launch;
        a.panel_focus = 0;
        a.selected = 0;
        // compile: NPU(RBLN) → 옵션 폼이 열리고, 관측 opts 로 초기화.
        a.compile_preview();
        let form = a.compile_form.clone().expect("compile form opens for RBLN");
        assert_eq!(form.vendor, "rbln");
        assert_eq!(form.get("tp"), "4");
        assert_eq!(form.get("max-len"), "8192");
        assert!(form.target().contains("tp4-s8192"));
        // Enter → 폼 값으로 Job 매니페스트 생성(모델 id/타깃/스토어 경로/옵션 env 포함).
        a.compile_form_submit();
        let (title, yaml) = a.preview.clone().expect("compile preview");
        assert!(title.contains("compile"));
        assert!(yaml.contains("kind: Job"));
        assert!(yaml.contains("KISTI-KONI/KONI-Llama3.1-8B-Instruct"));
        assert!(yaml.contains("compiled/KISTI-KONI--KONI-Llama3.1-8B-Instruct/rbln/rbln-ca22-tp4-s8192"));
        assert!(yaml.contains("RBLN_TENSOR_PARALLEL_SIZE"));
        assert!(yaml.contains("RBLN_MAX_SEQ_LEN"));
        // 실기 학습 반영: 인라인 스크립트 ConfigMap + create_runtimes=False (외부 compile-script 의존 없음).
        assert!(yaml.contains("kind: ConfigMap"), "RBLN compile inlines its script as ConfigMap");
        assert!(yaml.contains("rbln_create_runtimes=False"), "compile-only (no device runtime)");
        assert!(!yaml.contains("name: compile-script }"), "no external compile-script dependency");
        // deploy: 폼 열고 replicas/디바이스/배치 → Deployment 매니페스트.
        a.open_deploy_form();
        let dform = a.deploy_form.clone().expect("deploy form opens");
        assert_eq!(dform.vendor, "rbln");
        assert_eq!(dform.get("devices"), "4"); // 아티팩트 TP
        a.deploy_form_submit();
        let (_, dyaml) = a.preview.clone().expect("deploy preview");
        assert!(dyaml.contains("kind: Deployment"));
        assert!(dyaml.contains("model-store"));
        assert!(dyaml.contains("rebellions.ai/ATOM: 4"));
        // RBLN 서빙은 vllm_rbln 런타임 + VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK (제네릭 --model 아님).
        assert!(dyaml.contains("\"serve\""), "vllm serve subcommand");
        assert!(dyaml.contains("VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK"));
        // routing=llm-d(기본) → 게이트웨이 라우팅 리소스 동봉.
        assert!(dyaml.contains("kind: InferencePool"), "generates InferencePool");
        assert!(dyaml.contains("kind: HTTPRoute"), "generates HTTPRoute");
        assert!(dyaml.contains("llm-d-router-endpoint-picker"), "generates EPP");
        assert!(dyaml.contains("value: /atom/"), "rbln → /atom/<model> path");
        // 구조 유효성: 생성 매니페스트가 실제 파싱되는 YAML 인지 테스트 시점에 검증(들여쓰기 실수 조기 검출).
        // 컴파일(ConfigMap---Job)·배포(Deployment---라우팅) 모두 다중 문서라 문서별로 파싱.
        for doc in yaml.split("\n---\n") {
            serde_yaml::from_str::<serde_yaml::Value>(doc).expect("compile manifest doc is valid YAML");
        }
        for doc in dyaml.split("\n---\n") {
            serde_yaml::from_str::<serde_yaml::Value>(doc).expect("deploy manifest doc is valid YAML");
        }
    }

    #[test]
    fn furiosa_compile_uses_fxb_build() {
        use crate::collect::ModelArtifact;
        let mut a = App::new();
        a.snap = Snapshot {
            artifacts: vec![ModelArtifact {
                model: "exaone-rngd".into(),
                family: "lgai-exaone/exaone".into(),
                engine: "Furiosa-LLM".into(),
                node: "etri-001".into(),
                image: String::new(),
                source: "LGAI-EXAONE/EXAONE-4.0".into(),
                mount: String::new(),
                opts: vec![],
            }],
            ..Default::default()
        };
        a.view = View::Launch;
        a.panel_focus = 0;
        a.selected = 0;
        a.compile_preview(); // Furiosa 엔진 → 폼 열림
        a.compile_form_submit();
        let (_, yaml) = a.preview.clone().expect("furiosa compile preview");
        assert!(yaml.contains("fxb build"), "furiosa uses fxb build CLI directly");
        assert!(yaml.contains("furiosaai/furiosa-llm:latest"), "default furiosa image");
        assert!(!yaml.contains("compile-script"), "no custom script needed for furiosa");
        assert!(yaml.contains("furiosa.ai/rngd"));
        serde_yaml::from_str::<serde_yaml::Value>(&yaml).expect("furiosa manifest is valid YAML");
    }

    #[test]
    fn deploy_manifest_vendor_correct() {
        use crate::collect::ModelArtifact;
        let mk = |engine: &str, source: &str| {
            let mut a = App::new();
            a.snap = Snapshot {
                artifacts: vec![ModelArtifact {
                    model: "m".into(),
                    family: "f".into(),
                    engine: engine.into(),
                    node: "etri-001".into(),
                    image: String::new(),
                    source: source.into(),
                    mount: "/mnt/store/compiled/x ← PVC:model-store".into(),
                    opts: vec![("tp".into(), "4".into())],
                }],
                ..Default::default()
            };
            a.view = View::Launch;
            a.panel_focus = 0;
            a.selected = 0;
            a
        };
        // Furiosa: `furiosa-llm serve <model> --tensor-parallel-size` + HF_TOKEN, furiosaai 이미지.
        let mut fa = mk("Furiosa-LLM", "furiosa-ai/Qwen3-4B-FP8");
        fa.open_deploy_form();
        fa.deploy_form_submit();
        let (_, fy) = fa.preview.clone().expect("furiosa deploy");
        assert!(fy.contains("furiosaai/furiosa-llm:latest"));
        assert!(fy.contains("\"serve\", \"furiosa-ai/Qwen3-4B-FP8\""), "serve subcommand + model positional");
        assert!(fy.contains("--tensor-parallel-size"));
        assert!(fy.contains("hf-token"), "furiosa needs HF_TOKEN secret");
        assert!(fy.contains("furiosa.ai/rngd:"));
        assert!(fy.contains("value: /rngd/"), "furiosa → /rngd/<model> route");
        for doc in fy.split("\n---\n") {
            serde_yaml::from_str::<serde_yaml::Value>(doc).expect("furiosa deploy doc is valid YAML");
        }

        // GPU: `vllm serve <path>` on nvidia.com/gpu, 컴파일 불필요.
        let mut gp = mk("vLLM", "Qwen/Qwen2.5-7B-Instruct");
        gp.open_deploy_form();
        gp.deploy_form_submit();
        let (_, gy) = gp.preview.clone().expect("gpu deploy");
        assert!(gy.contains("\"serve\""));
        assert!(gy.contains("nvidia.com/gpu:"));
        assert!(gy.contains("value: /gpu/"), "gpu → /gpu/<model> route");
        for doc in gy.split("\n---\n") {
            serde_yaml::from_str::<serde_yaml::Value>(doc).expect("gpu deploy doc is valid YAML");
        }

        // routing=direct 이면 라우팅 리소스 없음(Deployment 만).
        let mut d = mk("vLLM", "Qwen/Qwen2.5-7B-Instruct");
        d.open_deploy_form();
        if let Some(f) = d.deploy_form.as_mut() {
            if let Some(fld) = f.fields.iter_mut().find(|x| x.key == "routing") { fld.value = "direct".into(); }
        }
        d.deploy_form_submit();
        let (_, dy) = d.preview.clone().expect("direct deploy");
        assert!(!dy.contains("kind: InferencePool"), "direct = no routing resources");
        assert!(!dy.contains("kind: HTTPRoute"));
    }

    #[test]
    fn compile_preflight_flags_prereqs() {
        use crate::collect::ModelArtifact;
        let mk = |source: &str| {
            let mut a = App::new();
            a.snap = Snapshot {
                artifacts: vec![ModelArtifact {
                    model: "m".into(),
                    family: "f".into(),
                    engine: "Furiosa-LLM".into(),
                    node: "etri-001".into(),
                    image: String::new(),
                    source: source.into(),
                    mount: String::new(),
                    opts: vec![],
                }],
                ..Default::default()
            };
            a.view = View::Launch;
            a.panel_focus = 0;
            a.selected = 0;
            a
        };
        // 등록 furiosa 양자화 모델 → registry preflight 통과.
        let mut a = mk("furiosa-ai/Qwen3-4B-FP8");
        a.compile_preview();
        let pf = a.compile_preflight(a.compile_form.as_ref().unwrap());
        assert!(pf.iter().any(|(ok, m)| *ok && m.starts_with("registry")), "registered model passes registry check");
        // 원본(미양자화) 모델 → registry preflight 실패(사전 경고).
        let mut b = mk("Qwen/Qwen2.5-0.5B-Instruct");
        b.compile_preview();
        let pfb = b.compile_preflight(b.compile_form.as_ref().unwrap());
        assert!(pfb.iter().any(|(ok, m)| !*ok && m.starts_with("registry")), "unregistered model flagged before compile");
    }

    #[test]
    fn action_menu_offers_contextual_actions() {
        use crate::collect::ModelArtifact;
        let mut a = App::new();
        a.snap = Snapshot {
            artifacts: vec![ModelArtifact {
                model: "koni-rbln".into(),
                family: "kisti-koni/koni".into(),
                engine: "vLLM-RBLN".into(),
                node: "etri-001".into(),
                image: String::new(),
                source: "KISTI-KONI/KONI".into(),
                mount: String::new(),
                opts: vec![],
            }],
            ..Default::default()
        };
        a.view = View::Launch;
        a.panel_focus = 0;
        a.selected = 0;
        // NPU(RBLN) 빌드 → Info + Compile→RBLN + Deploy(배포 안 된 상태라 Stop 없음).
        a.open_action_menu();
        let m = a.action_menu.clone().expect("menu opens on Launch panel 0");
        let acts: Vec<Action> = m.items.iter().map(|i| i.action).collect();
        assert!(acts.contains(&Action::Info));
        assert!(acts.contains(&Action::Compile("rbln")));
        assert!(acts.contains(&Action::Deploy));
        assert!(!acts.contains(&Action::Stop)); // 미배포
        assert_eq!(m.by_key('c'), Some(Action::Compile("rbln")));
        assert_eq!(m.by_key('z'), None);
        // GPU vLLM 이지만 모델이 Llama 계열 → 지원 목록으로 Compile→RBLN·Furiosa 노출(GPU→NPU 경로).
        a.snap.artifacts[0].engine = "vLLM".into();
        a.action_menu = None;
        a.open_action_menu();
        let g = a.action_menu.clone().unwrap();
        assert!(g.items.iter().any(|i| i.action == Action::Compile("rbln")));
        assert!(g.items.iter().any(|i| i.action == Action::Compile("furiosa")));
    }

    #[test]
    fn objective_advice_flags_and_recommends() {
        use crate::collect::PerfRow;
        let mut a = App::new();
        a.objectives.insert(
            "koni".into(),
            Objective { ttft_ms: Some(2000.0), tpot_ms: None, e2e_ms: Some(1000.0), min_tps: Some(100.0) },
        );
        // 느슨한 매칭(서빙명 ≠ 키).
        assert!(a.objective_for("koni-llama3.1-8b").is_some());
        // E2E 위반 + decode 지배 + tok/s 미달.
        let row = PerfRow {
            model: "koni-llama3.1-8b".into(),
            req: 5.0,
            tps: 40.0,             // < 100 → 위반
            ttft_p95: 0.5,         // 500ms ≤ 2000 → 충족
            tpot_p95: f64::NAN,
            e2e_p95: 3.0,          // 3000ms > 1000 → 위반
            in_tok_p95: f64::NAN,
            out_tok_p95: f64::NAN,
            queue_p95: 0.1,
            prefill_p95: 0.2,
            decode_p95: 2.5,       // decode 지배
            preempt: 0.0,
        };
        let adv = a.perf_advice(&row);
        assert!(adv.has_obj);
        assert!(!adv.all_met());
        assert!(adv.checks.iter().any(|(m, ok)| *m == "E2E" && !*ok));
        assert!(adv.checks.iter().any(|(m, ok)| *m == "TTFT" && *ok));
        assert!(adv.tips.iter().any(|t| t.contains("decode")));
        assert!(adv.tips.iter().any(|t| t.contains("tok/s")));
        // 목표 없는 모델 → has_obj=false.
        let row2 = PerfRow { model: "other".into(), ..row };
        assert!(!a.perf_advice(&row2).has_obj);
    }

    #[test]
    fn compile_fit_flags_oom_and_recommends() {
        use crate::collect::ModelArtifact;
        let mk = |source: &str| App {
            snap: Snapshot {
                artifacts: vec![ModelArtifact {
                    model: "m".into(),
                    family: source.to_lowercase(),
                    engine: "vLLM-RBLN".into(),
                    node: "etri-001".into(),
                    image: String::new(),
                    source: source.into(),
                    mount: String::new(),
                    opts: vec![],
                }],
                ..Default::default()
            },
            view: View::Launch,
            panel_focus: 0,
            selected: 0,
            ..App::new()
        };
        // 8B on RBLN TP4 s8192 batch1 → fits (칩당 ~여유).
        let mut a = mk("meta-llama/Llama-3.1-8B-Instruct");
        a.compile_preview();
        let form = a.compile_form.clone().unwrap();
        let fit = a.compile_fit(&form);
        assert_eq!(fit.params_b, Some(8.0));
        assert!(matches!(fit.verdict, FitVerdict::Fits | FitVerdict::Tight));
        // 70B on RBLN TP4 → 칩당 초과 → OOM + 양자화 제안.
        let mut b = mk("meta-llama/Llama-3.1-70B-Instruct");
        b.compile_preview();
        let bform = b.compile_form.clone().unwrap();
        let bfit = b.compile_fit(&bform);
        assert_eq!(bfit.params_b, Some(70.0));
        assert_eq!(bfit.verdict, FitVerdict::Oom);
        assert!(bfit.tips.iter().any(|t| t.contains("양자화")));
        // 크기 미상 → Unknown.
        let mut c = mk("some-org/mystery-model");
        c.compile_preview();
        let cform = c.compile_form.clone().unwrap();
        assert_eq!(c.compile_fit(&cform).verdict, FitVerdict::Unknown);
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
