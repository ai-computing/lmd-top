//! Accelerator coverage: is the fleet we draw the fleet that exists?
//!
//! Two failures used to look like an idle or empty cluster:
//! * the metrics source is down, so every accelerator reads as zero or "no accelerators";
//! * one vendor's exporter or device plugin is gone (evicted under DiskPressure, crash-looping),
//!   so that vendor's devices silently disappear while everything else looks healthy.
//!
//! Both are answered here from data the collectors already have: per-source quality for the
//! first, and node labels (which outlive the evicted pods) against exporter samples for the
//! second. Every renderer — Overview, the accelerator panel, `--snapshot`, `--json` — asks this
//! module instead of interpreting an empty `accel` list itself.
use crate::collect::Snapshot;
use crate::quality::Quality;
use serde::Serialize;

/// Why the accelerator list cannot be trusted as a statement of what exists, if it cannot.
///
/// `None` from [`unknown_reason`] means at least the sources that answered answered
/// successfully; an empty list is then a real "no accelerators reported".
pub fn unknown_reason(s: &Snapshot, now: u64) -> Option<String> {
    let max_age = s.stale_after_secs.max(10);
    let failed: Vec<(&str, &crate::quality::SourceStatus)> = s
        .sources
        .iter()
        .filter(|(k, _)| k.starts_with("accel/"))
        .filter(|(_, st)| matches!(st.state(now, max_age), Quality::Error | Quality::Stale))
        .map(|(k, st)| (k.trim_start_matches("accel/"), st))
        .collect();
    if failed.is_empty() {
        return None;
    }
    let total = s.sources.keys().filter(|k| k.starts_with("accel/")).count();
    let cause = failed
        .iter()
        .find_map(|(_, st)| st.error.as_deref())
        .map(root_cause)
        .unwrap_or("source stale");
    if failed.len() == total {
        Some(format!("accelerator metrics unavailable — {cause}"))
    } else {
        let names: Vec<&str> = failed.iter().map(|(k, _)| *k).collect();
        Some(format!(
            "{} metrics unavailable — {cause}",
            names.join(", ")
        ))
    }
}

/// The part of a collector error that names the cause, without per-query prefixes.
fn root_cause(error: &str) -> &str {
    let first = error.split("; ").next().unwrap_or(error);
    let first = first.split(" ×").next().unwrap_or(first);
    match first.find("]: ") {
        Some(i) if first.starts_with("prom[") => &first[i + 3..],
        _ => first,
    }
}

/// A node declares devices of one family that the exporters do not report.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Gap {
    pub node: String,
    /// Pack label: `RBLN`, `RNGD`, `GPU`.
    pub family: &'static str,
    pub declared: u32,
    pub reporting: u32,
    /// Devices the scheduler can hand out; `None` when the node no longer lists the resource.
    pub allocatable: Option<u32>,
    /// Most likely reason, from node state and the device plugin's view.
    pub cause: String,
}

/// Declared-vs-reporting shortfalls, per node and family.
///
/// A family whose metrics source is itself failing is skipped: that is [`unknown_reason`]'s
/// statement to make, and calling every device of it "missing" would bury the real cause.
pub fn gaps(s: &Snapshot, now: u64) -> Vec<Gap> {
    let max_age = s.stale_after_secs.max(10);
    let mut out = Vec::new();
    for node in &s.nodes {
        for want in &node.devices {
            let source_ok = crate::accel::packs()
                .iter()
                .find(|p| p.label == want.label)
                .and_then(|p| s.sources.get(&format!("accel/{}", p.id)))
                .is_some_and(|st| {
                    matches!(st.state(now, max_age), Quality::Fresh | Quality::Empty)
                });
            if !source_ok {
                continue;
            }
            let reporting = s
                .accel
                .iter()
                .filter(|a| a.node == node.name && a.kind.label() == want.label)
                .count() as u32;
            if reporting >= want.declared {
                continue;
            }
            out.push(Gap {
                node: node.name.clone(),
                family: want.label,
                declared: want.declared,
                reporting,
                allocatable: want.allocatable,
                cause: cause(node, want),
            });
        }
    }
    out
}

fn cause(node: &crate::collect::NodeInfo, want: &crate::collect::DeviceExpectation) -> String {
    if !node.ready {
        return "node NotReady".into();
    }
    if !node.pressure_kinds.is_empty() {
        return format!(
            "{} — vendor pods likely evicted",
            node.pressure_kinds.join("+")
        );
    }
    match want.allocatable {
        None | Some(0) => "device plugin not advertising the resource".into(),
        Some(n) if n < want.declared => {
            format!(
                "device plugin advertises {n}/{} — check the cards",
                want.declared
            )
        }
        Some(_) => "metrics exporter not reporting these devices".into(),
    }
}

/// One line for status bands and diagnosis.
pub fn gap_line(g: &Gap) -> String {
    format!(
        "{}: {} {}/{} devices reporting — {}",
        g.node, g.family, g.reporting, g.declared, g.cause
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{Accel, AccelKind, DeviceExpectation, NodeInfo};
    use crate::quality::SourceStatus;

    fn node(name: &str, devices: Vec<DeviceExpectation>) -> NodeInfo {
        NodeInfo {
            name: name.into(),
            ready: true,
            devices,
            ..Default::default()
        }
    }
    fn rbln(declared: u32, allocatable: Option<u32>) -> DeviceExpectation {
        DeviceExpectation {
            label: "RBLN",
            declared,
            allocatable,
        }
    }
    fn device(node: &str, kind: AccelKind, id: &str) -> Accel {
        Accel {
            kind,
            model: String::new(),
            id: id.into(),
            node: node.into(),
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
        }
    }
    fn snap() -> Snapshot {
        let mut s = Snapshot {
            stale_after_secs: 30,
            ..Default::default()
        };
        for id in ["rbln", "furiosa", "gpu"] {
            s.sources
                .insert(format!("accel/{id}"), SourceStatus::observed(100, 0, &[]));
        }
        s
    }

    #[test]
    fn prometheus_outage_is_unknown_not_zero() {
        let mut s = snap();
        let down =
            "prom[RBLN_DEVICE_STATUS:UTILIZATI]: prometheus unreachable (probe failed this cycle)";
        for id in ["rbln", "furiosa", "gpu"] {
            s.sources.insert(
                format!("accel/{id}"),
                SourceStatus::observed(100, 0, &[down.into()]),
            );
        }
        assert_eq!(
            unknown_reason(&s, 100).as_deref(),
            Some("accelerator metrics unavailable — prometheus unreachable (probe failed this cycle)")
        );
        // A failing source must not also be reported as a device shortfall.
        s.nodes.push(node("etri-001", vec![rbln(4, Some(4))]));
        assert!(gaps(&s, 100).is_empty());
    }

    #[test]
    fn one_failing_vendor_is_named() {
        let mut s = snap();
        s.sources.insert(
            "accel/rbln".into(),
            SourceStatus::observed(100, 0, &["prom[x]: timeout".into()]),
        );
        assert_eq!(
            unknown_reason(&s, 100).as_deref(),
            Some("rbln metrics unavailable — timeout")
        );
    }

    #[test]
    fn healthy_empty_cluster_is_not_unknown() {
        assert_eq!(unknown_reason(&snap(), 100), None);
    }

    #[test]
    fn evicted_exporter_under_disk_pressure_is_a_gap_with_its_cause() {
        // 2026-10-14: Prometheus fine, RNGD reporting, every RBLN pod evicted by DiskPressure.
        let mut s = snap();
        let mut n = node(
            "etri-001",
            vec![
                rbln(4, None),
                DeviceExpectation {
                    label: "RNGD",
                    declared: 2,
                    allocatable: Some(2),
                },
            ],
        );
        n.pressure_kinds = vec!["DiskPressure".into()];
        n.pressure = true;
        s.nodes.push(n);
        s.accel.push(device("etri-001", AccelKind::Rngd, "npu0"));
        s.accel.push(device("etri-001", AccelKind::Rngd, "npu1"));
        let found = gaps(&s, 100);
        assert_eq!(found.len(), 1, "RNGD is complete, only RBLN is missing");
        assert_eq!(
            gap_line(&found[0]),
            "etri-001: RBLN 0/4 devices reporting — DiskPressure — vendor pods likely evicted"
        );
    }

    #[test]
    fn cause_distinguishes_plugin_from_exporter() {
        let mut s = snap();
        s.nodes.push(node("a", vec![rbln(4, None)]));
        s.nodes.push(node("b", vec![rbln(4, Some(4))]));
        s.nodes.push(node("c", vec![rbln(4, Some(2))]));
        let causes: Vec<String> = gaps(&s, 100).into_iter().map(|g| g.cause).collect();
        assert_eq!(
            causes,
            vec![
                "device plugin not advertising the resource",
                "metrics exporter not reporting these devices",
                "device plugin advertises 2/4 — check the cards",
            ]
        );
    }

    #[test]
    fn full_report_and_extra_devices_are_not_gaps() {
        let mut s = snap();
        s.nodes.push(node("a", vec![rbln(1, Some(1))]));
        s.accel.push(device("a", AccelKind::Rbln, "rbln0"));
        s.accel.push(device("a", AccelKind::Rbln, "rbln1"));
        assert!(gaps(&s, 100).is_empty());
    }
}
