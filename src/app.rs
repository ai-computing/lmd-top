//! UI state machine — current view, selection, sparkline history. Separate from data (Snapshot).

use crate::collect::Snapshot;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};

// impl App is split across submodules (see each file's header for scope).
mod action;
mod activity;
mod compile;
mod deploy;
mod filter;
mod library;
mod nav;
mod objective;
mod order;
mod select;
mod setup;
mod sort;
mod state;
mod zoo;

pub use setup::{CheckState, SetupFix};

/// Alert severity.
#[derive(Clone, Copy, PartialEq)]
pub enum Sev {
    Warn,
    Bad,
}

/// Permission mode (prevents operational accidents) — declaration order = privilege level (Observe < … < Danger).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Observe, // view only
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

/// A single sortable column (label + default direction). Used by per-view sort_cols().
#[derive(Clone, Copy)]
pub struct SortCol {
    pub label: &'static str,
    pub desc: bool, // default direction when this column is selected (true=descending) — for numeric columns, largest-first is useful
}

/// Cross-layer drill breadcrumb — pivot pushes the current position, esc retraces it.
#[derive(Clone)]
pub struct NavState {
    pub view: View,
    pub selected: usize,
    pub filter: String,
    pub detail: bool,
}

/// A mutating operation awaiting confirmation (y/n). Executed in the event loop (main).
#[derive(Clone)]
pub enum Pending {
    Scale {
        name: String,
        target: i64,
    },
    Restart {
        name: String,
    },
    Stop {
        name: String,
    }, // stop serving = replicas to 0 (frees devices, reversible)
    Apply {
        title: String,
        yaml: String,
    }, // kubectl apply the previewed manifest
    ApplyUrl {
        title: String,
        url: String,
    }, // kubectl apply -f <url> — 상류 릴리스 매니페스트(예: Gateway/Inference CRD) 설치
    Cordon {
        node: String,
        on: bool,
    }, // block/allow scheduling on a node
    DeletePod {
        name: String,
    }, // delete pod
    DeleteJob {
        name: String,
    }, // delete compile Job (cancel / clean up)
    RouteRename {
        route: String,
        old: String,
        new: String,
    }, // change HTTPRoute path
    RouteRetarget {
        route: String,
        path: String,
        backend: String,
        kind: String,
    }, // change backend
    RouteDelete {
        route: String,
        path: String,
    }, // delete route rule
}
impl Pending {
    /// Confirmation prompt text.
    pub fn prompt(&self) -> String {
        match self {
            Pending::Scale { name, target } => format!("scale {} → {} replica(s)?", name, target),
            Pending::Restart { name } => format!("rollout restart {} (rolling)?", name),
            Pending::Stop { name } => format!("stop serving {} (scale → 0, frees devices)?", name),
            Pending::Apply { title, .. } => format!("apply manifest to cluster — {}?", title),
            Pending::ApplyUrl { title, url } => {
                format!("apply upstream manifest — {} (from {})?", title, url)
            }
            Pending::Cordon { node, on } => format!(
                "{} node {}?",
                if *on {
                    "cordon (block scheduling on)"
                } else {
                    "uncordon (allow scheduling on)"
                },
                node
            ),
            Pending::DeletePod { name } => format!("delete pod {} (reschedules)?", name),
            Pending::DeleteJob { name } => {
                format!("delete compile job {} (cancel / clean up)?", name)
            }
            Pending::RouteRename { old, new, .. } => format!("rename route {} → {}?", old, new),
            Pending::RouteRetarget {
                path,
                backend,
                kind,
                ..
            } => format!("retarget {} → {}:{}?", path, kind, backend),
            Pending::RouteDelete { path, route } => {
                format!("delete route {} from {}?", path, route)
            }
        }
    }
}

/// A single threshold-exceeded / abnormal-state event. key = stable identifier for dedup / edge detection.
#[derive(Clone)]
pub struct Alert {
    pub ts: u64,
    pub sev: Sev,
    pub key: String,
    pub msg: String,
}

// Feature types (CompileForm/DeployForm/Action/Objective/Fit etc.) are split into crate::ops and re-exported.
pub use crate::ops::*;

/// Compile Job name — `compile-{model}-{target}`. Including the target (vendor·chip·tp·pp·seq)
/// lets "same model, different options" compiles coexist as distinct Jobs (same identity as the store path).
/// 컴파일 타깃 문자열(예: "rbln-ca22-tp4-s8192" / "rngd-tp4-pp1-s8192" / "RNGD-tp4-s8192")을
/// 사람이 읽는 컴파일 옵션 (라벨, 값) 목록으로 디코드. 스토어 컴파일본의 compiled_for 가 무슨
/// 옵션을 담았는지(TP/PP/seq/칩) 상세 패널·행에서 풀어 보여주기 위함. `target()` 인코딩의 역함수.
pub fn decode_compiled_for(s: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let digits_after = |lc: &str, p: &str| -> Option<String> {
        lc.strip_prefix(p)
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .map(|n| n.to_string())
    };
    for tok in s.split(['-', '_', ' ']).filter(|t| !t.is_empty()) {
        let lc = tok.to_lowercase();
        if lc == "rbln" {
            out.push(("vendor", "RBLN (Rebellions)".into()));
        } else if lc == "rngd" || lc == "furiosa" {
            out.push(("vendor", "RNGD (Furiosa)".into()));
        } else if lc == "hf" {
            out.push(("format", "HF source weights".into()));
        } else if let Some(n) = digits_after(&lc, "tp") {
            out.push(("tensor-parallel", n));
        } else if let Some(n) = digits_after(&lc, "pp") {
            out.push(("pipeline-parallel", n));
        } else if let Some(n) = digits_after(&lc, "s") {
            out.push(("max-seq-len", n));
        } else if lc.starts_with("ca") || lc.starts_with("rbln") {
            out.push(("npu-chip", tok.to_uppercase()));
        } else {
            out.push(("target", tok.to_string()));
        }
    }
    out
}

/// Guarantees DNS-1123 label rules (lowercase [a-z0-9-], ≤63 chars, alphanumeric at both ends). When exceeded, the model part is truncated
/// and a short option-identifying hash is appended to keep uniqueness (target is always preserved).
pub fn compile_job_name(repo_dir: &str, target: &str) -> String {
    let sanitize = |s: &str| -> String {
        s.to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    };
    let model = sanitize(repo_dir);
    let target = sanitize(target);
    // 63 (DNS-1123) - "-script" (RBLN ConfigMap suffix) = 56, with margin. Safe for both Job and CM.
    const MAX: usize = 56;
    // Fixed part: "compile-" + "-" + target. Allot the remaining budget to the model part.
    let fixed = "compile-".len() + 1 + target.len();
    let budget = MAX.saturating_sub(fixed);
    let model = if model.len() > budget {
        // Preserve uniqueness with a short hash (truncating only the prefix could let different models collide).
        let mut h: u64 = 1469598103934665603; // FNV-1a
        for b in repo_dir.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        let hash = format!("{:x}", h & 0xffffff); // up to 6 hex
        let keep = budget.saturating_sub(hash.len() + 1);
        format!("{}-{}", &model[..keep.min(model.len())], hash)
    } else {
        model
    };
    // Trim so it doesn't end with '-' at either end (DNS-1123).
    let raw = format!("compile-{}-{}", model, target);
    raw.trim_matches('-').to_string()
}

/// Alert threshold (conceptually matches ui.rs color thresholds — here it's the "alarm" trigger line).
const ALERT_TEMP_BAD: f64 = 80.0;

/// Global theme index (0=default, 1=high-contrast, 2=colorblind-friendly). Read by the ui color functions.
pub static THEME: AtomicUsize = AtomicUsize::new(0);
pub const N_THEMES: usize = 4;
pub fn theme() -> usize {
    THEME.load(Ordering::Relaxed)
}
/// Set the startup theme (from the config value). Out-of-range values are ignored.
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
    Epp,
    Routing,
    Pods,
    Perf,
    Serving,  // Serving 섹션 랜딩: 현재 서빙 중인 배포(라이브 아티팩트 트리) + 라이프사이클
    Library,  // Deploy 섹션: 위=Model List(배포 가능) · 아래=Activity(compile/deploy 작업) 2패널
    Events,
    Nodes,
    Topo,  // Nodes hub's topology / device pressure map (Canvas)
    Zoo,   // Deploy 섹션: 벤더(Furiosa/Rebellions) 모델 zoo — prefetch/compile/deploy
    Setup, // 새 클러스터 부트스트랩 점검(Doctor): 플랫폼 전제조건 present/missing + 가이드된 apply
}

impl View {
    /// Every view — for headless render coverage and exhaustive iteration (not a nav order).
    pub const EVERY: [View; 13] = [
        View::Overview,
        View::Routing,
        View::Epp,
        View::Serving,
        View::Perf,
        View::Pods,
        View::Nodes,
        View::Accel,
        View::Topo,
        View::Library,
        View::Zoo,
        View::Events,
        View::Setup,
    ];
    /// Which top-level section this view belongs to (a view is one sub-tab of its section).
    pub fn section(&self) -> Section {
        match self {
            View::Overview => Section::Overview,
            View::Routing | View::Epp => Section::Traffic,
            View::Serving | View::Perf | View::Pods => Section::Serving,
            View::Nodes | View::Accel | View::Topo => Section::Infra,
            View::Library | View::Zoo => Section::Deploy,
            View::Events => Section::Events,
            View::Setup => Section::Setup,
        }
    }
    /// Sub-tab label within a section (short; the section carries the group name).
    pub fn title(&self) -> &'static str {
        match self {
            View::Overview => "Overview",
            View::Accel => "Devices",
            View::Epp => "EPP",
            View::Routing => "Flow",
            View::Pods => "Pods",
            View::Perf => "Perf",
            View::Serving => "Serving",  // 현재 서빙 중인 배포(라이프사이클)
            View::Library => "Deploy",   // Model List + Activity (2패널)
            View::Events => "Events",
            View::Nodes => "Nodes",
            View::Topo => "Topology",
            View::Zoo => "Zoo",
            View::Setup => "Setup",
        }
    }
}

/// Top-level navigation section — the request path reads Gateway → EPP → Model → Infra.
/// Each section groups one or more views as sub-tabs (cycled with `←` `→` / `[` `]`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Overview, // cluster at-a-glance
    Traffic,  // Flow · EPP  (gateway → route → pool → picker)
    Serving,  // Serving · Perf · Pods  (running deployments — manage & observe)
    Infra,    // Nodes · Devices · Topology  (heterogeneous accelerators)
    Deploy,   // Model List · Activity  (provision: deployable models + compile/deploy jobs)
    Events,   // events + alerts
    Setup,    // 플랫폼 부트스트랩 점검(새 환경 셋업 시 전제조건 진단 + 가이드된 apply)
}
impl Section {
    /// Number-key order (0-6) and tab order.
    pub const ALL: [Section; 7] = [
        Section::Overview,
        Section::Traffic,
        Section::Serving,
        Section::Infra,
        Section::Deploy,
        Section::Events,
        Section::Setup,
    ];
    pub fn idx(&self) -> usize {
        Section::ALL.iter().position(|s| s == self).unwrap_or(0)
    }
    pub fn title(&self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Traffic => "Traffic",
            Section::Serving => "Serving",
            Section::Infra => "Infra",
            Section::Deploy => "Deploy",
            Section::Events => "Events",
            Section::Setup => "Setup",
        }
    }
    /// Sub-tabs (views) of this section, in `[`/`]` cycle order. First entry is the landing view.
    pub fn members(&self) -> &'static [View] {
        match self {
            Section::Overview => &[View::Overview],
            Section::Traffic => &[View::Routing, View::Epp],
            // Serving: 현재 서빙 중인 배포(랜딩) → Perf → Pods.
            Section::Serving => &[View::Serving, View::Perf, View::Pods],
            Section::Infra => &[View::Nodes, View::Accel, View::Topo],
            // Deploy: Library(위 Model List · 아래 Activity) + Zoo(벤더 모델 zoo).
            Section::Deploy => &[View::Library, View::Zoo],
            Section::Events => &[View::Events],
            Section::Setup => &[View::Setup],
        }
    }
}

/// Deploy▸Library 통합 배포 트리의 한 항목 — 조직 카탈로그 정의 또는 스토어에 실재하는 컴파일본.
/// 둘을 한 리스트에 섞어 family 로 묶으므로, 사용자는 패널을 옮기지 않고 배포 가능한 모든 것을 한 곳에서 고른다.
#[derive(Clone, Copy)]
pub enum LibItem {
    Catalog(usize), // self.catalog[i]
    Stored(usize),  // self.snap.stored[i]
}

pub const HIST: usize = 40;

pub struct App {
    pub view: View,
    pub selected: usize,
    pub snap: Snapshot,
    // ── Deploy/compile target context (used for manifest generation · apply) ──
    pub ns: String, // target namespace (cfg.ns) — injected instead of hardcoded
    pub img_rbln: Option<String>, // LMD_COMPILE_IMAGE_RBLN — placeholder if absent
    pub img_furiosa: Option<String>, // LMD_COMPILE_IMAGE_FURIOSA — default furiosaai/furiosa-llm:latest
    pub img_serving: Option<String>, // LMD_SERVING_IMAGE — placeholder if absent
    pub hist: HashMap<String, VecDeque<u64>>, // accel util history
    pub toast: Option<String>,
    pub detail: bool,       // whether to show the selected row's detail (drill-down)
    pub sort: usize,        // current view's sort column index (per sort_cols, cycled with o)
    pub sort_desc: bool, // sort direction (true=descending) — toggled with O, resets to default on column change
    pub tick: u64,       // render tick (for marquee/spinner animation)
    pub filter: String,  // row filter (substring match)
    pub filtering: bool, // filter input mode
    pub help: bool,      // help/legend overlay
    pub zoom: bool,      // focus (zoom) — hide header/tabs and maximize body
    pub paused: bool,    // pause screen refresh (data frozen, for reading)
    pub detail_scroll: u16, // vertical scroll within detail
    pub dev_sel: usize, // device cursor within Node detail: 0=node summary, 1..=n=that device's history
    pub panel_focus: usize, // active (focused) panel index in multi-panel views (moved with Ctrl-w + hjkl)
    pub panel_move: bool, // vi/tmux-style panel-focus mode — armed by Ctrl-w, hjkl/arrows move, Esc exits
    pub preview: Option<(String, String)>, // (title, YAML) preview overlay (generated manifest or read-only YAML)
    pub preview_scroll: u16,
    pub preview_apply: bool, // true=generated manifest (v to verify, a to apply), false=read-only (describe/yaml)
    pub compile_form: Option<CompileForm>, // NPU compile options edit form (c → edit → Enter → preview)
    pub deploy_form: Option<DeployForm>, // deploy (serving) options edit form (d → edit → Enter → preview)
    pub place_picker: Option<PlacePick>, // deploy 폼의 place 필드 → 후보 노드 상태 목록에서 선택
    pub action_menu: Option<ActionMenu>, // Enter context action menu (Info/Compile/Deploy/Stop…)
    pub objectives: HashMap<String, Objective>, // per-model serving objective (SLO) — user input
    pub objective_form: Option<ObjectiveForm>, // objective edit form
    pub logs_mode: bool,                 // logs overlay
    pub logs_target: String,             // logs target pod
    pub logs: Vec<String>,               // log lines
    pub logs_scroll: u16,
    pub cols: HashMap<String, Vec<String>>, // per-view displayed columns (order) — config file
    pub catalog: Vec<crate::catalog::CatModel>, // model catalog (launcher)
    pub zoo: Vec<crate::catalog::ZooModel>, // vendor model zoo (Deploy▸Zoo)
    // ── Active alerts ──
    pub alerts: VecDeque<Alert>,         // history (newest first), cap 50
    pub active_alerts: HashSet<String>,  // currently active keys (for edge detection)
    pub alerts_panel: bool,              // alert history overlay (A)
    pub flash_until: u64,                // epoch secs — flash the summary bar until this time
    pub toast_until: u64,                // epoch secs — toast expiry
    pub toast_bad: bool,                 // toast background color (red=critical)
    prev_restarts: HashMap<String, i64>, // track pod restart delta
    // ── Permission mode ──
    pub mode: Mode, // observe(default)/debug/admin/danger — --mode at startup
    pub confirm: Option<Pending>, // mutating operation awaiting confirmation (popup)
    pub confirm_yes: bool, // confirm popup's Yes/No selection state (default No=safe)
    pub exit_confirm: bool, // quit confirmation popup
    pub inflight: Option<String>, // label of an in-flight mutating operation (worker thread) — shows spinner. None=none
    pub route_form: Option<RouteForm>, // route edit form (rename/retarget)
    pub palette: Option<crate::palette::Palette>, // command palette (open with `:` for fuzzy search of views/display actions)
    // ── Cross-layer drill ──
    pub nav_stack: Vec<NavState>, // pivot breadcrumb (retraced with esc)
    // ── Perf drill ──
    pub perf_detail: Option<crate::collect::PerfDetail>, // selected model's latency distribution (on-demand on Enter)
    // ── EPP scorer weight what-if (local sim, no cluster change) ──
    pub epp_weights: HashMap<String, f64>, // scorer name → adjusted weight override
    // ── Session energy (cumulative mJ baseline) ──
    pub energy_base: HashMap<String, f64>, // device key → cumulative energy at session start (mJ)
    pub energy_since: u64,                 // session start epoch secs
}

/// Load columns: {view: [col,...]} from ~/.config/lmd-top/lmd-top.yaml. Empty map if absent (=default all).
fn load_columns() -> HashMap<String, Vec<String>> {
    let v = match crate::config::load_yaml() {
        Some(v) => v,
        None => return HashMap::new(),
    };
    let mut out = HashMap::new();
    if let Some(m) = v.get("columns").and_then(|c| c.as_mapping()) {
        for (k, val) in m {
            if let (Some(view), Some(seq)) = (k.as_str(), val.as_sequence()) {
                let cols: Vec<String> = seq
                    .iter()
                    .filter_map(|s| s.as_str().map(|x| x.to_string()))
                    .collect();
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
            img_furiosa: env("LMD_COMPILE_IMAGE_FURIOSA")
                .or_else(|| Some("furiosaai/furiosa-llm:latest".into())),
            img_serving: env("LMD_SERVING_IMAGE"),
            hist: HashMap::new(),
            toast: None,
            detail: false,
            sort: 0,
            sort_desc: true,
            tick: 0,
            filter: String::new(),
            filtering: false,
            help: false,
            zoom: false,
            paused: false,
            detail_scroll: 0,
            dev_sel: 0,
            panel_focus: 0,
            panel_move: false,
            preview: None,
            preview_scroll: 0,
            preview_apply: false,
            compile_form: None,
            deploy_form: None,
            place_picker: None,
            action_menu: None,
            objectives: HashMap::new(),
            objective_form: None,
            logs_mode: false,
            logs_target: String::new(),
            logs: Vec::new(),
            logs_scroll: 0,
            cols: load_columns(),
            catalog: crate::catalog::load(),
            zoo: crate::catalog::load_zoo(),
            alerts: VecDeque::new(),
            active_alerts: HashSet::new(),
            alerts_panel: false,
            flash_until: 0,
            toast_until: 0,
            toast_bad: false,
            prev_restarts: HashMap::new(),
            mode: Mode::Observe,
            confirm: None,
            confirm_yes: false, // default No — Enter cancels unless the user explicitly selects Yes
            exit_confirm: false,
            inflight: None,
            route_form: None,
            palette: None,
            nav_stack: Vec::new(),
            perf_detail: None,
            epp_weights: HashMap::new(),
            energy_base: HashMap::new(),
            energy_since: 0,
        }
    }

    /// Stable device key (shared by energy/history).
    pub fn accel_key(a: &crate::collect::Accel) -> String {
        format!("{}:{}:{}", a.kind.label(), a.node, a.id)
    }
    /// Session energy (Wh) = (current cumulative − baseline) mJ / 3.6e6. NaN if no data.
    pub fn energy_session_wh(&self, a: &crate::collect::Accel) -> f64 {
        if a.energy_mj.is_nan() {
            return f64::NAN;
        }
        let base = self
            .energy_base
            .get(&Self::accel_key(a))
            .copied()
            .unwrap_or(a.energy_mj);
        (a.energy_mj - base).max(0.0) / 3.6e6
    }
    /// Reset session energy (R) — set the current cumulative as the new baseline.
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

    /// EPP what-if: adjust the selected scorer's weight by delta (local override, ≥0).
    pub fn epp_adjust(&mut self, delta: f64) {
        if self.view != View::Epp || self.panel_focus != 0 {
            return; // only when the scorers panel is focused
        }
        let ord = self.order();
        if let (Some(cfg), Some(&i)) = (&self.snap.epp, ord.get(self.selected)) {
            if let Some((name, base)) = cfg.scorers.get(i) {
                let cur = *self.epp_weights.get(name).unwrap_or(base);
                self.epp_weights
                    .insert(name.clone(), (cur + delta).max(0.0));
            }
        }
    }
    /// Effective scorer weight (the override if present, otherwise base).
    pub fn epp_weight(&self, name: &str, base: f64) -> f64 {
        *self.epp_weights.get(name).unwrap_or(&base)
    }

    /// Backend (model) name of the route selected in Flow(Topo) — for pivoting to a layer from the path.
    pub fn selected_route_backend(&self) -> Option<String> {
        if self.view == View::Routing {
            self.sel_orig()
                .and_then(|i| self.snap.routes.get(i))
                .map(|r| r.backend.clone())
        } else {
            None
        }
    }

    /// Route selected in Flow (only when panel 0 is focused).
    pub fn selected_route(&self) -> Option<crate::collect::Route> {
        if self.view == View::Routing && self.panel_focus == 0 {
            self.sel_orig()
                .and_then(|i| self.snap.routes.get(i))
                .cloned()
        } else {
            None
        }
    }

    /// Open the route rename form — current path as the initial value, text-editable.
    pub fn open_route_rename(&mut self) {
        let Some(r) = self.selected_route() else {
            return;
        };
        if r.route.is_empty() {
            self.notify("route: HTTPRoute name unknown — cannot edit".into());
            return;
        }
        self.route_form = Some(RouteForm {
            route: r.route,
            path: r.path.clone(),
            rename: true,
            value: r.path,
            choices: vec![],
            cursor: 0,
        });
    }

    /// Open the route retarget form — pick from a list of candidate backends (InferencePool/Service).
    pub fn open_route_retarget(&mut self) {
        let Some(r) = self.selected_route() else {
            return;
        };
        if r.route.is_empty() {
            self.notify("route: HTTPRoute name unknown — cannot edit".into());
            return;
        }
        // Candidates: current InferencePools + serving Services (kind:name).
        let mut choices: Vec<String> = self
            .snap
            .pools
            .iter()
            .map(|p| format!("InferencePool:{}", p.name))
            .collect();
        for m in &self.snap.models {
            let s = format!("Service:{}", m.name);
            if !choices.contains(&s) {
                choices.push(s);
            }
        }
        let cur = format!("{}:{}", r.kind, r.backend);
        let cursor = choices.iter().position(|c| *c == cur).unwrap_or(0);
        let value = choices.get(cursor).cloned().unwrap_or_default();
        self.route_form = Some(RouteForm {
            route: r.route,
            path: r.path,
            rename: false,
            value,
            choices,
            cursor,
        });
    }

    /// Model (service) name of the selected per-model perf row — for Perf drill. Via sel_orig (sort/filter-safe).
    pub fn selected_perf_model(&self) -> Option<String> {
        if self.view == View::Perf && self.panel_focus == 0 {
            self.sel_orig()
                .and_then(|i| self.snap.perf_rows.get(i))
                .map(|r| r.model.clone())
        } else {
            None
        }
    }

    /// Pod name → owning model (deployment) name. Prefix match, else the pod name as-is.
    fn model_of_pod(&self, pod: &str) -> String {
        self.snap
            .models
            .iter()
            .find(|m| pod.starts_with(&m.name))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| pod.to_string())
    }

    // (navigation — pivot/tabs/panels/list/detail — moved to src/app/nav.rs)

    // impl App is split across submodules; see the `mod` declarations at the top
    // of this file. This block keeps core state, navigation, and route helpers.
}

/// 상태 없는 임계/헬스 조건 → 알림 목록(엣지 검출·재시작 델타 제외). UI 알림과 agent JSON 공유.
pub fn snapshot_alerts(snap: &Snapshot) -> Vec<Alert> {
    let now = snap.ts;
    let mut out: Vec<Alert> = Vec::new();
    for a in &snap.accel {
        let base = format!("{}/{}/{}", a.disp(), a.node, a.id);
        if !a.alive {
            out.push(Alert {
                ts: now,
                sev: Sev::Bad,
                key: format!("dead:{}", base),
                msg: format!("{} {} not alive @{}", a.disp(), a.id, a.node),
            });
        } else if a.throttle > 0.0 {
            out.push(Alert {
                ts: now,
                sev: Sev::Warn,
                key: format!("thr:{}", base),
                msg: format!("{} {} throttling @{}", a.disp(), a.id, a.node),
            });
        }
        if a.temp > ALERT_TEMP_BAD {
            out.push(Alert {
                ts: now,
                sev: Sev::Warn,
                key: format!("temp:{}", base),
                msg: format!("{} {} hot {:.0}°C @{}", a.disp(), a.id, a.temp, a.node),
            });
        }
    }
    for n in &snap.nodes {
        if n.cordoned {
            out.push(Alert {
                ts: now,
                sev: Sev::Warn,
                key: format!("cordon:{}", n.name),
                msg: format!("node {} cordoned", n.name),
            });
        } else if !n.ready {
            out.push(Alert {
                ts: now,
                sev: Sev::Bad,
                key: format!("notready:{}", n.name),
                msg: format!("node {} NotReady", n.name),
            });
        } else if n.pressure {
            out.push(Alert {
                ts: now,
                sev: Sev::Warn,
                key: format!("pressure:{}", n.name),
                msg: format!("node {} under pressure", n.name),
            });
        }
        // 루트 디스크 고갈 경보(90% 초과).
        if n.disk_total_gb > 0.0 {
            let dp = n.disk_used_gb / n.disk_total_gb * 100.0;
            if dp > 90.0 {
                out.push(Alert {
                    ts: now,
                    sev: Sev::Warn,
                    key: format!("disk:{}", n.name),
                    msg: format!("node {} disk {:.0}% full", n.name, dp),
                });
            }
        }
    }
    for p in &snap.pods {
        if p.phase == "Failed" {
            out.push(Alert {
                ts: now,
                sev: Sev::Bad,
                key: format!("failed:{}", p.name),
                msg: format!("pod {} Failed", p.name),
            });
        }
    }
    out
}

/// 크로스레이어 1줄 진단 → (문구, 심각도). None = 정상(healthy). UI diagnosis 와 agent JSON 공유.
pub fn diagnose(s: &Snapshot) -> (String, Option<Sev>) {
    let serving = s.serving_count();
    if s.accel.is_empty() && serving == 0 {
        return (
            "no accelerator metrics + no serving models — check Prometheus / model state".into(),
            Some(Sev::Bad),
        );
    }
    if serving == 0 {
        return (
            "0 models serving — press 's' in Models to start one (no backend)".into(),
            Some(Sev::Warn),
        );
    }
    let warns = s.events.iter().filter(|e| e.typ == "Warning").count();
    if warns > 0 {
        let top = s
            .events
            .iter()
            .find(|e| e.typ == "Warning")
            .map(|e| e.reason.clone())
            .unwrap_or_default();
        return (
            format!(
                "{} model(s) serving · {} warning event(s) (top: {}) — see Events",
                serving, warns, top
            ),
            Some(Sev::Warn),
        );
    }
    let busy = s.accel.iter().filter(|a| a.util > 80.0).count();
    if busy > 0 {
        return (
            format!(
                "{} model(s) serving, {} accelerator(s) hot (>80%)",
                serving, busy
            ),
            None,
        );
    }
    (
        format!("{} model(s) serving, accelerators have headroom", serving),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// form_submit 후 확인 팝업(Pending::Apply)에 담긴 (title, yaml) 추출 — 테스트 헬퍼.
    fn submitted(a: &App) -> (String, String) {
        match a.confirm.as_ref() {
            Some(Pending::Apply { title, yaml }) => (title.clone(), yaml.clone()),
            _ => panic!("expected Apply confirm after form_submit"),
        }
    }
    use crate::collect::{ModelRow, PodRow, Snapshot};

    fn model(name: &str) -> ModelRow {
        ModelRow {
            name: name.into(),
            ready: 1,
            desired: 1,
            status: "● Running".into(),
            route: "/x".into(),
            engine: "vllm".into(),
            accel: "-".into(),
            running: None,
            waiting: None,
            tps: None,
            kv: None,
            ttft: None,
        }
    }
    fn pod(name: &str) -> PodRow {
        PodRow {
            name: name.into(),
            phase: "Running".into(),
            ready: "1/1".into(),
            node: "n1".into(),
            restarts: 0,
            age_secs: 0,
        }
    }
    fn app_with(models: Vec<ModelRow>, pods: Vec<PodRow>) -> App {
        let mut a = App::new();
        a.snap = Snapshot {
            models,
            pods,
            ..Default::default()
        };
        a.view = View::Overview; // flat model list + pivots live on Overview now
        a
    }

    #[test]
    fn confirm_defaults_to_no_and_destructive_actions_need_danger() {
        let a = App::new();
        assert!(!a.confirm_yes, "confirmation popups should default to No");
        assert_eq!(Action::Scale.required_mode(), Mode::Admin);
        assert_eq!(Action::RouteRetarget.required_mode(), Mode::Admin);
        assert_eq!(Action::Delete.required_mode(), Mode::Danger);
        assert_eq!(Action::DeleteJob.required_mode(), Mode::Danger);
        assert_eq!(Action::RouteDelete.required_mode(), Mode::Danger);
    }

    #[test]
    fn pivot_roundtrip_restores_state() {
        let mut a = app_with(vec![model("m1")], vec![pod("m1-abc")]);
        a.pivot('p'); // Overview → Pods filtered by m1
        assert_eq!(a.view, View::Pods);
        assert_eq!(a.filter, "m1");
        assert!(a.nav_back());
        assert_eq!(a.view, View::Overview);
        assert_eq!(a.filter, "");
        assert_eq!(a.selected, 0);
        assert!(a.nav_stack.is_empty());
    }

    #[test]
    fn pivot_empty_landing_reverts() {
        // 매칭되는 pod 없음 → 막다른 빈 화면 대신 되짚어야 함
        let mut a = app_with(vec![model("lonely")], vec![]);
        a.pivot('p');
        assert_eq!(a.view, View::Overview);
        assert_eq!(a.filter, "");
        assert!(a.nav_stack.is_empty());
    }

    #[test]
    fn panel_cycle_and_reverse_tab() {
        use crate::app::Section;
        let mut a = App::new();
        a.goto_view(View::Epp); // Traffic▸EPP: 2 패널(scorers / InferencePool)
        assert_eq!(a.panel_count(), 2);
        a.selected = 2;
        a.cycle_panel_dir(1); // Ctrl-w — 패널 포커스만 이동(서브탭과 직교)
        assert_eq!(a.panel_focus, 1);
        assert_eq!(a.selected, 0, "패널 전환 시 선택 리셋");
        a.cycle_panel_dir(1);
        assert_eq!(a.panel_focus, 0, "2패널 순환");
        // Deploy(Library)는 2패널(Model List / Activity), Serving 은 단일 패널.
        a.goto_view(View::Library);
        assert_eq!(a.panel_count(), 2);
        a.goto_view(View::Serving);
        assert_eq!(a.panel_count(), 1);
        // 단일 패널 뷰는 순환 무시.
        a.goto_view(View::Nodes);
        assert_eq!(a.panel_count(), 1);
        a.cycle_panel_dir(1);
        assert_eq!(a.panel_focus, 0);
        // 뷰 전환하면 포커스 리셋.
        a.goto_view(View::Epp);
        assert_eq!(a.panel_focus, 0);
        // Shift+Tab = 이전 섹션.
        let before = a.view.section().idx();
        a.prev_tab();
        let n = Section::ALL.len();
        assert_eq!(a.view.section().idx(), (before + n - 1) % n);
    }

    #[test]
    fn subtab_cycles_within_section() {
        let mut a = App::new();
        a.goto_view(View::Serving); // Serving 섹션: Serving→Perf→Pods
        a.cycle_subtab(1);
        assert_eq!(a.view, View::Perf);
        a.cycle_subtab(1);
        assert_eq!(a.view, View::Pods);
        a.cycle_subtab(1);
        assert_eq!(a.view, View::Serving, "서브탭 순환");
        // 단일 멤버 섹션(Overview)은 서브탭 순환 무시.
        a.goto_view(View::Overview);
        a.cycle_subtab(1);
        assert_eq!(a.view, View::Overview);
    }

    // 회귀: pivot 으로 브레드크럼을 쌓은 뒤 서브탭을 바꾸면 Esc(nav_back)가 옛 pivot 출발점으로
    // 튀지 않아야 한다(브레드크럼이 비워짐). — 사용자 보고 "Esc 가 이상한 곳으로".
    #[test]
    fn subtab_switch_clears_stale_breadcrumb() {
        let mut a = app_with(vec![model("m1")], vec![pod("m1-abc")]);
        a.pivot('p'); // Models → Pods, nav_stack=[Models]
        assert_eq!(a.view, View::Pods);
        assert!(!a.nav_stack.is_empty(), "pivot 은 브레드크럼을 쌓는다");
        a.cycle_subtab(1); // Pods → Models(같은 섹션 서브탭) = 새 내비게이션
        assert!(
            a.nav_stack.is_empty(),
            "서브탭 전환이 stale 브레드크럼을 비운다"
        );
        assert!(!a.nav_back(), "Esc 는 이제 엉뚱한 곳으로 되짚지 않는다");
    }

    #[test]
    fn section_number_lands_on_first_subtab() {
        use crate::app::Section;
        let mut a = App::new();
        a.goto_section(Section::Infra.idx()); // Infra: Nodes→Devices→Topology
        assert_eq!(a.view, View::Nodes);
        a.goto_section(Section::Traffic.idx()); // Traffic: Flow→EPP
        assert_eq!(a.view, View::Routing);
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
        a.view = View::Serving;
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
        let (title, yaml) = submitted(&a);
        assert!(title.contains("compile"));
        assert!(yaml.contains("kind: Job"));
        assert!(yaml.contains("KISTI-KONI/KONI-Llama3.1-8B-Instruct"));
        assert!(yaml
            .contains("compiled/KISTI-KONI--KONI-Llama3.1-8B-Instruct/rbln/rbln-ca22-tp4-s8192"));
        assert!(yaml.contains("RBLN_TENSOR_PARALLEL_SIZE"));
        assert!(yaml.contains("RBLN_MAX_SEQ_LEN"));
        // 실기 학습 반영: 인라인 스크립트 ConfigMap + create_runtimes=False (외부 compile-script 의존 없음).
        assert!(
            yaml.contains("kind: ConfigMap"),
            "RBLN compile inlines its script as ConfigMap"
        );
        assert!(
            yaml.contains("rbln_create_runtimes=False"),
            "compile-only (no device runtime)"
        );
        assert!(
            !yaml.contains("name: compile-script }"),
            "no external compile-script dependency"
        );
        // deploy: 폼 열고 replicas/디바이스/배치 → Deployment 매니페스트.
        a.open_deploy_form();
        let dform = a.deploy_form.clone().expect("deploy form opens");
        assert_eq!(dform.vendor, "rbln");
        assert_eq!(dform.get("devices"), "4"); // 아티팩트 TP
        a.deploy_form_submit();
        let (_, dyaml) = submitted(&a);
        assert!(dyaml.contains("kind: Deployment"));
        assert!(dyaml.contains("model-store"));
        assert!(dyaml.contains("rebellions.ai/ATOM: 4"));
        // RBLN serving uses either a configured vllm_rbln image (`vllm serve`) or the host-stack fallback (`api_server --model`).
        assert!(
            dyaml.contains("\"serve\"") || dyaml.contains("api_server --model="),
            "RBLN serving command"
        );
        assert!(dyaml.contains("VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK"));
        // routing=llm-d(기본) → 게이트웨이 라우팅 리소스 동봉.
        assert!(
            dyaml.contains("kind: InferencePool"),
            "generates InferencePool"
        );
        assert!(dyaml.contains("kind: HTTPRoute"), "generates HTTPRoute");
        assert!(
            dyaml.contains("llm-d-router-endpoint-picker"),
            "generates EPP"
        );
        assert!(dyaml.contains("value: /atom/"), "rbln → /atom/<model> path");
        // 구조 유효성: 생성 매니페스트가 실제 파싱되는 YAML 인지 테스트 시점에 검증(들여쓰기 실수 조기 검출).
        // 컴파일(ConfigMap---Job)·배포(Deployment---라우팅) 모두 다중 문서라 문서별로 파싱.
        for doc in yaml.split("\n---\n") {
            serde_yaml::from_str::<serde_yaml::Value>(doc)
                .expect("compile manifest doc is valid YAML");
        }
        for doc in dyaml.split("\n---\n") {
            serde_yaml::from_str::<serde_yaml::Value>(doc)
                .expect("deploy manifest doc is valid YAML");
        }
    }

    // GPU(HF) 아티팩트를 NPU 로 컴파일(GPU→NPU 경로)해도 Job 이름이 모델+옵션(vendor·tp·seq)으로
    // 유일해야 하고, 같은 모델의 RBLN·Furiosa Job 이 서로 충돌하지 않아야 한다. Job 은 끝나면
    // 자동정리(ttlSecondsAfterFinished)되어야 한다. (사용자 보고: qwen 재컴파일 "field is immutable")
    #[test]
    fn compile_job_name_encodes_model_and_options() {
        use crate::collect::ModelArtifact;
        let mk = |engine: &str| {
            let mut a = App::new();
            a.mode = Mode::Admin;
            a.snap = Snapshot {
                artifacts: vec![ModelArtifact {
                    model: "vllm-qwen05b-gb10".into(),
                    family: "qwen/qwen2.5-0.5b".into(),
                    engine: engine.into(),
                    node: String::new(),
                    image: String::new(),
                    source: "Qwen/Qwen2.5-0.5B-Instruct".into(),
                    mount: String::new(),
                    opts: vec![("max-len".into(), "8192".into())],
                }],
                ..Default::default()
            };
            a.view = View::Serving;
            a.panel_focus = 0;
            a.selected = 0;
            a
        };
        let name_for = |vendor: &'static str| -> String {
            let mut a = mk("vLLM"); // GPU 엔진 → GPU→NPU 컴파일 경로
            a.compile_form_for(vendor);
            a.compile_form_submit();
            let (_t, yaml) = submitted(&a);
            for doc in yaml.split("\n---\n") {
                serde_yaml::from_str::<serde_yaml::Value>(doc).expect("compile doc valid YAML");
            }
            assert!(
                yaml.contains("ttlSecondsAfterFinished"),
                "job auto-cleans after finish"
            );
            yaml.lines()
                .find(|l| {
                    l.contains("kind: Job")
                        || l.trim_start().starts_with("metadata: { name: compile-")
                })
                .map(|_| ())
                .unwrap_or(());
            // Job metadata.name 추출(ConfigMap 의 -script 는 제외).
            yaml.lines()
                .filter_map(|l| l.trim().strip_prefix("metadata: { name: "))
                .map(|s| s.split(',').next().unwrap_or("").trim().to_string())
                .find(|n| n.starts_with("compile-") && !n.ends_with("-script"))
                .expect("job name present")
        };
        let rbln = name_for("rbln");
        let furiosa = name_for("furiosa");
        assert_ne!(rbln, furiosa, "rbln·furiosa jobs must not collide");
        assert!(
            rbln.contains("rbln") && rbln.contains("tp") && rbln.contains("s8192"),
            "rbln name: {}",
            rbln
        );
        assert!(
            furiosa.contains("rngd") && furiosa.contains("tp"),
            "furiosa name: {}",
            furiosa
        );
        assert!(
            rbln.len() <= 56 && furiosa.len() <= 56,
            "DNS-1123 + -script 여유"
        );
    }

    // 스토어에 같은 모델·같은 target 산출물이 이미 있으면 preflight 에 재컴파일 경고(⚠, 블로커 아님).
    #[test]
    fn compile_preflight_flags_already_stored() {
        use crate::collect::{ModelArtifact, StoredModel};
        let mut a = App::new();
        a.mode = Mode::Admin;
        a.snap = Snapshot {
            artifacts: vec![ModelArtifact {
                model: "koni-rbln".into(),
                family: "kisti-koni/koni-llama3.1-8b".into(),
                engine: "vLLM-RBLN".into(),
                node: "etri-001".into(),
                source: "KISTI-KONI/KONI-Llama3.1-8B-Instruct".into(),
                mount: "/mnt/store".into(),
                opts: vec![],
                ..Default::default()
            }],
            // 폼 기본값(tp4·s8192·RBLN-CA22)과 정확히 일치하는 산출물이 이미 스토어에 있음.
            stored: vec![StoredModel {
                repo: "KISTI-KONI/KONI-Llama3.1-8B-Instruct".into(),
                family: "kisti-koni/koni-llama3.1-8b".into(),
                format: "rbln".into(),
                compiled_for: "rbln-ca22-tp4-s8192".into(),
                revision: "-".into(),
                size: "8G".into(),
                path: "compiled/KISTI-KONI--KONI-Llama3.1-8B-Instruct/rbln/rbln-ca22-tp4-s8192/"
                    .into(),
            }],
            ..Default::default()
        };
        a.view = View::Serving;
        a.panel_focus = 0;
        a.selected = 0;
        a.compile_preview();
        let form = a.compile_form.clone().expect("rbln form");
        assert!(
            a.compile_already_stored(&form).is_some(),
            "정확 일치 산출물 감지"
        );
        let pf = a.compile_preflight(&form);
        assert!(
            pf.iter().any(|(_, m)| m.contains("이미 컴파일됨")),
            "preflight 에 재컴파일 경고"
        );
        // 경고는 블로커가 아니므로 ok=true(폼 진행 가능).
        assert!(pf
            .iter()
            .find(|(_, m)| m.contains("이미 컴파일됨"))
            .map(|(ok, _)| *ok)
            .unwrap());
        // 옵션이 다르면(seq 변경) 다른 target → 중복 아님.
        let mut form2 = form.clone();
        if let Some(f) = form2.fields.iter_mut().find(|f| f.key == "max-len") {
            f.value = "16384".into();
        }
        assert!(
            a.compile_already_stored(&form2).is_none(),
            "옵션 다르면 재컴파일 아님"
        );
    }

    #[test]
    fn compile_job_name_dns1123_bounds() {
        // 아주 긴 모델 id 라도 63자 이하, 소문자/영숫자-하이픈, 양끝 영숫자, 타깃 식별자 보존.
        let long =
            "some-org/an-extremely-long-model-name-that-easily-exceeds-the-limit-14b-instruct";
        let repo = long.replace('/', "--");
        let n = compile_job_name(&repo, "rbln-ca22-tp4-s8192");
        assert!(n.len() <= 56, "len {}: {}", n.len(), n);
        assert!(n.starts_with("compile-") && n.ends_with(|c: char| c.is_ascii_alphanumeric()));
        assert!(n
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        // 다른 긴 모델은 (해시로) 다른 이름이어야 함.
        let n2 = compile_job_name(
            &"some-org/another-extremely-long-model-name-that-also-exceeds-limit-14b"
                .replace('/', "--"),
            "rbln-ca22-tp4-s8192",
        );
        assert_ne!(n, n2, "긴 이름도 해시로 구분");
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
        a.view = View::Serving;
        a.panel_focus = 0;
        a.selected = 0;
        a.compile_preview(); // Furiosa 엔진 → 폼 열림
        a.compile_form_submit();
        let (_, yaml) = submitted(&a);
        assert!(
            yaml.contains("fxb build"),
            "furiosa uses fxb build CLI directly"
        );
        assert!(
            yaml.contains("furiosaai/furiosa-llm:latest"),
            "default furiosa image"
        );
        assert!(
            !yaml.contains("compile-script"),
            "no custom script needed for furiosa"
        );
        // 컴파일은 AOT → 가속기 디바이스를 예약하지 않음(furiosa.ai/rngd limits 없음). cpu/mem 만.
        assert!(
            !yaml.contains("furiosa.ai/rngd:"),
            "compile is AOT — no device reservation"
        );
        assert!(
            yaml.contains("cpu:") && yaml.contains("memory:"),
            "compile requests cpu/mem only"
        );
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
            a.view = View::Serving;
            a.panel_focus = 0;
            a.selected = 0;
            a
        };
        // Furiosa: `furiosa-llm serve <model> --tensor-parallel-size` + HF_TOKEN, furiosaai 이미지.
        let mut fa = mk("Furiosa-LLM", "furiosa-ai/Qwen3-4B-FP8");
        fa.open_deploy_form();
        fa.deploy_form_submit();
        let (_, fy) = submitted(&fa);
        assert!(fy.contains("furiosaai/furiosa-llm:latest"));
        assert!(
            fy.contains("\"serve\", \"furiosa-ai/Qwen3-4B-FP8\""),
            "serve subcommand + model positional"
        );
        assert!(fy.contains("--tensor-parallel-size"));
        assert!(fy.contains("hf-token"), "furiosa needs HF_TOKEN secret");
        assert!(fy.contains("furiosa.ai/rngd:"));
        assert!(
            fy.contains("value: /rngd/"),
            "furiosa → /rngd/<model> route"
        );
        for doc in fy.split("\n---\n") {
            serde_yaml::from_str::<serde_yaml::Value>(doc)
                .expect("furiosa deploy doc is valid YAML");
        }

        // GPU: `vllm serve <path>` on nvidia.com/gpu, 컴파일 불필요.
        let mut gp = mk("vLLM", "Qwen/Qwen2.5-7B-Instruct");
        gp.open_deploy_form();
        gp.deploy_form_submit();
        let (_, gy) = submitted(&gp);
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
            if let Some(fld) = f.fields.iter_mut().find(|x| x.key == "routing") {
                fld.value = "direct".into();
            }
        }
        d.deploy_form_submit();
        let (_, dy) = submitted(&d);
        assert!(
            !dy.contains("kind: InferencePool"),
            "direct = no routing resources"
        );
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
            a.view = View::Serving;
            a.panel_focus = 0;
            a.selected = 0;
            a
        };
        // 등록 furiosa 양자화 모델 → registry preflight 통과.
        let mut a = mk("furiosa-ai/Qwen3-4B-FP8");
        a.compile_preview();
        let pf = a.compile_preflight(a.compile_form.as_ref().unwrap());
        assert!(
            pf.iter().any(|(ok, m)| *ok && m.starts_with("registry")),
            "registered model passes registry check"
        );
        // 원본(미양자화) 모델 → registry preflight 실패(사전 경고).
        let mut b = mk("Qwen/Qwen2.5-0.5B-Instruct");
        b.compile_preview();
        let pfb = b.compile_preflight(b.compile_form.as_ref().unwrap());
        assert!(
            pfb.iter().any(|(ok, m)| !*ok && m.starts_with("registry")),
            "unregistered model flagged before compile"
        );
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
        a.view = View::Serving;
        a.panel_focus = 0;
        a.selected = 0;
        // Serving = 운영 렌즈: Info/Logs/YAML/Scale/Restart/Objective. 컴파일·신규 배포는 Model List 로 이동.
        a.open_action_menu();
        let m = a.action_menu.clone().expect("menu opens on Serving panel 0");
        let acts: Vec<Action> = m.items.iter().map(|i| i.action).collect();
        assert!(acts.contains(&Action::Info));
        assert!(acts.contains(&Action::Scale));
        assert!(acts.contains(&Action::Restart));
        assert!(acts.contains(&Action::Objective));
        assert!(acts.contains(&Action::Logs));
        assert!(
            !acts.iter().any(|x| matches!(x, Action::Compile(_))),
            "compile 은 Model List 로 이동"
        );
        assert!(!acts.contains(&Action::Deploy), "deploy 는 Model List 로 이동");
        assert!(!acts.contains(&Action::Stop), "미배포(models 없음)면 Stop 없음");
        assert_eq!(m.by_key('s'), Some(Action::Scale));
        assert_eq!(m.by_key('z'), None);
        // 같은 이름의 배포가 desired>0 이면 Stop 노출.
        a.snap.models = vec![model("koni-rbln")]; // model(): desired=1
        a.action_menu = None;
        a.open_action_menu();
        let d = a.action_menu.clone().unwrap();
        assert!(
            d.items.iter().any(|i| i.action == Action::Stop),
            "배포 상태면 Stop 노출"
        );
    }

    #[test]
    fn objective_advice_flags_and_recommends() {
        use crate::collect::PerfRow;
        let mut a = App::new();
        a.objectives.insert(
            "koni".into(),
            Objective {
                ttft_ms: Some(2000.0),
                tpot_ms: None,
                e2e_ms: Some(1000.0),
                min_tps: Some(100.0),
            },
        );
        // 느슨한 매칭(서빙명 ≠ 키).
        assert!(a.objective_for("koni-llama3.1-8b").is_some());
        // E2E 위반 + decode 지배 + tok/s 미달.
        let row = PerfRow {
            model: "koni-llama3.1-8b".into(),
            req: 5.0,
            tps: 40.0,     // < 100 → 위반
            ttft_p95: 0.5, // 500ms ≤ 2000 → 충족
            tpot_p95: f64::NAN,
            e2e_p95: 3.0, // 3000ms > 1000 → 위반
            in_tok_p95: f64::NAN,
            out_tok_p95: f64::NAN,
            queue_p95: 0.1,
            prefill_p95: 0.2,
            decode_p95: 2.5, // decode 지배
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
        let row2 = PerfRow {
            model: "other".into(),
            ..row
        };
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
            view: View::Serving,
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
        assert_eq!(a.view, View::Overview);
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
            artifacts: vec![crate::collect::ModelArtifact {
                model: "m1".into(),
                family: "m1".into(),
                engine: "vLLM".into(),
                node: String::new(),
                image: String::new(),
                source: "org/m1".into(),
                mount: String::new(),
                opts: vec![],
            }],
            routes: vec![Route {
                path: "/v1".into(),
                backend: "m1".into(),
                kind: "InferencePool".into(),
                route: "openai-route".into(),
            }],
            ..Default::default()
        };
        a.view = View::Routing;
        a.selected = 0;
        // toggle_detail 은 상세 없는 Routing 에선 detail 을 켜지 않음(↑↓ 네비 잠김 방지).
        a.toggle_detail();
        assert!(
            !a.detail,
            "Routing has no detail panel; detail must stay off so nav is not trapped"
        );
        // Enter 의 실제 동작: 백엔드 배포 상세로 드릴(Serving).
        a.drill_route();
        assert_eq!(a.view, View::Serving);
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

    #[test]
    fn compile_progress_bar_renders() {
        use crate::collect::CompileJob;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut a = App::new();
        a.snap = Snapshot {
            compiles: vec![
                CompileJob {
                    name: "compile-qwen-rbln-tp4".into(),
                    model: "qwen05b".into(),
                    vendor: "RBLN".into(),
                    target: "rbln-tp4".into(),
                    status: "Running".into(),
                    age_secs: 42,
                    duration_secs: None,
                    phase: "compiling 45%".into(),
                    progress: Some(0.45),
                },
                CompileJob {
                    name: "compile-llama-rngd".into(),
                    model: "llama".into(),
                    vendor: "RNGD".into(),
                    target: "rngd".into(),
                    status: "Running".into(),
                    age_secs: 10,
                    duration_secs: None,
                    phase: "loading weights".into(), // 진행률 없음 → indeterminate
                    progress: None,
                },
            ],
            ..Default::default()
        };
        a.view = View::Library; // Deploy 하단 Activity 패널 — compile 진행바 표시
        a.panel_focus = 1; // 하단 Activity 패널 포커스
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(140, 30)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(c) = buf.cell((x, y)) {
                    text.push_str(c.symbol());
                }
            }
        }
        assert!(
            text.contains("45%"),
            "determinate bar should show parsed percent"
        );
        assert!(
            text.contains('█'),
            "progress bar should render filled cells"
        );
        assert!(text.contains('░'), "progress bar should render empty cells");
    }

    #[test]
    fn inflight_spinner_renders_in_title() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut a = App::new();
        a.inflight = Some("scale ds4 → 0".into());
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(120, 24)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let buf = t.backend().buffer().clone();
        // 타이틀 행(0)에 진행 라벨이 보여야.
        let mut row0 = String::new();
        for x in 0..buf.area.width {
            if let Some(c) = buf.cell((x, 0)) {
                row0.push_str(c.symbol());
            }
        }
        assert!(
            row0.contains("scale ds4"),
            "in-flight label should render in title: {:?}",
            row0
        );
    }

    #[test]
    fn column_sort_cycles_and_toggles_direction() {
        let mut a = App::new();
        let mk = |name: &str, tps: f64| {
            let mut m = model(name);
            m.tps = Some(tps);
            m
        };
        a.snap = Snapshot {
            models: vec![mk("aaa", 10.0), mk("bbb", 30.0), mk("ccc", 20.0)],
            ..Default::default()
        };
        a.goto_view(View::Overview);
        // 기본: col0=name, 오름차순.
        assert_eq!(a.sort_label(), "name");
        assert!(!a.sort_desc, "name column defaults ascending");
        let names = |a: &App| {
            a.order()
                .iter()
                .map(|&i| a.snap.models[i].name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&a), vec!["aaa", "bbb", "ccc"]);
        // o 를 tok/s 컬럼까지 순환(name→status→ready→tok/s = 3회).
        a.cycle_sort();
        a.cycle_sort();
        a.cycle_sort();
        assert_eq!(a.sort_label(), "tok/s");
        assert!(a.sort_desc, "tok/s (numeric) defaults descending");
        assert_eq!(names(&a), vec!["bbb", "ccc", "aaa"]); // 30,20,10 내림차순
                                                          // O 로 방향 토글 → 오름차순.
        a.toggle_sort_dir();
        assert!(!a.sort_desc);
        assert_eq!(names(&a), vec!["aaa", "ccc", "bbb"]); // 10,20,30 오름차순
    }

    #[test]
    fn sort_arrow_marks_table_header() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut a = App::new();
        // 헤더 마킹 검증엔 행 데이터 불필요 — 빈 테이블도 헤더는 그려진다.
        a.goto_view(View::Accel);
        // 기본: util 내림차순 → 헤더에 "UTIL▼".
        assert_eq!(a.sort_header_label(), "UTIL");
        assert_eq!(a.sort_arrow(), "▼");
        let render = |a: &App| {
            let mut fx = crate::ui::FxState::disabled();
            let mut t = Terminal::new(TestBackend::new(140, 20)).unwrap();
            t.draw(|f| crate::ui::draw(f, a, &mut fx)).unwrap();
            let buf = t.backend().buffer().clone();
            let mut s = String::new();
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    if let Some(c) = buf.cell((x, y)) {
                        s.push_str(c.symbol());
                    }
                }
            }
            s
        };
        assert!(
            render(&a).contains("UTIL▼"),
            "active sort column header should show ▼"
        );
        // O 로 방향 토글 → ▲.
        a.toggle_sort_dir();
        assert!(
            render(&a).contains("UTIL▲"),
            "toggled direction should show ▲"
        );
    }

    #[test]
    fn nodes_and_events_are_now_sortable() {
        use crate::collect::{EventRow, NodeInfo};
        let mut a = App::new();
        a.snap = Snapshot {
            nodes: vec![
                NodeInfo {
                    name: "n1".into(),
                    cpu_pct: 20.0,
                    ..Default::default()
                },
                NodeInfo {
                    name: "n2".into(),
                    cpu_pct: 80.0,
                    ..Default::default()
                },
            ],
            events: vec![
                EventRow {
                    typ: "Normal".into(),
                    reason: "Started".into(),
                    object: "p1".into(),
                    message: "".into(),
                    count: 1,
                },
                EventRow {
                    typ: "Warning".into(),
                    reason: "Failed".into(),
                    object: "p2".into(),
                    message: "".into(),
                    count: 5,
                },
            ],
            ..Default::default()
        };
        // Nodes: 이전엔 정렬 불가(sort_modes==1) → 이제 컬럼 여러 개.
        a.goto_view(View::Nodes);
        assert!(a.sort_modes() > 1, "Nodes should now be sortable");
        a.cycle_sort(); // name → cpu
        assert_eq!(a.sort_label(), "cpu");
        // cpu 내림차순 → n2(80) 먼저.
        assert_eq!(a.order().first().copied(), Some(1));
        // Events: 이전엔 정렬 불가 → count 컬럼 정렬.
        a.goto_view(View::Events);
        assert!(a.sort_modes() > 1, "Events should now be sortable");
        while a.sort_label() != "count" {
            a.cycle_sort();
        }
        assert_eq!(a.order().first().copied(), Some(1)); // count 5 먼저(내림)
    }

    #[test]
    fn overlay_precedence_single_source() {
        use crate::ui::Overlay;
        // PRECEDENCE must include every variant exactly once; missing entries are not drawn/consumed.
        assert_eq!(Overlay::PRECEDENCE.len(), 13);
        let mut seen = std::collections::HashSet::new();
        for ov in Overlay::PRECEDENCE {
            assert!(
                seen.insert(format!("{:?}", ov)),
                "duplicate in PRECEDENCE: {:?}",
                ov
            );
        }
        // 아무 오버레이도 없으면 top()==None → 단일키 디스패치.
        let mut a = App::new();
        assert_eq!(Overlay::top(&a), None);
        // 단독으로 열면 그 오버레이가 top.
        a.palette = Some(crate::palette::Palette::global());
        assert_eq!(Overlay::top(&a), Some(Overlay::Palette));
        a.palette = None;
        a.logs_mode = true;
        a.preview = Some(("t".into(), "y".into()));
        // preview 가 logs 보다 위(PRECEDENCE 순서).
        assert_eq!(Overlay::top(&a), Some(Overlay::Preview));
        // confirm 은 preview/logs 보다 위.
        a.confirm = Some(Pending::Stop { name: "m1".into() });
        assert_eq!(Overlay::top(&a), Some(Overlay::Confirm));
        // help 가 최상위.
        a.help = true;
        assert_eq!(Overlay::top(&a), Some(Overlay::Help));
    }

    #[test]
    fn palette_opens_filters_and_renders() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut a = App::new();
        a.open_palette();
        assert!(a.palette.is_some());
        // "epp" 로 필터 → 최상위 선택이 EPP 뷰.
        for c in "epp".chars() {
            a.palette.as_mut().unwrap().push(c);
        }
        assert_eq!(
            a.palette.as_ref().unwrap().selected(),
            Some(crate::palette::PaletteAction::Goto(View::Epp))
        );
        // 오버레이가 실제로 그려지는지 — 버퍼에 프롬프트/라벨이 나타나야(패닉 없음 + 내용 검증).
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(c) = buf.cell((x, y)) {
                    text.push_str(c.symbol());
                }
            }
        }
        assert!(
            text.contains("command palette"),
            "palette title should render"
        );
        assert!(text.contains("EPP"), "filtered EPP row should render");
        // 작은 터미널에서도 패닉 없이.
        let mut t2 = Terminal::new(TestBackend::new(40, 12)).unwrap();
        t2.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
    }

    // Deploy 개편: Serving/Library 두 렌즈가 family›version›target 계층으로 렌더되는지 +
    // 적응형 접기(family 에 version 하나면 version 티어 생략)가 동작하는지.
    #[test]
    fn deploy_views_render_and_group_hierarchically() {
        use crate::collect::ModelArtifact;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let buf_text = |buf: &ratatui::buffer::Buffer| {
            let mut s = String::new();
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    if let Some(c) = buf.cell((x, y)) {
                        s.push_str(c.symbol());
                    }
                }
            }
            s
        };
        let art = |model: &str, family: &str, source: &str, engine: &str| ModelArtifact {
            model: model.into(),
            family: family.into(),
            engine: engine.into(),
            node: "etri-001".into(),
            image: String::new(),
            source: source.into(),
            mount: String::new(),
            opts: vec![("tp".into(), "4".into())],
        };
        let mut running = model("llama-a");
        running.ready = 2;
        running.desired = 2;
        let mut a = App::new();
        a.snap = Snapshot {
            artifacts: vec![
                // 같은 family(llama3) · 서로 다른 version(source) 둘 → version 티어 노출.
                art("llama-a", "llama3", "meta-llama/Llama-3.1-8B-Instruct", "vLLM-RBLN"),
                art("llama-b", "llama3", "meta-llama/Llama-3.1-8B-Instruct-FP8", "vLLM"),
                // family 하나에 version 하나 → version 티어 접힘(적응형).
                art("qwen-a", "qwen2.5", "Qwen/Qwen2.5-0.5B", "Furiosa-LLM"),
            ],
            models: vec![running],
            ..Default::default()
        };
        // Serving 렌즈 — 균일 정렬표. 3개 배포 모두 행으로.
        a.view = View::Serving;
        a.selected = 0;
        assert_eq!(a.order().len(), 3, "3개 배포 모두 표에");
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let text = buf_text(t.backend().buffer());
        assert!(
            text.contains("STATUS") && text.contains("MODEL"),
            "정렬표 헤더\n{text}"
        );
        assert!(text.contains("llama-a"), "모델 행 표시\n{text}");
        assert!(
            text.contains("Serving") && text.contains("2/2"),
            "상태·replica 반영(Serving · 2/2)\n{text}"
        );

        // Model List 렌즈 — 카탈로그 트리(임베드 카탈로그) + 패닉 없음.
        a.view = View::Library;
        a.panel_focus = 0;
        a.selected = 0;
        let mut t2 = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t2.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let ltext = buf_text(t2.backend().buffer());
        assert!(ltext.contains("Model List"), "Model List 타이틀\n{ltext}");
        assert!(!a.library_items().is_empty(), "임베드 카탈로그 로드됨");
    }

    // 회귀: Furiosa 로 컴파일한 스토어 빌드가 Library 에서 선택·배포 가능해야 한다.
    // (개편 직후 store-only 컴파일본이 어느 뷰에도 안 나와 선택 불가였던 버그 — 사용자 보고.)
    #[test]
    fn furiosa_store_build_is_selectable_and_deployable() {
        use crate::collect::StoredModel;
        let mut a = App::new();
        a.mode = Mode::Admin;
        a.snap = Snapshot {
            stored: vec![StoredModel {
                repo: "furiosa-ai/Qwen3-4B-FP8".into(),
                family: "qwen3".into(),
                revision: "-".into(),
                format: "furiosa".into(),
                compiled_for: "RNGD-tp4-s8192".into(),
                size: "9G".into(),
                path: "compiled/furiosa-ai--Qwen3-4B-FP8/furiosa/rngd-tp4".into(),
            }],
            ..Default::default()
        };
        a.view = View::Library;
        a.panel_focus = 0; // 통합 배포 트리
        // 통합 트리에서 스토어 빌드의 위치로 커서 이동(카탈로그가 먼저 나열됨).
        a.selected = a
            .library_items()
            .iter()
            .position(|it| matches!(it, LibItem::Stored(_)))
            .expect("stored build present in unified library tree");
        // 선택 가능해야(예전엔 어느 뷰에도 안 떠서 선택 불가).
        let s = a
            .selected_stored()
            .expect("furiosa store build selectable in Library panel 0");
        assert_eq!(s.format, "furiosa");
        // 액션 메뉴에 Deploy 노출.
        a.open_action_menu();
        let acts: Vec<Action> = a
            .action_menu
            .as_ref()
            .expect("menu opens on store build")
            .items
            .iter()
            .map(|i| i.action)
            .collect();
        assert!(acts.contains(&Action::Deploy), "store build offers Deploy");
        // Deploy 폼 → 제출 → Furiosa Deployment 매니페스트.
        a.open_deploy_form();
        assert!(a.deploy_form.is_some(), "deploy form opens from store build");
        a.deploy_form_submit();
        let (_, yaml) = submitted(&a);
        assert!(yaml.contains("kind: Deployment"), "generates Deployment\n{yaml}");
        assert!(
            yaml.to_lowercase().contains("furiosa") || yaml.contains("rngd"),
            "furiosa serving manifest\n{yaml}"
        );
    }

    // 통합 트리: 카탈로그가 상단(스토어 빌드보다 먼저), 선택 항목 상세 패널이 뜬다.
    #[test]
    fn library_catalog_on_top_and_detail_renders() {
        use crate::collect::StoredModel;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut a = App::new(); // 임베드 카탈로그 로드됨
        a.snap = Snapshot {
            stored: vec![StoredModel {
                repo: "furiosa-ai/Qwen3-4B-FP8".into(),
                family: "qwen3".into(),
                revision: "-".into(),
                format: "furiosa".into(),
                compiled_for: "RNGD-tp4-s8192".into(),
                size: "9G".into(),
                path: "compiled/x".into(),
            }],
            ..Default::default()
        };
        // 카탈로그가 상단: 통합 리스트의 앞쪽은 전부 카탈로그, 스토어 빌드는 뒤.
        let items = a.library_items();
        let first_stored = items
            .iter()
            .position(|it| matches!(it, LibItem::Stored(_)))
            .unwrap();
        assert!(
            items[..first_stored]
                .iter()
                .all(|it| matches!(it, LibItem::Catalog(_))),
            "카탈로그가 스토어 빌드보다 위에 온다"
        );
        // 선택 항목 상세 패널 — 카탈로그 모델의 placement/소스가 보인다.
        a.view = View::Library;
        a.panel_focus = 0;
        a.selected = a
            .library_items()
            .iter()
            .position(|it| matches!(it, LibItem::Catalog(c) if a.catalog[*c].id == "qwen3-4b-fp8"))
            .expect("qwen3-4b-fp8 in catalog");
        a.detail = true;
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(120, 20)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let buf = t.backend().buffer();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(c) = buf.cell((x, y)) {
                    text.push_str(c.symbol());
                }
            }
        }
        assert!(text.contains("qwen3-4b-fp8"), "상세: 모델 id\n{text}");
        assert!(text.contains("placement"), "상세: 배치 후보 섹션\n{text}");
        assert!(text.contains("furiosa.ai/rngd"), "상세: 리소스\n{text}");

        // 스토어 컴파일본 상세 — compiled_for 가 사람이 읽는 옵션으로 풀려야 한다.
        a.selected = a
            .library_items()
            .iter()
            .position(|it| matches!(it, LibItem::Stored(_)))
            .expect("stored build present");
        let mut t2 = Terminal::new(TestBackend::new(120, 20)).unwrap();
        t2.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let b2 = t2.backend().buffer();
        let mut st = String::new();
        for y in 0..b2.area.height {
            for x in 0..b2.area.width {
                if let Some(c) = b2.cell((x, y)) {
                    st.push_str(c.symbol());
                }
            }
        }
        assert!(st.contains("compile options"), "스토어 상세: 옵션 섹션\n{st}");
        assert!(st.contains("tensor-parallel"), "스토어 상세: TP 디코드\n{st}");
        assert!(st.contains("max-seq-len"), "스토어 상세: seq 디코드\n{st}");
    }

    #[test]
    fn decode_compiled_for_expands_options() {
        let d = decode_compiled_for("rbln-ca22-tp4-s8192");
        assert!(d.contains(&("vendor", "RBLN (Rebellions)".to_string())));
        assert!(d.contains(&("npu-chip", "CA22".to_string())));
        assert!(d.contains(&("tensor-parallel", "4".to_string())));
        assert!(d.contains(&("max-seq-len", "8192".to_string())));
        let f = decode_compiled_for("RNGD-tp4-pp1-s8192");
        assert!(f.contains(&("vendor", "RNGD (Furiosa)".to_string())));
        assert!(f.contains(&("tensor-parallel", "4".to_string())));
        assert!(f.contains(&("pipeline-parallel", "1".to_string())));
        assert!(f.contains(&("max-seq-len", "8192".to_string())));
    }

    fn accel(
        kind: crate::collect::AccelKind,
        node: &str,
        alive: bool,
        busy: &str,
    ) -> crate::collect::Accel {
        crate::collect::Accel {
            kind,
            model: String::new(),
            id: "d".into(),
            node: node.into(),
            util: 50.0,
            mem_used_gb: 0.0, // 기본은 유휴(모델 미로드) — busy 파라미터로만 점유 표현
            mem_total_gb: 48.0,
            temp: 40.0,
            power: 100.0,
            busy_model: busy.into(),
            alive,
            throttle: 0.0,
            unified_mem: false,
            mem_bw: f64::NAN,
            clock_mhz: f64::NAN,
            mem_temp: f64::NAN,
            energy_mj: f64::NAN,
        }
    }

    fn deploy_form(vendor: &'static str, replicas: &str, devices: &str, place: &str) -> DeployForm {
        let f = |k: &str, v: &str| CompileField {
            key: k.into(),
            label: k.into(),
            value: v.into(),
            choices: vec![],
            numeric: true,
            help: String::new(),
        };
        DeployForm {
            model: "m".into(),
            model_id: "m".into(),
            engine: "vLLM".into(),
            vendor,
            mount: "/mnt/store/x".into(),
            fields: vec![f("replicas", replicas), f("devices", devices)],
            place: place.into(), // placement 는 이제 폼 필드가 아니라 구조체 값(피커에서 선택)
            cursor: 0,
            editing: false,
        }
    }

    #[test]
    fn deploy_fit_capacity_verdicts() {
        use crate::collect::AccelKind::Gpu;
        let mk = |accel: Vec<crate::collect::Accel>, inv_free: i64| App {
            snap: Snapshot {
                accel,
                inventory: vec![("nvidia.com/gpu".to_string(), inv_free, 0)],
                ..Default::default()
            },
            ..App::new()
        };

        // Fits: 4 유휴, 수요 1×2=2, 각 노드 2개라 패킹 OK.
        let a = mk(
            vec![
                accel(Gpu, "n1", true, ""),
                accel(Gpu, "n1", true, ""),
                accel(Gpu, "n2", true, ""),
                accel(Gpu, "n2", true, ""),
            ],
            4,
        );
        let fit = a.deploy_fit(&deploy_form("gpu", "1", "2", "any"));
        assert_eq!(fit.demand, 2);
        assert_eq!(fit.verdict, FitVerdict::Fits);

        // Oom(리소스 예약): metric 은 2 유휴지만 인벤토리 예약으로 resource_free=0.
        let b = App {
            snap: Snapshot {
                accel: vec![accel(Gpu, "n1", true, ""), accel(Gpu, "n1", true, "")],
                inventory: vec![("nvidia.com/gpu".to_string(), 2, 2)], // free = 0
                ..Default::default()
            },
            ..App::new()
        };
        let bfit = b.deploy_fit(&deploy_form("gpu", "1", "1", "any"));
        assert_eq!(bfit.verdict, FitVerdict::Oom);
        assert!(bfit.tips.iter().any(|t| t.contains("예약")), "리소스 예약 부족 안내");
        assert!(
            bfit.tips.iter().any(|t| t.contains("metric 유휴")),
            "metric 유휴 ≠ 리소스 유휴 불일치 안내"
        );

        // Tight: 총량은 되지만 노드 패킹으로 일부만(n1=3, n2=1, per=2 → placeable=1 < 2).
        let c = mk(
            vec![
                accel(Gpu, "n1", true, ""),
                accel(Gpu, "n1", true, ""),
                accel(Gpu, "n1", true, ""),
                accel(Gpu, "n2", true, ""),
            ],
            4,
        );
        let cfit = c.deploy_fit(&deploy_form("gpu", "2", "2", "any"));
        assert_eq!(cfit.verdict, FitVerdict::Tight);
    }

    #[test]
    fn agg_summary_reports_scope_and_counts() {
        use crate::collect::AccelKind::Gpu;
        let mut a = App {
            snap: Snapshot {
                accel: vec![
                    accel(Gpu, "n1", true, "m1"), // busy
                    accel(Gpu, "n1", true, ""),   // idle
                ],
                ..Default::default()
            },
            ..App::new()
        };
        a.view = View::Accel;
        let s = a.agg_summary().expect("Accel view has a summary");
        assert!(s.starts_with("Σall"), "필터 없으면 scope=all: {s}");
        assert!(s.contains("2dev"), "총 디바이스 수: {s}");
        assert!(s.contains("1busy"), "busy 카운트: {s}");

        // 필터가 있으면 보이는 행만 집계하고 scope=filt.
        a.filter = "m1".into();
        let sf = a.agg_summary().expect("filtered summary");
        assert!(sf.starts_with("Σfilt"), "필터 있으면 scope=filt: {sf}");
        assert!(sf.contains("1dev"), "필터로 1개만 집계: {sf}");
    }

    #[test]
    fn apply_detects_alerts_and_toasts() {
        use crate::collect::AccelKind::Rbln;
        let mut a = App::new();
        // 죽은 가속기 → Bad 알림 + 토스트(엣지 검출).
        a.apply(Snapshot {
            ts: 1,
            accel: vec![accel(Rbln, "n1", false, "")],
            ..Default::default()
        });
        assert!(!a.alerts.is_empty(), "죽은 디바이스는 알림을 만든다");
        assert!(a.toast.is_some(), "신규 알림은 토스트를 띄운다");
        assert!(a.toast_bad, "Bad 심각도는 빨강 토스트");

        // pod 재시작 증가(델타) → 신규 알림.
        let before = a.alerts.len();
        let p = |restarts: i64| crate::collect::PodRow {
            name: "p1".into(),
            phase: "Running".into(),
            ready: "1/1".into(),
            node: "n1".into(),
            restarts,
            age_secs: 0,
        };
        a.apply(Snapshot {
            ts: 2,
            pods: vec![p(0)],
            ..Default::default()
        });
        a.apply(Snapshot {
            ts: 3,
            pods: vec![p(3)],
            ..Default::default()
        });
        assert!(
            a.alerts.iter().any(|al| al.msg.contains("restarted")),
            "재시작 델타는 알림을 만든다"
        );
        assert!(a.alerts.len() > before);
    }

    #[test]
    fn deploy_phase_reflects_replicas_and_pods() {
        let mut a = App::new();
        let p = |name: &str, restarts: i64| crate::collect::PodRow {
            name: name.into(),
            phase: "Running".into(),
            ready: "1/1".into(),
            node: "n1".into(),
            restarts,
            age_secs: 0,
        };
        a.snap = Snapshot {
            pods: vec![p("healthy-abc", 0), p("crash-xyz", 5)],
            ..Default::default()
        };
        assert_eq!(a.deploy_phase("healthy", 0, 0).label(), "Scaled-0");
        assert_eq!(a.deploy_phase("healthy", 2, 2).label(), "Serving");
        assert_eq!(a.deploy_phase("healthy", 2, 0).label(), "Starting");
        // 크래시 파드(restarts≥3): ready 0 → Failed, ready>0 → Degraded.
        assert_eq!(a.deploy_phase("crash", 2, 0).label(), "Failed");
        assert_eq!(a.deploy_phase("crash", 2, 1).label(), "Degraded");
    }

    #[test]
    fn activity_rows_unify_compile_and_deploys() {
        use crate::collect::CompileJob;
        let mut a = App::new();
        let mut steady = model("steady");
        steady.ready = 1;
        steady.desired = 1; // Serving → 노출(서빙 중)
        let mut starting = model("starting");
        starting.ready = 0;
        starting.desired = 2; // Starting → 노출(시도 중), 서빙 중보다 위
        let scaled = model("scaled0-x"); // desired 기본 1 → 0 으로 낮춤
        let mut scaled = scaled;
        scaled.desired = 0;
        scaled.ready = 0; // Scaled-0 → 제외
        a.snap = Snapshot {
            compiles: vec![CompileJob {
                name: "compile-x".into(),
                model: "x".into(),
                vendor: "RBLN".into(),
                target: "tp4".into(),
                status: "Running".into(),
                age_secs: 5,
                duration_secs: None,
                phase: "compiling".into(),
                progress: Some(0.5),
            }],
            models: vec![steady, starting, scaled],
            ..Default::default()
        };
        let rows = a.activity_rows();
        assert!(
            rows.iter().any(|r| r.kind == "compile" && r.target.contains('x')),
            "compile Job 이 피드에 있어야"
        );
        assert!(
            rows.iter().any(|r| r.kind == "deploy" && r.target.contains("starting")),
            "시도 중(Starting) 배포 노출"
        );
        assert!(
            rows.iter().any(|r| r.kind == "deploy" && r.target.contains("steady")),
            "서빙 중(Serving) 배포도 노출(어디서 도는지)"
        );
        assert!(
            !rows.iter().any(|r| r.target.contains("scaled0")),
            "Scaled-0 은 작업이 아니므로 제외"
        );
        // 문제 있는 배포가 서빙 중보다 위.
        let dep: Vec<&_> = rows.iter().filter(|r| r.kind == "deploy").collect();
        let ps = dep.iter().position(|r| r.target.contains("starting"));
        let pv = dep.iter().position(|r| r.target.contains("steady"));
        assert!(ps < pv, "Starting 이 Serving 보다 먼저");
        assert!(
            rows.iter().any(|r| r.running_compile && r.progress == Some(0.5)),
            "진행 중 compile 은 진행률을 싣는다"
        );
    }

    #[test]
    fn place_picker_lists_candidate_nodes() {
        use crate::collect::AccelKind::Rbln;
        let mut a = App::new();
        let mut node = crate::collect::NodeInfo {
            name: "n-drv".into(),
            ..Default::default()
        };
        node.ready = true;
        node.npu = "RBLN drv3.0".into();
        // 노드에 2개 디바이스, 그중 1개가 파드에 할당(requests)됨 → 유휴는 1.
        let mut alloc = std::collections::BTreeMap::new();
        alloc.insert(
            "n-drv".to_string(),
            std::iter::once(("rebellions.ai/ATOM".to_string(), 1)).collect(),
        );
        a.snap = Snapshot {
            accel: vec![
                accel(Rbln, "n-drv", true, ""),
                accel(Rbln, "n-drv", true, ""),
            ],
            nodes: vec![node],
            node_alloc: alloc,
            ..Default::default()
        };
        a.deploy_form = Some(deploy_form("rbln", "1", "1", "any"));
        a.open_place_picker();
        let p = a.place_picker.clone().expect("picker opens");
        assert_eq!(p.rows[0].value, "any");
        assert_eq!(p.rows[1].value, "spread");
        let n = p.rows.iter().find(|r| r.value == "n-drv").expect("node row");
        assert_eq!((n.total, n.free), (2, 1), "총 2 · 할당 1 → 유휴 1");
        assert!(n.schedulable, "ready + 드라이버 → 스케줄 가능");
        // 노드 선택 → 배치 확정 + 매니페스트 생성(제출)까지. 피커·폼 닫힘, 확인 팝업에 nodeSelector 반영.
        a.place_picker.as_mut().unwrap().cursor = 2;
        a.place_pick_apply();
        assert!(a.place_picker.is_none(), "선택 후 피커 닫힘");
        assert!(a.deploy_form.is_none(), "placement 확정 → 폼 제출(소비)");
        let (_, yaml) = submitted(&a);
        assert!(
            yaml.contains("kubernetes.io/hostname: n-drv"),
            "선택 노드가 nodeSelector 로 반영\n{yaml}"
        );
    }

    #[test]
    fn place_picker_renders_without_panic() {
        use crate::collect::AccelKind::Rbln;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut a = App::new();
        a.snap = Snapshot {
            accel: vec![accel(Rbln, "etri-001", true, "")],
            ..Default::default()
        };
        a.view = View::Library;
        a.deploy_form = Some(deploy_form("rbln", "1", "1", "any"));
        a.open_place_picker();
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(120, 24)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(c) = buf.cell((x, y)) {
                    s.push_str(c.symbol());
                }
            }
        }
        assert!(s.contains("NODE"), "picker 헤더\n{s}");
        assert!(s.contains("etri-001"), "후보 노드 행");
        assert!(s.contains("placement"), "picker 타이틀");
    }

    #[test]
    fn activity_auto_cleans_old_done_jobs() {
        use crate::collect::CompileJob;
        let mk = |name: &str, status: &str, age: u64, dur: Option<u64>| CompileJob {
            name: name.into(),
            model: "x".into(),
            vendor: "RBLN".into(),
            target: "tp4".into(),
            status: status.into(),
            age_secs: age,
            duration_secs: dur,
            phase: String::new(),
            progress: None,
        };
        let mut a = App::new();
        a.snap = Snapshot {
            compiles: vec![
                mk("running", "Running", 100, None),      // 진행 중 → 유지
                mk("fresh", "Complete", 200, Some(100)),  // 끝난 지 100s → 유지
                mk("old", "Complete", 5000, Some(100)),   // 끝난 지 4900s(>30m) → 자동 감춤
            ],
            ..Default::default()
        };
        let jobs: Vec<String> = a
            .activity_rows()
            .iter()
            .filter(|r| r.kind == "compile")
            .filter_map(|r| r.job.clone())
            .collect();
        assert!(jobs.iter().any(|j| j == "running"), "진행 중 유지");
        assert!(jobs.iter().any(|j| j == "fresh"), "방금 끝난 것 유지");
        assert!(
            !jobs.iter().any(|j| j == "old"),
            "오래 전 끝난 것은 자동 감춤"
        );
    }
}
