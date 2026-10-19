use super::*;
use crate::collect::{Accel, NodeInfo};
use ratatui::{backend::TestBackend, Terminal};

fn missing_metrics() -> App {
    let mut app = App::new();
    app.snap.ts = crate::collect::now_secs();
    app.snap.prom_ok = true;
    app.snap.accel.push(Accel {
        kind: AccelKind::Gpu,
        model: "H100".into(),
        id: "gpu0".into(),
        node: "node-a".into(),
        util: f64::NAN,
        mem_used_gb: f64::NAN,
        mem_total_gb: f64::NAN,
        temp: f64::NAN,
        power: f64::NAN,
        busy_model: String::new(),
        alive: true,
        throttle: f64::NAN,
        unified_mem: false,
        mem_bw: f64::NAN,
        clock_mhz: f64::NAN,
        mem_temp: f64::NAN,
        energy_mj: f64::NAN,
    });
    app.snap.nodes.push(NodeInfo {
        name: "node-a".into(),
        cpu_pct: f64::NAN,
        mem_used_gb: f64::NAN,
        mem_total_gb: f64::NAN,
        disk_used_gb: f64::NAN,
        disk_total_gb: f64::NAN,
        load1: f64::NAN,
        ready: true,
        ..Default::default()
    });
    app
}

fn render(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| draw(f, app, &mut FxState::disabled()))
        .unwrap();
    let buf = terminal.backend().buffer();
    (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn missing_metrics_render_across_views_and_sizes() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_theme = crate::app::theme();
    let mut app = missing_metrics();
    for theme in 0..crate::app::N_THEMES {
        crate::app::set_theme(theme);
        for view in View::EVERY {
            app.view = view;
            for detail in [false, true] {
                app.detail = detail;
                for (width, height) in [(40, 12), (80, 24), (140, 45)] {
                    let text = render(&app, width, height);
                    assert!(
                        !text.contains("NaN"),
                        "{view:?} detail={detail} {width}x{height}:\n{text}"
                    );
                    assert!(!text.contains("inf%"), "{view:?}:\n{text}");
                }
            }
        }
    }
    crate::app::set_theme(original_theme);
}

#[test]
fn unknown_gauges_are_distinct_from_measured_zero() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(format!("{:>4.0}", Metric(value)), "   –");
        assert_eq!(dot_bar(value, 4, C_OK()).to_string(), "————");
        assert_eq!(bar_line(value, 4, C_OK()).to_string(), "————");
        assert!(percent(value, 100.0).is_nan());
    }
    assert_eq!(percent(0.0, 100.0), 0.0);
    assert!(percent(0.0, 0.0).is_nan());
    assert_eq!(format!("{:>4.0}", Metric(0.0)), "   0");
    assert_eq!(dot_bar(0.0, 4, C_OK()).to_string(), "····");
}

#[test]
fn missing_throttle_sample_is_not_healthy() {
    let mut app = missing_metrics();
    let device = &mut app.snap.accel[0];
    device.kind = AccelKind::Rngd;
    assert_eq!(device.throttling(), None);
    device.throttle = 0.0;
    assert_eq!(device.throttling(), Some(false));
    device.throttle = 1.0;
    assert_eq!(device.throttling(), Some(true));
}

#[test]
fn incomplete_slo_is_rendered_as_unknown_instead_of_met() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = missing_metrics();
    app.view = View::Perf;
    app.objectives.insert(
        "model".into(),
        crate::ops::Objective {
            ttft_ms: Some(200.0),
            tpot_ms: Some(50.0),
            ..Default::default()
        },
    );
    app.snap.perf_rows.push(crate::collect::PerfRow {
        model: "model".into(),
        req: 1.0,
        tps: 100.0,
        ttft_p95: 0.1,
        tpot_p95: f64::NAN,
        e2e_p95: 1.0,
        in_tok_p95: f64::NAN,
        out_tok_p95: f64::NAN,
        queue_p95: f64::NAN,
        prefill_p95: f64::NAN,
        decode_p95: f64::NAN,
        preempt: f64::NAN,
    });
    let text = render(&app, 160, 45);
    assert!(text.contains("? TPOT"), "{text}");
    assert!(!text.contains("meets objective"), "{text}");
    app.snap.sources.insert(
        "performance".into(),
        crate::quality::SourceStatus::observed(crate::collect::now_secs(), 0, &["timeout".into()]),
    );
    let text = render(&app, 160, 45);
    assert!(text.contains("stale or failed"), "{text}");
    assert!(!text.contains("meets objective"), "{text}");
}

#[test]
fn compile_forms_render_missing_data_across_terminal_sizes() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = missing_metrics();
    for vendor in ["rbln", "furiosa"] {
        assert!(app
            .plan_compile_for_model("org/Llama-8B", vendor, &[("tp".into(), "0".into())])
            .is_err());
        let form = app.compile_form.as_mut().unwrap();
        form.cursor = form.fields.len() - 1;
        let selected_label = form.fields[form.cursor].label.clone();
        for (width, height) in [(20, 8), (40, 12), (80, 24), (140, 45)] {
            let text = render(&app, width, height);
            assert!(
                text.contains(&selected_label),
                "selected field hidden at {width}x{height}: {text}"
            );
            assert!(!text.contains("NaN"), "{vendor} {width}x{height}: {text}");
        }
    }
}

#[test]
fn deploy_form_keeps_selected_field_visible_after_validation_error() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = missing_metrics();
    assert!(app
        .plan_deploy_for_model("org/model", "gpu", &[("replicas".into(), "invalid".into())])
        .is_err());
    let form = app.deploy_form.as_mut().unwrap();
    form.cursor = form.fields.len() - 1;
    let label = form.fields[form.cursor].label.clone();
    for (width, height) in [(20, 8), (40, 12), (80, 24), (140, 45)] {
        let text = render(&app, width, height);
        assert!(
            text.contains(&label),
            "selected field hidden at {width}x{height}: {text}"
        );
        assert!(!text.contains("NaN"));
    }
}

#[test]
fn epp_profile_switch_and_missing_configuration_render_honestly() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = App::new();
    app.view = View::Epp;
    app.snap.epp = crate::epp_config::parse("schedulingProfiles:\n- name: alpha\n  plugins: [{pluginRef: queue-scorer}]\n- name: beta\n  plugins: [{pluginRef: max-score-picker}]\n");
    let first = render(&app, 160, 40);
    assert!(first.contains("profile: alpha"));
    app.cycle_epp_profile();
    let second = render(&app, 160, 40);
    assert!(second.contains("profile: beta"));
    assert!(second.contains("max-score-picker"));
    assert!(second.contains("profile attribution unavailable"));
    for (width, height) in [(40, 12), (80, 24), (120, 32)] {
        assert!(!render(&app, width, height).contains("NaN"));
    }
    app.snap.epp = None;
    app.snap.sources.insert(
        "epp_config".into(),
        crate::quality::SourceStatus::observed(
            crate::collect::now_secs(),
            0,
            &["malformed configuration".into()],
        ),
    );
    let failed = render(&app, 160, 40);
    assert!(failed.contains("configuration unavailable"));
    assert!(!failed.contains("ConfigMap not found"));
}

#[test]
fn latency_detail_shows_query_age_and_stale_refresh_guidance() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = App::new();
    app.view = View::Perf;
    app.detail = true;
    app.snap.stale_after_secs = 30;
    let now = crate::collect::now_secs();
    app.perf_detail = Some(crate::collect::PerfDetail {
        namespace: "ns".into(),
        service: "model-service".into(),
        model: "model".into(),
        checked_at: Some(now.saturating_sub(120)),
        e2e: [0.1, 0.2, 0.3],
        ..crate::collect::PerfDetail::default()
    });
    let text = render(&app, 120, 32);
    assert!(text.contains("stale · checked"));
    assert!(text.contains("Enter refresh"));
    assert!(text.contains("ns/model-service"));
    for (width, height) in [(40, 12), (80, 24)] {
        assert!(!render(&app, width, height).contains("NaN"));
    }
    app.perf_detail.as_mut().unwrap().checked_at = Some(crate::collect::now_secs());
    assert!(!render(&app, 120, 32).contains("stale · checked"));
    app.perf_detail.as_mut().unwrap().checked_at = None;
    assert!(render(&app, 120, 32).contains("query time unknown"));
}

#[test]
fn manifest_target_review_is_scrollable_and_omits_resource_data() {
    let mut app = App::new();
    let body = r#"{"apiVersion":"v1","kind":"Secret","metadata":{"name":"credentials","namespace":"ns","uid":"reviewed-uid","resourceVersion":"42"},"data":{"token":"PRIVATE-PAYLOAD"}}"#;
    let plan = crate::manifest_plan::Reviewed::new("ns", "source", vec![("replace", body.into())]);
    app.confirm = Some(crate::app::Pending::Apply {
        title: "test".into(),
        yaml: "source".into(),
        required: crate::app::Mode::Admin,
        reviewed: Some(plan),
    });
    app.manifest_review_details = true;
    for (width, height) in [(40, 12), (80, 24), (120, 32)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| confirm_overlay(frame, &app)).unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(!text.contains("PRIVATE-PAYLOAD"));
        let pending = app.confirm.as_ref().unwrap();
        let details = manifest_review_text(pending, None);
        assert!(details.contains("reviewed-uid") && details.contains("42"));
        assert!(!details.contains("PRIVATE-PAYLOAD"));
        app.manifest_review_scroll = manifest_review_scroll_max(pending, width, height, None);
        terminal.draw(|frame| confirm_overlay(frame, &app)).unwrap();
    }
}

#[test]
fn routing_displays_distinct_conditions_for_identical_paths() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new();
    app.view = View::Routing;
    for (index, method) in ["GET", "POST"].iter().enumerate() {
        let condition = serde_json::json!({"path":{"value":"/same"},"method":method});
        app.snap.routes.push(crate::collect::Route {
            configuration: None,
            identity: None,
            path: "/same".into(),
            backend: "public-api".into(),
            kind: "Service".into(),
            route: "tenant-route".into(),
            match_summary: crate::routing::match_summary(&condition, 0, index),
            backend_summary: String::new(),
            single_local_backend: true,
        });
    }
    let text = render(&app, 140, 45);
    for expected in [
        "tenant-route",
        "rule 1 / match 1",
        "rule 1 / match 2",
        "GET",
        "POST",
        "PathPrefix",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    for (width, height) in [(40, 12), (80, 24)] {
        let text = render(&app, width, height);
        assert!(!text.contains("NaN"));
    }
}

#[test]
fn routing_pod_tree_uses_verified_membership_instead_of_prefixes() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new();
    app.view = View::Routing;
    app.snap.routes.push(crate::collect::Route {
        configuration: None,
        identity: None,
        path: "/a".into(),
        backend: "public-api".into(),
        kind: "Service".into(),
        route: "route".into(),
        match_summary: String::new(),
        backend_summary: String::new(),
        single_local_backend: true,
    });
    for name in ["unrelated-worker", "public-api-imposter"] {
        app.snap.pods.push(crate::collect::PodRow {
            identity: None,
            name: name.into(),
            phase: "Running".into(),
            ready: "1/1".into(),
            node: "node".into(),
            restarts: 0,
            age_secs: 1,
        });
    }
    crate::routing::fresh_relationship_sources(&mut app.snap);
    app.snap
        .service_bindings
        .insert("deployment".into(), vec!["public-api".into()]);
    app.snap
        .deployment_pods
        .insert("deployment".into(), vec!["unrelated-worker".into()]);
    let text = render(&app, 140, 45);
    assert!(text.contains("unrelated-worker"), "{text}");
    assert!(!text.contains("public-api-imposter"), "{text}");
    app.snap.service_bindings.clear();
    let text = render(&app, 140, 45);
    assert!(!text.contains("unrelated-worker"));
    assert!(text.contains("model relation unknown or shared"));
    assert!(!text.contains("(no serving)"));
}

#[test]
fn terminal_busy_count_agrees_with_json_for_missing_and_boundary_samples() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = missing_metrics();
    for (util, label, count) in [
        (f64::NAN, "accel —/1 busy", None),
        (7.0, "accel 0/1 busy", Some(0)),
        (10.0, "accel 0/1 busy", Some(0)),
        (10.1, "accel 1/1 busy", Some(1)),
    ] {
        app.snap.accel[0].util = util;
        let screen = render(&app, 220, 45);
        assert!(screen.contains(label), "{screen}");
        let json: serde_json::Value = serde_json::from_str(
            &crate::agent::to_json(&app.snap, &crate::config::Config::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(json["cluster"]["busy"].as_u64(), count);
    }
}

#[test]
fn nonfinite_hardware_metrics_never_render_as_numbers_in_any_view() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = missing_metrics();
    for invalid in [f64::INFINITY, f64::NEG_INFINITY] {
        let a = &mut app.snap.accel[0];
        a.util = invalid;
        a.mem_used_gb = invalid;
        a.mem_total_gb = invalid;
        a.temp = invalid;
        a.power = invalid;
        a.mem_bw = invalid;
        a.clock_mhz = invalid;
        a.mem_temp = invalid;
        let node = &mut app.snap.nodes[0];
        node.cpu_pct = invalid;
        node.mem_used_gb = invalid;
        node.mem_total_gb = invalid;
        node.disk_used_gb = invalid;
        node.disk_total_gb = invalid;
        node.load1 = invalid;
        app.snap.prefix_idx = invalid;
        for view in View::EVERY {
            app.view = view;
            for detail in [false, true] {
                app.detail = detail;
                let text = render(&app, 160, 50);
                for bad in [
                    "NaN", "inf%", "infG", "infW", "inf°", "infMHz", " inf ", " inf·",
                ] {
                    assert!(
                        !text.contains(bad),
                        "{view:?} detail={detail} contains {bad}:\n{text}"
                    );
                }
            }
        }
    }
}

/// Prometheus outage: the Overview must say the fleet is unknown, never draw a measured zero.
/// (2026-10-14: the card read "0 accel │ util 0% temp 0°C ⚡0W" while the cluster had 11 devices.)
#[test]
fn prometheus_outage_overview_says_unknown_not_zero() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new();
    app.snap.ts = crate::collect::now_secs();
    app.snap.stale_after_secs = 30;
    let down =
        "prom[RBLN_DEVICE_STATUS:UTILIZATI]: prometheus unreachable (probe failed this cycle)";
    for id in ["rbln", "furiosa", "gpu"] {
        app.snap.sources.insert(
            format!("accel/{id}"),
            crate::quality::SourceStatus::observed(app.snap.ts, 0, &[down.into()]),
        );
    }
    let text = render(&app, 140, 40);
    assert!(text.contains("accel unknown"), "{text}");
    assert!(text.contains("prometheus unreachable"), "{text}");
    assert!(
        !text.contains("util 0%"),
        "unknown must not render as 0%:\n{text}"
    );
    assert!(
        !text.contains("0°C"),
        "unknown must not render as 0°C:\n{text}"
    );
}

/// Exporter evicted under DiskPressure: the vendor's devices are listed as not reporting.
#[test]
fn evicted_vendor_devices_are_listed_not_dropped() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new();
    app.snap.ts = crate::collect::now_secs();
    app.snap.stale_after_secs = 30;
    for id in ["rbln", "furiosa", "gpu"] {
        app.snap.sources.insert(
            format!("accel/{id}"),
            crate::quality::SourceStatus::observed(app.snap.ts, 0, &[]),
        );
    }
    app.snap.nodes.push(NodeInfo {
        name: "etri-001".into(),
        ready: true,
        pressure: true,
        pressure_kinds: vec!["DiskPressure".into()],
        devices: vec![crate::collect::DeviceExpectation {
            label: "RBLN",
            declared: 4,
            allocatable: None,
        }],
        ..Default::default()
    });
    let text = render(&app, 140, 40);
    assert!(
        text.contains("etri-001: RBLN 0/4 devices reporting — DiskPressure"),
        "status band names the gap and its cause:\n{text}"
    );
    assert!(
        text.contains("0/4 reporting"),
        "accelerator panel lists it:\n{text}"
    );
}

/// A crash-looping system pod outside the serving namespace: the status band and the Pods view
/// show the log line and the fix, not a restart count.
#[test]
fn crash_loop_shows_its_cause_and_fix() {
    let _guard = RENDER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new();
    app.snap.ts = crate::collect::now_secs();
    app.snap.pod_problems.push(crate::pod_health::PodProblem {
        namespace: "rbln-system".into(),
        pod: "rbln-npu-operator-controller-manager-x".into(),
        container: "manager".into(),
        init: false,
        node: "gx10-4744".into(),
        state: "CrashLoopBackOff".into(),
        restarts: 40,
        exit_code: Some(255),
        image: "rebellions/rbln-npu-operator:v0.2.0".into(),
        floating_tag: None,
        cause: Some("exec /manager: exec format error".into()),
        hint: Some("image has no arm64 build — pin nodeSelector kubernetes.io/arch".into()),
        cause_at_restart: 40,
    });
    let status = render(&app, 160, 40);
    assert!(
        status.contains("CrashLoopBackOff ×40 — exec /manager: exec format error"),
        "status band leads with the failing container:\n{status}"
    );
    app.view = View::Pods;
    let pods = render(&app, 200, 40);
    assert!(
        pods.contains("Failing containers (all namespaces)")
            && pods.contains("image has no arm64 build"),
        "Pods view lists it with the fix:\n{pods}"
    );
}
