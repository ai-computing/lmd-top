//! lmd-top — terminal observability tool for llm-d clusters (Phase 1: monitor).
//! Run: `lmd-top`            → TUI
//!      `lmd-top --snapshot` → collect once and print text (for headless verification)

mod accel;
mod advisor;
mod agent;
mod app;
mod audit;
mod bootstrap;
mod cast;
mod catalog;
mod collect;
mod compat;
mod config;
mod coverage;
mod diagnose;
mod doctor;
mod drain;
mod epp_config;
mod history;
mod inventory;
mod kube;
mod kube_session;
mod latency;
mod local_file;
mod manifest;
mod manifest_plan;
mod measurement;
mod metrics;
mod mutation;
mod performance;
mod permissions;
use mutation::{run_mutation, MutationOutcome};
mod ops;
mod palette;
mod policy;
mod probe;
mod process;
mod prom;
mod quality;
mod quote;
mod resource_guard;
mod rollback;
mod route_edit;
mod routing;
mod service_binding;
mod setup_probe;
mod snapshot;
mod snapshot_text;
mod store;
mod ui;
mod workload;

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
        use ratatui::crossterm::event::KeyCode;
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
                KeyCode::Up | KeyCode::Char('k') => $app.$form.as_mut().unwrap().move_cursor(-1),
                KeyCode::Down | KeyCode::Char('j') => $app.$form.as_mut().unwrap().move_cursor(1),
                KeyCode::Left | KeyCode::Char('h') => $app.$form.as_mut().unwrap().cycle(-1),
                KeyCode::Right | KeyCode::Char('l') => $app.$form.as_mut().unwrap().cycle(1),
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
    --init           guided local configuration (preserves existing files)
    --check-setup    check platform prerequisites; exit 1 if required items are missing
    --version       print build version and exit
    --doctor         survey Prometheus: exporters, metric coverage, gaps
    --audit          print the audit log of applied mutations, then exit
    --history        print recorded compile/serving outcomes, then exit
    --snapshot, -s   collect once, print headless text summary
    --render         render each view to text via TestBackend (CI / no-tty)
    --cast [FILE]    write a demo asciicast (default: docs/demo.cast)
    --policy <OP>    policy workspace: init | validate | render | plan | test | replay | build
    --workspace DIR policy developer workspace (default: policy-workspace)
    --file FILE     policy YAML (default: policy.yaml)
    --base FILE     existing EndpointPickerConfig for offline policy render
    --plan <OP>      generate a manifest headlessly: compile | deploy | train
                     train = LoRA fine-tuning Job on RNGD (npu-lora), e.g.
                     --plan train --vendor furiosa --model M --set data=/mnt/store/datasets/x.jsonl
    --model <HF_ID>  model id for --plan, e.g. Qwen/Qwen2.5-0.5B-Instruct
    --vendor <NAME>  target vendor for --plan: rbln | furiosa | gpu
    --set k=v        override a form field for --plan (repeatable); train keys: data name steps
                     seq_len rank alpha targets lr accum node hf_host_path cache_host_path image merge
    --dry-run        server-side validate the generated --plan manifest
    --apply          apply the generated --plan manifest (requires --mode admin+)
    --help, -h       show this help and exit

ENVIRONMENT:
    LMD_CONFIG       configuration file (default: ~/.config/lmd-top/lmd-top.yaml)
    LMD_PROM         Prometheus host:port
    LMD_NS           namespace (default: llm-serving)
    LMD_GRAFANA      Grafana base URL (opened via the `:graf` palette command)
    LMD_THEME        startup theme: soft | default | high-contrast | colorblind
    LMD_AUDIT        audit log path (default: ~/.config/lmd-top/audit.log)
    LMD_HISTORY      compile/serving history path (default: ~/.config/lmd-top/history.jsonl)
    LMD_COMPILE_ENV  extra env for compile Jobs, K=V,K=V (e.g. vendor debug flags)
                     RBLN builds on TVM, so TVM_LOG_DEBUG=1 / TVM_BACKTRACE=1 work
    LMD_RBLN_TOOLCHAIN  wheel dir in the store to install RBLN from, e.g.
                     /mnt/store/rbln-toolchain/0.10.3 — pins every version together and
                     avoids inheriting the node's python environment
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
    BadSet(String),
}

/// Scan args[1..] and allow only known flags. Reject unsupported flags/invalid --mode.
fn check_args(args: &[String]) -> ArgCheck {
    // Help takes top priority if present.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return ArgCheck::Help;
    }
    const NOVALUE: &[&str] = &[
        "--doctor",
        "--init",
        "--check-setup",
        "--version",
        "--json",
        "--snapshot",
        "-s",
        "--render",
        "--audit",
        "--history",
        "--dry-run",
        "--apply",
    ];
    const VALUE: &[&str] = &[
        "--plan",
        "--model",
        "--vendor",
        "--set",
        "--policy",
        "--workspace",
        "--file",
        "--base",
    ];
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
            if a == "--set"
                && !args[i + 1]
                    .split_once('=')
                    .is_some_and(|(k, v)| !k.trim().is_empty() && !v.is_empty())
            {
                return ArgCheck::BadSet(args[i + 1].clone());
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

/// Resolve `--vendor` through the accelerator registry, so a new accelerator's spellings
/// come from its pack rather than from a match arm here.
fn plan_vendor(v: &str) -> Result<&'static str> {
    accel::by_id(v).map(|p| p.id).ok_or_else(|| {
        let known: Vec<&str> = accel::packs().iter().map(|p| p.id).collect();
        anyhow::anyhow!("unsupported --vendor '{}': expected {}", v, known.join("|"))
    })
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
    // Whether an accelerator compiles ahead of time is a declared capability, not a special
    // case here: `gpu` used to slip through and produce a manifest mixing the RBLN script with
    // the Furiosa image (BUG-07).
    if op == "compile" && !accel::by_id(vendor).is_some_and(|p| p.caps.compiles_ahead_of_time) {
        let targets: Vec<&str> = accel::compilable().map(|p| p.id).collect();
        anyhow::bail!(
            "compile targets accelerators that need an ahead-of-time build ({}). '{}' models \
             are served from their Hugging Face weights directly — use `--plan deploy`.",
            targets.join("|"),
            vendor
        );
    }
    let mode = args
        .iter()
        .position(|a| a == "--mode")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| Mode::parse(s))
        .unwrap_or(Mode::Observe);
    if args.iter().any(|a| a == "--apply") {
        Pending::Apply {
            reviewed: None,
            title: "CLI plan".into(),
            yaml: String::new(),
            required: Mode::Admin,
        }
        .authorize(mode)
        .map_err(anyhow::Error::msg)?;
    }
    let snap = collect(cfg).await;
    let mut app = App::new();
    app.ns = cfg.ns.clone();
    app.apply(snap);
    let (title, yaml) = match op.as_str() {
        "compile" => app.plan_compile_for_model(&model, vendor, &overrides),
        "deploy" => app.plan_deploy_for_model(&model, vendor, &overrides),
        "train" => app::train::plan(&model, vendor, &overrides, &cfg.ns),
        _ => Err(format!(
            "unsupported --plan '{}': expected compile|deploy|train",
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
        let outcome = run_mutation(
            Pending::Apply {
                reviewed: Some(kube::prepare_manifest(&cfg.ns, &yaml)?),
                title,
                yaml: yaml.clone(),
                required: Mode::Admin,
            },
            &cfg.ns,
            mode,
        );
        if let Some(error) = outcome.audit_error {
            eprintln!(
                "audit log write failed: {error}; do not repeat the operation to retry logging"
            );
        }
        let result = outcome.result.map_err(anyhow::Error::msg)?;
        eprintln!("{}", result.notify);
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
        ArgCheck::BadSet(value) => {
            eprintln!("invalid --set {:?}: expected nonempty key=value", value);
            std::process::exit(2);
        }
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

    if args.iter().any(|a| a == "--version") {
        println!("lmd-top {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--init") {
        return bootstrap::run(&cfg);
    }
    let _kube_session_cleanup = kube_session::Cleanup;
    if args.iter().any(|a| a == "--check-setup") {
        let mut app = App::new();
        app.ns = cfg.ns.clone();
        app.snap = collect(&cfg).await;
        println!("Setup: namespace={} prometheus={}", cfg.ns, cfg.prom);
        let mut missing = 0;
        let mut unknown = 0;
        for check in app.setup_checks() {
            println!(
                "{} [{}] {}: {}",
                check.state.glyph(),
                check.category,
                check.name,
                check.detail
            );
            if check.state == app::CheckState::Missing {
                missing += 1;
            } else if check.state == app::CheckState::Unknown {
                unknown += 1;
            }
            if let app::SetupFix::Command(command) = check.fix {
                println!("  {}", command.replace('\n', "\n  "));
            }
        }
        println!("{missing} required checks missing; {unknown} checks unknown. Open section 6 (Setup) for details.");
        if missing > 0 || unknown > 0 {
            anyhow::bail!(
                "setup incomplete: {missing} required checks missing; {unknown} checks unknown"
            );
        }
        return Ok(());
    }

    if args.iter().any(|a| a == "--policy") {
        return policy::cli(&cfg, &args).await;
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

    // Recorded compile/serving outcomes — what the option recommendations are drawn from.
    if args.iter().any(|a| a == "--history") {
        history::print_log();
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
        print!("{}", snapshot_text::render(&snap, &cfg, &history::load()));
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

/// Apply a mutation result to the audit log + UI (toast/failure preview). (main thread, unit-testable)
fn apply_outcome(app: &mut App, mut o: MutationOutcome) {
    o.record_once();
    let audit_error = o.audit_error.clone();
    match o.result {
        Ok(ok) => {
            // Reflect the change locally so the next keystroke computes from what we just did
            // rather than from a snapshot up to a full tick old (ACT-01).
            if let Some((name, replicas)) = ok.desired_now {
                app.pending_desired.insert(name, replicas);
            }
            if let Some((title, body)) = ok.window {
                app.preview = Some((title, body));
                app.preview_scroll = 0;
                app.preview_apply = false;
            } else if ok.clear_preview {
                app.preview = None;
            }
            app.notify(ok.notify);
        }
        Err(e) => {
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
    if let Some(error) = audit_error {
        let status = app.toast.clone().unwrap_or_default();
        app.notify(format!("{status} · audit log write failed: {error} (do not repeat the operation to retry logging)"));
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
        RoutingPolicy => app.open_policy_form(),
        PerformanceAssessment => {
            let mut lines = vec![
                "Performance assessment · configured targets and current evidence".to_string(),
            ];
            for row in &app.snap.perf_rows {
                let advice = app.perf_advice(row);
                lines.push(format!("\n{}", row.model));
                if !advice.has_obj {
                    lines.push(
                        "  No uniquely matched objective; set a target for this model.".into(),
                    );
                }
                for (metric, met) in advice.checks {
                    lines.push(format!(
                        "  {metric}: {}",
                        if met { "met" } else { "violated" }
                    ));
                }
                for (metric, reason) in advice.unknown {
                    lines.push(format!("  {metric}: unknown — {reason}"));
                }
                for tip in advice.tips {
                    lines.push(format!("  {tip}"));
                }
            }
            if app.snap.perf_rows.is_empty() {
                lines.push("No per-model performance measurements available.".into());
            }
            lines.push("\nTargets are session-local. Independent stage percentiles are not additive. This is an observation, not proof of capacity or a benchmark comparison.".into());
            app.preview = Some(("Performance assessment".into(), lines.join("\n")));
            app.preview_apply = false;
            app.preview_scroll = 0;
        }
        Workloads => {
            app.preview = Some((
                "Workload classification".into(),
                workload::report(&app.snap.workloads),
            ));
            app.preview_apply = false;
            app.preview_scroll = 0;
        }
        DataSources => {
            app.preview = Some((
                "Data sources · collection quality".into(),
                app.snap.source_report(collect::now_secs()),
            ));
            app.preview_apply = false;
            app.preview_scroll = 0;
        }
        EppCapabilities => {
            app.preview = Some((
                "EPP capabilities · pinned vs dev".into(),
                include_str!("../docs/EPP-CAPABILITIES.md").into(),
            ));
            app.preview_apply = false;
            app.preview_scroll = 0;
        }
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
    route: Option<crate::collect::Route>,
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
                    expected: app.snap.deployment_versions.get(subject).cloned(),
                    name: subject.to_string(),
                });
            }
        }
        Action::Scale => {
            if require_action(app, action) {
                // Resolve the replica toggle from the menu's subject — reading the live selection
                // here let a selection change between opening the menu and running the action
                // produce "scale A → 1" when the user meant to stop A (BUG-02 / FORM-02).
                let target = app
                    .effective_desired(subject)
                    .map(|d| if d == 0 { 1 } else { 0 })
                    .unwrap_or(1);
                app.confirm = Some(Pending::Scale {
                    expected: app.snap.deployment_versions.get(subject).cloned(),
                    name: subject.to_string(),
                    target,
                });
            }
        }
        Action::Restart => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Restart {
                    expected: app.snap.deployment_versions.get(subject).cloned(),
                    name: subject.to_string(),
                });
            }
        }
        Action::Rollback => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Rollback {
                    expected: app.snap.deployment_versions.get(subject).cloned(),
                    name: subject.to_string(),
                });
            }
        }
        Action::Drain => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Drain {
                    pod: subject.to_string(),
                });
            }
        }
        Action::Yaml => {
            if let Some((kind, nsd, name)) = app.yaml_target() {
                app.request_yaml(kind, nsd, name);
            }
        }
        Action::Delete => {
            if require_action(app, action) {
                app.confirm = Some(Pending::DeletePod {
                    expected: app
                        .snap
                        .pods
                        .iter()
                        .find(|r| r.name == subject)
                        .and_then(|r| r.identity.clone()),
                    name: subject.to_string(),
                });
            }
        }
        Action::DeleteJob => {
            if require_action(app, action) {
                app.confirm = Some(Pending::DeleteJob {
                    expected: app
                        .snap
                        .compiles
                        .iter()
                        .find(|r| r.name == subject)
                        .and_then(|r| r.identity.clone()),
                    name: subject.to_string(),
                });
            }
        }
        Action::Pivot(c) => app.pivot(c), // cross-layer jump — reads current selection, pushes breadcrumb
        Action::Objective => app.open_objective_form(), // set objectives (observe-only, no permissions needed)
        Action::Cordon | Action::Uncordon => {
            if require_action(app, action) {
                app.confirm = Some(Pending::Cordon {
                    expected: app
                        .snap
                        .nodes
                        .iter()
                        .find(|n| n.name == subject)
                        .and_then(|n| n.identity.clone()),
                    node: subject.to_string(),
                    on: matches!(action, Action::Cordon),
                });
            }
        }
        Action::RouteRename => {
            if require_action(app, action) {
                if let Some(route) = route {
                    app.open_route_rename(route);
                } else {
                    app.notify("route target unavailable — reopen its action menu".into());
                }
            }
        }
        Action::RouteRetarget => {
            if require_action(app, action) {
                if let Some(route) = route {
                    app.open_route_retarget(route);
                } else {
                    app.notify("route target unavailable — reopen its action menu".into());
                }
            }
        }
        // Shared-store housekeeping. Both build a Job manifest and hand it to the preview,
        // so `rm -rf` is reviewable (and dry-run validatable) before it ever runs.
        Action::StoreDelete => app.open_store_delete(),
        Action::StoreMove => app.open_store_move(),
        Action::StoreRefresh => app.confirm = Some(Pending::StoreRefresh),
        Action::Provenance => {
            if let Some((deployment, node, path)) = app.provenance_target() {
                app.confirm = Some(Pending::Provenance {
                    deployment,
                    node,
                    path,
                });
            }
        }
        Action::RouteDelete => {
            if require_action(app, action) {
                // Same rule as Scale: the target is the route the menu was opened on (BUG-02).
                if let Some(r) = route {
                    app.confirm = Some(Pending::RouteDelete {
                        expected: r.identity,
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
                app.request_logs(pod);
            } else {
                app.notify("logs: no pod for selection".to_string());
            }
        }
    }
    let _ = (ns, prom, rt); // currently unused (reserved for future on-demand queries)
}

/// Route a wheel notch to whatever the user is actually looking at.
///
/// The wheel used to always move the background list, even with a modal overlay open — so while
/// choosing compile options the cursor moved in the panel *behind* the floating form (and, worse,
/// retargeted the action an open menu was about to run: REG-01/BUG-02). The top overlay owns the
/// wheel, exactly as it owns the keyboard; only with no overlay does it reach the list.
fn scroll_wheel(app: &mut App, dir: i64) {
    use ui::Overlay;
    let d = dir as i32;
    match Overlay::top(app) {
        None => app.move_sel(dir),
        // Option forms: move the field cursor, unless a free-text field is being edited
        // (there the wheel has no meaning and must not silently change which field is active).
        Some(Overlay::PolicyForm) => {
            if let Some(f) = app.policy_form.as_mut().filter(|f| !f.editing) {
                f.move_cursor(d);
            }
        }
        Some(Overlay::CompileForm) => {
            if let Some(f) = app.compile_form.as_mut().filter(|f| !f.editing) {
                f.move_cursor(d);
            }
        }
        Some(Overlay::PrefetchForm) => {
            if let Some(f) = app.prefetch_form.as_mut().filter(|f| !f.editing) {
                f.move_cursor(d);
            }
        }
        Some(Overlay::DeployForm) => {
            if let Some(f) = app.deploy_form.as_mut().filter(|f| !f.editing) {
                f.move_cursor(d);
            }
        }
        Some(Overlay::ObjectiveForm) => {
            if let Some(f) = app.objective_form.as_mut().filter(|f| !f.editing) {
                f.move_cursor(d);
            }
        }
        Some(Overlay::PlacePicker) => app.place_pick_move(dir),
        Some(Overlay::ActionMenu) => {
            if let Some(mn) = app.action_menu.as_mut() {
                mn.move_cursor(d);
            }
        }
        Some(Overlay::Palette) => {
            if let Some(pl) = app.palette.as_mut() {
                pl.move_cursor(d);
            }
        }
        Some(Overlay::Preview) => {
            let lines = app
                .preview
                .as_ref()
                .map(|(_, y)| y.lines().count())
                .unwrap_or(0) as u16;
            app.preview_scroll = if dir > 0 {
                app.preview_scroll
                    .saturating_add(3)
                    .min(lines.saturating_sub(1))
            } else {
                app.preview_scroll.saturating_sub(3)
            };
        }
        Some(Overlay::Logs) => {
            let last = (app.logs.len() as u16).saturating_sub(1);
            app.logs_scroll = if dir > 0 {
                app.logs_scroll.saturating_add(3).min(last)
            } else {
                app.logs_scroll.saturating_sub(3)
            };
        }
        // Help / alerts / confirms / route form — nothing scrollable; swallow the notch rather
        // than letting it fall through to the list underneath.
        Some(_) => {}
    }
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
    let directory = std::env::var("LMD_SAVE_DIR").unwrap_or_else(|_| ".".into());
    Ok(
        local_file::save_manifest(std::path::Path::new(&directory), title, yaml)?
            .to_string_lossy()
            .into_owned(),
    )
}

/// Edit the manifest in $EDITOR (default vi) — briefly suspend the TUI, run the editor, then restore.
/// Returns the edited content on save and clean exit, otherwise None.
fn edit_in_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    content: &str,
) -> Option<String> {
    let session = local_file::EditFile::new(content).ok()?;
    let path = session.path();
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".into());
    // Suspend TUI (leave alt-screen, disable raw) → editor → restore.
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    let status = std::process::Command::new(&editor).arg(path).status();
    let _ = enable_raw_mode();
    let _ = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture);
    let _ = terminal.clear();
    match status {
        Ok(s) if s.success() => std::fs::read_to_string(path).ok(),
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
    // Start the UI immediately; unavailable collectors must not block quit/help.
    let shared = Arc::new(Mutex::new(collect::Snapshot::default()));

    // full collection loop (3s) — heavy items: models/EPP/perf/events etc.
    {
        let shared = shared.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(cfg.interval_full));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let snap = collect(&cfg).await;
                if let Ok(mut g) = shared.lock() {
                    g.merge_full(snap);
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
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await;
            loop {
                tick.tick().await;
                let observation = collect::collect_fast(&cfg).await;
                if let Ok(mut g) = shared.lock() {
                    g.merge_fast(observation);
                }
            }
        });
    }

    // UI loop (blocking)
    let rt = tokio::runtime::Handle::current(); // for on-demand Perf drill queries
    let res = tokio::task::spawn_blocking(move || ui_loop(shared, cfg, mode, rt)).await?;
    res
}

/// Server dry-run uses the same operation slot as mutations but never mutates UI state
/// from a worker or marks a later edited manifest as validated.
fn start_validation(
    app: &mut App,
    tx: &std::sync::mpsc::Sender<Result<String, String>>,
    ns: &str,
    yaml: &str,
) {
    if app.inflight.is_some() {
        app.notify("operation in progress — retry validation after it completes".into());
        return;
    }
    app.inflight = Some("validating manifest (server dry-run)".into());
    let tx = tx.clone();
    let ns = ns.to_string();
    let yaml = yaml.to_string();
    std::thread::spawn(move || {
        let result = kube::apply_manifest(&ns, &yaml, true)
            .map(|out| {
                format!(
                    "submitted manifest valid ✓ {}{}",
                    out.lines().next().unwrap_or("ok"),
                    if manifest_has_placeholder(&yaml) {
                        " · TODO placeholders remain"
                    } else {
                        ""
                    }
                )
            })
            .map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
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
    let (validation_tx, validation_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let (review_tx, review_rx) =
        std::sync::mpsc::channel::<(u64, Result<manifest_plan::Reviewed, String>)>();
    let mut review_busy = false;
    let (logs_tx, logs_rx) = std::sync::mpsc::channel();
    let mut logs_busy = false;
    let (zoo_tx, zoo_rx) = std::sync::mpsc::channel::<crate::catalog::ZooRefresh>();
    let mut zoo_busy = false;
    let (perf_tx, perf_rx) = std::sync::mpsc::channel();
    let mut perf_busy = false;
    let (yaml_tx, yaml_rx) = std::sync::mpsc::channel();
    let mut yaml_busy = false;
    let mut collector_dead = false; // report a poisoned snapshot lock once, not every frame
    let result = (|| -> Result<()> {
        loop {
            if !app.paused {
                // A poisoned lock means a collector thread panicked. Recovering the guard keeps the
                // last good snapshot on screen; `unwrap_or_default()` used to paint an empty cluster
                // (0 nodes, 0 models, 0 warnings) — an observability tool quietly lying (BUG-16).
                match shared.lock() {
                    Ok(g) => app.apply(g.clone()),
                    Err(poisoned) => {
                        app.apply(poisoned.into_inner().clone());
                        if !collector_dead {
                            collector_dead = true;
                            app.notify(
                                "collector thread panicked — data is frozen at the last good snapshot"
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            app.manifest_review.sync(app.confirm.as_ref(), &ns);
            while let Ok((generation, result)) = review_rx.try_recv() {
                review_busy = false;
                if let Some(result) =
                    app.manifest_review
                        .finish(&mut app.confirm, &ns, generation, result)
                {
                    app.confirm_yes = false;
                    app.manifest_review_details = false;
                    app.notify(match result {
                        Ok(()) => {
                            "manifest targets prepared — review with p before confirming".into()
                        }
                        Err(error) => format!("manifest preparation failed: {error}; v to retry"),
                    });
                }
            }
            if !review_busy {
                if let Some((generation, yaml)) =
                    app.manifest_review
                        .request(app.confirm.as_ref(), &ns, app.mode)
                {
                    review_busy = true;
                    let tx = review_tx.clone();
                    let ns = ns.clone();
                    std::thread::spawn(move || {
                        let result = kube::prepare_manifest(&ns, &yaml).map_err(|e| e.to_string());
                        let _ = tx.send((generation, result));
                    });
                }
            }
            // Apply completed mutations (audit + toast/failure window). Clear in-flight.
            while let Ok(outcome) = mut_rx.try_recv() {
                app.inflight = None;
                apply_outcome(&mut app, outcome);
            }
            while let Ok(result) = validation_rx.try_recv() {
                app.inflight = None;
                app.notify(match result {
                    Ok(message) => message,
                    Err(e) => format!("submitted manifest invalid: {e}"),
                });
            }
            while let Ok((generation, result)) = logs_rx.try_recv() {
                logs_busy = false;
                app.finish_logs(generation, result);
            }
            // One worker at a time; a newer target replaces the queued request.
            if !logs_busy {
                if let Some((generation, pod)) = app.logs_request.take() {
                    logs_busy = true;
                    let tx = logs_tx.clone();
                    let ns = ns.clone();
                    std::thread::spawn(move || {
                        let result = kube::logs(&ns, &pod, 400).map_err(|e| e.to_string());
                        let _ = tx.send((generation, result));
                    });
                }
            }
            while let Ok(mut result) = zoo_rx.try_recv() {
                zoo_busy = false;
                let previous = app.zoo.len();
                app.zoo = crate::catalog::merge_zoo(
                    std::mem::take(&mut app.zoo),
                    std::mem::take(&mut result.models),
                );
                app.notify(result.message(app.zoo.len().saturating_sub(previous)));
            }
            app.invalidate_perf_request();
            while let Ok((generation, detail)) = perf_rx.try_recv() {
                perf_busy = false;
                app.finish_perf_detail(generation, detail);
            }
            if !perf_busy {
                if let Some((generation, model, target)) = app.perf_request.take() {
                    perf_busy = true;
                    let tx = perf_tx.clone();
                    let prom = prom.clone();
                    let ns = ns.clone();
                    rt.spawn(async move {
                        let detail = collect::perf_detail(&prom, &ns, &model, &target).await;
                        let _ = tx.send((generation, detail));
                    });
                }
            }
            app.invalidate_yaml_request();
            while let Ok((generation, result)) = yaml_rx.try_recv() {
                yaml_busy = false;
                app.finish_yaml(generation, result);
            }
            if !yaml_busy {
                if let Some(request) = app.yaml_request.take() {
                    yaml_busy = true;
                    let tx = yaml_tx.clone();
                    let ns = ns.clone();
                    std::thread::spawn(move || {
                        let result = kube::resource_yaml(
                            &request.kind,
                            request.namespaced.then_some(ns.as_str()),
                            &request.name,
                        )
                        .map_err(|e| e.to_string());
                        let _ = tx.send((request.generation, result));
                    });
                }
            }
            app.poll_policy();
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|f| ui::draw(f, &app, &mut fx))?;

            // Poll briefly while animating (smooth frames), otherwise 100ms.
            let poll_ms = if fx.animating() { 16 } else { 100 };
            if event::poll(Duration::from_millis(poll_ms))? {
                let ev = event::read()?;
                if let Event::Mouse(m) = ev {
                    let dir = match m.kind {
                        MouseEventKind::ScrollDown => 1,
                        MouseEventKind::ScrollUp => -1,
                        _ => 0,
                    };
                    if dir != 0 {
                        scroll_wheel(&mut app, dir);
                    }
                    continue;
                }
                if let Event::Key(k) = ev {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    // One source of truth for who owns this key: the topmost open overlay,
                    // by Overlay::PRECEDENCE. Each block below asserts *which* overlay it is
                    // rather than merely "mine is open", so the dispatch order cannot drift
                    // from the declared z-order — which is how the compile options form came
                    // to consume keys while its destination picker was the visible window.
                    let top = ui::Overlay::top(&app);

                    // Filter input mode: capture text (a mode on the view, not an overlay).
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
                    if top == Some(ui::Overlay::Palette) {
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
                    // Help scrolling stays inside the modal.
                    if top == Some(ui::Overlay::Help) {
                        let size = terminal.size()?;
                        let max = ui::help_scroll_max(size.width, size.height);
                        app.help_scroll = app.help_scroll.min(max);
                        match k.code {
                            KeyCode::Up => app.help_scroll = app.help_scroll.saturating_sub(1),
                            KeyCode::Down => app.help_scroll = app.help_scroll.saturating_add(1),
                            KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(10),
                            KeyCode::PageDown => {
                                app.help_scroll = app.help_scroll.saturating_add(10)
                            }
                            KeyCode::Home => app.help_scroll = 0,
                            KeyCode::End => app.help_scroll = max,
                            _ => app.help = false,
                        }
                        continue;
                    }
                    // Quit confirmation: opened by q in the background. Keep it separate from mutation confirms.
                    if top == Some(ui::Overlay::ExitConfirm) {
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
                    if let Some(pending) = app
                        .confirm
                        .clone()
                        .filter(|_| top == Some(ui::Overlay::Confirm))
                    {
                        if app.manifest_review_details {
                            let size = terminal.size()?;
                            let max = ui::manifest_review_scroll_max(
                                &pending,
                                size.width,
                                size.height,
                                app.manifest_review.error.as_deref(),
                            );
                            app.manifest_review_scroll = app.manifest_review_scroll.min(max);
                            match k.code {
                                KeyCode::Esc | KeyCode::Char('p') | KeyCode::Char('q') => {
                                    app.manifest_review_details = false
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    app.manifest_review_scroll =
                                        app.manifest_review_scroll.saturating_add(1).min(max)
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    app.manifest_review_scroll =
                                        app.manifest_review_scroll.saturating_sub(1)
                                }
                                KeyCode::PageDown => {
                                    app.manifest_review_scroll =
                                        app.manifest_review_scroll.saturating_add(10).min(max)
                                }
                                KeyCode::PageUp => {
                                    app.manifest_review_scroll =
                                        app.manifest_review_scroll.saturating_sub(10)
                                }
                                KeyCode::Home => app.manifest_review_scroll = 0,
                                KeyCode::End => app.manifest_review_scroll = max,
                                _ => {}
                            }
                            continue;
                        }
                        // Only for Apply confirms: e=edit in vi, v=server dry-run validation (YAML reachable only via specific keys).
                        if let Pending::Apply {
                            yaml,
                            title,
                            required,
                            ..
                        } = &pending
                        {
                            match k.code {
                                KeyCode::Char('p') | KeyCode::Char('P') => {
                                    app.manifest_review_details = true;
                                    app.manifest_review_scroll = 0;
                                    continue;
                                }
                                KeyCode::Char('e') | KeyCode::Char('E') => {
                                    if let Some(edited) = edit_in_editor(&mut terminal, yaml) {
                                        app.confirm = Some(Pending::Apply {
                                            reviewed: None,
                                            required: *required,
                                            title: title.clone(),
                                            yaml: edited,
                                        });
                                        app.manifest_review.reset();
                                        app.confirm_yes = false;
                                        app.notify(
                                            "manifest edited — preparing targets again".to_string(),
                                        );
                                    }
                                    continue;
                                }
                                KeyCode::Char('v') | KeyCode::Char('V') => {
                                    if let Some(Pending::Apply { reviewed, .. }) = &mut app.confirm
                                    {
                                        *reviewed = None;
                                    }
                                    app.manifest_review.reset();
                                    app.confirm_yes = false;
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
                        let Some(pending) = app.resolve_confirmation(execute) else {
                            continue;
                        };
                        // The reviewed operation now owns the execution slot.
                        let tx = mut_tx.clone();
                        let ns2 = ns.clone();
                        let amode = app.mode;
                        std::thread::spawn(move || {
                            let _ = tx.send(run_mutation(pending, &ns2, amode));
                        });
                        continue;
                    }
                    // Alert history overlay: esc/q/A closes it.
                    if top == Some(ui::Overlay::Alerts) {
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
                    if top == Some(ui::Overlay::ActionMenu) {
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
                            let menu = app.action_menu.take().unwrap();
                            dispatch_action(
                                &mut app,
                                action,
                                &menu.subject,
                                menu.route,
                                &ns,
                                &prom,
                                &rt,
                            );
                        }
                        continue;
                    }
                    // Manual form viewport movement never edits or submits a field.
                    if matches!(
                        top,
                        Some(
                            ui::Overlay::CompileForm
                                | ui::Overlay::DeployForm
                                | ui::Overlay::PrefetchForm
                                | ui::Overlay::ObjectiveForm
                        )
                    ) {
                        let scroll = app.form_view_position.get();
                        let next = match k.code {
                            KeyCode::PageUp => Some(scroll.saturating_sub(10)),
                            KeyCode::PageDown => Some(scroll.saturating_add(10)),
                            KeyCode::Home => Some(0),
                            KeyCode::End => Some(u16::MAX),
                            _ => None,
                        };
                        app.form_scroll.set(next);
                        if next.is_some() {
                            continue;
                        }
                    } else {
                        app.form_scroll.set(None);
                    }
                    // Serving objective edit form overlay.
                    if top == Some(ui::Overlay::ObjectiveForm) {
                        handle_edit_form!(app, objective_form, objective_form_submit, k.code);
                        continue;
                    }
                    // Masked secret entry. Enter stores it (behind a confirm), Esc discards.
                    if top == Some(ui::Overlay::SecretForm) {
                        match k.code {
                            KeyCode::Esc => app.secret_form = None,
                            KeyCode::Backspace => {
                                if let Some(f) = app.secret_form.as_mut() {
                                    f.backspace();
                                }
                            }
                            KeyCode::Enter => {
                                let form = app.secret_form.take().expect("open");
                                if form.is_empty() {
                                    app.notify("nothing entered — secret unchanged".to_string());
                                } else {
                                    let (name, key, purpose) =
                                        (form.name.clone(), form.key.clone(), form.purpose.clone());
                                    app.confirm = Some(Pending::SetSecret {
                                        name,
                                        key,
                                        purpose,
                                        value: form.into_value(),
                                    });
                                    app.confirm_yes = false;
                                }
                            }
                            // Everything printable goes into the value — no `q` shortcut here,
                            // because a token may legitimately contain one.
                            KeyCode::Char(c) => {
                                if let Some(f) = app.secret_form.as_mut() {
                                    f.push(c);
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // Shared-store move form — a single editable destination path.
                    // Enter only builds the manifest; applying it is a further keystroke in
                    // the preview, so a typo cannot move a 17GB artifact on its own.
                    if top == Some(ui::Overlay::StoreForm) {
                        match k.code {
                            KeyCode::Esc => app.store_form = None,
                            KeyCode::Enter => app.submit_store_move(),
                            KeyCode::Backspace => {
                                app.store_form.as_mut().unwrap().value.pop();
                            }
                            KeyCode::Char(c) => app.store_form.as_mut().unwrap().value.push(c),
                            _ => {}
                        }
                        continue;
                    }
                    // Route edit form (rename text / retarget selection).
                    if top == Some(ui::Overlay::RouteForm) {
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
                                            expected: form.expected,
                                            route: form.route,
                                            old: form.path,
                                            new,
                                        });
                                    }
                                } else if let Some((kind, name)) = form.value.split_once(':') {
                                    app.confirm = Some(Pending::RouteRetarget {
                                        expected: form.expected,
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
                    // Placement/Destination picker (deploy/compile/prefetch 2단계) — 후보 노드/PVC 상태 목록.
                    // Overlay::PRECEDENCE 순서와 일치: 자식 picker 가 열려있으면 부모 폼보다 먼저 키를 소비해야 함.
                    if top == Some(ui::Overlay::PlacePicker) {
                        match k.code {
                            KeyCode::Up | KeyCode::Char('k') => app.place_pick_move(-1),
                            KeyCode::Down | KeyCode::Char('j') => app.place_pick_move(1),
                            KeyCode::Enter => app.place_pick_apply(),
                            KeyCode::Esc | KeyCode::Char('q') => app.place_picker = None,
                            _ => {}
                        }
                        continue;
                    }
                    // NPU compile options form overlay. Enter(옵션 확정) → 목적지 노드 picker(2단계).
                    if top == Some(ui::Overlay::PolicyForm) {
                        handle_edit_form!(app, policy_form, policy_form_submit, k.code);
                        continue;
                    }
                    if top == Some(ui::Overlay::CompileForm) {
                        let editing = app.compile_form.as_ref().unwrap().editing;
                        if !editing && matches!(k.code, KeyCode::Enter) {
                            app.open_compile_dest_picker();
                            continue;
                        }
                        handle_edit_form!(app, compile_form, compile_form_submit, k.code);
                        continue;
                    }
                    // Prefetch form. Enter(옵션 확정) → 목적지 PVC picker(2단계).
                    if top == Some(ui::Overlay::PrefetchForm) {
                        let editing = app.prefetch_form.as_ref().unwrap().editing;
                        if !editing && matches!(k.code, KeyCode::Enter) {
                            app.open_prefetch_dest_picker();
                            continue;
                        }
                        handle_edit_form!(app, prefetch_form, prefetch_form_submit, k.code);
                        continue;
                    }
                    // Deploy/serving options form overlay.
                    if top == Some(ui::Overlay::DeployForm) {
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
                    if top == Some(ui::Overlay::Preview) {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => app.preview = None,
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.preview_scroll = app.preview_scroll.saturating_sub(1)
                            }
                            // Clamp at the last line — unbounded `j` used to scroll the content
                            // off into a blank window with no way to tell you had passed the end (BUG-15).
                            KeyCode::Down | KeyCode::Char('j') => {
                                let lines =
                                    app.preview
                                        .as_ref()
                                        .map(|(_, y)| y.lines().count())
                                        .unwrap_or(0) as u16;
                                app.preview_scroll = app
                                    .preview_scroll
                                    .saturating_add(3)
                                    .min(lines.saturating_sub(1));
                            }
                            // 파일로 저장(어느 preview 든) — 편집 후 kubectl apply 하거나 보관.
                            KeyCode::Char('w') if app.yaml_is_loading() => {
                                app.notify("YAML is still loading".into());
                            }
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
                                    start_validation(&mut app, &validation_tx, &ns, &yaml);
                                }
                            }
                            // 실제 적용(admin+, 확인, 생성 매니페스트만). TODO placeholder 있으면 거부.
                            KeyCode::Char('a')
                                if app.preview_apply && !app.can(app.preview_required) =>
                            {
                                app.notify(format!(
                                    "apply needs --mode {}+ (current: {})",
                                    app.preview_required.name(),
                                    app.mode.name()
                                ));
                            }
                            KeyCode::Char('a') if app.preview_apply => {
                                if let Some((title, yaml)) = app.preview.clone() {
                                    // placeholder(TODO-...) 필드가 남아 있으면 거부. 이미지 env 로 채우면 통과.
                                    if manifest_has_placeholder(&yaml) {
                                        app.notify("apply blocked: TODO- placeholder remains; set the LMD_*_IMAGE env var".to_string());
                                    } else {
                                        app.confirm = Some(Pending::Apply {
                                            reviewed: None,
                                            title,
                                            yaml,
                                            required: app.preview_required,
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // Logs overlay.
                    if top == Some(ui::Overlay::Logs) {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => app.close_logs(),
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.logs_scroll = app.logs_scroll.saturating_sub(1)
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                // Same clamp as preview — and it keeps the footer's line counter
                                // (already clamped for display) honest about where you are (BUG-15).
                                app.logs_scroll = app
                                    .logs_scroll
                                    .saturating_add(3)
                                    .min((app.logs.len() as u16).saturating_sub(1));
                            }
                            KeyCode::Char('r') => {
                                app.request_logs(app.logs_target.clone());
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
                            if !zoo_busy {
                                zoo_busy = true;
                                app.notify("refreshing model zoo…".into());
                                let tx = zoo_tx.clone();
                                rt.spawn(async move {
                                    let result = crate::catalog::fetch_zoo_live().await;
                                    let _ = tx.send(result);
                                });
                            } else {
                                app.notify("model zoo refresh already running".into());
                            }
                        }
                        // 크로스레이어 드릴 pivot — 선택 엔티티에서 관련 레이어로 점프
                        KeyCode::Char(c @ ('p' | 'i' | 'r' | 'e' | 'm' | 'n')) => app.pivot(c),
                        // EPP scorer 가중치 what-if(로컬 시뮬) — EPP 뷰에서만 반응
                        KeyCode::Char('+') | KeyCode::Char('=') => app.epp_adjust(1.0),
                        KeyCode::Char('-') | KeyCode::Char('_') => app.epp_adjust(-1.0),
                        // 세션 에너지 리셋(all-smi 식)
                        KeyCode::Char('R') => app.reset_energy(),
                        KeyCode::Char('P') if app.view == app::View::Epp => app.cycle_epp_profile(),
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
                                if let Some(model) = app.perf_model_to_query() {
                                    app.request_perf_detail(model);
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
                                app.request_logs(pod);
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
                                let name = m.name.clone();
                                // effective_desired, not m.desired: the snapshot is up to a
                                // full tick behind the scale we may have just applied, and
                                // reading it here made a second `s` scale to 0 again (ACT-01).
                                let now = app.effective_desired(&name).unwrap_or(m.desired);
                                let target = if now == 0 { 1 } else { 0 };
                                app.confirm = Some(Pending::Scale {
                                    expected: app.snap.deployment_versions.get(&name).cloned(),
                                    name,
                                    target,
                                });
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
                                    expected: app.snap.deployment_versions.get(&m.name).cloned(),
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
                                // Same reason as `s`: right after a stop the snapshot still
                                // says it is running, so this would offer to stop it again.
                                let now =
                                    app.effective_desired(&m.name.clone()).unwrap_or(m.desired);
                                if now == 0 {
                                    app.notify(format!(
                                        "{} is already stopped (0 replicas)",
                                        m.name
                                    ));
                                } else {
                                    app.confirm = Some(Pending::Stop {
                                        expected: app
                                            .snap
                                            .deployment_versions
                                            .get(&m.name)
                                            .cloned(),
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
                                app.request_yaml(kind, nsd, name);
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

#[cfg(test)]
mod tests {
    use super::{apply_outcome, check_args, ArgCheck, MutationOutcome};
    use super::{App, Mode};
    use crate::mutation::OkInfo;

    fn args(v: &[&str]) -> Vec<String> {
        std::iter::once("lmd-top")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn worker_audit_is_not_duplicated_by_ui_delivery() {
        let _sink = crate::audit::TestSink::new();
        let path = std::env::temp_dir().join(format!("lmd-audit-once-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::env::set_var("LMD_AUDIT", &path);
        let outcome =
            super::run_mutation(crate::app::Pending::StoreRefresh, "test-ns", Mode::Observe);
        assert!(outcome.audit_attempted);
        assert!(outcome.audit_error.is_none());
        let before = std::fs::read_to_string(&path).unwrap();
        apply_outcome(&mut App::new(), outcome);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(before.lines().count(), 1);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn audit_failure_does_not_turn_completed_operation_into_retry() {
        let _sink = crate::audit::TestSink::new();
        std::env::set_var("LMD_AUDIT", std::env::temp_dir());
        let mut app = App::new();
        apply_outcome(
            &mut app,
            MutationOutcome {
                audit_scope: Default::default(),
                audit_attempted: false,
                audit_error: None,
                mode: Mode::Admin,
                audit_action: "scale".into(),
                audit_target: "model".into(),
                fail_label: "scale",
                result: Ok(OkInfo {
                    audit_detail: "scaled".into(),
                    notify: "scaled model".into(),
                    clear_preview: false,
                    window: None,
                    desired_now: Some(("model".into(), 2)),
                }),
            },
        );
        assert_eq!(app.pending_desired.get("model"), Some(&2));
        assert!(app
            .toast
            .as_deref()
            .unwrap()
            .contains("audit log write failed"));
        assert!(app.toast.as_deref().unwrap().contains("do not repeat"));
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
                audit_scope: Default::default(),
                audit_attempted: false,
                audit_error: None,
                mode: Mode::Admin,
                audit_action: "apply".into(),
                audit_target: "manifest-x".into(),
                fail_label: "apply",
                result: Ok(OkInfo {
                    audit_detail: "created".into(),
                    notify: "applied — created".into(),
                    clear_preview: true,
                    window: None,
                    desired_now: None,
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

    /// ACT-01: pressing `s` twice must toggle twice. The toggle reads `desired` from the
    /// snapshot, so a successful scale has to be reflected locally or the second press
    /// recomputes from the pre-scale value and scales to 0 again.
    #[test]
    fn a_successful_scale_updates_the_local_desired_so_the_toggle_flips() {
        let _g = crate::audit::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join("lmd-audit-desired-now.log");
        let _ = std::fs::remove_file(&path);
        std::env::set_var("LMD_AUDIT", &path);
        let mut app = App::new();
        app.snap.models = vec![crate::collect::ModelRow {
            name: "qa-dummy".into(),
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
        }];
        apply_outcome(
            &mut app,
            MutationOutcome {
                audit_scope: Default::default(),
                audit_attempted: false,
                audit_error: None,
                mode: Mode::Admin,
                audit_action: "scale→0".into(),
                audit_target: "qa-dummy".into(),
                fail_label: "scale",
                result: Ok(OkInfo {
                    audit_detail: "scaled".into(),
                    notify: "scaled qa-dummy → 0".into(),
                    clear_preview: false,
                    window: None,
                    desired_now: Some(("qa-dummy".into(), 0)),
                }),
            },
        );
        assert_eq!(
            app.effective_desired("qa-dummy"),
            Some(0),
            "the next toggle must see 0, or it will scale to 0 again"
        );
        // The snapshot itself is untouched — the main loop overwrites it from the collector at
        // the top of every iteration, which is exactly why this lives outside it.
        assert_eq!(app.snap.models[0].desired, 1);
        assert_eq!(app.snap.models[0].ready, 1);

        // Once the collector reports the new value, the override must retire rather than
        // pinning the toggle forever.
        let mut fresh = app.snap.clone();
        fresh.ts += 1;
        fresh.models[0].desired = 0;
        app.apply(fresh);
        assert!(
            app.pending_desired.is_empty(),
            "the override must clear once the collector agrees"
        );
        assert_eq!(app.effective_desired("qa-dummy"), Some(0));

        // An unknown deployment must be ignored rather than panicking or inventing a row.
        apply_outcome(
            &mut app,
            MutationOutcome {
                audit_scope: Default::default(),
                audit_attempted: false,
                audit_error: None,
                mode: Mode::Admin,
                audit_action: "scale→1".into(),
                audit_target: "gone".into(),
                fail_label: "scale",
                result: Ok(OkInfo {
                    audit_detail: "scaled".into(),
                    notify: "scaled gone → 1".into(),
                    clear_preview: false,
                    window: None,
                    desired_now: Some(("gone".into(), 1)),
                }),
            },
        );
        assert_eq!(app.snap.models.len(), 1);
        // An override for a deployment that is not in the snapshot must not survive a collect.
        let mut fresh2 = app.snap.clone();
        fresh2.ts += 2;
        app.apply(fresh2);
        assert!(
            !app.pending_desired.contains_key("gone"),
            "an override for a vanished deployment must be dropped"
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
                audit_scope: Default::default(),
                audit_attempted: false,
                audit_error: None,
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

    fn model(name: &str, desired: i64) -> crate::collect::ModelRow {
        crate::collect::ModelRow {
            name: name.into(),
            ready: desired,
            desired,
            status: String::new(),
            route: String::new(),
            engine: String::new(),
            accel: String::new(),
            running: None,
            waiting: None,
            tps: None,
            kv: None,
            ttft: None,
        }
    }

    /// BUG-02: the action menu freezes its subject, so Scale must derive the replica toggle from
    /// that subject. Reading the live selection produced "scale A → 1" when the user, having
    /// opened the menu on running model A, meant to stop it.
    #[test]
    fn scale_target_follows_menu_subject_not_selection() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut app = App::new();
        app.mode = Mode::Admin;
        app.view = crate::app::View::Overview;
        // A is running (stop → 0); B is stopped (start → 1).
        app.snap.models = vec![model("model-a", 1), model("model-b", 0)];
        let reviewed = crate::resource_guard::Identity {
            uid: "original-a".into(),
            version: "12".into(),
        };
        app.snap
            .deployment_versions
            .insert("model-a".into(), reviewed.clone());
        app.selected = 1; // selection sits on B, as a stray wheel event would leave it

        super::dispatch_action(
            &mut app,
            crate::app::Action::Scale,
            "model-a", // subject frozen when the menu opened
            None,
            "llm-serving",
            "127.0.0.1:9090",
            rt.handle(),
        );
        // A background collection must not retarget an already displayed confirmation.
        app.snap.deployment_versions.insert(
            "model-a".into(),
            crate::resource_guard::Identity {
                uid: "replacement-a".into(),
                version: "13".into(),
            },
        );
        match app.confirm.take() {
            Some(crate::app::Pending::Scale {
                name,
                target,
                expected,
            }) => {
                assert_eq!(expected, Some(reviewed));
                assert_eq!(name, "model-a");
                assert_eq!(target, 0, "running subject must scale to 0, not follow B");
            }
            other => panic!("expected a Scale confirm, got {:?}", other.is_some()),
        }
    }

    /// 사용자 리포트 회귀: compile 옵션 폼이 열려 있을 때 휠은 **폼의 필드 커서**를 움직여야 하고,
    /// 배경 리스트 선택은 손대지 않아야 한다. (예전엔 반대였다 — 배경만 움직였다.)
    #[test]
    fn wheel_drives_the_top_overlay_not_the_background() {
        let mut app = App::new();
        app.mode = Mode::Admin;
        app.view = crate::app::View::Library;
        app.catalog = crate::catalog::load();
        // 배경 리스트에 여러 행이 있어야 "안 움직였다"가 의미를 가진다.
        assert!(app.list_len() > 1, "need a multi-row background list");
        app.selected = 1;
        app.compile_form_for("rbln");
        assert!(app.compile_form.is_some(), "compile form should open");
        let field0 = app.compile_form.as_ref().unwrap().cursor;

        super::scroll_wheel(&mut app, 1);
        let field1 = app.compile_form.as_ref().unwrap().cursor;
        assert_ne!(field1, field0, "wheel should move the form's field cursor");
        assert_eq!(app.selected, 1, "background selection must not move");

        super::scroll_wheel(&mut app, -1);
        assert_eq!(
            app.compile_form.as_ref().unwrap().cursor,
            field0,
            "wheel up should move back"
        );
        assert_eq!(app.selected, 1, "background selection must still not move");

        // 자유 입력(e) 중에는 휠이 필드를 바꾸지 않는다 — 타이핑 중 대상이 바뀌면 안 된다.
        app.compile_form.as_mut().unwrap().editing = true;
        let during_edit = app.compile_form.as_ref().unwrap().cursor;
        super::scroll_wheel(&mut app, 1);
        assert_eq!(
            app.compile_form.as_ref().unwrap().cursor,
            during_edit,
            "wheel must be inert while a field is being edited"
        );

        // 오버레이를 닫으면 휠은 다시 리스트를 움직인다.
        app.compile_form = None;
        super::scroll_wheel(&mut app, 1);
        assert_ne!(app.selected, 1, "with no overlay the wheel moves the list");
    }

    /// Every overlay must have a dispatch arm, and each arm must name the overlay it handles.
    /// Without this a new overlay silently falls through to the background keymap, or — as in
    /// BUG-18 — a hand-ordered chain of "is mine open?" checks disagrees with the z-order and
    /// the wrong window consumes the keys.
    #[test]
    fn every_overlay_has_a_dispatch_arm_naming_it() {
        // Only the real code — this test's own literals would otherwise match themselves.
        let full = include_str!("main.rs");
        let src = &full[..full.find("\n#[cfg(test)]").unwrap_or(full.len())];
        for overlay in crate::ui::Overlay::PRECEDENCE {
            let arm = format!("Some(ui::Overlay::{:?})", overlay);
            assert!(
                src.contains(&arm),
                "no input dispatch arm for {:?} — add one, or it will never receive keys",
                overlay
            );
        }
        // And nothing dispatches on bare field presence any more, which is what allowed the
        // order to drift.
        for stale in [
            "if app.compile_form.is_some() {",
            "if app.place_picker.is_some() {",
            "if app.preview.is_some() {",
            "if app.logs_mode {",
        ] {
            assert!(
                !src.contains(stale),
                "input dispatch still keys off field presence: {}",
                stale
            );
        }
    }

    /// BUG-02 (REG-01): while any overlay is open the wheel must not reach the background list.
    /// The mouse branch is gated on exactly this predicate.
    #[test]
    fn overlay_open_claims_input() {
        let mut app = App::new();
        assert!(crate::ui::Overlay::top(&app).is_none());
        app.action_menu = Some(crate::app::ActionMenu {
            route: None,
            title: "model-a".into(),
            subject: "model-a".into(),
            items: Vec::new(),
            cursor: 0,
        });
        assert!(
            crate::ui::Overlay::top(&app).is_some(),
            "an open action menu must claim input so the wheel cannot retarget it"
        );
    }

    #[test]
    fn handle_edit_form_supports_vim_navigation() {
        let mut app = App::new();
        app.compile_form = Some(crate::ops::CompileForm {
            model: "test".into(),
            model_id: "test".into(),
            vendor: "rbln",
            engine: "rbln".into(),
            fields: vec![
                crate::ops::CompileField {
                    key: "f1".into(),
                    label: "f1".into(),
                    value: "a".into(),
                    choices: vec!["a".into(), "b".into(), "c".into()],
                    numeric: false,
                    help: "".into(),
                },
                crate::ops::CompileField {
                    key: "f2".into(),
                    label: "f2".into(),
                    value: "1".into(),
                    choices: vec!["1".into(), "2".into()],
                    numeric: true,
                    help: "".into(),
                },
            ],
            cursor: 0,
            editing: false,
            dest: String::new(),
        });

        // Down via 'j'
        handle_edit_form!(app, compile_form, compile_form_submit, KeyCode::Char('j'));
        assert_eq!(app.compile_form.as_ref().unwrap().cursor, 1);

        // Up via 'k'
        handle_edit_form!(app, compile_form, compile_form_submit, KeyCode::Char('k'));
        assert_eq!(app.compile_form.as_ref().unwrap().cursor, 0);

        // Cycle right via 'l'
        handle_edit_form!(app, compile_form, compile_form_submit, KeyCode::Char('l'));
        assert_eq!(app.compile_form.as_ref().unwrap().fields[0].value, "b");

        // Cycle left via 'h'
        handle_edit_form!(app, compile_form, compile_form_submit, KeyCode::Char('h'));
        assert_eq!(app.compile_form.as_ref().unwrap().fields[0].value, "a");
    }

    #[test]
    fn compile_2step_picker_takes_precedence() {
        let mut app = App::new();
        app.compile_form = Some(crate::ops::CompileForm {
            model: "test".into(),
            model_id: "test".into(),
            vendor: "rbln",
            engine: "rbln".into(),
            fields: vec![],
            cursor: 0,
            editing: false,
            dest: String::new(),
        });
        app.place_picker = Some(crate::ops::PlacePick {
            cursor: 0,
            rows: vec![
                crate::ops::PlaceRow {
                    value: "node1".into(),
                    label: "node1".into(),
                    free: 1,
                    total: 1,
                    util: 0.0,
                    mem_used: 0.0,
                    mem_total: 0.0,
                    schedulable: true,
                    note: "".into(),
                    info_only: false,
                },
                crate::ops::PlaceRow {
                    value: "node2".into(),
                    label: "node2".into(),
                    free: 1,
                    total: 1,
                    util: 0.0,
                    mem_used: 0.0,
                    mem_total: 0.0,
                    schedulable: true,
                    note: "".into(),
                    info_only: false,
                },
            ],
        });

        // Top overlay must be PlacePicker, not CompileForm
        assert_eq!(
            crate::ui::Overlay::top(&app),
            Some(crate::ui::Overlay::PlacePicker)
        );

        // Moving cursor down on place picker
        app.place_pick_move(1);
        assert_eq!(app.place_picker.as_ref().unwrap().cursor, 1);
        assert_eq!(app.compile_form.as_ref().unwrap().cursor, 0);
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

#[cfg(test)]
mod route_menu_subject_tests {
    use super::*;
    #[test]
    fn route_mutations_use_the_frozen_menu_object_even_with_duplicate_paths() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for action in [
            app::Action::RouteRename,
            app::Action::RouteRetarget,
            app::Action::RouteDelete,
        ] {
            let mut app = App::new();
            app.mode = Mode::Danger;
            app.view = app::View::Routing;
            app.snap.service_names = vec!["destination".into()];
            for name in ["first-route", "selected-route"] {
                app.snap.routes.push(collect::Route {
                    configuration: None,
                    match_summary: String::new(),
                    backend_summary: String::new(),
                    single_local_backend: true,
                    identity: Some(resource_guard::Identity {
                        uid: format!("{name}-uid"),
                        version: "42".into(),
                    }),
                    route: name.into(),
                    path: "/v1".into(),
                    backend: "destination".into(),
                    kind: "Service".into(),
                });
            }
            app.selected = 1;
            app.open_action_menu();
            let menu = app.action_menu.take().unwrap();
            let selected = menu.route.as_ref().unwrap();
            assert_eq!(selected.route, "selected-route");
            assert!(menu.title.contains("selected-route"));
            let expected = selected.identity.clone();
            // Simulate refresh/re-sort/removal while the operator reads the menu.
            app.snap.routes.remove(1);
            app.selected = 0;
            dispatch_action(
                &mut app,
                action,
                &menu.subject,
                menu.route,
                "ns",
                "unused:1",
                rt.handle(),
            );
            if matches!(action, app::Action::RouteDelete) {
                let Some(Pending::RouteDelete {
                    route,
                    path,
                    expected: actual,
                }) = app.confirm
                else {
                    panic!("expected deletion confirmation")
                };
                assert_eq!(route, "selected-route");
                assert_eq!(path, "/v1");
                assert_eq!(actual, expected);
            } else {
                let form = app.route_form.unwrap();
                assert_eq!(form.route, "selected-route");
                assert_eq!(form.path, "/v1");
                assert_eq!(form.expected, expected);
            }
        }
    }
}
