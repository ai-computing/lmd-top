//! lmd-top — terminal observability tool for llm-d clusters (Phase 1: monitor).
//! Run: `lmd-top`            → TUI
//!      `lmd-top --snapshot` → collect once and print text (for headless verification)

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
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Key handling for the three symmetric option forms (objective / compile / deploy).
/// They share one shape and differ only by the form field and the submit method:
///   editing mode — Enter/Esc commit, Backspace, free text;
///   nav mode     — Esc/q close, ↑↓ move, ←→ cycle choices, e=edit, digit=type, Enter=submit.
macro_rules! handle_edit_form {
    ($app:expr, $form:ident, $submit:ident, $code:expr) => {{
        if $app.$form.as_ref().unwrap().editing {
            match $code {
                KeyCode::Enter | KeyCode::Esc => $app.$form.as_mut().unwrap().editing = false,
                KeyCode::Backspace => $app.$form.as_mut().unwrap().backspace(),
                KeyCode::Char(c) => $app.$form.as_mut().unwrap().type_char(c),
                _ => {}
            }
        } else {
            match $code {
                KeyCode::Esc | KeyCode::Char('q') => $app.$form = None,
                KeyCode::Up => $app.$form.as_mut().unwrap().move_cursor(-1),
                KeyCode::Down => $app.$form.as_mut().unwrap().move_cursor(1),
                KeyCode::Left => $app.$form.as_mut().unwrap().cycle(-1),
                KeyCode::Right => $app.$form.as_mut().unwrap().cycle(1),
                KeyCode::Char('e') => $app.$form.as_mut().unwrap().editing = true,
                KeyCode::Backspace => $app.$form.as_mut().unwrap().backspace(),
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    $app.$form.as_mut().unwrap().type_digit(c)
                }
                KeyCode::Enter => $app.$submit(),
                _ => {}
            }
        }
    }};
}

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
    --plan <OP>      generate a compile/deploy manifest headlessly: compile | deploy
    --model <HF_ID>  model id for --plan, e.g. Qwen/Qwen2.5-0.5B-Instruct
    --vendor <NAME>  target vendor for --plan: rbln | furiosa | gpu
    --set k=v        override a form field for --plan (repeatable)
    --dry-run        server-side validate the generated --plan manifest
    --apply          apply the generated --plan manifest
    --help, -h       show this help and exit

ENVIRONMENT:
    LMD_PROM         Prometheus host:port
    LMD_NS           namespace (default: llm-serving)
    LMD_GRAFANA      Grafana base URL (opened via the `:graf` palette command)
    LMD_THEME        startup theme: soft | default | high-contrast | colorblind
    LMD_AUDIT        audit log path (default: ~/.config/lmd-top/audit.log)
    LMD_W / LMD_H    size for --render

With no options, lmd-top launches the interactive TUI. See `?` in the TUI for keybindings.";

/// Argument validation result — main prints help/errors and branches on this.
#[derive(Debug, PartialEq)]
enum ArgCheck {
    Ok,
    Help,
    Unknown(String),
    BadMode(String),
    MissingMode,
}

/// Scan args[1..] and allow only known flags. Reject unsupported flags/invalid --mode.
fn check_args(args: &[String]) -> ArgCheck {
    // Help takes top priority if present.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return ArgCheck::Help;
    }
    const NOVALUE: &[&str] = &[
        "--doctor",
        "--json",
        "--snapshot",
        "-s",
        "--render",
        "--audit",
        "--dry-run",
        "--apply",
    ];
    const VALUE: &[&str] = &["--plan", "--model", "--vendor", "--set"];
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
        if VALUE.contains(&a) {
            if args.get(i + 1).map(|s| s.starts_with('-')).unwrap_or(true) {
                return ArgCheck::Unknown(args[i].clone());
            }
            i += 2;
            continue;
        }
        if a == "--cast" {
            // Optional output-path value (consumed if not a flag).
            if args
                .get(i + 1)
                .map(|s| !s.starts_with('-'))
                .unwrap_or(false)
            {
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

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone())
}

fn arg_sets(args: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--set" {
            if let Some((k, v)) = args[i + 1].split_once('=') {
                out.push((k.to_string(), v.to_string()));
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

fn plan_vendor(v: &str) -> Result<&'static str> {
    match v.to_lowercase().as_str() {
        "rbln" | "atom" | "rebellions" => Ok("rbln"),
        "furiosa" | "rngd" => Ok("furiosa"),
        "gpu" | "nvidia" => Ok("gpu"),
        _ => anyhow::bail!("unsupported --vendor '{}': expected rbln|furiosa|gpu", v),
    }
}

fn manifest_has_placeholder(yaml: &str) -> bool {
    yaml.lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with('#'))
        .any(|line| line.contains("TODO-"))
}

async fn run_plan(cfg: &Config, args: &[String]) -> Result<()> {
    let op = arg_value(args, "--plan").unwrap_or_default();
    let model = arg_value(args, "--model")
        .ok_or_else(|| anyhow::anyhow!("--plan requires --model <HF_ID>"))?;
    let vendor = plan_vendor(
        &arg_value(args, "--vendor")
            .ok_or_else(|| anyhow::anyhow!("--plan requires --vendor <rbln|furiosa|gpu>"))?,
    )?;
    let overrides = arg_sets(args);
    let snap = collect(cfg).await;
    let mut app = App::new();
    app.ns = cfg.ns.clone();
    app.apply(snap);
    let (title, yaml) = match op.as_str() {
        "compile" => app.plan_compile_for_model(&model, vendor, &overrides),
        "deploy" => app.plan_deploy_for_model(&model, vendor, &overrides),
        _ => Err(format!(
            "unsupported --plan '{}': expected compile|deploy",
            op
        )),
    }
    .map_err(anyhow::Error::msg)?;

    if args.iter().any(|a| a == "--dry-run") {
        let out = kube::apply_manifest(&cfg.ns, &yaml, true)?;
        eprintln!("dry-run ok: {}", out.lines().next().unwrap_or("ok"));
    }
    if args.iter().any(|a| a == "--apply") {
        if manifest_has_placeholder(&yaml) {
            anyhow::bail!("apply blocked: generated manifest still contains TODO- placeholders");
        }
        let out = kube::apply_manifest(&cfg.ns, &yaml, false)?;
        eprintln!("applied {}:\n{}", title, out.trim());
    }
    if !args.iter().any(|a| a == "--apply") {
        println!("{}", yaml);
    }
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let cfg = Config::default();
    let args: Vec<String> = std::env::args().collect();

    // Argument validation — help/unsupported flags/invalid --mode handled here (before entering TUI).
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
            eprintln!(
                "lmd-top: invalid --mode '{}' (expected observe|debug|admin|danger)\n",
                v
            );
            eprintln!("{}", HELP);
            std::process::exit(2);
        }
        ArgCheck::MissingMode => {
            eprintln!("lmd-top: --mode requires a value (observe|debug|admin|danger)\n");
            eprintln!("{}", HELP);
            std::process::exit(2);
        }
    }

    if args.iter().any(|a| a == "--plan") {
        run_plan(&cfg, &args).await?;
        return Ok(());
    }

    // Full metric survey + gap analysis (diagnose why views are empty).
    if args.iter().any(|a| a == "--doctor") {
        doctor::run(&cfg).await;
        return Ok(());
    }

    // View audit log — print the history (file) of mutations applied by lmd-top.
    if args.iter().any(|a| a == "--audit") {
        audit::print_log();
        return Ok(());
    }

    // Machine-readable state (agent) — --json (or --snapshot --json). Collect once, print JSON.
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

    // Generate a demo asciicast (for GIF conversion via agg). --cast [out.cast]
    if let Some(pos) = args.iter().position(|a| a == "--cast") {
        let out = args
            .get(pos + 1)
            .filter(|s| !s.starts_with('-'))
            .cloned()
            .unwrap_or_else(|| "docs/demo.cast".to_string());
        cast::run(&cfg, &out).await;
        return Ok(());
    }

    // Permission mode (at startup) — --mode observe|debug|admin|danger (default observe)
    let mode = args
        .iter()
        .position(|a| a == "--mode")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| Mode::parse(s))
        .unwrap_or(Mode::Observe);

    run_tui(cfg, mode).await
}

/// Mutation execution result — kube is called on a worker thread, then this is passed to the main thread for audit/UI.
/// Moving kube calls off the UI thread keeps rendering from freezing (removes up-to-8s freezes).
struct MutationOutcome {
    mode: Mode,
    audit_action: String,           // audit log action field
    audit_target: String,           // audit log target field
    fail_label: &'static str,       // window title on failure (show_fail)
    result: Result<OkInfo, String>, // Ok=success display info / Err=failure reason (1+ lines)
}
/// Display info on success — audit detail, toast text, whether to close preview (Apply success).
struct OkInfo {
    audit_detail: String,
    notify: String,
    clear_preview: bool,
}

/// Execute a confirmed mutation via actual kubectl and return a MutationOutcome. (for worker threads)
/// Every branch calls kube, so unit testing needs a real cluster — result handling lives in apply_outcome (testable).
fn run_mutation(pending: Pending, ns: &str, mode: Mode) -> MutationOutcome {
    let mk = |audit_action: String,
              audit_target: String,
              fail_label: &'static str,
              result: Result<OkInfo, String>| MutationOutcome {
        mode,
        audit_action,
        audit_target,
        fail_label,
        result,
    };
    match pending {
        Pending::Scale { name, target } => {
            let r = kube::scale_deploy(ns, &name, target)
                .map(|_| OkInfo {
                    audit_detail: "scaled".into(),
                    notify: format!("scaled {} → {}", name, target),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk(format!("scale→{}", target), name, "scale", r)
        }
        Pending::Restart { name } => {
            let r = kube::rollout_restart(ns, &name)
                .map(|_| OkInfo {
                    audit_detail: "restarted".into(),
                    notify: format!("rollout restart {}", name),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk("rollout-restart".into(), name, "restart", r)
        }
        Pending::Stop { name } => {
            let r = kube::scale_deploy(ns, &name, 0)
                .map(|_| OkInfo {
                    audit_detail: "stopped".into(),
                    notify: format!("stopped {} (scaled → 0)", name),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk("stop(scale→0)".into(), name, "stop", r)
        }
        Pending::Cordon { node, on } => {
            let act = if on { "cordon" } else { "uncordon" };
            let r = kube::cordon(&node, on)
                .map(|_| OkInfo {
                    audit_detail: "ok".into(),
                    notify: format!("{} {}", if on { "cordoned" } else { "uncordoned" }, node),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk(act.into(), node, "cordon", r)
        }
        Pending::DeletePod { name } => {
            let r = kube::delete_pod(ns, &name)
                .map(|_| OkInfo {
                    audit_detail: "deleted".into(),
                    notify: format!("deleted pod {}", name),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk("delete-pod".into(), name, "delete", r)
        }
        Pending::DeleteJob { name } => {
            let r = kube::delete_job(ns, &name)
                .map(|_| OkInfo {
                    audit_detail: "deleted".into(),
                    notify: format!("deleted job {}", name),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk("delete-job".into(), name, "delete job", r)
        }
        Pending::RouteRename { route, old, new } => {
            let r = kube::route_set_path(ns, &route, &old, &new)
                .map(|_| OkInfo {
                    audit_detail: "renamed".into(),
                    notify: format!("renamed route {} → {}", old, new),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk(
                format!("route-rename→{}", new),
                format!("{}:{}", route, old),
                "route rename",
                r,
            )
        }
        Pending::RouteRetarget {
            route,
            path,
            backend,
            kind,
        } => {
            let r = kube::route_retarget(ns, &route, &path, &backend, &kind)
                .map(|_| OkInfo {
                    audit_detail: "retargeted".into(),
                    notify: format!("retargeted {} → {}", path, backend),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk(
                format!("route-retarget→{}:{}", kind, backend),
                format!("{}:{}", route, path),
                "route retarget",
                r,
            )
        }
        Pending::RouteDelete { route, path } => {
            let r = kube::route_delete_rule(ns, &route, &path)
                .map(|_| OkInfo {
                    audit_detail: "deleted".into(),
                    notify: format!("deleted route {}", path),
                    clear_preview: false,
                })
                .map_err(|e| e.to_string());
            mk(
                "route-delete".into(),
                format!("{}:{}", route, path),
                "route delete",
                r,
            )
        }
        Pending::Apply { yaml, title } => {
            let r = kube::apply_manifest(ns, &yaml, false)
                .map(|o| {
                    let line = o.lines().next().unwrap_or("ok").to_string();
                    OkInfo {
                        audit_detail: line.clone(),
                        notify: format!("applied — {}", line),
                        clear_preview: true,
                    }
                })
                .map_err(|e| e.to_string());
            mk("apply".into(), title, "apply", r)
        }
        Pending::ApplyUrl { url, title } => {
            let r = kube::apply_url(&url)
                .map(|o| {
                    let n = o.lines().count();
                    OkInfo {
                        audit_detail: format!("{} ({} objects)", url, n),
                        notify: format!("applied {} — {} object(s)", title, n),
                        clear_preview: true,
                    }
                })
                .map_err(|e| e.to_string());
            mk("apply-url".into(), title, "apply", r)
        }
    }
}

/// Apply a mutation result to the audit log + UI (toast/failure preview). (main thread, unit-testable)
fn apply_outcome(app: &mut App, o: MutationOutcome) {
    match o.result {
        Ok(ok) => {
            audit::record(
                o.mode,
                &o.audit_action,
                &o.audit_target,
                Ok(&ok.audit_detail),
            );
            if ok.clear_preview {
                app.preview = None;
            }
            app.notify(ok.notify);
        }
        Err(e) => {
            audit::record(o.mode, &o.audit_action, &o.audit_target, Err(&e));
            // Failure reason goes to a scrollable window, not a vanishing toast.
            app.notify(format!("{} failed — see window for details", o.fail_label));
            app.preview = Some((
                format!("⚠ {} failed — reason (q to close)", o.fail_label),
                e,
            ));
            app.preview_scroll = 0;
            app.preview_apply = false;
        }
    }
}

/// Run a command palette selection — navigation/display actions only (no cluster changes, no permissions needed).
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
        ResetEnergy => app.reset_energy(),
    }
}

/// Run the action chosen in the action menu — includes permission gating. Forms/confirms as overlays, logs immediately.
fn require_action(app: &mut App, action: app::Action) -> bool {
    let required = action.required_mode();
    if app.can(required) {
        true
    } else {
        app.notify(format!(
            "{} needs --mode {}+ (current: {})",
            action.verb(),
            required.name(),
            app.mode.name()
        ));
        false
    }
}

fn dispatch_action(
    app: &mut App,
    action: app::Action,
    subject: &str,
    ns: &str,
    prom: &str,
    rt: &tokio::runtime::Handle,
) {
    use app::{Action, Pending, View};
    match action {
        Action::Info => {
            if app.view == View::Routing {
                app.drill_route();
            } else {
                // Library(통합 트리)·Serving·Models 등 — 선택 항목 상세 패널.
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
        Action::Prefetch => {
            if require_action(app, action) {
                app.open_prefetch_form();
            }
        }
        Action::Stop => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Stop {
                    name: subject.to_string(),
                });
            }
        }
        Action::Scale => {
            if require_action(app, action) {
                let target = app
                    .selected_model()
                    .map(|m| if m.desired == 0 { 1 } else { 0 })
                    .unwrap_or(1);
                app.confirm = Some(Pending::Scale {
                    name: subject.to_string(),
                    target,
                });
            }
        }
        Action::Restart => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Restart {
                    name: subject.to_string(),
                });
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
                    Err(e) => app.notify(format!(
                        "yaml: {}",
                        e.to_string().lines().next().unwrap_or("")
                    )),
                }
            }
        }
        Action::Delete => {
            if require_action(app, action) {
                app.confirm = Some(Pending::DeletePod {
                    name: subject.to_string(),
                });
            }
        }
        Action::DeleteJob => {
            if require_action(app, action) {
                app.confirm = Some(Pending::DeleteJob {
                    name: subject.to_string(),
                });
            }
        }
        Action::Pivot(c) => app.pivot(c), // cross-layer jump — reads current selection, pushes breadcrumb
        Action::Objective => app.open_objective_form(), // set objectives (observe-only, no permissions needed)
        Action::Cordon | Action::Uncordon => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Cordon {
                    node: subject.to_string(),
                    on: matches!(action, Action::Cordon),
                });
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
                    app.confirm = Some(Pending::RouteDelete {
                        route: r.route,
                        path: r.path,
                    });
                }
            }
        }
        Action::Logs => {
            if !require_action(app, action) {
                // already notified
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
    let _ = (prom, rt); // currently unused (reserved for future on-demand queries)
}

fn open_actions_or_detail(app: &mut App, detail_fallback: bool) {
    if app.view == View::Routing && app.panel_focus == 0 && app.snap.routes.is_empty() {
        app.notify("actions: no HTTPRoute discovered in this namespace".to_string());
        return;
    }
    app.open_action_menu();
    if app.action_menu.is_none() && detail_fallback {
        app.toggle_detail();
    } else if app.action_menu.is_none() {
        app.notify("actions: no actions for this selection".to_string());
    }
}

/// Save preview content to a file — for editing then kubectl apply, or archiving. Filename inferred from title.
fn save_manifest(title: &str, yaml: &str) -> Result<String> {
    // First tokens of the title → safe filename.
    let base: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() {
        "manifest".to_string()
    } else {
        base
    };
    let dir = std::env::var("LMD_SAVE_DIR").unwrap_or_else(|_| ".".to_string());
    let path = format!(
        "{}/lmd-{}.yaml",
        dir.trim_end_matches('/'),
        &base[..base.len().min(48)]
    );
    std::fs::write(&path, yaml)?;
    Ok(path)
}

/// Edit the manifest in $EDITOR (default vi) — briefly suspend the TUI, run the editor, then restore.
/// Returns the edited content on save and clean exit, otherwise None.
fn edit_in_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    content: &str,
) -> Option<String> {
    let path = std::env::temp_dir().join("lmd-top-manifest.yaml");
    if std::fs::write(&path, content).is_err() {
        return None;
    }
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".into());
    // Suspend TUI (leave alt-screen, disable raw) → editor → restore.
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

/// Headless render verification — draw one frame of each view via TestBackend, print as text.
fn render_dump(snap: collect::Snapshot) {
    use app::View;
    use ratatui::backend::TestBackend;
    let mut a = App::new();
    a.apply(snap);
    let rw: u16 = std::env::var("LMD_W")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let rh: u16 = std::env::var("LMD_H")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(26);
    let mut fx = ui::FxState::disabled(); // text dump — effects off (avoids partial frames)
                                          // Render coverage for every view (all sections and their sub-tabs).
    for v in View::EVERY {
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
    let shared = Arc::new(Mutex::new(collect(&cfg).await)); // first collection (display immediately)

    // full collection loop (3s) — heavy items: models/EPP/perf/events etc.
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
    // fast tier (1s) — refresh only accelerator util/mem/temp + nodes quickly (responsiveness)
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
                    // Don't overwrite existing tables with empty results — keeps a momentary Prometheus outage from reading as "no accelerators".
                    if !accel.is_empty() || g.accel.is_empty() {
                        g.accel = accel;
                    }
                    if !nodes.is_empty() || g.nodes.is_empty() {
                        // fast tier 는 node.npu(드라이버, 노드 라벨 기반)를 full tier 에서만 채운다.
                        // 새 노드에 npu 가 비었으면 직전 값을 이어받아 "미싱↔존재" 깜빡임을 막는다.
                        let mut nodes = nodes;
                        for n in &mut nodes {
                            if n.npu.is_empty() {
                                if let Some(prev) = g.nodes.iter().find(|o| o.name == n.name) {
                                    n.npu = prev.npu.clone();
                                }
                            }
                        }
                        g.nodes = nodes;
                    }
                    g.ts = ts;
                }
            }
        });
    }

    // UI loop (blocking)
    let rt = tokio::runtime::Handle::current(); // for on-demand Perf drill queries
    let res = tokio::task::spawn_blocking(move || ui_loop(shared, cfg, mode, rt)).await?;
    res
}

fn ui_loop(
    shared: Arc<Mutex<collect::Snapshot>>,
    cfg: Config,
    mode: Mode,
    rt: tokio::runtime::Handle,
) -> Result<()> {
    let ns = cfg.ns.clone();
    let prom = cfg.prom.clone();
    // Restore terminal even on panic (disable raw mode/alt-screen) — otherwise the shell is left broken.
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
    app.ns = ns.clone(); // manifest/action namespace = cfg.ns (LMD_NS)
    app::set_theme(cfg.theme); // startup theme (LMD_THEME / yaml)
    let mut fx = ui::FxState::new();
    // Mutation result channel — worker thread sends the result after running kube; the main loop drains and applies it.
    let (mut_tx, mut_rx) = std::sync::mpsc::channel::<MutationOutcome>();
    let result = (|| -> Result<()> {
        loop {
            if !app.paused {
                let snap = shared.lock().map(|g| g.clone()).unwrap_or_default();
                app.apply(snap);
            }
            // Apply completed mutations (audit + toast/failure window). Clear in-flight.
            while let Ok(outcome) = mut_rx.try_recv() {
                app.inflight = None;
                apply_outcome(&mut app, outcome);
            }
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|f| ui::draw(f, &app, &mut fx))?;

            // Poll briefly while animating (smooth frames), otherwise 100ms.
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
                    // Filter input mode: capture text
                    if app.filtering {
                        match k.code {
                            KeyCode::Esc | KeyCode::Enter => app.stop_filter(),
                            KeyCode::Backspace => app.filter_pop(),
                            KeyCode::Char(c) => app.filter_push(c),
                            _ => {}
                        }
                        continue;
                    }
                    // Command palette (`:`) — text capture + fuzzy selection. Enter runs, Esc closes.
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
                    // Help overlay: any key closes it
                    if app.help {
                        app.help = false;
                        continue;
                    }
                    // Quit confirmation: opened by q in the background. Keep it separate from mutation confirms.
                    if app.exit_confirm {
                        match k.code {
                            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => break,
                            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                                app.exit_confirm = false;
                                app.notify("quit cancelled".to_string());
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // Mutation confirm (y/n) — other keys ignored. Execution happens here (permission check at trigger time).
                    if let Some(pending) = app.confirm.clone() {
                        // Only for Apply confirms: e=edit in vi, v=server dry-run validation (YAML reachable only via specific keys).
                        if let Pending::Apply { yaml, title } = &pending {
                            match k.code {
                                KeyCode::Char('e') | KeyCode::Char('E') => {
                                    if let Some(edited) = edit_in_editor(&mut terminal, yaml) {
                                        app.confirm = Some(Pending::Apply {
                                            title: title.clone(),
                                            yaml: edited,
                                        });
                                        app.notify(
                                            "manifest edited (vi) — Enter to apply".to_string(),
                                        );
                                    }
                                    continue;
                                }
                                KeyCode::Char('v') | KeyCode::Char('V') => {
                                    match kube::apply_manifest(&ns, yaml, true) {
                                        Ok(o) => app.notify(format!(
                                            "valid ✓ {}",
                                            o.lines().next().unwrap_or("ok")
                                        )),
                                        Err(e) => app.notify(format!(
                                            "invalid: {}",
                                            e.to_string().lines().next().unwrap_or("")
                                        )),
                                    }
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        // ←→/h/l toggles Yes/No. Enter follows the highlighted choice; default is No.
                        let execute = match k.code {
                            KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Char('h')
                            | KeyCode::Char('l')
                            | KeyCode::Tab => {
                                app.confirm_yes = !app.confirm_yes;
                                continue;
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => true,
                            KeyCode::Enter => app.confirm_yes,
                            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => false,
                            _ => continue, // ignore other keys (popup stays)
                        };
                        app.confirm = None;
                        app.confirm_yes = false; // next popup defaults to No
                        if !execute {
                            app.notify("cancelled".to_string());
                            continue;
                        }
                        // Mutations run on a worker thread — so kube (up to 8s) doesn't freeze the UI.
                        // Result arrives via mut_rx; the main loop runs apply_outcome (audit + toast/failure window).
                        if app.inflight.is_some() {
                            app.notify(
                                "mutation in progress — retry after it completes".to_string(),
                            );
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
                    // Alert history overlay: esc/q/A closes it.
                    if app.alerts_panel {
                        match k.code {
                            KeyCode::Esc
                            | KeyCode::Char('q')
                            | KeyCode::Char('A')
                            | KeyCode::Char('a') => app.alerts_panel = false,
                            _ => {}
                        }
                        continue;
                    }
                    // Action menu — ↑↓/jk select, Enter/key runs, q/Esc closes.
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
                    // Serving objective edit form overlay.
                    if app.objective_form.is_some() {
                        handle_edit_form!(app, objective_form, objective_form_submit, k.code);
                        continue;
                    }
                    // Route edit form (rename text / retarget selection).
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
                                        app.notify("route: no changes".to_string());
                                    } else {
                                        app.confirm = Some(Pending::RouteRename {
                                            route: form.route,
                                            old: form.path,
                                            new,
                                        });
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
                    // NPU compile options form overlay.
                    if app.compile_form.is_some() {
                        handle_edit_form!(app, compile_form, compile_form_submit, k.code);
                        continue;
                    }
                    if app.prefetch_form.is_some() {
                        handle_edit_form!(app, prefetch_form, prefetch_form_submit, k.code);
                        continue;
                    }
                    // Placement picker (deploy 폼의 place 필드에서 드릴) — 후보 노드 상태 목록.
                    if app.place_picker.is_some() {
                        match k.code {
                            KeyCode::Up | KeyCode::Char('k') => app.place_pick_move(-1),
                            KeyCode::Down | KeyCode::Char('j') => app.place_pick_move(1),
                            KeyCode::Enter => app.place_pick_apply(),
                            KeyCode::Esc | KeyCode::Char('q') => app.place_picker = None,
                            _ => {}
                        }
                        continue;
                    }
                    // Deploy/serving options form overlay.
                    if app.deploy_form.is_some() {
                        // 옵션을 다 고른 뒤 Enter → placement 선택 화면(다음 단계) → 거기서 매니페스트.
                        let editing = app.deploy_form.as_ref().unwrap().editing;
                        if !editing && matches!(k.code, KeyCode::Enter) {
                            app.open_place_picker();
                            continue;
                        }
                        handle_edit_form!(app, deploy_form, deploy_form_submit, k.code);
                        continue;
                    }
                    // Preview overlay — generated manifests support validate/apply/save, read-only YAML supports save.
                    if app.preview.is_some() {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => app.preview = None,
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.preview_scroll = app.preview_scroll.saturating_sub(1)
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.preview_scroll = app.preview_scroll.saturating_add(3)
                            }
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
                                    let ph = if manifest_has_placeholder(&yaml) {
                                        " · ⚠ still has TODO- placeholders (apply blocked)"
                                    } else {
                                        ""
                                    };
                                    match kube::apply_manifest(&ns, &yaml, true) {
                                        Ok(o) => app.notify(format!(
                                            "valid ✓ {}{}",
                                            o.lines().next().unwrap_or("ok"),
                                            ph
                                        )),
                                        Err(e) => app.notify(format!(
                                            "invalid: {}",
                                            e.to_string().lines().next().unwrap_or("")
                                        )),
                                    }
                                }
                            }
                            // 실제 적용(admin+, 확인, 생성 매니페스트만). TODO placeholder 있으면 거부.
                            KeyCode::Char('a') if app.preview_apply && !app.can(Mode::Admin) => {
                                app.notify(format!(
                                    "apply needs --mode admin+ (current: {})",
                                    app.mode.name()
                                ));
                            }
                            KeyCode::Char('a') if app.preview_apply => {
                                if let Some((title, yaml)) = app.preview.clone() {
                                    // placeholder(TODO-...) 필드가 남아 있으면 거부. 이미지 env 로 채우면 통과.
                                    if manifest_has_placeholder(&yaml) {
                                        app.notify("apply blocked: TODO- placeholder remains; set the LMD_*_IMAGE env var".to_string());
                                    } else {
                                        app.confirm = Some(Pending::Apply { title, yaml });
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // Logs overlay.
                    if app.logs_mode {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => app.logs_mode = false,
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.logs_scroll = app.logs_scroll.saturating_sub(1)
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.logs_scroll = app.logs_scroll.saturating_add(3)
                            }
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
                    // vi/tmux panel-focus mode (armed by Ctrl-w): h/j/k/l or arrows move focus, anything else exits.
                    if app.panel_move {
                        match k.code {
                            KeyCode::Left
                            | KeyCode::Up
                            | KeyCode::Char('h')
                            | KeyCode::Char('k') => app.cycle_panel_dir(-1),
                            KeyCode::Right
                            | KeyCode::Down
                            | KeyCode::Char('l')
                            | KeyCode::Char('j') => app.cycle_panel_dir(1),
                            _ => app.panel_move = false, // Esc / Enter / any other key leaves panel-focus mode
                        }
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('q') => app.exit_confirm = true,
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
                        // Zoo: r → furiosa-ai HF org 라이브 새로고침(curl shell-out; pivot 보다 우선).
                        KeyCode::Char('r') if app.view == View::Zoo => {
                            let live = rt.block_on(crate::catalog::fetch_zoo_live());
                            let n0 = app.zoo.len();
                            app.zoo = crate::catalog::merge_zoo(std::mem::take(&mut app.zoo), live);
                            let added = app.zoo.len().saturating_sub(n0);
                            app.notify(if added > 0 {
                                format!("zoo refreshed — +{} new from furiosa-ai", added)
                            } else {
                                "zoo refreshed — up to date (or offline)".to_string()
                            });
                        }
                        // 크로스레이어 드릴 pivot — 선택 엔티티에서 관련 레이어로 점프
                        KeyCode::Char(c @ ('p' | 'i' | 'r' | 'e' | 'm' | 'n')) => app.pivot(c),
                        // EPP scorer 가중치 what-if(로컬 시뮬) — EPP 뷰에서만 반응
                        KeyCode::Char('+') | KeyCode::Char('=') => app.epp_adjust(1.0),
                        KeyCode::Char('-') | KeyCode::Char('_') => app.epp_adjust(-1.0),
                        // 세션 에너지 리셋(all-smi 식)
                        KeyCode::Char('R') => app.reset_energy(),
                        KeyCode::Char('?') => app.toggle_help(),
                        KeyCode::Char('A') => app.toggle_alerts(),
                        KeyCode::Char('a') => open_actions_or_detail(&mut app, false),
                        KeyCode::Char('t') => app.cycle_theme(),
                        KeyCode::Char('f') => {
                            let on = fx.toggle();
                            app.notify(format!("animations {}", if on { "on" } else { "off" }));
                        }
                        KeyCode::Char('z') => app.zoom = !app.zoom,
                        KeyCode::Char(' ') => app.paused = !app.paused,
                        // g/G — jump selection to first/last row (Grafana moved to palette `:graf`).
                        KeyCode::Char('g') => {
                            if app.detail {
                                app.detail_scroll = 0;
                            } else {
                                app.sel_edge(false);
                            }
                        }
                        KeyCode::Char('G') => {
                            if !app.detail {
                                app.sel_edge(true);
                            }
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
                                open_actions_or_detail(&mut app, false);
                            } else if app.view == View::Setup {
                                app.setup_enter(); // 부트스트랩 점검 행의 조치(apply/show cmd)
                            } else if app.view == View::Zoo {
                                app.open_action_menu(); // Prefetch / Compile→벤더
                            } else if matches!(
                                app.view,
                                View::Serving | View::Library | View::Overview | View::Pods
                            ) {
                                open_actions_or_detail(&mut app, true);
                            } else {
                                app.toggle_detail();
                            }
                        }
                        KeyCode::Char('o') => app.cycle_sort(),
                        KeyCode::Char('O') => app.toggle_sort_dir(), // 정렬 방향 토글(오름/내림)
                        KeyCode::Tab => app.next_tab(),
                        KeyCode::BackTab => app.prev_tab(), // Shift+Tab → 이전 섹션
                        // Ctrl-w — arm vi/tmux panel-focus mode; then h/j/k/l or arrows move focus, Esc exits.
                        KeyCode::Char('w') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.arm_panel_move(); // footer banner communicates the mode; no toast needed
                        }
                        // [ ] — cycle the current section's sub-tabs (same as ←/→).
                        KeyCode::Char(']') => app.cycle_subtab(1),
                        KeyCode::Char('[') => app.cycle_subtab(-1),
                        // Ctrl-u / Ctrl-d — half-page jump through the list.
                        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.move_sel(-10)
                        }
                        KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.move_sel(10)
                        }
                        KeyCode::Char(c @ '0'..='6') => app.goto_section(c as usize - '0' as usize),
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
                        KeyCode::Left => app.cycle_subtab(-1), // ← 이전 서브탭
                        KeyCode::Right => app.cycle_subtab(1), // → 다음 서브탭
                        KeyCode::Char('l') if !app.can(Mode::Debug) => {
                            app.notify(format!(
                                "logs needs --mode debug+ (current: {})",
                                app.mode.name()
                            ));
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
                                app.notify(
                                    "logs: select a pod/model in Pods or Models".to_string(),
                                );
                            }
                        }
                        KeyCode::Char('s') if !app.can(Mode::Admin) => {
                            app.notify(format!(
                                "scale needs --mode admin+ (current: {})",
                                app.mode.name()
                            ));
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
                            app.notify(format!(
                                "restart needs --mode admin+ (current: {})",
                                app.mode.name()
                            ));
                        }
                        KeyCode::Char('S') => {
                            if let Some(m) = app.selected_model() {
                                app.confirm = Some(Pending::Restart {
                                    name: m.name.clone(),
                                });
                            } else {
                                app.notify(
                                    "restart: select a model in Models/Overview".to_string(),
                                );
                            }
                        }
                        // 서빙 중지(admin+, 확인) — replicas 0 으로(되돌릴 수 있음).
                        KeyCode::Char('x') if !app.can(Mode::Admin) => {
                            app.notify(format!(
                                "stop needs --mode admin+ (current: {})",
                                app.mode.name()
                            ));
                        }
                        KeyCode::Char('x') => {
                            if let Some(m) = app.selected_model() {
                                if m.desired == 0 {
                                    app.notify(format!(
                                        "{} is already stopped (0 replicas)",
                                        m.name
                                    ));
                                } else {
                                    app.confirm = Some(Pending::Stop {
                                        name: m.name.clone(),
                                    });
                                }
                            } else {
                                app.notify(
                                    "stop: select a model in Models or Overview".to_string(),
                                );
                            }
                        }
                        // 선택 리소스의 live YAML 보기(읽기전용 preview) — k9s y.
                        KeyCode::Char('y') => {
                            if let Some((kind, nsd, name)) = app.yaml_target() {
                                let nsopt = if nsd { Some(ns.as_str()) } else { None };
                                match kube::resource_yaml(kind, nsopt, &name) {
                                    Ok(y) => {
                                        app.preview = Some((
                                            format!("{} {} · yaml (read-only)", kind, name),
                                            y,
                                        ));
                                        app.preview_scroll = 0;
                                        app.preview_apply = false;
                                    }
                                    Err(e) => app.notify(format!(
                                        "yaml: {}",
                                        e.to_string().lines().next().unwrap_or("")
                                    )),
                                }
                            } else {
                                app.notify(
                                    "yaml: select a resource in Models, Pods, Nodes, or Deploy"
                                        .to_string(),
                                );
                            }
                        }
                        // Deploy: compile(NPU)/deploy 매니페스트 미리보기(dry-run, admin+).
                        KeyCode::Char('c') | KeyCode::Char('d')
                            if matches!(app.view, View::Serving | View::Library) =>
                        {
                            if !app.can(Mode::Admin) {
                                app.notify(format!(
                                    "compile/deploy needs --mode admin+ (current: {})",
                                    app.mode.name()
                                ));
                            } else if app.selected_artifact().is_none()
                                && app.selected_catalog_model().is_none()
                                && app.selected_stored().is_none()
                            {
                                app.notify(
                                    "select a store build, catalog row, or serving deployment"
                                        .to_string(),
                                );
                            } else if k.code == KeyCode::Char('c') {
                                app.compile_preview(); // stored 빌드는 compile 경로 없음(카탈로그에서 컴파일)
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
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .ok();
    terminal.show_cursor().ok();
    result
}

fn print_snapshot(s: &collect::Snapshot, cfg: &Config) {
    println!("== lmd-top snapshot (prom={}, ns={}) ==", cfg.prom, cfg.ns);
    println!(
        "gateway: {} {}",
        if s.gw_addr.is_empty() {
            "—"
        } else {
            &s.gw_addr
        },
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
    println!(
        "\n[routes] {} (epp_in_path={})",
        s.routes.len(),
        s.epp_in_path
    );
    for r in &s.routes {
        println!("  {:<10} → {}/{}", r.path, r.kind, r.backend);
    }
    if !s.objectives.is_empty() {
        println!(
            "[SLO] {}",
            s.objectives
                .iter()
                .map(|o| format!("{}(p{})", o.name, o.priority))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("\n[pods] {}", s.pods.len());
    for p in &s.pods {
        println!(
            "  {:<40} {:<8} {} {} restarts={}",
            p.name, p.phase, p.ready, p.node, p.restarts
        );
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
        std::iter::once("lmd-top")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn apply_outcome_success_notifies_and_clears_preview() {
        let _g = crate::audit::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
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
                result: Ok(OkInfo {
                    audit_detail: "created".into(),
                    notify: "applied — created".into(),
                    clear_preview: true,
                }),
            },
        );
        std::env::remove_var("LMD_AUDIT");
        assert!(app.preview.is_none(), "Apply 성공은 preview 를 닫아야");
        assert_eq!(app.toast.as_deref(), Some("applied — created"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("\tapply\tmanifest-x\tok\tcreated"),
            "audit ok line: {}",
            body
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_outcome_failure_opens_preview_and_audits_fail() {
        let _g = crate::audit::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
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
        assert!(
            body.contains("\tscale→2\tds4\tFAIL\t"),
            "audit fail line: {}",
            body
        );
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
        assert_eq!(
            check_args(&args(&["--nope"])),
            ArgCheck::Unknown("--nope".into())
        );
        assert_eq!(
            check_args(&args(&["--jsonx"])),
            ArgCheck::Unknown("--jsonx".into())
        );
        assert_eq!(check_args(&args(&["foo"])), ArgCheck::Unknown("foo".into()));
    }

    #[test]
    fn mode_validation() {
        assert_eq!(
            check_args(&args(&["--mode", "bogus"])),
            ArgCheck::BadMode("bogus".into())
        );
        assert_eq!(check_args(&args(&["--mode"])), ArgCheck::MissingMode);
        // --mode 값 뒤의 실제 미지원 플래그도 잡힌다.
        assert_eq!(
            check_args(&args(&["--mode", "admin", "--x"])),
            ArgCheck::Unknown("--x".into())
        );
    }
}
