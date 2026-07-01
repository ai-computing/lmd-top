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

/// 확인(y/n) 대기 중인 변경 작업. 실행은 이벤트 루프(main)에서.
#[derive(Clone)]
pub enum Pending {
    Scale { name: String, target: i64 },
}
impl Pending {
    /// 확인 프롬프트 문구.
    pub fn prompt(&self) -> String {
        match self {
            Pending::Scale { name, target } => format!("scale {} → {} replica(s)?", name, target),
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
pub const N_THEMES: usize = 3;
pub fn theme() -> usize {
    THEME.load(Ordering::Relaxed)
}
pub fn theme_name(i: usize) -> &'static str {
    match i {
        1 => "high-contrast",
        2 => "colorblind",
        _ => "default",
    }
}

#[derive(Clone, Copy, PartialEq)]
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
            View::Routing => "Topo",
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
}

/// ~/.config/lmd-top/lmd-top.yaml 의 columns: {view: [col,...]} 로드. 없으면 빈 맵(=기본 전체).
fn load_columns() -> HashMap<String, Vec<String>> {
    let path = std::env::var("HOME").map(|h| format!("{}/.config/lmd-top/lmd-top.yaml", h)).unwrap_or_default();
    let txt = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };
    let v: serde_yaml::Value = match serde_yaml::from_str(&txt) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
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
            _ => String::new(),
        }
    }

    pub fn toggle_detail(&mut self) {
        if self.list_len() > 0 {
            self.detail = !self.detail;
        }
    }

    /// 현재 뷰의 정렬 모드 수(순환용).
    pub fn sort_modes(&self) -> usize {
        match self.view {
            View::Accel => 4,  // util / temp / mem / name
            View::Models => 3, // name / status / ready
            View::Pods => 3,   // name / phase / restarts
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
            self.detect_alerts(&snap);
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
        let mut current: Vec<Alert> = Vec::new();
        // 가속기: not-alive / throttle / 고온
        for a in &snap.accel {
            let base = format!("{}/{}/{}", a.kind.label(), a.node, a.id);
            if !a.alive {
                current.push(Alert { ts: now, sev: Sev::Bad, key: format!("dead:{}", base), msg: format!("{} {} not alive @{}", a.kind.label(), a.id, a.node) });
            } else if a.throttle > 0.0 {
                current.push(Alert { ts: now, sev: Sev::Warn, key: format!("thr:{}", base), msg: format!("{} {} throttling @{}", a.kind.label(), a.id, a.node) });
            }
            if a.temp > ALERT_TEMP_BAD {
                current.push(Alert { ts: now, sev: Sev::Warn, key: format!("temp:{}", base), msg: format!("{} {} hot {:.0}°C @{}", a.kind.label(), a.id, a.temp, a.node) });
            }
        }
        // 노드: cordon / notready / pressure
        for n in &snap.nodes {
            if n.cordoned {
                current.push(Alert { ts: now, sev: Sev::Warn, key: format!("cordon:{}", n.name), msg: format!("node {} cordoned", n.name) });
            } else if !n.ready {
                current.push(Alert { ts: now, sev: Sev::Bad, key: format!("notready:{}", n.name), msg: format!("node {} NotReady", n.name) });
            } else if n.pressure {
                current.push(Alert { ts: now, sev: Sev::Warn, key: format!("pressure:{}", n.name), msg: format!("node {} under pressure", n.name) });
            }
        }
        // pod: 재시작 증가(델타) / Failed
        for p in &snap.pods {
            let prev = self.prev_restarts.get(&p.name).copied().unwrap_or(p.restarts);
            if p.restarts > prev {
                current.push(Alert { ts: now, sev: Sev::Warn, key: format!("restart:{}:{}", p.name, p.restarts), msg: format!("pod {} restarted (x{})", p.name, p.restarts) });
            }
            if p.phase == "Failed" {
                current.push(Alert { ts: now, sev: Sev::Bad, key: format!("failed:{}", p.name), msg: format!("pod {} Failed", p.name) });
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
    }
    pub fn scroll_detail(&mut self, delta: i64) {
        self.detail_scroll = (self.detail_scroll as i64 + delta).max(0) as u16;
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
            _ => Vec::new(),
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
