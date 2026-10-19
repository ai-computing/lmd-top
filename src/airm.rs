//! ai-rm protocol v0.1 — read-only consumer of the formats rm-core defines.
//!
//! lmd-top is Rust, so it does not import rm-core; it reads what rm-core writes:
//! - `AcceleratorProfile` (`airm.ai-computing.io/v1alpha1`, cluster-scoped) — one per device,
//!   published by the node daemon (`rm_core.k8s.accelerator_profile`).
//! - Prometheus series `airm_<name>` for the names in `rm_core.metrics.METRICS`.
//! - ConfigMap `airm-placement` — placement decisions (`rm_core.workload.PlacementDecision`).
//!
//! Every source is optional. A cluster without the CRD, the series or the ConfigMap shows exactly
//! what it showed before: absence is `None`/empty, never a warning.

use crate::collect::{Accel, AccelKind};
use crate::prom::Series;
use std::collections::BTreeMap;

pub const PROFILE_RESOURCE: &str = "acceleratorprofiles.airm.ai-computing.io";
pub const PLACEMENT_CONFIGMAP: &str = "airm-placement";
pub const PROM_PREFIX: &str = "airm_";

/// `rm_core.metrics.METRICS` (v0.1): name → unit. Display uses exactly these units.
pub const METRICS: &[(&str, &str)] = &[
    ("ttft_p50_ms", "ms"),
    ("ttft_p90_ms", "ms"),
    ("ttft_p99_ms", "ms"),
    ("tpot_p50_ms", "ms"),
    ("tpot_p90_ms", "ms"),
    ("tpot_p99_ms", "ms"),
    ("e2e_p90_ms", "ms"),
    ("tok_s", "tokens/s"),
    ("req_s", "requests/s"),
    ("power_w", "W"),
    ("energy_j", "J"),
    ("gflops_per_w", "GFLOPS/W"),
    ("mfu_pct", "%"),
    ("kv_transfer_ms", "ms"),
    ("kv_transfer_gbps", "GB/s"),
    ("accuracy_pct", "%"),
    ("slo_attainment_pct", "%"),
    ("accelerators", "count"),
];

/// `rm_core.metrics.TAG_KEYS` — the only labels shown next to a value.
pub const TAG_KEYS: &[&str] = &[
    "component",
    "model",
    "accelerator",
    "vendor",
    "scope",
    "peak_source",
    "dataset",
    "workload",
    "kv_transport",
    "run_id",
    "commit",
];

pub fn unit(name: &str) -> Option<&'static str> {
    METRICS.iter().find(|(n, _)| *n == name).map(|(_, u)| *u)
}

/// One `AcceleratorProfile` (spec only). Unknown values stay unknown: `peak_tflops` empty,
/// `power_limit_w` None.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String, // "<node>/<vendor>:<index>"
    pub node: String,
    pub vendor: String,
    pub model: String,
    pub memory_bytes: u64,
    pub memory_kind: String,
    pub peak_tflops: Vec<(String, f64)>, // precision → dense TFLOPS
    pub peak_source: String,
    pub power_limit_w: Option<f64>,
    pub interconnect: Vec<String>,
}

impl Profile {
    /// Device index from the id (`node/vendor:3` → 3).
    pub fn index(&self) -> Option<u32> {
        self.id.rsplit_once(':')?.1.parse().ok()
    }

    pub fn memory_gb(&self) -> f64 {
        self.memory_bytes as f64 / 1e9
    }

    /// "bf16 512 · fp8 1024 TF (datasheet)" — empty when no peak is published.
    pub fn peak_label(&self) -> String {
        if self.peak_tflops.is_empty() {
            return String::new();
        }
        let peaks: Vec<String> = self
            .peak_tflops
            .iter()
            .map(|(p, v)| format!("{p} {}", trim_num(*v)))
            .collect();
        let src = if self.peak_source.is_empty() {
            "source ?".to_string()
        } else {
            self.peak_source.clone()
        };
        format!("{} TF ({src})", peaks.join(" · "))
    }

    /// Does this profile describe that collected device? Matched on node, vendor family and
    /// device index — the profile id is `<node>/<vendor>:<index>`, lmd-top ids are `gpu0`,
    /// `rbln0`, `npu0`.
    pub fn matches(&self, a: &Accel) -> bool {
        let family = match a.kind {
            AccelKind::Gpu => "nvidia",
            AccelKind::Rbln => "rebellions",
            AccelKind::Rngd => "furiosa",
            AccelKind::Other(_) => return false,
        };
        let digits: String =
            a.id.chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect();
        let index = digits.chars().rev().collect::<String>().parse::<u32>().ok();
        self.vendor == family && self.node == a.node && index.is_some() && self.index() == index
    }
}

fn trim_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

/// Number written as a string (rm-core keeps exact values as strings) or as a JSON number.
fn num(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Parse one AcceleratorProfile object. Objects missing a required field are skipped, as
/// `rm_core.k8s.from_profile` would reject them.
pub fn parse_profile(obj: &serde_json::Value) -> Option<Profile> {
    let s = &obj["spec"];
    let text = |k: &str| s[k].as_str().map(str::to_string);
    let memory_bytes = num(&s["memoryBytes"]).filter(|v| *v > 0.0)? as u64;
    let mut peak_tflops: Vec<(String, f64)> = s["peakTflops"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| num(v).filter(|v| *v > 0.0).map(|v| (k.clone(), v)))
                .collect()
        })
        .unwrap_or_default();
    peak_tflops.sort_by(|a, b| a.0.cmp(&b.0));
    Some(Profile {
        id: text("id")?,
        node: text("node")?,
        vendor: text("vendor")?,
        model: text("model")?,
        memory_bytes,
        memory_kind: text("memoryKind")?,
        peak_tflops,
        peak_source: text("peakSource").unwrap_or_default(),
        power_limit_w: num(&s["powerLimitW"]),
        interconnect: s["interconnect"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// `kubectl get acceleratorprofiles -o json` → profiles, sorted by id.
pub fn parse_profiles(list: &serde_json::Value) -> Vec<Profile> {
    let mut out: Vec<Profile> = list["items"]
        .as_array()
        .map(|items| items.iter().filter_map(parse_profile).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// One placement decision: which accelerators a workload got, the KV path, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub workload: String,
    pub accelerator_ids: Vec<String>,
    pub kv_transport: String,
    pub reason: String,
}

/// Parse ConfigMap `airm-placement`. Two layouts are read:
/// - hetero-llm-d's documented one: key `decisions.json` holding
///   `{protocol, generation, placement, decisions: [{pool, workload: {kind, ...}, acceleratorIds,
///   kvTransport, reason}]}`; each decision is shown as `<pool>/<kind>`.
/// - one key per workload whose value is a `PlacementDecision`
///   `{accelerator_ids, kv_transport, reason}` (camelCase accepted too).
///
/// An empty ConfigMap or an empty `decisions` list (the v0.1 hetero-llm-d default) yields no
/// decisions. A value that does not parse is kept with the parse failure as its reason, so a bad
/// write stays visible.
pub fn parse_placement(cm: &serde_json::Value) -> Vec<Decision> {
    let Some(data) = cm["data"].as_object() else {
        return Vec::new();
    };
    let mut out: Vec<Decision> = Vec::new();
    for (key, raw) in data {
        let raw = raw.as_str().unwrap_or_default();
        let parsed: Option<serde_json::Value> = serde_json::from_str(raw)
            .ok()
            .or_else(|| serde_yaml::from_str(raw).ok())
            .filter(serde_json::Value::is_object);
        let Some(v) = parsed else {
            out.push(Decision {
                workload: key.clone(),
                accelerator_ids: Vec::new(),
                kv_transport: String::new(),
                reason: "unparsable decision".into(),
            });
            continue;
        };
        if let Some(list) = v.get("decisions").and_then(serde_json::Value::as_array) {
            for d in list.iter().filter(|d| d.is_object()) {
                let kind = d["workload"]["kind"].as_str().unwrap_or("?");
                let workload = match d["pool"].as_str() {
                    Some(pool) if !pool.is_empty() => format!("{pool}/{kind}"),
                    _ => kind.to_string(),
                };
                out.push(decision_from(workload, d));
            }
        } else {
            out.push(decision_from(key.clone(), &v));
        }
    }
    out.sort_by(|a, b| a.workload.cmp(&b.workload));
    out
}

fn decision_from(workload: String, v: &serde_json::Value) -> Decision {
    let field = |snake: &str, camel: &str| {
        v.get(snake)
            .or_else(|| v.get(camel))
            .cloned()
            .unwrap_or_default()
    };
    Decision {
        workload,
        accelerator_ids: field("accelerator_ids", "acceleratorIds")
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        kv_transport: field("kv_transport", "kvTransport")
            .as_str()
            .unwrap_or_default()
            .to_string(),
        reason: field("reason", "reason")
            .as_str()
            .unwrap_or_default()
            .to_string(),
    }
}

/// One `airm_*` sample, with only protocol tags kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: String, // protocol name, without the airm_ prefix
    pub value: f64,
    pub tags: BTreeMap<String, String>,
}

impl Sample {
    pub fn tag_label(&self) -> String {
        self.tags
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Result of `{__name__=~"airm_.+"}` → protocol samples. Series whose name is not in the v0.1
/// table are dropped (PROTOCOL §6: a metric exists only once rm-core lists it).
pub fn parse_metrics(series: Vec<Series>) -> Vec<Sample> {
    let mut out: Vec<Sample> = series
        .into_iter()
        .filter(|s| s.value.is_finite())
        .filter_map(|s| {
            let name = s.l("__name__").strip_prefix(PROM_PREFIX)?.to_string();
            unit(&name)?;
            let tags = s
                .labels
                .iter()
                .filter(|(k, _)| TAG_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Some(Sample {
                name,
                value: s.value,
                tags,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.tag_label().cmp(&b.tag_label()))
    });
    out
}

/// Everything lmd-top reads from ai-rm. `None` = source absent (no CRD / no ConfigMap).
#[derive(Clone, Default)]
pub struct Airm {
    pub profiles: Option<Vec<Profile>>,
    pub metrics: Vec<Sample>,
    pub placement: Option<Vec<Decision>>,
}

impl Airm {
    /// Is any ai-rm source present? When false the UI shows nothing ai-rm related.
    pub fn present(&self) -> bool {
        self.profiles.is_some() || !self.metrics.is_empty() || self.placement.is_some()
    }

    pub fn profile_for(&self, a: &Accel) -> Option<&Profile> {
        self.profiles.as_ref()?.iter().find(|p| p.matches(a))
    }

    /// Card power for a device from `airm_power_w{scope="card",accelerator=<profile id>}`.
    pub fn card_power(&self, a: &Accel) -> Option<f64> {
        let id = &self.profile_for(a)?.id;
        self.metrics
            .iter()
            .find(|s| {
                s.name == "power_w"
                    && s.tags.get("scope").map(String::as_str) == Some("card")
                    && s.tags.get("accelerator") == Some(id)
            })
            .map(|s| s.value)
    }
}

/// A row in the ai-rm metric panel: the airm value when reported, otherwise the vendor-derived
/// value lmd-top already collects (marked as such), otherwise absent.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricRow {
    pub name: &'static str,
    pub unit: &'static str,
    pub value: f64,
    pub tags: String,
    pub source: &'static str, // "airm" | "vendor"
}

/// Rows for the panel: every reported airm sample, then a vendor fallback for each of
/// `tok_s`, `req_s`, `power_w`, `accelerators` that has no airm sample. Vendor latency
/// percentiles (p95) are not mapped — the protocol has p50/p90/p99 only, and relabelling a
/// p95 as one of those would misstate it.
pub fn metric_rows(airm: &Airm, snap: &crate::collect::Snapshot) -> Vec<MetricRow> {
    let mut rows: Vec<MetricRow> = airm
        .metrics
        .iter()
        .filter_map(|s| {
            let (name, unit) = METRICS.iter().find(|(n, _)| *n == s.name)?;
            Some(MetricRow {
                name,
                unit,
                value: s.value,
                tags: s.tag_label(),
                source: "airm",
            })
        })
        .collect();
    let reported = |n: &str| airm.metrics.iter().any(|s| s.name == n);
    let mut fallback = |name: &'static str, value: f64, tags: String| {
        if !reported(name) && value.is_finite() {
            rows.push(MetricRow {
                name,
                unit: unit(name).unwrap_or(""),
                value,
                tags,
                source: "vendor",
            });
        }
    };
    fallback("tok_s", snap.perf.tps, String::new());
    fallback("req_s", snap.perf.req_rate, String::new());
    let powers: Vec<f64> = snap
        .accel
        .iter()
        .map(|a| a.power)
        .filter(|p| p.is_finite())
        .collect();
    if !powers.is_empty() {
        fallback("power_w", powers.iter().sum(), "scope=card(sum)".into());
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for a in &snap.accel {
        *counts.entry(a.disp().to_string()).or_default() += 1;
    }
    for (model, n) in counts {
        fallback("accelerators", n as f64, format!("model={model}"));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile_obj(id: &str, node: &str, vendor: &str) -> serde_json::Value {
        // Shape of rm_core.k8s.accelerator_profile(); numbers are strings as rm-core writes them.
        json!({
            "apiVersion": "airm.ai-computing.io/v1alpha1",
            "kind": "AcceleratorProfile",
            "metadata": {"name": id.replace('/', ".").replace(':', "-")},
            "spec": {
                "id": id, "node": node, "vendor": vendor, "model": "RNGD",
                "memoryBytes": 51539607552u64, "memoryKind": "hbm",
                "peakTflops": {"bf16": "256", "fp8": "512.5"}, "peakSource": "datasheet",
                "powerLimitW": "180", "interconnect": ["pcie5"]
            }
        })
    }

    fn accel(kind: AccelKind, id: &str, node: &str) -> Accel {
        Accel {
            kind,
            model: String::new(),
            id: id.into(),
            node: node.into(),
            util: 0.0,
            mem_used_gb: f64::NAN,
            mem_total_gb: f64::NAN,
            temp: f64::NAN,
            power: 42.0,
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

    fn series(name: &str, value: f64, labels: &[(&str, &str)]) -> Series {
        let mut l: BTreeMap<String, String> = labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        l.insert("__name__".into(), name.into());
        Series { labels: l, value }
    }

    #[test]
    fn parses_profile_written_by_rm_core() {
        let p = parse_profile(&profile_obj("node-a/furiosa:1", "node-a", "furiosa")).unwrap();
        assert_eq!(p.id, "node-a/furiosa:1");
        assert_eq!(p.index(), Some(1));
        assert_eq!(p.memory_bytes, 51539607552);
        assert_eq!(p.power_limit_w, Some(180.0));
        assert_eq!(
            p.peak_tflops,
            vec![("bf16".into(), 256.0), ("fp8".into(), 512.5)]
        );
        assert_eq!(p.peak_label(), "bf16 256 · fp8 512.5 TF (datasheet)");
        assert_eq!(p.interconnect, vec!["pcie5".to_string()]);
    }

    #[test]
    fn unknown_peak_and_power_stay_unknown() {
        let mut obj = profile_obj("node-a/cpu:0", "node-a", "cpu");
        obj["spec"]["peakTflops"] = json!({});
        obj["spec"]["powerLimitW"] = serde_json::Value::Null;
        let p = parse_profile(&obj).unwrap();
        assert!(p.peak_tflops.is_empty());
        assert_eq!(p.peak_label(), "");
        assert_eq!(p.power_limit_w, None);
    }

    #[test]
    fn profile_missing_required_field_is_skipped() {
        let mut bad = profile_obj("node-a/furiosa:0", "node-a", "furiosa");
        bad["spec"].as_object_mut().unwrap().remove("memoryKind");
        let list = json!({"items": [bad, profile_obj("node-a/furiosa:0", "node-a", "furiosa")]});
        assert_eq!(parse_profiles(&list).len(), 1);
        assert!(parse_profiles(&json!({"items": []})).is_empty());
    }

    #[test]
    fn profile_matches_collected_device_by_node_vendor_index() {
        let p = parse_profile(&profile_obj("node-a/furiosa:1", "node-a", "furiosa")).unwrap();
        assert!(p.matches(&accel(AccelKind::Rngd, "npu1", "node-a")));
        assert!(!p.matches(&accel(AccelKind::Rngd, "npu0", "node-a")));
        assert!(!p.matches(&accel(AccelKind::Rngd, "npu1", "node-b")));
        assert!(!p.matches(&accel(AccelKind::Rbln, "rbln1", "node-a")));
    }

    #[test]
    fn airm_series_keep_protocol_names_units_and_tags() {
        let got = parse_metrics(vec![
            series(
                "airm_power_w",
                150.0,
                &[
                    ("scope", "card"),
                    ("accelerator", "node-a/furiosa:1"),
                    ("instance", "192.0.2.10:9400"),
                ],
            ),
            series("airm_tok_s", 61.7, &[("model", "m")]),
            series("airm_not_in_protocol", 1.0, &[]),
            series("airm_ttft_p90_ms", f64::NAN, &[]),
        ]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "power_w");
        assert_eq!(unit(&got[0].name), Some("W"));
        // non-protocol labels (scrape address) are dropped
        assert_eq!(
            got[0].tag_label(),
            "accelerator=node-a/furiosa:1,scope=card"
        );
        assert_eq!(unit(&got[1].name), Some("tokens/s"));
    }

    #[test]
    fn card_power_prefers_airm_series_for_matching_device() {
        let airm = Airm {
            profiles: Some(vec![parse_profile(&profile_obj(
                "node-a/furiosa:1",
                "node-a",
                "furiosa",
            ))
            .unwrap()]),
            metrics: parse_metrics(vec![series(
                "airm_power_w",
                150.0,
                &[("scope", "card"), ("accelerator", "node-a/furiosa:1")],
            )]),
            placement: None,
        };
        assert_eq!(
            airm.card_power(&accel(AccelKind::Rngd, "npu1", "node-a")),
            Some(150.0)
        );
        assert_eq!(
            airm.card_power(&accel(AccelKind::Rngd, "npu0", "node-a")),
            None
        );
    }

    #[test]
    fn metric_rows_fall_back_to_vendor_values_only_when_airm_absent() {
        let mut snap = crate::collect::Snapshot::default();
        snap.perf.tps = 66.8;
        snap.accel = vec![accel(AccelKind::Rngd, "npu0", "node-a")];
        let airm = Airm {
            metrics: parse_metrics(vec![series("airm_tok_s", 61.7, &[])]),
            ..Airm::default()
        };
        let rows = metric_rows(&airm, &snap);
        let tok: Vec<_> = rows.iter().filter(|r| r.name == "tok_s").collect();
        assert_eq!(tok.len(), 1);
        assert_eq!((tok[0].value, tok[0].source), (61.7, "airm"));
        let power = rows.iter().find(|r| r.name == "power_w").unwrap();
        assert_eq!(
            (power.value, power.source, power.unit),
            (42.0, "vendor", "W")
        );
        // req_rate is NaN in the snapshot → no row rather than a zero
        assert!(rows.iter().all(|r| r.name != "req_s"));
    }

    #[test]
    fn placement_configmap_parses_decisions() {
        let cm = json!({"data": {
            "decode-llama": r#"{"accelerator_ids": ["node-a/furiosa:0", "node-a/furiosa:1"], "kv_transport": "p2p", "reason": "most free HBM"}"#,
            "prefill-llama": "acceleratorIds: [node-b/nvidia:0]\nkvTransport: p2p\nreason: highest bf16 peak\n",
            "broken": "{not json",
        }});
        let d = parse_placement(&cm);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].workload, "broken");
        assert_eq!(d[0].reason, "unparsable decision");
        assert_eq!(d[1].accelerator_ids.len(), 2);
        assert_eq!(d[1].kv_transport, "p2p");
        assert_eq!(d[1].reason, "most free HBM");
        assert_eq!(d[2].accelerator_ids, vec!["node-b/nvidia:0".to_string()]);
        // the v0.1 default is an empty ConfigMap
        assert!(parse_placement(&json!({"metadata": {"name": "airm-placement"}})).is_empty());
    }

    #[test]
    fn placement_configmap_reads_llm_d_setup_decisions_document() {
        let doc = r#"{"protocol": "0.1", "generation": 1, "placement": "manual", "decisions": [
            {"pool": "llama8b-pd-pool", "workload": {"kind": "prefill", "model": "m"},
             "acceleratorIds": ["n/furiosa:0"], "kvTransport": "shm", "reason": "pinned"},
            {"pool": "llama8b-pd-pool", "workload": {"kind": "decode", "model": "m"},
             "acceleratorIds": ["n/furiosa:1"], "kvTransport": "shm", "reason": "next to prefill"}]}"#;
        let got = parse_placement(&json!({"data": {"decisions.json": doc}}));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].workload, "llama8b-pd-pool/decode");
        assert_eq!(got[0].accelerator_ids, vec!["n/furiosa:1".to_string()]);
        assert_eq!(got[1].workload, "llama8b-pd-pool/prefill");
        assert_eq!(got[1].kv_transport, "shm");
        assert_eq!(got[1].reason, "pinned");
        // the v0.1 default document has no decisions: nothing to show
        let empty = r#"{"protocol": "0.1", "generation": 0, "updatedAt": null, "decisions": []}"#;
        assert!(parse_placement(&json!({"data": {"decisions.json": empty}})).is_empty());
    }

    #[test]
    fn nothing_present_by_default() {
        assert!(!Airm::default().present());
    }
}
