//! UI state machine — current view, selection, sparkline history. Separate from data (Snapshot).

use crate::collect::Snapshot;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};

// impl App is split across submodules (see each file's header for scope).
mod compile;

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
    Models,
    Epp,
    Routing,
    Pods,
    Perf,
    Library, // Deploy 섹션 ①: 배포 가능한 모델 라이브러리(카탈로그 · 컴파일 Job) — family›version›target
    Serving, // Deploy 섹션 ②: 현재 서빙 중인 배포(라이브 아티팩트) — family›version›running-target
    Events,
    Nodes,
    Topo, // Nodes hub's topology / device pressure map (Canvas)
}

impl View {
    /// Every view — for headless render coverage and exhaustive iteration (not a nav order).
    pub const EVERY: [View; 12] = [
        View::Overview,
        View::Routing,
        View::Epp,
        View::Models,
        View::Perf,
        View::Pods,
        View::Nodes,
        View::Accel,
        View::Topo,
        View::Serving,
        View::Library,
        View::Events,
    ];
    /// Which top-level section this view belongs to (a view is one sub-tab of its section).
    pub fn section(&self) -> Section {
        match self {
            View::Overview => Section::Overview,
            View::Routing | View::Epp => Section::Traffic,
            View::Models | View::Perf | View::Pods => Section::Models,
            View::Nodes | View::Accel | View::Topo => Section::Infra,
            View::Library | View::Serving => Section::Deploy,
            View::Events => Section::Events,
        }
    }
    /// Sub-tab label within a section (short; the section carries the group name).
    pub fn title(&self) -> &'static str {
        match self {
            View::Overview => "Overview",
            View::Accel => "Devices",
            View::Models => "Models",
            View::Epp => "EPP",
            View::Routing => "Flow",
            View::Pods => "Pods",
            View::Perf => "Perf",
            View::Serving => "Serving", // 현재 서빙 중(라이프사이클 렌즈)
            View::Library => "Library", // 배포 가능 모델 라이브러리(배포 렌즈)
            View::Events => "Events",
            View::Nodes => "Nodes",
            View::Topo => "Topology",
        }
    }
}

/// Top-level navigation section — the request path reads Gateway → EPP → Model → Infra.
/// Each section groups one or more views as sub-tabs (cycled with `←` `→` / `[` `]`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Overview, // cluster at-a-glance
    Traffic,  // Flow · EPP  (gateway → route → pool → picker)
    Models,   // Models · Perf · Pods  (the serving workloads)
    Infra,    // Nodes · Devices · Topology  (heterogeneous accelerators)
    Deploy,   // compile / deploy lifecycle
    Events,   // events + alerts
}
impl Section {
    /// Number-key order (0-5) and tab order.
    pub const ALL: [Section; 6] = [
        Section::Overview,
        Section::Traffic,
        Section::Models,
        Section::Infra,
        Section::Deploy,
        Section::Events,
    ];
    pub fn idx(&self) -> usize {
        Section::ALL.iter().position(|s| s == self).unwrap_or(0)
    }
    pub fn title(&self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Traffic => "Traffic",
            Section::Models => "Models",
            Section::Infra => "Infra",
            Section::Deploy => "Deploy",
            Section::Events => "Events",
        }
    }
    /// Sub-tabs (views) of this section, in `[`/`]` cycle order. First entry is the landing view.
    pub fn members(&self) -> &'static [View] {
        match self {
            Section::Overview => &[View::Overview],
            Section::Traffic => &[View::Routing, View::Epp],
            Section::Models => &[View::Models, View::Perf, View::Pods],
            Section::Infra => &[View::Nodes, View::Accel, View::Topo],
            // Deploy: Serving(현재 서빙 중, 랜딩) → Tab → Library(배포 가능 라이브러리).
            Section::Deploy => &[View::Serving, View::Library],
            Section::Events => &[View::Events],
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
    pub action_menu: Option<ActionMenu>, // Enter context action menu (Info/Compile/Deploy/Stop…)
    pub objectives: HashMap<String, Objective>, // per-model serving objective (SLO) — user input
    pub objective_form: Option<ObjectiveForm>, // objective edit form
    pub logs_mode: bool,                 // logs overlay
    pub logs_target: String,             // logs target pod
    pub logs: Vec<String>,               // log lines
    pub logs_scroll: u16,
    pub cols: HashMap<String, Vec<String>>, // per-view displayed columns (order) — config file
    pub catalog: Vec<crate::catalog::CatModel>, // model catalog (launcher)
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

    /// Cross-layer drill: jump from the selected entity to a related layer (view switch + correlation filter).
    /// Pushes the current position onto the breadcrumb so esc can retrace it. The collector is already wired.
    pub fn pivot(&mut self, key: char) {
        // Avoid mutable-borrow conflicts — extract the selected entity's values first.
        let model = self.selected_model().map(|m| m.name.clone());
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
            View::Accel => accel
                .filter(|(b, _)| !b.is_empty())
                .and_then(|(bm, nd)| match key {
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
            View::Models | View::Overview => self
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
                .snap
                .compiles
                .get(i)
                .map(|c| format!("{} {} {}", c.name, c.status, c.phase))
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
        }
    }

    /// 상세 패널을 가진 뷰인지(detail=true 가 실제로 렌더에 반영되는 뷰).
    /// 없는 뷰(Routing/Epp/Launch/Events)에서 detail=true 로 두면 ↑↓ 가 스크롤로 빠져 네비가 잠김.
    pub fn view_has_detail(&self) -> bool {
        matches!(
            self.view,
            View::Accel
                | View::Models
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
        self.pivot('m'); // → Models, filter=backend (매칭 0건이면 pivot 이 되짚음)
        if self.view == View::Models && self.list_len() > 0 {
            self.detail = true;
        }
    }

    /// 현재 뷰의 정렬 모드 수(순환용).
    /// 정렬 가능한 컬럼 — 뷰가 실제로 보여주는 컬럼 기준. `o` 로 순환, `O` 로 방향 토글.
    /// desc=true 면 그 컬럼 선택 시 기본이 내림차순(수치 컬럼은 큰 값 먼저가 유용).
    pub fn sort_cols(&self) -> &'static [SortCol] {
        use View::*;
        match self.view {
            Accel => &[
                SortCol {
                    label: "util",
                    desc: true,
                },
                SortCol {
                    label: "temp",
                    desc: true,
                },
                SortCol {
                    label: "mem",
                    desc: true,
                },
                SortCol {
                    label: "power",
                    desc: true,
                },
                SortCol {
                    label: "name",
                    desc: false,
                },
            ],
            Models | Overview => &[
                SortCol {
                    label: "name",
                    desc: false,
                },
                SortCol {
                    label: "status",
                    desc: false,
                },
                SortCol {
                    label: "ready",
                    desc: true,
                },
                SortCol {
                    label: "tok/s",
                    desc: true,
                },
                SortCol {
                    label: "kv%",
                    desc: true,
                },
                SortCol {
                    label: "waiting",
                    desc: true,
                },
                SortCol {
                    label: "node",
                    desc: false,
                },
            ],
            Pods => &[
                SortCol {
                    label: "name",
                    desc: false,
                },
                SortCol {
                    label: "phase",
                    desc: false,
                },
                SortCol {
                    label: "restarts",
                    desc: true,
                },
                SortCol {
                    label: "node",
                    desc: false,
                },
                SortCol {
                    label: "ready",
                    desc: false,
                },
            ],
            Nodes => &[
                SortCol {
                    label: "name",
                    desc: false,
                },
                SortCol {
                    label: "cpu",
                    desc: true,
                },
                SortCol {
                    label: "mem",
                    desc: true,
                },
                SortCol {
                    label: "disk",
                    desc: true,
                },
                SortCol {
                    label: "load",
                    desc: true,
                },
            ],
            Events => &[
                SortCol {
                    label: "recent",
                    desc: false,
                },
                SortCol {
                    label: "type",
                    desc: false,
                },
                SortCol {
                    label: "reason",
                    desc: false,
                },
                SortCol {
                    label: "count",
                    desc: true,
                },
            ],
            // Perf 는 기존 다지표 정렬(perf_rows_order) 유지 — 전부 desc 기본이라 진입 시 자연순서, O 로 역순.
            Perf => &[
                SortCol {
                    label: "tok/s",
                    desc: true,
                },
                SortCol {
                    label: "E2E",
                    desc: true,
                },
                SortCol {
                    label: "TTFT",
                    desc: true,
                },
                SortCol {
                    label: "queue",
                    desc: true,
                },
                SortCol {
                    label: "name",
                    desc: true,
                },
            ],
            _ => &[],
        }
    }
    pub fn sort_modes(&self) -> usize {
        self.sort_cols().len().max(1)
    }
    /// 뷰 진입/전환 시 정렬 초기화 — 첫 컬럼 + 그 컬럼의 기본 방향.
    pub fn reset_sort(&mut self) {
        self.sort = 0;
        self.sort_desc = self.sort_cols().first().map(|c| c.desc).unwrap_or(true);
    }
    pub fn cycle_sort(&mut self) {
        let cols = self.sort_cols();
        if cols.len() <= 1 {
            return;
        }
        self.sort = (self.sort + 1) % cols.len();
        self.sort_desc = self.sort_cols()[self.sort].desc; // 새 컬럼의 기본 방향
    }
    /// 정렬 방향 토글(`O`) — 정렬 가능한 뷰에서만.
    pub fn toggle_sort_dir(&mut self) {
        if !self.sort_cols().is_empty() {
            self.sort_desc = !self.sort_desc;
        }
    }
    pub fn sort_label(&self) -> &'static str {
        self.sort_cols()
            .get(self.sort)
            .map(|c| c.label)
            .unwrap_or("—")
    }
    /// 현재 정렬 컬럼에 대응하는 **헤더 텍스트**(테이블 헤더에 화살표를 붙일 대상 매칭용).
    /// 헤더 라벨과 sort 컬럼 라벨이 달라(예: util→"UTIL", name→"MODEL") 뷰별로 명시 매핑.
    /// 대응 헤더가 없으면(예: Events recent, Nodes) 빈 문자열 → 마킹 안 함.
    pub fn sort_header_label(&self) -> &'static str {
        use View::*;
        match (self.view, self.sort) {
            (Accel, 0) => "UTIL",
            (Accel, 1) => "TEMP",
            (Accel, 2) => "MEM",
            (Accel, 3) => "PWR",
            (Accel, _) => "KIND",
            (Models, 0) => "MODEL",
            (Models, 1) => "STATUS",
            (Models, 2) => "READY",
            (Models, 3) => "t/s",
            (Models, 4) => "KV",
            (Models, 5) => "WAIT",
            (Models, _) => "ACCEL",
            (Pods, 0) => "POD",
            (Pods, 1) => "PHASE",
            (Pods, 2) => "RESTARTS",
            (Pods, 3) => "NODE",
            (Pods, _) => "READY",
            (Events, 1) => "TYPE",
            (Events, 2) => "REASON",
            (Events, 3) => "CNT",
            (Perf, 0) => "tok/s",
            (Perf, 1) => "E2E",
            (Perf, 2) => "TTFT",
            (Perf, 3) => "QUEUE",
            (Perf, 4) => "MODEL",
            _ => "", // Events recent, Nodes(헤더 없음), 그 외 → 마킹 안 함
        }
    }

    /// 정렬 방향 표시 글리프(내림 ▼ / 오름 ▲). 정렬 불가 뷰는 공백.
    pub fn sort_arrow(&self) -> &'static str {
        if self.sort_cols().is_empty() {
            ""
        } else if self.sort_desc {
            "▼"
        } else {
            "▲"
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
                self.push_hist(
                    &format!("{}:util", k),
                    a.util.round().clamp(0.0, 100.0) as u64,
                );
                let memp = if a.mem_total_gb > 0.0 {
                    a.mem_used_gb / a.mem_total_gb * 100.0
                } else {
                    0.0
                };
                self.push_hist(&format!("{}:mem", k), memp.round().clamp(0.0, 100.0) as u64);
                self.push_hist(&format!("{}:temp", k), a.temp.round().max(0.0) as u64);
            }
            // per-node: cpu% / mem% / load
            for n in &snap.nodes {
                let k = format!("nod:{}", n.name);
                if !n.cpu_pct.is_nan() {
                    self.push_hist(
                        &format!("{}:cpu", k),
                        n.cpu_pct.round().clamp(0.0, 100.0) as u64,
                    );
                }
                let memp = if n.mem_total_gb > 0.0 {
                    n.mem_used_gb / n.mem_total_gb * 100.0
                } else {
                    0.0
                };
                self.push_hist(&format!("{}:mem", k), memp.round().clamp(0.0, 100.0) as u64);
                if n.disk_total_gb > 0.0 {
                    let dp = n.disk_used_gb / n.disk_total_gb * 100.0;
                    self.push_hist(&format!("{}:disk", k), dp.round().clamp(0.0, 100.0) as u64);
                }
                if !n.load1.is_nan() {
                    self.push_hist(
                        &format!("{}:load", k),
                        (n.load1 * 10.0).round().max(0.0) as u64,
                    );
                }
            }
            // 클러스터 추이 — 실제 존재하는 가속기 종류만 집계(GPU/RBLN/RNGD 각각)
            let mean = |v: &[f64]| {
                if v.is_empty() {
                    f64::NAN
                } else {
                    v.iter().sum::<f64>() / v.len() as f64
                }
            };
            let pct = |u: f64, t: f64| if t > 0.0 { u / t * 100.0 } else { 0.0 };
            let mut byk: std::collections::BTreeMap<&str, (Vec<f64>, f64, f64)> =
                std::collections::BTreeMap::new();
            for a in &snap.accel {
                let e = byk.entry(a.kind.label()).or_default();
                e.0.push(a.util);
                e.1 += a.mem_used_gb;
                e.2 += a.mem_total_gb;
            }
            for (k, (u, mu, mt)) in &byk {
                self.push_hist(
                    &format!("sys:{}_util", k),
                    mean(u).round().clamp(0.0, 100.0) as u64,
                );
                self.push_hist(
                    &format!("sys:{}_mem", k),
                    pct(*mu, *mt).round().clamp(0.0, 100.0) as u64,
                );
            }
            let cpus: Vec<f64> = snap
                .nodes
                .iter()
                .filter(|n| !n.cpu_pct.is_nan())
                .map(|n| n.cpu_pct)
                .collect();
            if !cpus.is_empty() {
                self.push_hist("sys:cpu", mean(&cpus).round().clamp(0.0, 100.0) as u64);
            }
            let (hmu, hmt): (f64, f64) = snap.nodes.iter().fold((0.0, 0.0), |(u, t), n| {
                (u + n.mem_used_gb, t + n.mem_total_gb)
            });
            self.push_hist(
                "sys:host_mem",
                pct(hmu, hmt).round().clamp(0.0, 100.0) as u64,
            );
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
                        s.push_hist(
                            &format!("{}:{}", k, sub),
                            (v * 1000.0).round().max(0.0) as u64,
                        );
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
                    self.energy_base
                        .entry(Self::accel_key(a))
                        .or_insert(a.energy_mj);
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
            let prev = self
                .prev_restarts
                .get(&p.name)
                .copied()
                .unwrap_or(p.restarts);
            if p.restarts > prev {
                current.push(Alert {
                    ts: now,
                    sev: Sev::Warn,
                    key: format!("restart:{}:{}", p.name, p.restarts),
                    msg: format!("pod {} restarted (x{})", p.name, p.restarts),
                });
            }
        }
        self.prev_restarts = snap
            .pods
            .iter()
            .map(|p| (p.name.clone(), p.restarts))
            .collect();

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
            View::Library => 2, // Deploy▸Library: 통합 배포 트리(카탈로그+스토어) / 진행 중 컴파일
            View::Serving => 1, // Deploy▸Serving: 라이브 배포 트리(단일 패널)
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
    /// The sub-tab reached by stepping `delta` from the current view (for prev/next preview). None if single-member.
    #[allow(dead_code)]
    pub fn subtab_peek(&self, delta: i64) -> Option<View> {
        let members = self.view.section().members();
        let n = members.len() as i64;
        if n <= 1 {
            return None;
        }
        let cur = members.iter().position(|v| *v == self.view).unwrap_or(0);
        Some(members[((cur as i64 + delta).rem_euclid(n)) as usize])
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

    fn entity_name(&self, i: usize) -> String {
        match self.view {
            View::Accel => self
                .snap
                .accel
                .get(i)
                .map(|a| format!("{} {}", a.kind.label(), a.id))
                .unwrap_or_default(),
            View::Models | View::Overview => self
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

    /// 현재 뷰의 표시 순서(정렬 적용된 원본 인덱스 목록). 렌더와 액션이 공유.
    pub fn order(&self) -> Vec<usize> {
        use crate::collect::{Accel, ModelRow, PodRow};
        use std::cmp::Ordering::Equal;
        let mut idx = match self.view {
            // ── 컬럼 기반 정렬(o=컬럼 순환 · O=방향 토글). 비교는 오름차순으로 쓰고 desc 면 reverse. ──
            View::Accel => {
                let v = &self.snap.accel;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&Accel, &Accel) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        0 => x.util.partial_cmp(&y.util).unwrap_or(Equal),
                        1 => x.temp.partial_cmp(&y.temp).unwrap_or(Equal),
                        2 => x.mem_used_gb.partial_cmp(&y.mem_used_gb).unwrap_or(Equal),
                        3 => x.power.partial_cmp(&y.power).unwrap_or(Equal),
                        _ => (x.kind as u8, x.node.as_str(), x.id.as_str()).cmp(&(
                            y.kind as u8,
                            y.node.as_str(),
                            y.id.as_str(),
                        )),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
                    asc.then_with(|| {
                        (x.node.as_str(), x.id.as_str()).cmp(&(y.node.as_str(), y.id.as_str()))
                    })
                });
                idx
            }
            View::Models | View::Overview => {
                let v = &self.snap.models;
                let desc = self.sort_desc;
                let oc = |a: Option<f64>, b: Option<f64>| {
                    a.unwrap_or(f64::NEG_INFINITY)
                        .partial_cmp(&b.unwrap_or(f64::NEG_INFINITY))
                        .unwrap_or(Equal)
                };
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&ModelRow, &ModelRow) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.status.cmp(&y.status),
                        2 => x.ready.cmp(&y.ready),
                        3 => oc(x.tps, y.tps),
                        4 => oc(x.kv, y.kv),
                        5 => oc(x.waiting, y.waiting),
                        6 => x.accel.cmp(&y.accel),
                        _ => x.name.cmp(&y.name),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
                    asc.then_with(|| x.name.cmp(&y.name)) // 동점은 이름 오름차순(안정)
                });
                idx
            }
            View::Pods => {
                let v = &self.snap.pods;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&PodRow, &PodRow) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.phase.cmp(&y.phase),
                        2 => x.restarts.cmp(&y.restarts),
                        3 => x.node.cmp(&y.node),
                        4 => x.ready.cmp(&y.ready),
                        _ => x.name.cmp(&y.name),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
                    asc.then_with(|| x.name.cmp(&y.name))
                });
                idx
            }
            View::Nodes => {
                let v = &self.snap.nodes;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.cpu_pct.partial_cmp(&y.cpu_pct).unwrap_or(Equal),
                        2 => x.mem_used_gb.partial_cmp(&y.mem_used_gb).unwrap_or(Equal),
                        3 => x.disk_used_gb.partial_cmp(&y.disk_used_gb).unwrap_or(Equal),
                        4 => x.load1.partial_cmp(&y.load1).unwrap_or(Equal),
                        _ => x.name.cmp(&y.name),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
                    asc.then_with(|| x.name.cmp(&y.name))
                });
                idx
            }
            View::Events => {
                let v = &self.snap.events;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.typ.cmp(&y.typ),
                        2 => x.reason.cmp(&y.reason),
                        3 => x.count.cmp(&y.count),
                        _ => a.cmp(&b), // recent: 수집 순서(최신 먼저) = 인덱스 오름차순
                    };
                    if desc {
                        asc.reverse()
                    } else {
                        asc
                    }
                });
                idx
            }
            // Serving: 배포된 아티팩트를 family›version 그룹 순서로(트리 내비게이션이 그룹을 따라가게).
            View::Serving => self.serving_order(),
            // Library: 0 통합 배포 트리(카탈로그+스토어) · 1 진행 중 컴파일.
            View::Library if self.panel_focus == 1 => (0..self.snap.compiles.len()).collect(),
            View::Library => (0..self.library_items().len()).collect(),
            View::Epp if self.panel_focus == 1 => (0..self.snap.pools.len()).collect(),
            View::Epp => {
                (0..self.snap.epp.as_ref().map(|e| e.scorers.len()).unwrap_or(0)).collect()
            }
            View::Perf if self.panel_focus == 1 => (0..self.snap.pod_queues.len()).collect(),
            // Perf 는 다지표 전용 정렬(perf_rows_order, 기본 best-first=내림). O 로 역순.
            View::Perf => {
                let mut o = self.perf_rows_order();
                if !self.sort_desc {
                    o.reverse();
                }
                o
            }
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
        let scope = if self.filter.is_empty() {
            "all"
        } else {
            "filt"
        };
        let n = order.len();
        match self.view {
            View::Accel => {
                let d: Vec<&Accel> = order
                    .iter()
                    .filter_map(|&i| self.snap.accel.get(i))
                    .collect();
                if d.is_empty() {
                    return None;
                }
                let util = d.iter().map(|x| x.util).sum::<f64>() / d.len() as f64;
                let mu: f64 = d.iter().map(|x| x.mem_used_gb).sum();
                let mt: f64 = d.iter().map(|x| x.mem_total_gb).sum();
                let pw: f64 = d.iter().map(|x| x.power).sum();
                let busy = d.iter().filter(|x| !x.busy_model.is_empty()).count();
                Some(format!(
                    "Σ{} {}dev · {}busy · util {:.0}% · VRAM {:.0}/{:.0}G · {:.0}W",
                    scope, n, busy, util, mu, mt, pw
                ))
            }
            View::Models | View::Overview => {
                let m: Vec<&ModelRow> = order
                    .iter()
                    .filter_map(|&i| self.snap.models.get(i))
                    .collect();
                if m.is_empty() {
                    return None;
                }
                let ready: i64 = m.iter().map(|x| x.ready).sum();
                let desired: i64 = m.iter().map(|x| x.desired).sum();
                let run: f64 = m.iter().filter_map(|x| x.running).sum();
                let wait: f64 = m.iter().filter_map(|x| x.waiting).sum();
                let tps: f64 = m.iter().filter_map(|x| x.tps).sum();
                Some(format!(
                    "Σ{} {}mdl · {}/{}ready · run {:.0} wait {:.0} · {:.0}tok/s",
                    scope, n, ready, desired, run, wait, tps
                ))
            }
            View::Nodes => {
                let nn: Vec<&NodeInfo> = order
                    .iter()
                    .filter_map(|&i| self.snap.nodes.get(i))
                    .collect();
                if nn.is_empty() {
                    return None;
                }
                let cpu = nn.iter().map(|x| x.cpu_pct).sum::<f64>() / nn.len() as f64;
                let mu: f64 = nn.iter().map(|x| x.mem_used_gb).sum();
                let mt: f64 = nn.iter().map(|x| x.mem_total_gb).sum();
                let du: f64 = nn.iter().map(|x| x.disk_used_gb).sum();
                let dt: f64 = nn.iter().map(|x| x.disk_total_gb).sum();
                let ready = nn.iter().filter(|x| x.ready).count();
                Some(format!(
                    "Σ{} {}node · {}ready · CPU {:.0}% · mem {:.0}/{:.0}G · disk {:.1}/{:.1}T",
                    scope,
                    n,
                    ready,
                    cpu,
                    mu,
                    mt,
                    du / 1024.0,
                    dt / 1024.0
                ))
            }
            View::Perf => {
                let p: Vec<&PerfRow> = order
                    .iter()
                    .filter_map(|&i| self.snap.perf_rows.get(i))
                    .collect();
                if p.is_empty() {
                    return None;
                }
                let tps: f64 = p
                    .iter()
                    .filter_map(|x| if x.tps.is_nan() { None } else { Some(x.tps) })
                    .sum();
                let e2e: Vec<f64> = p
                    .iter()
                    .map(|x| x.e2e_p95)
                    .filter(|v| !v.is_nan())
                    .collect();
                let e2e_avg = if e2e.is_empty() {
                    f64::NAN
                } else {
                    e2e.iter().sum::<f64>() / e2e.len() as f64
                };
                let e2e_s = if e2e_avg.is_nan() {
                    "–".to_string()
                } else {
                    format!("{:.0}ms", e2e_avg * 1000.0)
                };
                Some(format!(
                    "Σ{} {}active · E2E p95 {} · {:.0}tok/s",
                    scope, n, e2e_s, tps
                ))
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
            View::Models | View::Overview => self
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

    /// family›version 계층 그룹핑 — 키 목록을 (family, version) 로 받아, family/version
    /// 첫 등장 순서를 보존하며 같은 그룹을 인접시킨 원본 인덱스 순서를 돌려준다(트리 표시·내비 공용).
    fn grouped_indices(keys: &[(String, String)]) -> Vec<usize> {
        let mut fam_order: Vec<&str> = Vec::new();
        for (f, _) in keys {
            if !fam_order.contains(&f.as_str()) {
                fam_order.push(f);
            }
        }
        let mut out = Vec::with_capacity(keys.len());
        for f in &fam_order {
            let mut ver_order: Vec<&str> = Vec::new();
            for (ff, v) in keys {
                if ff == f && !ver_order.contains(&v.as_str()) {
                    ver_order.push(v);
                }
            }
            for v in &ver_order {
                for (i, (ff, vv)) in keys.iter().enumerate() {
                    if ff == f && vv == v {
                        out.push(i);
                    }
                }
            }
        }
        out
    }

    /// 아티팩트의 version(중간 티어) 라벨 — HF repo/소스(양자화·revision 을 가르는 자리).
    pub fn artifact_version(a: &crate::collect::ModelArtifact) -> String {
        if a.source.is_empty() {
            a.model.clone()
        } else {
            a.source.clone()
        }
    }
    /// 카탈로그 모델의 family 키 — NPU 지원목록 계열명, 없으면 id.
    pub fn catalog_family(m: &crate::catalog::CatModel) -> String {
        crate::compat::family_of(&m.id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| m.id.clone())
    }

    /// Serving 트리(배포 아티팩트)의 그룹 순서 원본 인덱스.
    pub fn serving_order(&self) -> Vec<usize> {
        let keys: Vec<(String, String)> = self
            .snap
            .artifacts
            .iter()
            .map(|a| (a.family.clone(), Self::artifact_version(a)))
            .collect();
        Self::grouped_indices(&keys)
    }
    /// family 그룹 키(소문자) — 카탈로그·스토어를 한 트리로 묶을 때 공용.
    fn lib_family(&self, it: LibItem) -> String {
        match it {
            LibItem::Catalog(i) => Self::catalog_family(&self.catalog[i]).to_lowercase(),
            LibItem::Stored(i) => self.snap.stored[i].family.to_lowercase(),
        }
    }
    /// family 첫 등장 순서를 보존하며 같은 family 를 인접시킨다(존 내부 그룹핑).
    fn group_by_family(&self, items: &[LibItem]) -> Vec<LibItem> {
        let mut fam_order: Vec<String> = Vec::new();
        for &it in items {
            let f = self.lib_family(it);
            if !fam_order.contains(&f) {
                fam_order.push(f);
            }
        }
        let mut out = Vec::with_capacity(items.len());
        for f in &fam_order {
            for &it in items {
                if &self.lib_family(it) == f {
                    out.push(it);
                }
            }
        }
        out
    }
    /// Library 패널0 통합 배포 트리 — 카탈로그(조직 제공)를 **상단**에, 그 아래 스토어 컴파일본.
    /// 각 존은 family 로 묶는다. order()·렌더 공용. 카탈로그가 "무엇을 배포할 수 있나"의 1차 관문.
    pub fn library_items(&self) -> Vec<LibItem> {
        let cat: Vec<LibItem> = (0..self.catalog.len()).map(LibItem::Catalog).collect();
        let sto: Vec<LibItem> = (0..self.snap.stored.len()).map(LibItem::Stored).collect();
        let mut out = self.group_by_family(&cat);
        out.extend(self.group_by_family(&sto));
        out
    }
    /// Library 패널0에서 선택된 항목(카탈로그 모델 또는 스토어 빌드).
    pub fn selected_lib_item(&self) -> Option<LibItem> {
        if self.view == View::Library && self.panel_focus == 0 {
            self.sel_orig()
                .and_then(|i| self.library_items().get(i).copied())
        } else {
            None
        }
    }
    /// Library 패널0에서 선택된 스토어 빌드(있으면).
    pub fn selected_stored(&self) -> Option<&crate::collect::StoredModel> {
        match self.selected_lib_item() {
            Some(LibItem::Stored(i)) => self.snap.stored.get(i),
            _ => None,
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
        a.opts
            .iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| def.to_string())
    }

    pub fn selected_catalog_model(&self) -> Option<&crate::catalog::CatModel> {
        match self.selected_lib_item() {
            Some(LibItem::Catalog(i)) => self.catalog.get(i),
            _ => None,
        }
    }

    fn preferred_catalog_placement<'a>(
        &self,
        m: &'a crate::catalog::CatModel,
    ) -> Option<&'a crate::catalog::CatPlacement> {
        m.placements.iter().max_by_key(|p| {
            let ready = match crate::catalog::solve(p, &self.snap.inventory).0 {
                crate::catalog::Ready::Ready => 3,
                crate::catalog::Ready::NeedsArtifact => 2,
                crate::catalog::Ready::NoCapacity => 1,
            };
            (ready, (!p.requires_artifact) as i32)
        })
    }

    fn placement_vendor(p: &crate::catalog::CatPlacement) -> &'static str {
        let sig = format!("{} {} {} {}", p.engine, p.accel, p.resource, p.uri).to_lowercase();
        if sig.contains("rbln") || sig.contains("rebellions") || sig.contains("atom") {
            "rbln"
        } else if sig.contains("furiosa") || sig.contains("rngd") {
            "furiosa"
        } else {
            "gpu"
        }
    }

    fn placement_engine(p: &crate::catalog::CatPlacement) -> &'static str {
        match Self::placement_vendor(p) {
            "rbln" => "vLLM-RBLN",
            "furiosa" => "Furiosa-LLM",
            _ => "vLLM",
        }
    }

    fn placement_model_id(
        m: &crate::catalog::CatModel,
        p: &crate::catalog::CatPlacement,
    ) -> String {
        let uri = p.uri.trim();
        if let Some(hf) = uri.strip_prefix("hf://") {
            hf.trim_start_matches('/').to_string()
        } else if uri.contains('/') && !uri.starts_with("pvc://") {
            uri.to_string()
        } else {
            m.id.clone()
        }
    }

    fn placement_mount(m: &crate::catalog::CatModel, p: &crate::catalog::CatPlacement) -> String {
        let uri = p.uri.trim();
        if let Some(path) = uri.strip_prefix("pvc://") {
            format!("/mnt/store/{}", path.trim_start_matches('/'))
        } else if let Some(hf) = uri.strip_prefix("hf://") {
            hf.trim_start_matches('/').to_string()
        } else if uri.is_empty() {
            let repo_dir = Self::placement_model_id(m, p).replace('/', "--");
            format!("/mnt/store/compiled/{}", repo_dir)
        } else {
            uri.to_string()
        }
    }

    fn catalog_artifact(
        m: &crate::catalog::CatModel,
        p: &crate::catalog::CatPlacement,
    ) -> crate::collect::ModelArtifact {
        let model_id = Self::placement_model_id(m, p);
        crate::collect::ModelArtifact {
            model: if m.display.is_empty() {
                m.id.clone()
            } else {
                m.display.clone()
            },
            family: m.id.clone(),
            engine: Self::placement_engine(p).to_string(),
            node: String::new(),
            image: String::new(),
            source: model_id,
            mount: Self::placement_mount(m, p),
            opts: vec![("tp".into(), p.count.max(1).to_string())],
        }
    }

    fn selected_catalog_artifact(&self) -> Option<crate::collect::ModelArtifact> {
        let m = self.selected_catalog_model()?;
        let p = self.preferred_catalog_placement(m)?;
        Some(Self::catalog_artifact(m, p))
    }

    // (compile flow moved to src/app/compile.rs)

    /// "Go: …" pivot entries for the current view's action menu — makes cross-layer jumps discoverable.
    /// Menu accelerator keys are chosen to not collide with the view's own action keys; the last field
    /// is the pivot key passed to `pivot()` (which decides the destination from the current selection).
    fn pivot_items(&self) -> Vec<ActionItem> {
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

    /// 모델 목표(SLO) 조회 — 정확 매칭 우선, 없으면 느슨한 부분일치(서빙명 ≠ deploy명 대비).
    pub fn objective_for(&self, model: &str) -> Option<&Objective> {
        if let Some(o) = self.objectives.get(model) {
            return Some(o);
        }
        self.objectives
            .iter()
            .find(|(k, _)| model.contains(k.as_str()) || k.contains(model))
            .map(|(_, o)| o)
    }

    /// Models 액션 메뉴 → Objective: 목표 편집 폼(기존 값 프리필).
    pub fn open_objective_form(&mut self) {
        let Some(m) = self.selected_model() else {
            return;
        };
        let name = m.name.clone();
        let cur = self.objectives.get(&name).cloned().unwrap_or_default();
        let numf =
            |key: &str, label: &str, cur: Option<f64>, choices: &[&str], help: &str| CompileField {
                key: key.into(),
                label: label.into(),
                value: cur
                    .map(|v| format!("{}", v as i64))
                    .unwrap_or_else(|| "none".into()),
                choices: std::iter::once("none")
                    .chain(choices.iter().copied())
                    .map(|s| s.to_string())
                    .collect(),
                numeric: true,
                help: help.into(),
            };
        let fields = vec![
            numf(
                "ttft",
                "TTFT p95 ≤ms",
                cur.ttft_ms,
                &["500", "1000", "2000", "4000"],
                "첫 토큰까지 목표 상한(ms) — 대화형 응답성",
            ),
            numf(
                "tpot",
                "TPOT p95 ≤ms",
                cur.tpot_ms,
                &["20", "50", "100", "200"],
                "토큰당 생성 시간 상한(ms) — 스트리밍 속도",
            ),
            numf(
                "e2e",
                "E2E p95 ≤ms",
                cur.e2e_ms,
                &["1000", "2000", "5000", "10000"],
                "요청 완료까지 상한(ms)",
            ),
            numf(
                "tps",
                "min tok/s ≥",
                cur.min_tps,
                &["10", "50", "100", "500"],
                "최소 처리량(tok/s) — 낮으면 처리량 부족",
            ),
        ];
        self.objective_form = Some(ObjectiveForm {
            model: name,
            fields,
            cursor: 0,
            editing: false,
        });
    }

    /// 목표 폼 제출 → objectives 에 반영(모두 none 이면 삭제).
    pub fn objective_form_submit(&mut self) {
        let Some(form) = self.objective_form.take() else {
            return;
        };
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
            return PerfAdvice {
                has_obj: false,
                checks: Vec::new(),
                tips: Vec::new(),
            };
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
                tips.push(
                    "KV/메모리 스래싱(preemption↑) — batch↓ 또는 max-seq-len↓, KV 여유 확보".into(),
                );
            }
            if !q.is_nan() && q > 0.0 && q >= pf.max(dc) {
                tips.push(format!(
                    "스케줄 대기 지배적({:.0}ms) — replicas↑ 또는 배치 버킷↑",
                    ms(q)
                ));
            } else if !pf.is_nan() && pf >= dc && pf > 0.0 {
                tips.push(format!(
                    "prefill 지배적({:.0}ms) — max-seq-len↓ 또는 chunked prefill",
                    ms(pf)
                ));
            } else if !dc.is_nan() && dc > 0.0 {
                tips.push(format!(
                    "decode 지배적({:.0}ms) — TPOT 개선: 동시성 조정, 필요시 TP↑",
                    ms(dc)
                ));
            }
            if let Some(mt) = o.min_tps {
                if !row.tps.is_nan() && row.tps < mt {
                    // 이 모델을 점유한 디바이스 평균 util 로 방향 제시.
                    let ut: Vec<f64> = self
                        .snap
                        .accel
                        .iter()
                        .filter(|a| {
                            !a.busy_model.is_empty() && row.model.contains(&a.busy_model)
                                || a.busy_model == row.model
                        })
                        .map(|a| a.util)
                        .collect();
                    let avg = if ut.is_empty() {
                        f64::NAN
                    } else {
                        ut.iter().sum::<f64>() / ut.len() as f64
                    };
                    if !avg.is_nan() && avg > 70.0 {
                        tips.push(
                            "tok/s 미달 · util 높음 — compute-bound: TP↑ 또는 replica↑".into(),
                        );
                    } else {
                        tips.push("tok/s 미달 · util 여유 — 동시성/배치↑ 로 처리량 확보".into());
                    }
                }
            }
        }
        PerfAdvice {
            has_obj: true,
            checks,
            tips,
        }
    }

    fn selected_deploy_spec(
        &self,
    ) -> Option<(
        String,
        String,
        String,
        &'static str,
        String,
        String,
        Option<String>,
    )> {
        if let Some(a) = self.selected_artifact() {
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
                a.mount
                    .split(" ← ")
                    .next()
                    .unwrap_or("/mnt/store")
                    .to_string()
            };
            let tp = Self::opt_or(a, "tp", if vendor == "furiosa" { "8" } else { "1" });
            let dev_default = if vendor == "furiosa" {
                let pe = tp.parse::<i64>().unwrap_or(8).max(1);
                ((pe as f64 / 8.0).ceil() as i64).max(1).to_string()
            } else {
                tp.clone()
            };
            return Some((
                a.model.clone(),
                model_id,
                a.engine.clone(),
                vendor,
                mount,
                dev_default,
                if vendor == "furiosa" { Some(tp) } else { None },
            ));
        }
        // Library 패널0: 스토어 컴파일본을 바로 배포 — repo/포맷/타깃(compiled_for)에서 spec 유도.
        if let Some(s) = self.selected_stored() {
            let vendor = match s.format.as_str() {
                "rbln" => "rbln",
                "furiosa" => "furiosa",
                _ => "gpu",
            };
            let engine = match vendor {
                "rbln" => "vLLM-RBLN",
                "furiosa" => "Furiosa-LLM",
                _ => "vLLM",
            };
            // compiled_for(예: RBLN-CA22-tp4-s8192) 에서 tp 추출.
            let tp = s
                .compiled_for
                .split(['-', '_', ' '])
                .find_map(|t| {
                    let t = t.to_lowercase();
                    t.strip_prefix("tp").and_then(|n| n.parse::<i64>().ok())
                })
                .unwrap_or(if vendor == "furiosa" { 8 } else { 1 });
            let dev_default = if vendor == "furiosa" {
                ((tp as f64 / 8.0).ceil() as i64).max(1).to_string()
            } else {
                tp.to_string()
            };
            let mount = if s.path.is_empty() {
                "/mnt/store".to_string()
            } else if s.path.starts_with('/') {
                s.path.clone()
            } else {
                format!("/mnt/store/{}", s.path)
            };
            let model = s.repo.rsplit('/').next().unwrap_or(&s.repo).to_string();
            return Some((
                model,
                s.repo.clone(),
                engine.to_string(),
                vendor,
                mount,
                dev_default,
                if vendor == "furiosa" {
                    Some(tp.to_string())
                } else {
                    None
                },
            ));
        }
        let m = self.selected_catalog_model()?;
        let p = self.preferred_catalog_placement(m)?;
        let model_id = Self::placement_model_id(m, p);
        let model = if m.display.is_empty() {
            m.id.clone()
        } else {
            m.display.clone()
        };
        let vendor = Self::placement_vendor(p);
        Some((
            model,
            model_id,
            Self::placement_engine(p).to_string(),
            vendor,
            Self::placement_mount(m, p),
            p.count.max(1).to_string(),
            if vendor == "furiosa" {
                Some("8".to_string())
            } else {
                None
            },
        ))
    }

    /// `[d] deploy` — 선택 모델의 배포(서빙) 옵션 편집 폼을 연다. replicas·디바이스·노드 배치.
    pub fn open_deploy_form(&mut self) {
        let Some((model, model_id, engine, vendor, mount, dev_default, serve_tp_default)) =
            self.selected_deploy_spec()
        else {
            return;
        };
        let want_kind = match vendor {
            "rbln" => crate::collect::AccelKind::Rbln,
            "furiosa" => crate::collect::AccelKind::Rngd,
            _ => crate::collect::AccelKind::Gpu,
        };
        let mut per_node: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for ac in self
            .snap
            .accel
            .iter()
            .filter(|x| x.kind == want_kind && !x.node.is_empty())
        {
            *per_node.entry(ac.node.clone()).or_insert(0) += 1;
        }
        let mut cand_nodes: Vec<String> = per_node.keys().cloned().collect();
        cand_nodes.sort();
        let mut place_choices = vec!["any".to_string(), "spread".to_string()];
        place_choices.extend(cand_nodes.iter().map(|n| format!("{}({})", n, per_node[n])));
        let mut fields = vec![CompileField {
            key: "replicas".into(),
            label: "replicas".into(),
            value: "1".into(),
            choices: ["1", "2", "3", "4", "6", "8"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            numeric: true,
            help: "Number of serving instances. Total device demand = replicas × devices.".into(),
        }];
        if vendor == "furiosa" {
            fields.push(CompileField {
                key: "tp".into(),
                label: "serve TP (PE)".into(),
                value: serve_tp_default.unwrap_or_else(|| "8".to_string()),
                choices: ["4", "8", "16"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Furiosa serving tensor parallel size in PE units. Device request stays separate.".into(),
            });
        }
        fields.extend(vec![
            CompileField {
                key: "devices".into(),
                label: "devices/replica".into(),
                value: dev_default,
                choices: ["1", "2", "4", "8"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Accelerators requested per replica (resources.limits). For Furiosa, this is RNGD count, not PE TP.".into(),
            },
            CompileField {
                key: "port".into(),
                label: "port".into(),
                value: "8000".into(),
                choices: ["8000", "8080"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Serving container port.".into(),
            },
            CompileField {
                key: "place".into(),
                label: "placement".into(),
                value: "any".into(),
                choices: place_choices,
                numeric: false,
                help: "Node placement: any=no extra constraint, spread=topology spread, hostname=pinned node.".into(),
            },
            CompileField {
                key: "routing".into(),
                label: "routing".into(),
                value: "llm-d".into(),
                choices: ["llm-d", "direct"].iter().map(|s| s.to_string()).collect(),
                numeric: false,
                help: "llm-d also creates InferencePool, EPP, and HTTPRoute; direct creates only Deployment.".into(),
            },
        ]);
        self.deploy_form = Some(DeployForm {
            model,
            model_id,
            engine,
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
        let devs: Vec<&crate::collect::Accel> = self
            .snap
            .accel
            .iter()
            .filter(|x| x.kind == want_kind)
            .collect();
        let total = devs.len() as i64;
        // 노드별 유휴(살아있고 미점유) — replica 는 한 노드에 per 개가 모여야 배치 가능(패킹).
        let mut free_by_node: std::collections::BTreeMap<&str, i64> =
            std::collections::BTreeMap::new();
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
            tips.push(format!(
                "(metric 유휴 {} ≠ 리소스 유휴 {} — 예약됐지만 idle 인 디바이스 있음)",
                free, resource_free
            ));
        }
        if form.get("place") == "spread" && replicas > nodes && nodes > 0 {
            tips.push(format!(
                "⚠ spread: replicas {} > 노드 {} — 일부는 같은 노드로",
                replicas, nodes
            ));
        }
        if per > 1 && form.vendor == "rbln" {
            tips.push("replica당 다중 칩은 컴파일 TP 와 일치해야 함".into());
        }
        DeployFit {
            demand,
            total,
            free,
            resource_free,
            nodes,
            verdict,
            tips,
        }
    }

    /// 배포 사전 점검(preflight) — apply 전에 서빙 전제조건 확인(사전 방어).
    pub fn deploy_preflight(&self, form: &DeployForm) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        // 이미지 — deploy_form_submit 의 벤더별 기본값과 동일 판정(불일치로 오탐 방지).
        //   furiosa=furiosaai/furiosa-llm:latest, gpu=vllm/vllm-openai:latest 기본 존재 → OK.
        //   rbln 은 vllm_rbln 런타임이 든 이미지가 필요(기본 없음) → LMD_SERVING_IMAGE 미지정이면 차단.
        let (img_ok, img_msg) = match form.vendor {
            "furiosa" => (
                true,
                "image ready: furiosaai/furiosa-llm:latest (furiosa-llm serve)".to_string(),
            ),
            "gpu" => {
                let img = self
                    .img_serving
                    .clone()
                    .unwrap_or_else(|| "vllm/vllm-openai:latest".into());
                (true, format!("image ready: {} (vLLM serve)", img))
            }
            _ => match &self.img_serving {
                Some(img) => (true, format!("image ready: {} (vllm_rbln runtime)", img)),
                None => (
                    true,
                    "image fallback: ubuntu:22.04 with host RBLN stack on the target node".into(),
                ),
            },
        };
        out.push((img_ok, format!("1. serving image: {}", img_msg)));
        // 2. Model artifact path — serving must load an HF id or compiled store path.
        out.push((
            !form.mount.is_empty(),
            format!(
                "2. model location: {}",
                if form.mount.is_empty() {
                    "unknown path: compile first or choose a store artifact".into()
                } else {
                    form.mount.clone()
                }
            ),
        ));
        // 3. NPU vendors need a node with the corresponding driver/resource plugin.
        if form.vendor != "gpu" {
            let want = if form.vendor == "rbln" {
                "RBLN"
            } else {
                "RNGD"
            };
            let any = self
                .snap
                .nodes
                .iter()
                .any(|n| n.npu.to_uppercase().contains(want));
            out.push((
                any,
                format!(
                    "3. accelerator driver: {} node {}",
                    want,
                    if any {
                        "exists: schedulable"
                    } else {
                        "missing: pods will stay Pending"
                    }
                ),
            ));
        }
        // 4. Capacity — requested devices must fit scheduler-visible free resources.
        let fit = self.deploy_fit(form);
        let cap_ok = matches!(fit.verdict, FitVerdict::Fits);
        out.push((
            cap_ok,
            format!(
                "4. capacity: needs {} device(s), {} free -> {}{}",
                fit.demand,
                fit.resource_free,
                fit.verdict.label(),
                if cap_ok {
                    ""
                } else {
                    " (stop another serving workload or lower replicas/devices)"
                }
            ),
        ));
        out
    }

    /// 배포 폼 → Deployment 매니페스트 미리보기(dry-run). Enter 시 호출.
    pub fn deploy_form_submit(&mut self) {
        let Some(form) = self.deploy_form.take() else {
            return;
        };
        let name = form.model_id.replace(['/', '.'], "-").to_lowercase();
        let name = format!("serve-{}", name);
        let replicas = form.get("replicas");
        let devices = form.get("devices");
        let serve_tp = if form.vendor == "furiosa" {
            let tp = form.get("tp");
            if tp.is_empty() {
                "8".to_string()
            } else {
                tp
            }
        } else {
            devices.clone()
        };
        let port = {
            let p = form.get("port");
            if p.is_empty() {
                "8000".to_string()
            } else {
                p
            }
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
            format!(
                "\x20     nodeSelector: {{ kubernetes.io/hostname: {} }}\n",
                place_host
            )
        } else if !product_label.is_empty() {
            format!("\x20     nodeSelector: {{ {} }}\n", product_label)
        } else {
            String::new()
        };
        // Vendor-specific serving specs. NPU engines do not accept the generic vLLM `--model` form.
        let served = if form.vendor == "furiosa" && form.mount.starts_with("/mnt/store/") {
            form.mount.clone()
        } else {
            form.model_id.clone()
        };
        let (image, note, container_spec, volumes_block) = match form.vendor {
            "furiosa" => {
                let img = self
                    .img_furiosa
                    .clone()
                    .unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into());
                let store_backed = form.mount.starts_with("/mnt/store/");
                let store_mount = if store_backed {
                    "\x20           - { name: store, mountPath: /mnt/store, readOnly: true }\n"
                } else {
                    ""
                };
                let fxb_args = if store_backed {
                    format!(
                        ", \"--fxb\", \"{}/model.fxb\"",
                        form.mount.trim_end_matches('/')
                    )
                } else {
                    String::new()
                };
                let spec = format!(
                    "\x20         args: [\"serve\", \"{model}\", \"--served-model-name\", \"{served_name}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{serve_tp}\"{fxb_args}]\n\
                     \x20         ports: [{{ containerPort: {port} }}]\n\
                     \x20         env:\n\
                     \x20           - {{ name: HF_HOME, value: /model-cache }}\n\
                     \x20           - {{ name: HF_TOKEN, valueFrom: {{ secretKeyRef: {{ name: hf-token, key: HF_TOKEN }} }} }}\n\
                     \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"4\", memory: \"16Gi\", {res_key}: {devices} }} }}\n\
                     \x20         volumeMounts:\n\
                     \x20           - {{ name: cache, mountPath: /model-cache }}\n\
                     {store_mount}",
                    model = form.model_id,
                    served_name = form.model_id,
                    port = port,
                    devices = devices,
                    serve_tp = serve_tp,
                    fxb_args = fxb_args,
                    res_key = res_key,
                    store_mount = store_mount
                );
                let vols = if store_backed {
                    "\x20     volumes:\n\x20       - { name: cache, emptyDir: {} }\n\x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n".to_string()
                } else {
                    "\x20     volumes:\n\x20       - { name: cache, emptyDir: {} }\n".to_string()
                };
                (img, "# Furiosa: furiosa-llm serve. Store-backed artifacts use the HF id plus --fxb so config/tokenizer still come from HF. Serving TP is PE count; resource devices are RNGD count.".to_string(), spec, vols)
            }
            "rbln" => {
                if let Some(img) = self.img_serving.clone() {
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
                    (img, "# RBLN: vllm_rbln runtime image from LMD_SERVING_IMAGE; loads the compiled artifact from model-store.".to_string(), spec, vols)
                } else {
                    let cmd = format!(
                        "set -eux\n\
                         export DEBIAN_FRONTEND=noninteractive\n\
                         apt-get update -qq\n\
                         apt-get install -y -qq --no-install-recommends python3.10 python3.10-dev python3-pip libdrm2 libnuma1 libgomp1 ca-certificates tzdata g++ libc6-dev >/dev/null\n\
                         ln -sf /usr/bin/python3.10 /usr/local/bin/python3\n\
                         python3 -m pip install -q --target=/opt/py-overrides --upgrade prometheus-fastapi-instrumentator\n\
                         export PYTHONPATH=\"/opt/py-overrides:${{PYTHONPATH}}\"\n\
                         exec python3 -m vllm.entrypoints.openai.api_server --model={mount} --served-model-name={served} --enforce-eager --max-num-seqs 1 --host=0.0.0.0 --port={port}\n",
                        mount = form.mount,
                        served = served,
                        port = port
                    );
                    let spec = format!(
                        "\x20         command: [\"bash\", \"-c\"]\n\
                         \x20         args:\n\
                         \x20           - |-\n\
                         {cmd_indented}\
                         \x20         ports: [{{ containerPort: {port} }}]\n\
                         \x20         env:\n\
                         \x20           - {{ name: PYTHONPATH, value: \"/home/gspark/.local/lib/python3.10/site-packages:/host-sys-local-pkgs:/host-sys-pkgs\" }}\n\
                         \x20           - {{ name: PYTHONUNBUFFERED, value: \"1\" }}\n\
                         \x20           - {{ name: VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK, value: \"{devices}\" }}\n\
                         \x20           - {{ name: LD_LIBRARY_PATH, value: \"/host-rbln-lib:/host-libs\" }}\n\
                         \x20           - {{ name: PATH, value: \"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/host-rbln-bin\" }}\n\
                         \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"8\", memory: \"32Gi\", {res_key}: {devices} }} }}\n\
                         \x20         volumeMounts:\n\
                         \x20           - {{ name: store, mountPath: /mnt/store, readOnly: true }}\n\
                         \x20           - {{ name: host-local-pkgs, mountPath: /home/gspark/.local/lib/python3.10/site-packages, readOnly: true }}\n\
                         \x20           - {{ name: host-sys-local-pkgs, mountPath: /host-sys-local-pkgs, readOnly: true }}\n\
                         \x20           - {{ name: host-sys-pkgs, mountPath: /host-sys-pkgs, readOnly: true }}\n\
                         \x20           - {{ name: host-libs, mountPath: /host-libs, readOnly: true }}\n\
                         \x20           - {{ name: host-rbln-lib, mountPath: /host-rbln-lib, readOnly: true }}\n\
                         \x20           - {{ name: host-rbln-bin, mountPath: /host-rbln-bin, readOnly: true }}\n\
                         \x20           - {{ name: shm, mountPath: /dev/shm }}\n",
                        cmd_indented = cmd.lines().map(|l| format!("             {}\n", l)).collect::<String>(),
                        port = port,
                        devices = devices,
                        res_key = res_key
                    );
                    let vols = "\x20     volumes:\n\
                                \x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n\
                                \x20       - { name: host-local-pkgs, hostPath: { path: /home/gspark/.local/lib/python3.10/site-packages, type: Directory } }\n\
                                \x20       - { name: host-sys-local-pkgs, hostPath: { path: /usr/local/lib/python3.10/dist-packages, type: Directory } }\n\
                                \x20       - { name: host-sys-pkgs, hostPath: { path: /usr/lib/python3/dist-packages, type: Directory } }\n\
                                \x20       - { name: host-libs, hostPath: { path: /usr/lib/x86_64-linux-gnu, type: Directory } }\n\
                                \x20       - { name: host-rbln-lib, hostPath: { path: /usr/lib, type: Directory } }\n\
                                \x20       - { name: host-rbln-bin, hostPath: { path: /usr/bin, type: Directory } }\n\
                                \x20       - { name: shm, emptyDir: { medium: Memory, sizeLimit: 16Gi } }\n"
                        .to_string();
                    ("ubuntu:22.04".to_string(), "# RBLN: using host RBLN stack fallback on the target node; loads the compiled artifact from model-store.".to_string(), spec, vols)
                }
            }
            _ => {
                let img = self
                    .img_serving
                    .clone()
                    .unwrap_or_else(|| "vllm/vllm-openai:latest".into());
                let spec = format!(
                    "\x20         args: [\"serve\", \"{mount}\", \"--served-model-name\", \"{served}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{devices}\"]\n\
                     \x20         ports: [{{ containerPort: {port} }}]\n\
                     \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"4\", memory: \"16Gi\", {res_key}: {devices} }} }}\n\
                     \x20         volumeMounts:\n\
                     \x20           - {{ name: store, mountPath: /mnt/store, readOnly: true }}\n",
                    mount = form.mount, served = served, port = port, devices = devices, res_key = res_key
                );
                let vols = "\x20     volumes:\n\x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n".to_string();
                (img, "# GPU: vLLM loads the model/store path directly; no NPU compile step required.".to_string(), spec, vols)
            }
        };
        let yaml = format!(
            "# Deployment manifest preview. Review, then apply with `kubectl apply -f -`.\n\
             # Serving model {model_id}. Engine: {engine}.\n\
             # Placement: {place}. Total device demand = {replicas} x {devices}.\n\
             # If the image contains a TODO- placeholder, set LMD_SERVING_IMAGE before applying.\n\
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
        // 자동화: YAML 을 덤프하지 않고 바로 apply 확인 팝업. (YAML 은 팝업에서 e=vi 편집·v=검증)
        self.confirm = Some(Pending::Apply {
            title: format!("deploy {} ×{}", form.model, replicas),
            yaml,
        });
        self.confirm_yes = false;
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
        let slug = served
            .rsplit('/')
            .next()
            .unwrap_or(served)
            .to_lowercase()
            .replace(['.', '_'], "-");
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
            View::Models | View::Overview => self.selected_model().and_then(|m| {
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
            // Deploy▸Library '진행 중 컴파일' 패널 — 선택 Job 의 파드 로그.
            View::Library if self.panel_focus == 1 => self
                .sel_orig()
                .and_then(|i| self.snap.compiles.get(i))
                .and_then(|c| {
                    self.snap
                        .pods
                        .iter()
                        .find(|p| p.name.starts_with(&c.name))
                        .map(|p| p.name.clone())
                }),
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
    let serving = s.models.iter().filter(|m| m.ready > 0).count();
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
        }
    }
    fn app_with(models: Vec<ModelRow>, pods: Vec<PodRow>) -> App {
        let mut a = App::new();
        a.snap = Snapshot {
            models,
            pods,
            ..Default::default()
        };
        a.view = View::Models;
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
        use crate::app::Section;
        let mut a = App::new();
        a.goto_view(View::Library); // Deploy▸Library: 2 패널(통합 배포 트리 / 진행 중 컴파일)
        assert_eq!(a.panel_count(), 2);
        a.selected = 2;
        a.cycle_panel_dir(1); // Ctrl-w — 패널 포커스만 이동(서브탭과 직교)
        assert_eq!(a.panel_focus, 1);
        assert_eq!(a.selected, 0, "패널 전환 시 선택 리셋");
        a.cycle_panel_dir(1);
        assert_eq!(a.panel_focus, 0, "2패널 순환");
        // Serving 서브탭은 단일 패널.
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
        a.goto_view(View::Models); // Models 섹션: Models→Perf→Pods
        a.cycle_subtab(1);
        assert_eq!(a.view, View::Perf);
        a.cycle_subtab(1);
        assert_eq!(a.view, View::Pods);
        a.cycle_subtab(1);
        assert_eq!(a.view, View::Models, "서브탭 순환");
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
        assert!(g
            .items
            .iter()
            .any(|i| i.action == Action::Compile("furiosa")));
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
        a.view = View::Library;
        a.panel_focus = 2; // Deploy▸Library 진행 중 컴파일 패널(0 스토어·1 카탈로그·2 컴파일 Job)
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
        a.goto_view(View::Models);
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
        assert_eq!(Overlay::PRECEDENCE.len(), 12);
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
        // serving_order 는 family›version 그룹 순서 — 같은 family 가 인접해야 트리 내비가 자연스럽다.
        let ord = a.serving_order();
        assert_eq!(ord.len(), 3);
        assert_eq!(a.snap.artifacts[ord[0]].family, "llama3");
        assert_eq!(a.snap.artifacts[ord[1]].family, "llama3");
        assert_eq!(a.snap.artifacts[ord[2]].family, "qwen2.5");

        // Serving 렌즈 — 패닉 없이 family/version/replica 표시.
        a.view = View::Serving;
        a.selected = 0;
        let mut fx = crate::ui::FxState::disabled();
        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let text = buf_text(t.backend().buffer());
        assert!(text.contains("llama3"), "family 헤더\n{text}");
        assert!(text.contains("qwen2.5"), "두 번째 family 헤더");
        assert!(
            text.contains("Instruct-FP8"),
            "version 이 둘 이상이면 version 티어 노출"
        );
        assert!(text.contains("2/2 rep"), "모델 replica 상태 반영");

        // Library 렌즈 — 카탈로그 트리(임베드 카탈로그) + 패닉 없음.
        a.view = View::Library;
        a.panel_focus = 0;
        a.selected = 0;
        let mut t2 = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t2.draw(|f| crate::ui::draw(f, &a, &mut fx)).unwrap();
        let ltext = buf_text(t2.backend().buffer());
        assert!(ltext.contains("Library"), "Library 타이틀\n{ltext}");
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
}
