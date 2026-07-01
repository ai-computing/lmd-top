//! UI 상태머신 — 현재 뷰, 선택, 스파크라인 히스토리. 데이터(Snapshot)와 분리.

use crate::collect::Snapshot;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};

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
    pub cols: HashMap<String, Vec<String>>, // 뷰별 표시 컬럼(순서) — 설정파일
    pub catalog: Vec<crate::catalog::CatModel>, // 모델 카탈로그(런처)
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
            cols: load_columns(),
            catalog: crate::catalog::load(),
        }
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
    pub fn cycle_theme(&mut self) {
        let n = (theme() + 1) % N_THEMES;
        THEME.store(n, Ordering::Relaxed);
        self.toast = Some(format!("theme: {}", theme_name(n)));
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
            // 클러스터 레벨 추이(timeline)
            let n = snap.accel.len().max(1);
            let util_avg = snap.accel.iter().map(|a| a.util).sum::<f64>() / n as f64;
            let (mu, mt): (f64, f64) = snap.accel.iter().fold((0.0, 0.0), |(u, t), a| (u + a.mem_used_gb, t + a.mem_total_gb));
            let vram_pct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
            self.push_hist("sys:util", util_avg.round().clamp(0.0, 100.0) as u64);
            self.push_hist("sys:vram", vram_pct.round().clamp(0.0, 100.0) as u64);
            let tps = snap.perf.tps;
            if !tps.is_nan() {
                self.push_hist("sys:tps", tps.round().max(0.0) as u64);
            }
            let lat = snap.perf.e2e_p95;
            if !lat.is_nan() {
                self.push_hist("sys:lat", (lat * 1000.0).round().max(0.0) as u64);
            }
            let rq = snap.perf.req_rate;
            if !rq.is_nan() {
                self.push_hist("sys:reqs", (rq * 100.0).round().max(0.0) as u64);
            }
        }
        self.snap = snap;
        let n = self.list_len();
        if n > 0 && self.selected >= n {
            self.selected = n - 1;
        }
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
}
