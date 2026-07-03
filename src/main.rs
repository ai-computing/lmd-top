//! lmd-top — llm-d 클러스터 터미널 관측 도구 (Phase 1: monitor).
//! 실행: `lmd-top`            → TUI
//!       `lmd-top --snapshot` → 1회 수집 후 텍스트 출력(헤드리스 검증용)

mod agent;
mod app;
mod audit;
mod cast;
mod catalog;
mod collect;
mod compat;
mod config;
mod doctor;
mod kube;
mod metrics;
mod ops;
mod palette;
mod prom;
mod ui;

use anyhow::Result;
use app::{App, Mode, Pending, View};
use collect::collect;
use config::Config;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HELP: &str = "\
lmd-top — terminal observability & operations for llm-d clusters

USAGE:
    lmd-top [OPTIONS]

OPTIONS:
    --mode <MODE>    permission mode: observe (default) | debug | admin | danger
    --json           print machine-readable agent state (JSON) and exit
    --doctor         survey Prometheus: exporters, metric coverage, gaps
    --audit          print the audit log of applied mutations, then exit
    --snapshot, -s   collect once, print headless text summary
    --render         render each view to text via TestBackend (CI / no-tty)
    --cast [FILE]    write a demo asciicast (default: docs/demo.cast)
    --help, -h       show this help and exit

ENVIRONMENT:
    LMD_PROM         Prometheus host:port
    LMD_NS           namespace (default: llm-serving)
    LMD_GRAFANA      Grafana base URL (opened by the `g` key)
    LMD_THEME        startup theme: soft | default | high-contrast | colorblind
    LMD_AUDIT        audit log path (default: ~/.config/lmd-top/audit.log)
    LMD_W / LMD_H    size for --render

With no options, lmd-top launches the interactive TUI. See `?` in the TUI for keybindings.";

/// 인자 검증 결과 — main 이 이에 따라 도움말/에러를 출력하고 분기.
#[derive(Debug, PartialEq)]
enum ArgCheck {
    Ok,
    Help,
    Unknown(String),
    BadMode(String),
    MissingMode,
}

/// args[1..] 를 훑어 알려진 플래그만 허용. 미지원 플래그/잘못된 --mode 는 거부.
fn check_args(args: &[String]) -> ArgCheck {
    // 도움말이 있으면 최우선.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return ArgCheck::Help;
    }
    const NOVALUE: &[&str] = &["--doctor", "--json", "--snapshot", "-s", "--render", "--audit"];
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--mode" {
            match args.get(i + 1) {
                None => return ArgCheck::MissingMode,
                Some(v) if Mode::parse(v).is_none() => return ArgCheck::BadMode(v.clone()),
                Some(_) => i += 2,
            }
            continue;
        }
        if a == "--cast" {
            // 선택적 출력 경로 값(플래그가 아니면 소비).
            if args.get(i + 1).map(|s| !s.starts_with('-')).unwrap_or(false) {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if NOVALUE.contains(&a) {
            i += 1;
            continue;
        }
        return ArgCheck::Unknown(args[i].clone());
    }
    ArgCheck::Ok
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let cfg = Config::default();
    let args: Vec<String> = std::env::args().collect();

    // 인자 검증 — 도움말/미지원 플래그/잘못된 --mode 는 여기서 처리(TUI 진입 전).
    match check_args(&args) {
        ArgCheck::Ok => {}
        ArgCheck::Help => {
            println!("{}", HELP);
            return Ok(());
        }
        ArgCheck::Unknown(f) => {
            eprintln!("lmd-top: unknown argument '{}'\n", f);
            eprintln!("{}", HELP);
            std::process::exit(2);
        }
        ArgCheck::BadMode(v) => {
            eprintln!("lmd-top: invalid --mode '{}' (expected observe|debug|admin|danger)\n", v);
            eprintln!("{}", HELP);
            std::process::exit(2);
        }
        ArgCheck::MissingMode => {
            eprintln!("lmd-top: --mode requires a value (observe|debug|admin|danger)\n");
            eprintln!("{}", HELP);
            std::process::exit(2);
        }
    }

    // 메트릭 전수조사 + 갭 분석(왜 뷰가 비었나 진단).
    if args.iter().any(|a| a == "--doctor") {
        doctor::run(&cfg).await;
        return Ok(());
    }

    // 감사 로그 열람 — lmd-top 이 적용한 변경 작업 이력(파일)을 출력.
    if args.iter().any(|a| a == "--audit") {
        audit::print_log();
        return Ok(());
    }

    // 기계가독 상태(agent) — --json (또는 --snapshot --json). 1회 수집 후 JSON 출력.
    if args.iter().any(|a| a == "--json") {
        let snap = collect(&cfg).await;
        agent::emit_json(&snap, &cfg);
        return Ok(());
    }

    if args.iter().any(|a| a == "--snapshot" || a == "-s") {
        let snap = collect(&cfg).await;
        print_snapshot(&snap, &cfg);
        return Ok(());
    }

    if args.iter().any(|a| a == "--render") {
        let snap = collect(&cfg).await;
        render_dump(snap);
        return Ok(());
    }

    // 데모 asciicast 생성(agg 로 GIF 변환용). --cast [out.cast]
    if let Some(pos) = args.iter().position(|a| a == "--cast") {
        let out = args.get(pos + 1).filter(|s| !s.starts_with('-')).cloned().unwrap_or_else(|| "docs/demo.cast".to_string());
        cast::run(&cfg, &out).await;
        return Ok(());
    }

    // 권한 모드(기동 시) — --mode observe|debug|admin|danger (기본 observe)
    let mode = args
        .iter()
        .position(|a| a == "--mode")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| Mode::parse(s))
        .unwrap_or(Mode::Observe);

    run_tui(cfg, mode).await
}

/// 변경 작업 실행 결과 — 워커 스레드에서 kube 호출 후 메인 스레드로 전달돼 audit/UI 반영.
/// kube 호출을 UI 스레드 밖으로 빼 렌더가 얼지 않게 한다(최대 8s 프리즈 제거).
struct MutationOutcome {
    mode: Mode,
    audit_action: String,       // audit 로그 action 필드
    audit_target: String,       // audit 로그 target 필드
    fail_label: &'static str,   // 실패 시 창 제목(show_fail)
    result: Result<OkInfo, String>, // Ok=성공 표시정보 / Err=실패 이유(1줄+)
}
/// 성공 시 표시 정보 — audit 상세, 토스트 문구, preview 닫기 여부(Apply 성공).
struct OkInfo {
    audit_detail: String,
    notify: String,
    clear_preview: bool,
}

/// 확정된 변경 작업을 실제 kubectl 로 수행하고 결과를 MutationOutcome 으로. (워커 스레드용)
/// 모든 분기가 kube 를 호출하므로 실클러스터 없이는 단위검증 불가 — 결과 처리는 apply_outcome 에서(검증 가능).
fn run_mutation(pending: Pending, ns: &str, mode: Mode) -> MutationOutcome {
    let mk = |audit_action: String, audit_target: String, fail_label: &'static str, result: Result<OkInfo, String>| MutationOutcome {
        mode,
        audit_action,
        audit_target,
        fail_label,
        result,
    };
    match pending {
        Pending::Scale { name, target } => {
            let r = kube::scale_deploy(ns, &name, target)
                .map(|_| OkInfo { audit_detail: "scaled".into(), notify: format!("scaled {} → {}", name, target), clear_preview: false })
                .map_err(|e| e.to_string());
            mk(format!("scale→{}", target), name, "scale", r)
        }
        Pending::Restart { name } => {
            let r = kube::rollout_restart(ns, &name)
                .map(|_| OkInfo { audit_detail: "restarted".into(), notify: format!("rollout restart {}", name), clear_preview: false })
                .map_err(|e| e.to_string());
            mk("rollout-restart".into(), name, "restart", r)
        }
        Pending::Stop { name } => {
            let r = kube::scale_deploy(ns, &name, 0)
                .map(|_| OkInfo { audit_detail: "stopped".into(), notify: format!("stopped {} (scaled → 0)", name), clear_preview: false })
                .map_err(|e| e.to_string());
            mk("stop(scale→0)".into(), name, "stop", r)
        }
        Pending::Cordon { node, on } => {
            let act = if on { "cordon" } else { "uncordon" };
            let r = kube::cordon(&node, on)
                .map(|_| OkInfo { audit_detail: "ok".into(), notify: format!("{} {}", if on { "cordoned" } else { "uncordoned" }, node), clear_preview: false })
                .map_err(|e| e.to_string());
            mk(act.into(), node, "cordon", r)
        }
        Pending::DeletePod { name } => {
            let r = kube::delete_pod(ns, &name)
                .map(|_| OkInfo { audit_detail: "deleted".into(), notify: format!("deleted pod {}", name), clear_preview: false })
                .map_err(|e| e.to_string());
            mk("delete-pod".into(), name, "delete", r)
        }
        Pending::DeleteJob { name } => {
            let r = kube::delete_job(ns, &name)
                .map(|_| OkInfo { audit_detail: "deleted".into(), notify: format!("deleted job {}", name), clear_preview: false })
                .map_err(|e| e.to_string());
            mk("delete-job".into(), name, "delete job", r)
        }
        Pending::RouteRename { route, old, new } => {
            let r = kube::route_set_path(ns, &route, &old, &new)
                .map(|_| OkInfo { audit_detail: "renamed".into(), notify: format!("renamed route {} → {}", old, new), clear_preview: false })
                .map_err(|e| e.to_string());
            mk(format!("route-rename→{}", new), format!("{}:{}", route, old), "route rename", r)
        }
        Pending::RouteRetarget { route, path, backend, kind } => {
            let r = kube::route_retarget(ns, &route, &path, &backend, &kind)
                .map(|_| OkInfo { audit_detail: "retargeted".into(), notify: format!("retargeted {} → {}", path, backend), clear_preview: false })
                .map_err(|e| e.to_string());
            mk(format!("route-retarget→{}:{}", kind, backend), format!("{}:{}", route, path), "route retarget", r)
        }
        Pending::RouteDelete { route, path } => {
            let r = kube::route_delete_rule(ns, &route, &path)
                .map(|_| OkInfo { audit_detail: "deleted".into(), notify: format!("deleted route {}", path), clear_preview: false })
                .map_err(|e| e.to_string());
            mk("route-delete".into(), format!("{}:{}", route, path), "route delete", r)
        }
        Pending::Apply { yaml, title } => {
            let r = kube::apply_manifest(ns, &yaml, false)
                .map(|o| {
                    let line = o.lines().next().unwrap_or("ok").to_string();
                    OkInfo { audit_detail: line.clone(), notify: format!("applied — {}", line), clear_preview: true }
                })
                .map_err(|e| e.to_string());
            mk("apply".into(), title, "apply", r)
        }
    }
}

/// 변경 작업 결과를 audit 로그 + UI(토스트/실패 preview)에 반영. (메인 스레드, 단위검증 가능)
fn apply_outcome(app: &mut App, o: MutationOutcome) {
    match o.result {
        Ok(ok) => {
            audit::record(o.mode, &o.audit_action, &o.audit_target, Ok(&ok.audit_detail));
            if ok.clear_preview {
                app.preview = None;
            }
            app.notify(ok.notify);
        }
        Err(e) => {
            audit::record(o.mode, &o.audit_action, &o.audit_target, Err(&e));
            // 실패 이유는 사라지는 토스트가 아니라 스크롤 가능한 창으로.
            app.notify(format!("{} 실패 — 자세한 이유는 창 참고", o.fail_label));
            app.preview = Some((format!("⚠ {} 실패 — 이유 (q 닫기)", o.fail_label), e));
            app.preview_scroll = 0;
            app.preview_apply = false;
        }
    }
}

/// 커맨드 팔레트 선택 실행 — 네비게이션·표시 액션 전용(클러스터 무변경, 권한 불필요).
fn dispatch_palette(
    app: &mut App,
    pa: palette::PaletteAction,
    cfg: &Config,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) {
    use palette::PaletteAction::*;
    match pa {
        Goto(v) => app.goto_view(v),
        Help => app.toggle_help(),
        Alerts => app.toggle_alerts(),
        Theme => app.cycle_theme(),
        Pause => app.paused = !app.paused,
        Zoom => app.zoom = !app.zoom,
        Grafana => {
            let base = cfg.grafana.clone();
            let _ = std::process::Command::new("xdg-open")
                .arg(&base)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            app.notify(format!("Grafana: {}", base));
            terminal.clear().ok();
        }
    }
}

/// 액션 메뉴에서 고른 동작 실행 — 권한 게이팅 포함. 폼/확인은 오버레이로, 로그는 즉시.
fn require_action(app: &mut App, action: app::Action) -> bool {
    let required = action.required_mode();
    if app.can(required) {
        true
    } else {
        app.notify(format!("{} needs --mode {}+ (current: {})", action.verb(), required.name(), app.mode.name()));
        false
    }
}

fn dispatch_action(app: &mut App, action: app::Action, subject: &str, ns: &str, prom: &str, rt: &tokio::runtime::Handle) {
    use app::{Action, Pending, View};
    match action {
        Action::Info => {
            if app.view == View::Routing {
                app.drill_route();
            } else if app.view == View::Launch && app.panel_focus == 1 {
                app.pivot_to_node(subject); // 노드 상세로 점프
            } else if app.view == View::Launch && app.panel_focus == 2 {
                let s = app.catalog_feasibility(subject);
                app.notify(s);
            } else {
                app.detail = true;
            }
        }
        Action::Compile(vendor) => {
            if require_action(app, action) {
                app.compile_form_for(vendor);
            }
        }
        Action::Deploy => {
            if require_action(app, action) {
                app.open_deploy_form();
            }
        }
        Action::Stop => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Stop { name: subject.to_string() });
            }
        }
        Action::Scale => {
            if require_action(app, action) {
                let target = app.selected_model().map(|m| if m.desired == 0 { 1 } else { 0 }).unwrap_or(1);
                app.confirm = Some(Pending::Scale { name: subject.to_string(), target });
            }
        }
        Action::Restart => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Restart { name: subject.to_string() });
            }
        }
        Action::Yaml => {
            if let Some((kind, nsd, name)) = app.yaml_target() {
                let nsopt = if nsd { Some(ns) } else { None };
                match kube::resource_yaml(kind, nsopt, &name) {
                    Ok(y) => {
                        app.preview = Some((format!("{} {} · yaml (read-only)", kind, name), y));
                        app.preview_scroll = 0;
                        app.preview_apply = false;
                    }
                    Err(e) => app.notify(format!("yaml: {}", e.to_string().lines().next().unwrap_or(""))),
                }
            }
        }
        Action::Delete => {
            if require_action(app, action) {
                app.confirm = Some(Pending::DeletePod { name: subject.to_string() });
            }
        }
        Action::DeleteJob => {
            if require_action(app, action) {
                app.confirm = Some(Pending::DeleteJob { name: subject.to_string() });
            }
        }
        Action::Objective => app.open_objective_form(), // 목표 설정(관측 전용, 권한 불필요)
        Action::Cordon | Action::Uncordon => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Cordon { node: subject.to_string(), on: matches!(action, Action::Cordon) });
            }
        }
        Action::RouteRename => {
            if require_action(app, action) {
                app.open_route_rename();
            }
        }
        Action::RouteRetarget => {
            if require_action(app, action) {
                app.open_route_retarget();
            }
        }
        Action::RouteDelete => {
            if require_action(app, action) {
                if let Some(r) = app.selected_route() {
                    app.confirm = Some(Pending::RouteDelete { route: r.route, path: r.path });
                }
            }
        }
        Action::Logs => {
            if !require_action(app, action) {
            } else if let Some(pod) = app.logs_target_pod() {
                match kube::logs(ns, &pod, 400) {
                    Ok(l) => {
                        app.logs_scroll = l.len().saturating_sub(30) as u16;
                        app.logs = l;
                        app.logs_target = pod;
                        app.logs_mode = true;
                    }
                    Err(e) => app.notify(format!("logs: {}", e)),
                }
            } else {
                app.notify("logs: no pod for selection".to_string());
            }
        }
    }
    let _ = (prom, rt); // 현재 미사용(향후 온디맨드 조회용 자리)
}

/// preview 내용을 파일로 저장 — 편집 후 kubectl apply 하거나 보관용. 제목에서 파일명 유추.
fn save_manifest(title: &str, yaml: &str) -> Result<String> {
    // 제목 첫 토큰들 → 안전한 파일명.
    let base: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() { "manifest".to_string() } else { base };
    let dir = std::env::var("LMD_SAVE_DIR").unwrap_or_else(|_| ".".to_string());
    let path = format!("{}/lmd-{}.yaml", dir.trim_end_matches('/'), &base[..base.len().min(48)]);
    std::fs::write(&path, yaml)?;
    Ok(path)
}

/// 매니페스트를 $EDITOR(기본 vi)로 편집 — TUI 를 잠시 내리고 편집기 실행 후 복원.
/// 저장하고 정상 종료하면 편집된 내용을 반환, 아니면 None.
fn edit_in_editor(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, content: &str) -> Option<String> {
    let path = std::env::temp_dir().join("lmd-top-manifest.yaml");
    if std::fs::write(&path, content).is_err() {
        return None;
    }
    let editor = std::env::var("EDITOR").or_else(|_| std::env::var("VISUAL")).unwrap_or_else(|_| "vi".into());
    // TUI 정지(대체화면 나가고 raw 해제) → 편집기 → 복원.
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    let status = std::process::Command::new(&editor).arg(&path).status();
    let _ = enable_raw_mode();
    let _ = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture);
    let _ = terminal.clear();
    match status {
        Ok(s) if s.success() => std::fs::read_to_string(&path).ok(),
        _ => None,
    }
}

/// 헤드리스 렌더 검증 — TestBackend 로 각 뷰를 한 프레임 그려 텍스트로 출력.
fn render_dump(snap: collect::Snapshot) {
    use app::View;
    use ratatui::backend::TestBackend;
    let mut a = App::new();
    a.apply(snap);
    let rw: u16 = std::env::var("LMD_W").ok().and_then(|s| s.parse().ok()).unwrap_or(100);
    let rh: u16 = std::env::var("LMD_H").ok().and_then(|s| s.parse().ok()).unwrap_or(26);
    let mut fx = ui::FxState::disabled(); // 텍스트 덤프 — 이펙트 끔(부분 프레임 방지)
    // ALL(8 탭) + 허브 하위 뷰(Accel/Perf) 까지 렌더 커버리지.
    let views: Vec<View> = View::ALL.iter().copied().chain([View::Accel, View::Perf, View::Topo]).collect();
    for v in views {
        a.view = v;
        a.selected = 0;
        let backend = TestBackend::new(rw, rh);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| ui::draw(f, &a, &mut fx)).unwrap();
        let buf = term.backend().buffer().clone();
        println!("\n========== VIEW: {} ==========", v.title());
        let area = buf.area;
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                if let Some(c) = buf.cell((x, y)) {
                    line.push_str(c.symbol());
                }
            }
            println!("{}", line.trim_end());
        }
    }
}

async fn run_tui(cfg: Config, mode: Mode) -> Result<()> {
    let shared = Arc::new(Mutex::new(collect(&cfg).await)); // 첫 수집(즉시 표시)

    // full 수집 루프 (3초) — 모델/EPP/perf/events 등 무거운 것
    {
        let shared = shared.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(cfg.interval_full));
            tick.tick().await;
            loop {
                tick.tick().await;
                let snap = collect(&cfg).await;
                if let Ok(mut g) = shared.lock() {
                    *g = snap;
                }
            }
        });
    }
    // fast tier (1초) — 가속기 util/mem/temp + 노드만 빠르게 갱신(반응성)
    {
        let shared = shared.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(cfg.interval_fast));
            tick.tick().await;
            loop {
                tick.tick().await;
                let (accel, nodes) = collect::collect_fast(&cfg).await;
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if let Ok(mut g) = shared.lock() {
                    // 빈 결과로 기존 테이블을 덮지 않음 — Prometheus 순간 장애가 "가속기 없음"으로 오인되는 것 방지.
                    if !accel.is_empty() || g.accel.is_empty() {
                        g.accel = accel;
                    }
                    if !nodes.is_empty() || g.nodes.is_empty() {
                        g.nodes = nodes;
                    }
                    g.ts = ts;
                }
            }
        });
    }

    // UI 루프(블로킹)
    let rt = tokio::runtime::Handle::current(); // Perf 드릴 온디맨드 조회용
    let res = tokio::task::spawn_blocking(move || ui_loop(shared, cfg, mode, rt)).await?;
    res
}

fn ui_loop(shared: Arc<Mutex<collect::Snapshot>>, cfg: Config, mode: Mode, rt: tokio::runtime::Handle) -> Result<()> {
    let ns = cfg.ns.clone();
    let prom = cfg.prom.clone();
    // 패닉 시에도 터미널 복원(raw mode/alt-screen 해제) — 안 하면 셸이 망가짐.
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        orig_hook(info);
    }));
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.mode = mode;
    app.ns = ns.clone(); // 매니페스트/액션 네임스페이스 = cfg.ns(LMD_NS)
    app::set_theme(cfg.theme); // 시작 테마(LMD_THEME / yaml)
    let mut fx = ui::FxState::new();
    // 변경 작업 결과 채널 — 워커 스레드가 kube 실행 후 결과를 보내고, 메인 루프가 drain 해 반영.
    let (mut_tx, mut_rx) = std::sync::mpsc::channel::<MutationOutcome>();
    let result = (|| -> Result<()> {
        loop {
            if !app.paused {
                let snap = shared.lock().map(|g| g.clone()).unwrap_or_default();
                app.apply(snap);
            }
            // 완료된 변경 작업 반영(audit + 토스트/실패창). in-flight 해제.
            while let Ok(outcome) = mut_rx.try_recv() {
                app.inflight = None;
                apply_outcome(&mut app, outcome);
            }
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|f| ui::draw(f, &app, &mut fx))?;

            // 애니메이션 중엔 짧게 폴링(부드러운 프레임), 평시엔 100ms.
            let poll_ms = if fx.animating() { 16 } else { 100 };
            if event::poll(Duration::from_millis(poll_ms))? {
                let ev = event::read()?;
                if let Event::Mouse(m) = ev {
                    match m.kind {
                        MouseEventKind::ScrollDown => app.move_sel(1),
                        MouseEventKind::ScrollUp => app.move_sel(-1),
                        _ => {}
                    }
                    continue;
                }
                if let Event::Key(k) = ev {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    // 필터 입력 모드: 텍스트 캡처
                    if app.filtering {
                        match k.code {
                            KeyCode::Esc | KeyCode::Enter => app.stop_filter(),
                            KeyCode::Backspace => app.filter_pop(),
                            KeyCode::Char(c) => app.filter_push(c),
                            _ => {}
                        }
                        continue;
                    }
                    // 커맨드 팔레트(`:`) — 텍스트 캡처 + 퍼지 선택. Enter 실행, Esc 닫기.
                    if app.palette.is_some() {
                        let action = match k.code {
                            KeyCode::Esc => {
                                app.palette = None;
                                None
                            }
                            KeyCode::Backspace => {
                                app.palette.as_mut().unwrap().pop();
                                None
                            }
                            KeyCode::Up => {
                                app.palette.as_mut().unwrap().move_cursor(-1);
                                None
                            }
                            KeyCode::Down => {
                                app.palette.as_mut().unwrap().move_cursor(1);
                                None
                            }
                            KeyCode::Enter => app.palette.as_ref().unwrap().selected(),
                            KeyCode::Char(c) => {
                                app.palette.as_mut().unwrap().push(c);
                                None
                            }
                            _ => None,
                        };
                        if let Some(pa) = action {
                            app.palette = None;
                            dispatch_palette(&mut app, pa, &cfg, &mut terminal);
                        }
                        continue;
                    }
                    // 도움말 오버레이: 아무 키나 닫기
                    if app.help {
                        app.help = false;
                        continue;
                    }
                    // 변경 작업 확인(y/n) — 다른 키 무시. 실행은 여기서(권한 검증은 트리거 시점).
                    if let Some(pending) = app.confirm.clone() {
                        // Apply 확인일 때만: e=vi 편집, v=서버 dry-run 검증 (YAML 은 특정 키로만 접근).
                        if let Pending::Apply { yaml, title } = &pending {
                            match k.code {
                                KeyCode::Char('e') | KeyCode::Char('E') => {
                                    if let Some(edited) = edit_in_editor(&mut terminal, yaml) {
                                        app.confirm = Some(Pending::Apply { title: title.clone(), yaml: edited });
                                        app.notify("manifest 편집됨 (vi) — Enter 로 적용".to_string());
                                    }
                                    continue;
                                }
                                KeyCode::Char('v') | KeyCode::Char('V') => {
                                    match kube::apply_manifest(&ns, yaml, true) {
                                        Ok(o) => app.notify(format!("valid ✓ {}", o.lines().next().unwrap_or("ok"))),
                                        Err(e) => app.notify(format!("invalid: {}", e.to_string().lines().next().unwrap_or(""))),
                                    }
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        // ←→/h/l 로 Yes/No 토글, Enter 로 선택 결정, y 즉시 실행, n/esc 취소.
                        let execute = match k.code {
                            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') | KeyCode::Tab => {
                                app.confirm_yes = !app.confirm_yes;
                                continue;
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => true,
                            KeyCode::Enter => app.confirm_yes,
                            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => false,
                            _ => continue, // 그 외 키는 무시(팝업 유지)
                        };
                        app.confirm = None;
                        app.confirm_yes = false; // 다음 팝업 기본값 No
                        if !execute {
                            app.notify("cancelled".to_string());
                            continue;
                        }
                        // 변경 작업은 워커 스레드로 실행 — kube(최대 8s)가 UI 를 얼리지 않게.
                        // 결과는 mut_rx 로 받아 메인 루프에서 apply_outcome(audit + 토스트/실패창).
                        if app.inflight.is_some() {
                            app.notify("변경 작업 진행 중 — 완료 후 다시 시도".to_string());
                            continue;
                        }
                        app.inflight = Some(pending.prompt());
                        let tx = mut_tx.clone();
                        let ns2 = ns.clone();
                        let amode = app.mode;
                        std::thread::spawn(move || {
                            let _ = tx.send(run_mutation(pending, &ns2, amode));
                        });
                        continue;
                    }
                    // 알림 히스토리 오버레이: esc/q/A 로 닫기(다른 키 무시)
                    if app.alerts_panel {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('A') | KeyCode::Char('a') => {
                                app.alerts_panel = false
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // Enter 컨텍스트 액션 메뉴 — ↑↓ 선택, Enter/단축키 실행, q/Esc 닫기.
                    if app.action_menu.is_some() {
                        let act = match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                app.action_menu = None;
                                None
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.action_menu.as_mut().unwrap().move_cursor(-1);
                                None
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.action_menu.as_mut().unwrap().move_cursor(1);
                                None
                            }
                            KeyCode::Enter => app.action_menu.as_ref().unwrap().current(),
                            KeyCode::Char(c) => app.action_menu.as_ref().unwrap().by_key(c),
                            _ => None,
                        };
                        if let Some(action) = act {
                            let subject = app.action_menu.as_ref().unwrap().subject.clone();
                            app.action_menu = None;
                            dispatch_action(&mut app, action, &subject, &ns, &prom, &rt);
                        }
                        continue;
                    }
                    // 서빙 목표(SLO) 편집 폼 오버레이
                    if app.objective_form.is_some() {
                        let editing = app.objective_form.as_ref().unwrap().editing;
                        if editing {
                            match k.code {
                                KeyCode::Enter | KeyCode::Esc => app.objective_form.as_mut().unwrap().editing = false,
                                KeyCode::Backspace => app.objective_form.as_mut().unwrap().backspace(),
                                KeyCode::Char(c) => app.objective_form.as_mut().unwrap().type_char(c),
                                _ => {}
                            }
                        } else {
                            match k.code {
                                KeyCode::Esc | KeyCode::Char('q') => app.objective_form = None,
                                KeyCode::Up => app.objective_form.as_mut().unwrap().move_cursor(-1),
                                KeyCode::Down => app.objective_form.as_mut().unwrap().move_cursor(1),
                                KeyCode::Left => app.objective_form.as_mut().unwrap().cycle(-1),
                                KeyCode::Right => app.objective_form.as_mut().unwrap().cycle(1),
                                KeyCode::Char('e') => app.objective_form.as_mut().unwrap().editing = true,
                                KeyCode::Backspace => app.objective_form.as_mut().unwrap().backspace(),
                                KeyCode::Char(c) if c.is_ascii_digit() => app.objective_form.as_mut().unwrap().type_digit(c),
                                KeyCode::Enter => app.objective_form_submit(),
                                _ => {}
                            }
                        }
                        continue;
                    }
                    // 라우트 편집 폼(rename 텍스트 / retarget 선택)
                    if app.route_form.is_some() {
                        let rename = app.route_form.as_ref().unwrap().rename;
                        match k.code {
                            KeyCode::Esc => app.route_form = None,
                            KeyCode::Char('q') if !rename => app.route_form = None,
                            KeyCode::Enter => {
                                let form = app.route_form.take().unwrap();
                                if rename {
                                    let new = form.value.trim().to_string();
                                    if new.is_empty() || new == form.path {
                                        app.notify("route: 변경 없음".to_string());
                                    } else {
                                        app.confirm = Some(Pending::RouteRename { route: form.route, old: form.path, new });
                                    }
                                } else if let Some((kind, name)) = form.value.split_once(':') {
                                    app.confirm = Some(Pending::RouteRetarget {
                                        route: form.route,
                                        path: form.path,
                                        backend: name.to_string(),
                                        kind: kind.to_string(),
                                    });
                                }
                            }
                            KeyCode::Backspace if rename => {
                                app.route_form.as_mut().unwrap().value.pop();
                            }
                            KeyCode::Char(c) if rename => {
                                app.route_form.as_mut().unwrap().value.push(c);
                            }
                            KeyCode::Up if !rename => {
                                let f = app.route_form.as_mut().unwrap();
                                if f.cursor > 0 {
                                    f.cursor -= 1;
                                    f.value = f.choices[f.cursor].clone();
                                }
                            }
                            KeyCode::Down if !rename => {
                                let f = app.route_form.as_mut().unwrap();
                                if f.cursor + 1 < f.choices.len() {
                                    f.cursor += 1;
                                    f.value = f.choices[f.cursor].clone();
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // NPU 컴파일 옵션 편집 폼 오버레이
                    if app.compile_form.is_some() {
                        let editing = app.compile_form.as_ref().unwrap().editing;
                        if editing {
                            // 자유 입력(커스텀 값) 모드: Enter/Esc 확정, 문자 입력.
                            match k.code {
                                KeyCode::Enter | KeyCode::Esc => app.compile_form.as_mut().unwrap().editing = false,
                                KeyCode::Backspace => app.compile_form.as_mut().unwrap().backspace(),
                                KeyCode::Char(c) => app.compile_form.as_mut().unwrap().type_char(c),
                                _ => {}
                            }
                        } else {
                            match k.code {
                                KeyCode::Esc | KeyCode::Char('q') => app.compile_form = None,
                                KeyCode::Up => app.compile_form.as_mut().unwrap().move_cursor(-1),
                                KeyCode::Down => app.compile_form.as_mut().unwrap().move_cursor(1),
                                KeyCode::Left => app.compile_form.as_mut().unwrap().cycle(-1),
                                KeyCode::Right => app.compile_form.as_mut().unwrap().cycle(1),
                                KeyCode::Char('e') => app.compile_form.as_mut().unwrap().editing = true,
                                KeyCode::Backspace => app.compile_form.as_mut().unwrap().backspace(),
                                KeyCode::Char(c) if c.is_ascii_digit() => app.compile_form.as_mut().unwrap().type_digit(c),
                                KeyCode::Enter => app.compile_form_submit(),
                                _ => {}
                            }
                        }
                        continue;
                    }
                    // 배포(서빙) 옵션 편집 폼 오버레이
                    if app.deploy_form.is_some() {
                        let editing = app.deploy_form.as_ref().unwrap().editing;
                        if editing {
                            match k.code {
                                KeyCode::Enter | KeyCode::Esc => app.deploy_form.as_mut().unwrap().editing = false,
                                KeyCode::Backspace => app.deploy_form.as_mut().unwrap().backspace(),
                                KeyCode::Char(c) => app.deploy_form.as_mut().unwrap().type_char(c),
                                _ => {}
                            }
                        } else {
                            match k.code {
                                KeyCode::Esc | KeyCode::Char('q') => app.deploy_form = None,
                                KeyCode::Up => app.deploy_form.as_mut().unwrap().move_cursor(-1),
                                KeyCode::Down => app.deploy_form.as_mut().unwrap().move_cursor(1),
                                KeyCode::Left => app.deploy_form.as_mut().unwrap().cycle(-1),
                                KeyCode::Right => app.deploy_form.as_mut().unwrap().cycle(1),
                                KeyCode::Char('e') => app.deploy_form.as_mut().unwrap().editing = true,
                                KeyCode::Backspace => app.deploy_form.as_mut().unwrap().backspace(),
                                KeyCode::Char(c) if c.is_ascii_digit() => app.deploy_form.as_mut().unwrap().type_digit(c),
                                KeyCode::Enter => app.deploy_form_submit(),
                                _ => {}
                            }
                        }
                        continue;
                    }
                    // 미리보기 오버레이 — 생성 매니페스트(v 검증·a 적용·w 저장) 또는 읽기전용 YAML(w 저장만)
                    if app.preview.is_some() {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => app.preview = None,
                            KeyCode::Up | KeyCode::Char('k') => app.preview_scroll = app.preview_scroll.saturating_sub(1),
                            KeyCode::Down | KeyCode::Char('j') => app.preview_scroll = app.preview_scroll.saturating_add(3),
                            // 파일로 저장(어느 preview 든) — 편집 후 kubectl apply 하거나 보관.
                            KeyCode::Char('w') => {
                                if let Some((title, yaml)) = app.preview.clone() {
                                    match save_manifest(&title, &yaml) {
                                        Ok(p) => app.notify(format!("saved → {}", p)),
                                        Err(e) => app.notify(format!("save failed: {}", e)),
                                    }
                                }
                            }
                            // 서버 dry-run 검증(무변경) — 생성 매니페스트에만.
                            KeyCode::Char('v') if app.preview_apply => {
                                if let Some((_, yaml)) = app.preview.clone() {
                                    // placeholder 여부 — dry-run 은 이미지 형식을 검증하지 않아 valid 로 통과함을 명시.
                                    let ph = if yaml.contains("TODO-") { " · ⚠ still has TODO- placeholders (apply blocked)" } else { "" };
                                    match kube::apply_manifest(&ns, &yaml, true) {
                                        Ok(o) => app.notify(format!("valid ✓ {}{}", o.lines().next().unwrap_or("ok"), ph)),
                                        Err(e) => app.notify(format!("invalid: {}", e.to_string().lines().next().unwrap_or(""))),
                                    }
                                }
                            }
                            // 실제 적용(admin+, 확인, 생성 매니페스트만). TODO placeholder 있으면 거부.
                            KeyCode::Char('a') if app.preview_apply && !app.can(Mode::Admin) => {
                                app.notify(format!("apply needs --mode admin+ (current: {})", app.mode.name()));
                            }
                            KeyCode::Char('a') if app.preview_apply => {
                                if let Some((title, yaml)) = app.preview.clone() {
                                    // placeholder(TODO-...) 필드가 남아 있으면 거부. 이미지 env 로 채우면 통과.
                                    if yaml.contains("TODO-") {
                                        app.notify("apply 불가: placeholder(TODO-) 남음 — LMD_*_IMAGE 로 이미지 지정".to_string());
                                    } else {
                                        app.confirm = Some(Pending::Apply { title, yaml });
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // 로그 오버레이 모드
                    if app.logs_mode {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => app.logs_mode = false,
                            KeyCode::Up | KeyCode::Char('k') => app.logs_scroll = app.logs_scroll.saturating_sub(1),
                            KeyCode::Down | KeyCode::Char('j') => app.logs_scroll = app.logs_scroll.saturating_add(3),
                            KeyCode::Char('r') => {
                                if let Ok(l) = kube::logs(&ns, &app.logs_target, 400) {
                                    app.logs = l;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // 안전장치: 오버레이가 열려 있으면 배경(단일키)으로 키가 새지 않게.
                    // 위 오버레이 블록들이 각각 continue 하므로 평시엔 도달 불가지만,
                    // 새 오버레이가 입력 블록을 빠뜨려도 Overlay 단일 출처가 누수를 막는다.
                    if ui::Overlay::top(&app).is_some() {
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('q') => break, // 종료는 q 만
                        KeyCode::Esc => {
                            // 뒤로가기만: 상세→브레드크럼→필터→줌 순 (종료 안 함)
                            if app.detail {
                                app.detail = false;
                            } else if app.nav_back() {
                                // 크로스레이어 드릴 되짚기
                            } else if !app.filter.is_empty() {
                                app.clear_filter();
                            } else if app.zoom {
                                app.zoom = false;
                            }
                        }
                        // 크로스레이어 드릴 pivot — 선택 엔티티에서 관련 레이어로 점프
                        KeyCode::Char(c @ ('p' | 'i' | 'r' | 'e' | 'm' | 'n')) => app.pivot(c),
                        // EPP scorer 가중치 what-if(로컬 시뮬) — EPP 뷰에서만 반응
                        KeyCode::Char('+') | KeyCode::Char('=') => app.epp_adjust(1.0),
                        KeyCode::Char('-') | KeyCode::Char('_') => app.epp_adjust(-1.0),
                        // 세션 에너지 리셋(all-smi 식)
                        KeyCode::Char('R') => app.reset_energy(),
                        KeyCode::Char('?') => app.toggle_help(),
                        KeyCode::Char('A') | KeyCode::Char('a') => app.toggle_alerts(),
                        KeyCode::Char('t') => app.cycle_theme(),
                        KeyCode::Char('f') => {
                            let on = fx.toggle();
                            app.notify(format!("animations {}", if on { "on" } else { "off" }));
                        }
                        KeyCode::Char('z') => app.zoom = !app.zoom,
                        KeyCode::Char(' ') => app.paused = !app.paused,
                        KeyCode::Char('g') => {
                            let base = cfg.grafana.clone();
                            // best-effort 브라우저 오픈 — stdio를 null로(터미널 화면 깨짐 방지)
                            let _ = std::process::Command::new("xdg-open")
                                .arg(&base)
                                .stdin(Stdio::null())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn();
                            app.notify(format!("Grafana: {}  (open in browser · llm-models dashboard)", base));
                            terminal.clear().ok(); // 혹시 모를 잔상 제거 → 전체 재그리기
                        }
                        KeyCode::Char('/') => app.start_filter(),
                        KeyCode::Char(':') => app.open_palette(), // 커맨드 팔레트(뷰/표시 액션 퍼지 검색)
                        KeyCode::Enter => {
                            if app.view == View::Perf {
                                // 선택 모델 지연 분포 온디맨드 조회 → 히스토그램 드릴
                                if let Some(model) = app.selected_perf_model() {
                                    let d = rt.block_on(collect::perf_detail(&prom, &model));
                                    app.perf_detail = Some(d);
                                    app.detail = true;
                                }
                            } else if app.view == View::Routing {
                                app.open_action_menu();
                                if app.action_menu.is_none() {
                                    app.drill_route();
                                }
                            } else if matches!(app.view, View::Launch | View::Models | View::Overview | View::Pods) {
                                // Enter = 컨텍스트 액션 메뉴(변형/노드/카탈로그/모델/파드). 없으면 상세로 폴백.
                                app.open_action_menu();
                                if app.action_menu.is_none() {
                                    app.toggle_detail();
                                }
                            } else {
                                app.toggle_detail();
                            }
                        }
                        KeyCode::Char('o') => app.cycle_sort(),
                        KeyCode::Char('O') => app.toggle_sort_dir(), // 정렬 방향 토글(오름/내림)
                        KeyCode::Tab => app.next_tab(),
                        KeyCode::BackTab => app.prev_tab(), // Shift+Tab → 이전 뷰
                        KeyCode::Char('w') => app.cycle_panel(), // 멀티패널 뷰에서 활성 패널 순환
                        KeyCode::Char(c @ '0'..='9') => {
                            app.set_view_idx(c as usize - '0' as usize)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if app.detail && app.view == View::Nodes {
                                app.dev_cursor(-1) // Node 상세: device 커서(0=요약)
                            } else if app.detail {
                                app.scroll_detail(-1)
                            } else {
                                app.move_sel(-1)
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.detail && app.view == View::Nodes {
                                app.dev_cursor(1)
                            } else if app.detail {
                                app.scroll_detail(1)
                            } else {
                                app.move_sel(1)
                            }
                        }
                        KeyCode::Left => app.move_sel(-1), // 이전 항목
                        KeyCode::Right => app.move_sel(1),  // 다음 항목
                        KeyCode::Char('l') if !app.can(Mode::Debug) => {
                            app.notify(format!("logs needs --mode debug+ (current: {})", app.mode.name()));
                        }
                        KeyCode::Char('l') => {
                            // 선택 pod/모델의 로그 오버레이
                            if let Some(pod) = app.logs_target_pod() {
                                match kube::logs(&ns, &pod, 400) {
                                    Ok(l) => {
                                        app.logs_scroll = l.len().saturating_sub(30) as u16;
                                        app.logs = l;
                                        app.logs_target = pod;
                                        app.logs_mode = true;
                                    }
                                    Err(e) => app.notify(format!("logs: {}", e)),
                                }
                            } else {
                                app.notify("logs: Pods/Models 뷰에서 pod/model 선택".to_string());
                            }
                        }
                        KeyCode::Char('s') if !app.can(Mode::Admin) => {
                            app.notify(format!("scale needs --mode admin+ (current: {})", app.mode.name()));
                        }
                        KeyCode::Char('s') => {
                            // Admin+ : 즉시 실행하지 않고 확인(y/n) 대기로 — dry-run→confirm.
                            if let Some(m) = app.selected_model() {
                                let (name, target) =
                                    (m.name.clone(), if m.desired == 0 { 1 } else { 0 });
                                app.confirm = Some(Pending::Scale { name, target });
                            } else {
                                app.notify("scale: select a model in Models/Overview".to_string());
                            }
                        }
                        // 롤아웃 재시작(admin+, 확인)
                        KeyCode::Char('S') if !app.can(Mode::Admin) => {
                            app.notify(format!("restart needs --mode admin+ (current: {})", app.mode.name()));
                        }
                        KeyCode::Char('S') => {
                            if let Some(m) = app.selected_model() {
                                app.confirm = Some(Pending::Restart { name: m.name.clone() });
                            } else {
                                app.notify("restart: select a model in Models/Overview".to_string());
                            }
                        }
                        // 서빙 중지(admin+, 확인) — replicas 0 으로(되돌릴 수 있음).
                        KeyCode::Char('x') if !app.can(Mode::Admin) => {
                            app.notify(format!("stop needs --mode admin+ (current: {})", app.mode.name()));
                        }
                        KeyCode::Char('x') => {
                            if let Some(m) = app.selected_model() {
                                if m.desired == 0 {
                                    app.notify(format!("{} 는 이미 중지됨(0 replica)", m.name));
                                } else {
                                    app.confirm = Some(Pending::Stop { name: m.name.clone() });
                                }
                            } else {
                                app.notify("stop: Models/Overview 에서 모델 선택".to_string());
                            }
                        }
                        // 선택 리소스의 live YAML 보기(읽기전용 preview) — k9s y.
                        KeyCode::Char('y') => {
                            if let Some((kind, nsd, name)) = app.yaml_target() {
                                let nsopt = if nsd { Some(ns.as_str()) } else { None };
                                match kube::resource_yaml(kind, nsopt, &name) {
                                    Ok(y) => {
                                        app.preview = Some((format!("{} {} · yaml (read-only)", kind, name), y));
                                        app.preview_scroll = 0;
                                        app.preview_apply = false;
                                    }
                                    Err(e) => app.notify(format!("yaml: {}", e.to_string().lines().next().unwrap_or(""))),
                                }
                            } else {
                                app.notify("yaml: Models/Pods/Nodes/Deploy 에서 리소스 선택".to_string());
                            }
                        }
                        // Deploy: compile(NPU)/deploy 매니페스트 미리보기(dry-run, admin+).
                        KeyCode::Char('c') | KeyCode::Char('d') if app.view == View::Launch => {
                            if !app.can(Mode::Admin) {
                                app.notify(format!("compile/deploy needs --mode admin+ (current: {})", app.mode.name()));
                            } else if app.selected_artifact().is_none() {
                                app.notify("select a model build in Deploy (variants panel)".to_string());
                            } else if k.code == KeyCode::Char('c') {
                                app.compile_preview();
                            } else {
                                app.open_deploy_form();
                            }
                        }
                        _ => {
                            app.toast = None;
                        }
                    }
                }
            }
        }
        Ok(())
    })();

    // teardown (에러여도 복원)
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).ok();
    terminal.show_cursor().ok();
    result
}

fn print_snapshot(s: &collect::Snapshot, cfg: &Config) {
    println!("== lmd-top snapshot (prom={}, ns={}) ==", cfg.prom, cfg.ns);
    println!(
        "gateway: {} {}",
        if s.gw_addr.is_empty() { "—" } else { &s.gw_addr },
        if s.gw_ok { "Programmed" } else { "" }
    );
    println!("\n[nodes] {}", s.nodes.len());
    for n in &s.nodes {
        println!(
            "  {:<24} load1 {:>5.2}  mem {:.0}/{:.0}G",
            n.name, n.load1, n.mem_used_gb, n.mem_total_gb
        );
    }
    println!("\n[accelerators] {}", s.accel.len());
    for a in &s.accel {
        println!(
            "  {:<5} {:<6} {:<16} util {:>5.1}%  mem {:.0}/{:.0}G  {:.0}°C {:.0}W  {}",
            a.disp(),
            a.id,
            a.node,
            a.util,
            a.mem_used_gb,
            a.mem_total_gb,
            a.temp,
            a.power,
            a.busy_model
        );
    }
    println!("\n[inference pools] {}", s.pools.len());
    for p in &s.pools {
        println!(
            "  {:<16} ready {:.0}  queue {:.1}  kv {:.2}  sat {:.2}",
            p.name, p.ready, p.queue, p.kv, p.sat
        );
    }
    if let Some(e) = &s.epp {
        println!(
            "\n[EPP] profile={} picker={}\n  scorers: {}",
            e.profile,
            e.picker,
            e.scorers
                .iter()
                .map(|(n, w)| format!("{}·w{:.0}", n, w))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("\n[models] {}", s.models.len());
    for m in &s.models {
        println!(
            "  {:<24} {}/{}  run={} wait={} tps={}  path={}  {}",
            m.name,
            m.ready,
            m.desired,
            m.running.map(|x| format!("{:.0}", x)).unwrap_or("-".into()),
            m.waiting.map(|x| format!("{:.0}", x)).unwrap_or("-".into()),
            m.tps.map(|x| format!("{:.0}", x)).unwrap_or("-".into()),
            if m.route.is_empty() { "-" } else { &m.route },
            m.status
        );
    }
    println!("\n[routes] {} (epp_in_path={})", s.routes.len(), s.epp_in_path);
    for r in &s.routes {
        println!("  {:<10} → {}/{}", r.path, r.kind, r.backend);
    }
    if !s.objectives.is_empty() {
        println!("[SLO] {}", s.objectives.iter().map(|o| format!("{}(p{})", o.name, o.priority)).collect::<Vec<_>>().join(", "));
    }
    println!("\n[pods] {}", s.pods.len());
    for p in &s.pods {
        println!("  {:<40} {:<8} {} {} restarts={}", p.name, p.phase, p.ready, p.node, p.restarts);
    }
    if !s.warnings.is_empty() {
        println!("\n[warnings] {}", s.warnings.len());
        for w in &s.warnings {
            println!("  ! {}", w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_outcome, check_args, ArgCheck, MutationOutcome, OkInfo};
    use super::{App, Mode};

    fn args(v: &[&str]) -> Vec<String> {
        std::iter::once("lmd-top").chain(v.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn apply_outcome_success_notifies_and_clears_preview() {
        let _g = crate::audit::TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join("lmd-audit-outcome-ok.log");
        let _ = std::fs::remove_file(&path);
        std::env::set_var("LMD_AUDIT", &path);
        let mut app = App::new();
        app.preview = Some(("stale".into(), "y".into())); // Apply 성공이면 닫혀야
        apply_outcome(
            &mut app,
            MutationOutcome {
                mode: Mode::Admin,
                audit_action: "apply".into(),
                audit_target: "manifest-x".into(),
                fail_label: "apply",
                result: Ok(OkInfo { audit_detail: "created".into(), notify: "applied — created".into(), clear_preview: true }),
            },
        );
        std::env::remove_var("LMD_AUDIT");
        assert!(app.preview.is_none(), "Apply 성공은 preview 를 닫아야");
        assert_eq!(app.toast.as_deref(), Some("applied — created"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\tapply\tmanifest-x\tok\tcreated"), "audit ok line: {}", body);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_outcome_failure_opens_preview_and_audits_fail() {
        let _g = crate::audit::TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join("lmd-audit-outcome-err.log");
        let _ = std::fs::remove_file(&path);
        std::env::set_var("LMD_AUDIT", &path);
        let mut app = App::new();
        apply_outcome(
            &mut app,
            MutationOutcome {
                mode: Mode::Admin,
                audit_action: "scale→2".into(),
                audit_target: "ds4".into(),
                fail_label: "scale",
                result: Err("Error from server: forbidden\ntrace...".into()),
            },
        );
        std::env::remove_var("LMD_AUDIT");
        // 실패 이유는 스크롤 가능한 창으로(사라지는 토스트 아님).
        let (title, detail) = app.preview.as_ref().expect("failure opens preview");
        assert!(title.contains("scale"), "title: {}", title);
        assert!(detail.contains("forbidden"));
        assert!(!app.preview_apply);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\tscale→2\tds4\tFAIL\t"), "audit fail line: {}", body);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn no_args_is_ok() {
        assert_eq!(check_args(&args(&[])), ArgCheck::Ok);
    }

    #[test]
    fn known_flags_ok() {
        assert_eq!(check_args(&args(&["--json"])), ArgCheck::Ok);
        assert_eq!(check_args(&args(&["--snapshot"])), ArgCheck::Ok);
        assert_eq!(check_args(&args(&["-s"])), ArgCheck::Ok);
        assert_eq!(check_args(&args(&["--doctor"])), ArgCheck::Ok);
        assert_eq!(check_args(&args(&["--render"])), ArgCheck::Ok);
        assert_eq!(check_args(&args(&["--cast"])), ArgCheck::Ok);
        assert_eq!(check_args(&args(&["--cast", "out.cast"])), ArgCheck::Ok);
        assert_eq!(check_args(&args(&["--mode", "admin"])), ArgCheck::Ok);
    }

    #[test]
    fn help_flag_detected() {
        assert_eq!(check_args(&args(&["--help"])), ArgCheck::Help);
        assert_eq!(check_args(&args(&["-h"])), ArgCheck::Help);
        // 도움말은 다른 인자보다 우선.
        assert_eq!(check_args(&args(&["--bogus", "--help"])), ArgCheck::Help);
    }

    #[test]
    fn unknown_flag_rejected() {
        assert_eq!(check_args(&args(&["--nope"])), ArgCheck::Unknown("--nope".into()));
        assert_eq!(check_args(&args(&["--jsonx"])), ArgCheck::Unknown("--jsonx".into()));
        assert_eq!(check_args(&args(&["foo"])), ArgCheck::Unknown("foo".into()));
    }

    #[test]
    fn mode_validation() {
        assert_eq!(check_args(&args(&["--mode", "bogus"])), ArgCheck::BadMode("bogus".into()));
        assert_eq!(check_args(&args(&["--mode"])), ArgCheck::MissingMode);
        // --mode 값 뒤의 실제 미지원 플래그도 잡힌다.
        assert_eq!(check_args(&args(&["--mode", "admin", "--x"])), ArgCheck::Unknown("--x".into()));
    }
}
