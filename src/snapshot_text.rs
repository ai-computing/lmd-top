//! Pure headless presentation, independent of collection, filesystem and stdout.
use crate::{collect, config::Config};
use std::fmt::Write;

/// Missing metric → em dash, matching how the TUI and `--json` (null) report absence.
/// The headless text used to print a raw `NaN`, which reads as a broken value rather than
/// "this node has no node-exporter" (BUG-14).
fn dash(v: f64) -> String {
    if !v.is_finite() {
        "—".to_string()
    } else {
        format!("{:.2}", v)
    }
}

/// Same, at a caller-chosen precision.
fn dashp(v: f64, prec: usize) -> String {
    if !v.is_finite() {
        "—".to_string()
    } else {
        format!("{:.*}", prec, v)
    }
}

fn snapshot_routes(s: &collect::Snapshot) -> String {
    use std::fmt::Write;
    let mut text = format!(
        "\n[routes] {} ({})\n",
        s.routes.len(),
        crate::routing::evidence(s).summary()
    );
    for route in &s.routes {
        writeln!(
            text,
            "  {} · {} → {}/{}",
            route.route, route.path, route.kind, route.backend
        )
        .unwrap();
        if !route.match_summary.is_empty() {
            writeln!(text, "    {}", route.match_summary).unwrap();
        }
        if !route.backend_summary.is_empty() {
            writeln!(text, "    {}", route.backend_summary).unwrap();
        }
    }
    text
}

pub fn render(s: &collect::Snapshot, cfg: &Config, hist: &[crate::history::Record]) -> String {
    let mut output = String::new();
    macro_rules! line {
        ($($args:tt)*) => { writeln!(output, $($args)*).unwrap() };
    }
    line!("== lmd-top snapshot (prom={}, ns={}) ==", cfg.prom, cfg.ns);
    let now = crate::collect::now_secs();
    line!(
        "[sources] missing source metadata means unknown; retained values are not current health"
    );
    for (name, status) in &s.sources {
        let age = status
            .last_success_at
            .and_then(|at| now.checked_sub(at))
            .map(|seconds| format!("{seconds}s"))
            .unwrap_or_else(|| "unknown".into());
        let success = status
            .last_success_at
            .map(|at| at.to_string())
            .unwrap_or_else(|| "unknown".into());
        line!(
            "  {}: {} samples={} last_success={} age={}",
            name,
            status.state(now, s.stale_after_secs.max(10)).as_str(),
            status.samples,
            success,
            age
        );
        if let Some(error) = &status.error {
            line!("    error: {}", error.replace(['\n', '\r'], " "));
        }
    }

    line!(
        "gateway: {} {} (observation: {})",
        if s.gw_addr.is_empty() {
            "—"
        } else {
            &s.gw_addr
        },
        if s.gw_ok { "Programmed" } else { "" },
        s.sources
            .get("gateway")
            .map(|status| status.state(now, s.stale_after_secs.max(10)))
            .unwrap_or(crate::quality::Quality::Unknown)
            .as_str()
    );
    line!("\n[nodes] {}", s.nodes.len());
    for n in &s.nodes {
        line!(
            "  {:<24} load1 {:>5}  mem {}/{}G",
            n.name,
            dash(n.load1),
            dashp(n.mem_used_gb, 0),
            dashp(n.mem_total_gb, 0)
        );
    }
    match crate::coverage::unknown_reason(s, now) {
        Some(reason) if s.accel.is_empty() => line!("\n[accelerators] unknown — {reason}"),
        Some(reason) => line!("\n[accelerators] {} (partial: {reason})", s.accel.len()),
        None => line!("\n[accelerators] {}", s.accel.len()),
    }
    for gap in crate::coverage::gaps(s, now) {
        line!("  ✗ {}", crate::coverage::gap_line(&gap));
    }
    for a in &s.accel {
        line!(
            "  {:<5} {:<6} {:<16} util {:>5}%  mem {}/{}G  {}°C {}W  {}",
            a.disp(),
            a.id,
            a.node,
            dashp(a.util, 1),
            dashp(a.mem_used_gb, 0),
            dashp(a.mem_total_gb, 0),
            dashp(a.temp, 0),
            dashp(a.power, 0),
            a.busy_model
        );
    }
    line!("\n[inference pools] {}", s.pools.len());
    for p in &s.pools {
        line!(
            "  {:<16} ready {}  queue {}  kv {}  sat {}",
            p.name,
            dashp(p.ready, 0),
            dashp(p.queue, 1),
            dash(p.kv),
            dash(p.sat)
        );
    }
    if let Some(e) = &s.epp {
        line!(
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
    line!("\n[models] {}", s.models.len());
    for m in &s.models {
        line!(
            "  {:<24} {}/{}  run={} wait={} tps={}  path={}  {}",
            m.name,
            m.ready,
            m.desired,
            m.running.map(|x| dashp(x, 0)).unwrap_or("-".into()),
            m.waiting.map(|x| dashp(x, 0)).unwrap_or("-".into()),
            m.tps.map(|x| dashp(x, 0)).unwrap_or("-".into()),
            if m.route.is_empty() { "-" } else { &m.route },
            m.status
        );
    }
    output.push_str(&snapshot_routes(s));
    if !s.objectives.is_empty() {
        line!(
            "[SLO] {}",
            s.objectives
                .iter()
                .map(|o| format!("{}(p{})", o.name, o.priority))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    line!("\n[pods] {}", s.pods.len());
    for p in &s.pods {
        line!(
            "  {:<40} {:<8} {} {} restarts={}",
            p.name,
            p.phase,
            p.ready,
            p.node,
            p.restarts
        );
    }
    if !s.trainings.is_empty() {
        line!("\n[training] {} npu-lora run(s)", s.trainings.len());
        for t in &s.trainings {
            let opt = |v: Option<f64>, p: usize| {
                v.map(|x| format!("{x:.p$}")).unwrap_or_else(|| "-".into())
            };
            line!(
                "  {:<28} {:<9} step {}/{} loss {} {} tok/s  {}",
                t.run,
                t.status,
                opt(t.step, 0),
                opt(t.steps_planned, 0),
                opt(t.loss, 4),
                opt(t.tokens_per_s, 0),
                t.model
            );
        }
    }
    // Compile-option advice from recorded history, for the models currently in the store.
    if !hist.is_empty() {
        line!("\n[history] {} record(s)", hist.len());
        let mut seen = std::collections::BTreeSet::new();
        for r in hist.iter().rev() {
            if !seen.insert((r.model.clone(), r.vendor.clone())) {
                continue;
            }
            let advice = crate::advisor::advise(hist, &r.model, &r.vendor);
            let line = advice.line();
            if !line.is_empty() {
                line!("  {} / {}: {}", r.model, r.vendor, line);
            }
        }
    }
    if !s.warnings.is_empty() {
        line!("\n[warnings] {}", s.warnings.len());
        for w in &s.warnings {
            line!("  ! {}", w);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_states_explain_retained_gateway_values_and_future_timestamps() {
        let now = crate::collect::now_secs();
        let mut snapshot = crate::collect::Snapshot {
            gw_ok: true,
            gw_addr: "gateway".into(),
            ..Default::default()
        };
        let successful = crate::quality::SourceStatus::observed(now, 1, &[]);
        let failed =
            crate::quality::SourceStatus::observed(now, 0, &["Forbidden\nretry read".into()])
                .after(Some(&successful));
        snapshot.sources.insert("gateway".into(), failed);
        snapshot.sources.insert(
            "pods".into(),
            crate::quality::SourceStatus::observed(now, 0, &[]),
        );
        snapshot.sources.insert(
            "nodes".into(),
            crate::quality::SourceStatus::observed(now + 3600, 1, &[]),
        );
        let text = super::render(&snapshot, &crate::config::Config::default(), &[]);
        for expected in [
            "gateway: stale",
            "pods: empty",
            "nodes: unknown",
            "age=unknown",
            "error: Forbidden retry read",
            "Programmed (observation: stale)",
        ] {
            assert!(text.contains(expected), "{text}");
        }
    }

    #[test]
    fn complete_snapshot_renders_missing_metrics_routes_and_warnings_offline() {
        let mut snapshot = crate::collect::Snapshot::default();
        snapshot.nodes.push(crate::collect::NodeInfo {
            name: "node-a".into(),
            load1: f64::INFINITY,
            mem_used_gb: f64::NAN,
            mem_total_gb: 64.0,
            ..Default::default()
        });
        snapshot.warnings.push("metrics query failed".into());
        let input = serde_json::json!({"items":[{"metadata":{"name":"public-route"},"spec":{"rules":[{"backendRefs":[{"name":"backend"}]}]}}]});
        (snapshot.routes, snapshot.epp_in_path) = crate::routing::rows(&input);
        let output = super::render(&snapshot, &crate::config::Config::default(), &[]);
        for section in [
            "[nodes] 1",
            "[accelerators] 0",
            "[inference pools] 0",
            "[models] 0",
            "[routes] 1",
            "[pods] 0",
            "[warnings] 1",
        ] {
            assert!(output.contains(section), "{output}");
        }
        for expected in [
            "node-a",
            "mem —/64G",
            "public-route",
            "implicit path",
            "backendRefs",
            "metrics query failed",
            "runtime unverified",
        ] {
            assert!(output.contains(expected), "{output}");
        }
        assert!(!output.contains("NaN"));
        assert!(!output.contains("load1   inf"));
        assert!(!output.contains("[history]"));
    }

    #[test]
    fn snapshot_metrics_hide_all_nonfinite_values_and_keep_real_zero() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(super::dash(value), "—");
            assert_eq!(super::dashp(value, 0), "—");
        }
        assert_eq!(super::dash(0.0), "0.00");
        assert_eq!(super::dashp(1.25, 1), "1.2");
    }

    #[test]
    fn snapshot_routes_expose_conditions_and_do_not_claim_runtime_routing() {
        let input = serde_json::json!({"items":[{"metadata":{"name":"route"},"spec":{"rules":[
            {"matches":[{"method":"POST"}],"backendRefs":[{"name":"pool","kind":"InferencePool","group":"inference.networking.k8s.io","weight":2}]}
        ]}}]});
        let mut snapshot = crate::collect::Snapshot::default();
        (snapshot.routes, snapshot.epp_in_path) = crate::routing::rows(&input);
        let unknown = super::snapshot_routes(&snapshot);
        assert!(unknown.contains("Route configuration unknown"));
        crate::routing::fresh_relationship_sources(&mut snapshot);
        let text = super::snapshot_routes(&snapshot);
        for expected in [
            "Pool backend configured; runtime unverified",
            "route",
            "POST",
            "implicit path",
            "weight",
            "inference.networking.k8s.io",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        assert!(!text.contains("epp_in_path=true"));
    }
}
