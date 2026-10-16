//! Service-scoped latency queries, evidence validation and engine collision handling.
//! No Kubernetes or UI state is required to validate observations.
use crate::prom::{self, Series};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyFamily {
    Vllm,
    Furiosa,
    Proxy,
}

#[derive(Clone, Default)]
pub struct PerformanceTarget {
    pub service: String,
    pub families: std::collections::BTreeSet<LatencyFamily>,
}

/// Perf 드릴다운(선택 모델) 온디맨드 상세 — 구간별 p50/p95/p99 + E2E 지연 버킷 분포(히스토그램).
#[derive(Clone)]
pub struct PerfDetail {
    pub checked_at: Option<u64>,
    pub service: String,
    pub namespace: String,
    pub warnings: Vec<String>,
    pub model: String,
    pub e2e: [f64; 3], // p50/p95/p99 (s)
    pub ttft: [f64; 3],
    pub tpot: [f64; 3],
    pub buckets: Vec<(f64, f64)>, // (le 상한 s, 해당 구간 rate) — 누적차 분포
}

impl Default for PerfDetail {
    fn default() -> Self {
        Self {
            checked_at: None,
            service: String::new(),
            namespace: String::new(),
            model: String::new(),
            e2e: [f64::NAN; 3],
            ttft: [f64::NAN; 3],
            tpot: [f64::NAN; 3],
            buckets: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

impl PerfDetail {
    fn checked(mut self) -> Self {
        self.checked_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs());
        self
    }

    pub fn age(&self, now: u64) -> Option<u64> {
        self.checked_at.and_then(|checked| now.checked_sub(checked))
    }
}

fn latency_scalar(result: anyhow::Result<Vec<Series>>) -> Result<f64, String> {
    let values = result.map_err(|e| e.to_string())?;
    match values.as_slice() {
        [] => Err("no samples".into()),
        [value] if value.value.is_finite() && value.value >= 0.0 => Ok(value.value),
        [_] => Err("no finite nonnegative observation".into()),
        _ => Err("ambiguous aggregate".into()),
    }
}

fn latency_distribution(series: &[Series]) -> Result<Vec<(f64, f64)>, String> {
    if series.is_empty() {
        return Ok(Vec::new());
    }
    let mut cumulative = Vec::new();
    for sample in series {
        let upper = sample
            .l("le")
            .parse::<f64>()
            .map_err(|_| "invalid bucket boundary")?;
        if upper.is_nan() || upper < 0.0 || !sample.value.is_finite() || sample.value < 0.0 {
            return Err("invalid latency bucket".into());
        }
        cumulative.push((upper, sample.value));
    }
    cumulative.sort_by(|a, b| a.0.total_cmp(&b.0));
    if cumulative.last().map(|x| x.0) != Some(f64::INFINITY) {
        return Err("histogram missing +Inf bucket".into());
    }
    let mut previous = 0.0;
    let mut boundary = None;
    let mut distribution = Vec::new();
    for (upper, count) in cumulative {
        if boundary == Some(upper) || count < previous {
            return Err("duplicate or decreasing cumulative buckets".into());
        }
        distribution.push((upper, count - previous));
        previous = count;
        boundary = Some(upper);
    }
    Ok(distribution)
}

/// Bind a collector query template to the configured namespace.
pub(crate) fn performance_query_text(template: &str, namespace: &str) -> String {
    template.replace(
        "__LMD_NAMESPACE__",
        &serde_json::to_string(namespace).unwrap(),
    )
}

pub(crate) async fn performance_query(
    base: &str,
    namespace: &str,
    template: &str,
) -> anyhow::Result<Vec<Series>> {
    prom::query(base, &performance_query_text(template, namespace)).await
}

fn latency_queries(namespace: &str, model: &str, family: LatencyFamily) -> Vec<String> {
    // Scope every detail query to the selected namespace and Service. JSON string
    // encoding also escapes quotes/backslashes/newlines in remote label values.
    let sel = format!(
        "{{namespace={},service={}}}",
        serde_json::to_string(namespace).unwrap(),
        serde_json::to_string(model).unwrap()
    );
    let (e2e_m, ttft_m, tpot_m) = match family {
        LatencyFamily::Proxy => (
            "ds4_proxy_request_duration_seconds",
            "ds4_proxy_ttft_seconds",
            "",
        ),
        LatencyFamily::Vllm => (
            "vllm:e2e_request_latency_seconds",
            "vllm:time_to_first_token_seconds",
            "vllm:request_time_per_output_token_seconds",
        ),
        // Only families supported by the existing collector contract; do not guess E2E.
        LatencyFamily::Furiosa => (
            "",
            "furiosa_llm_time_to_first_token_seconds",
            "furiosa_llm_inter_token_latency_seconds",
        ),
    };
    let q = |base: &str, quant: f64| {
        if base.is_empty() {
            return String::new();
        }
        format!(
            "histogram_quantile({}, sum by (le)(rate({}_bucket{}[5m])))",
            quant, base, sel
        )
    };
    let mut queries = Vec::new();
    for metric in [e2e_m, ttft_m, tpot_m] {
        for quantile in [0.5, 0.95, 0.99] {
            queries.push(q(metric, quantile));
        }
    }
    queries.push(if e2e_m.is_empty() {
        String::new()
    } else {
        format!("sum by (le)(rate({}_bucket{}[5m]))", e2e_m, sel)
    });
    queries
}

pub async fn perf_detail(
    prom: &str,
    namespace: &str,
    model: &str,
    target: &PerformanceTarget,
) -> PerfDetail {
    let mut detail = PerfDetail {
        namespace: namespace.into(),
        model: model.into(),
        service: target.service.clone(),
        ..PerfDetail::default()
    };
    if target.families.len() != 1 || target.service.is_empty() {
        detail.warnings.push(
            "Metric family/Service unavailable or ambiguous; refresh the performance list".into(),
        );
        return detail.checked();
    }
    let family = *target.families.first().unwrap();
    let queries = latency_queries(namespace, &target.service, family);
    let supported: Vec<(usize, &str)> = queries
        .iter()
        .enumerate()
        .filter(|(_, q)| !q.is_empty())
        .map(|(i, q)| (i, q.as_str()))
        .collect();
    let refs: Vec<&str> = supported.iter().map(|(_, q)| *q).collect();
    let results = prom::query_all(prom, &refs).await;
    if family == LatencyFamily::Furiosa {
        detail
            .warnings
            .push("E2E distribution unsupported for the observed Furiosa metric contract".into());
    }
    if family == LatencyFamily::Proxy {
        detail
            .warnings
            .push("TPOT unsupported for proxy metrics".into());
    }
    let labels = [
        "E2E p50", "E2E p95", "E2E p99", "TTFT p50", "TTFT p95", "TTFT p99", "TPOT p50",
        "TPOT p95", "TPOT p99",
    ];
    for ((index, _), result) in supported.into_iter().zip(results) {
        if index == 9 {
            match result
                .map_err(|e| e.to_string())
                .and_then(|series| latency_distribution(&series))
            {
                Ok(distribution) => detail.buckets = distribution,
                Err(error) => detail.warnings.push(format!("E2E buckets: {error}")),
            }
            continue;
        }
        let value = match latency_scalar(result) {
            Ok(value) => value,
            Err(error) => {
                detail.warnings.push(format!("{}: {error}", labels[index]));
                f64::NAN
            }
        };
        match index / 3 {
            0 => detail.e2e[index % 3] = value,
            1 => detail.ttft[index % 3] = value,
            _ => detail.tpot[index % 3] = value,
        }
    }
    detail.checked()
}

/// Preserve metric-family evidence independently of display names.
pub(crate) fn record_metric_family(
    targets: &mut BTreeMap<String, PerformanceTarget>,
    service: &str,
    query: &str,
) {
    let family = if query.contains("furiosa_llm_") {
        LatencyFamily::Furiosa
    } else if query.contains("ds4_proxy_") {
        LatencyFamily::Proxy
    } else {
        LatencyFamily::Vllm
    };
    targets
        .entry(service.into())
        .or_insert_with(|| PerformanceTarget {
            service: service.into(),
            ..PerformanceTarget::default()
        })
        .families
        .insert(family);
}

pub(crate) fn merge_engine_metrics(
    left: &mut BTreeMap<String, Series>,
    right: BTreeMap<String, Series>,
    field: &str,
    warnings: &mut Vec<String>,
) {
    for (service, sample) in right {
        if left.remove(&service).is_some() {
            warnings.push(format!(
                "{service} {field}: multiple engine metric families; value unavailable"
            ));
        } else {
            left.insert(service, sample);
        }
    }
}

pub(crate) fn unique_performance_value(
    seen: &mut std::collections::HashSet<(String, String)>,
    service: &str,
    field: &str,
    value: f64,
    warnings: &mut Vec<String>,
) -> f64 {
    if !seen.insert((service.into(), field.into())) {
        warnings.push(format!(
            "{service} {field}: multiple metric observations; value unavailable"
        ));
        f64::NAN
    } else if value.is_finite() && value >= 0.0 {
        value
    } else {
        f64::NAN
    }
}

pub(crate) fn single_engine_percentile(first: f64, second: f64, warnings: &mut Vec<String>) -> f64 {
    match (first.is_finite(), second.is_finite()) {
        (true, false) => first,
        (false, true) => second,
        (true, true) => {
            warnings.push(
                "TTFT: mixed engine percentiles cannot be combined; inspect per-Service values"
                    .into(),
            );
            f64::NAN
        }
        _ => f64::NAN,
    }
}

#[cfg(test)]
mod latency_detail_tests {
    use super::*;
    fn bucket(upper: &str, value: f64) -> Series {
        Series {
            labels: BTreeMap::from([("le".into(), upper.into())]),
            value,
        }
    }
    #[test]
    fn histogram_requires_valid_monotonic_complete_cumulative_evidence() {
        let result =
            latency_distribution(&[bucket("+Inf", 9.0), bucket("1", 4.0), bucket("0.1", 1.0)])
                .unwrap();
        assert_eq!(result, [(0.1, 1.0), (1.0, 3.0), (f64::INFINITY, 5.0)]);
        for values in [
            vec![bucket("1", 4.0)],
            vec![bucket("1", 4.0), bucket("+Inf", 3.0)],
            vec![bucket("1", 1.0), bucket("1.0", 2.0), bucket("+Inf", 3.0)],
            vec![bucket("NaN", 1.0), bucket("+Inf", 2.0)],
            vec![bucket("-1", 1.0), bucket("+Inf", 2.0)],
            vec![bucket("1", f64::NAN), bucket("+Inf", 2.0)],
            vec![bucket("+Inf", f64::INFINITY)],
        ] {
            assert!(latency_distribution(&values).is_err());
        }
        assert!(latency_distribution(&[]).unwrap().is_empty());
        assert_eq!(
            latency_distribution(&[bucket("+Inf", 0.0)]).unwrap(),
            [(f64::INFINITY, 0.0)]
        );
    }
    #[test]
    fn missing_latency_is_not_zero_or_first_of_ambiguous_results() {
        assert!(PerfDetail::default().e2e.iter().all(|v| v.is_nan()));
        assert_eq!(latency_scalar(Ok(vec![bucket("1", 0.0)])).unwrap(), 0.0);
        assert!(latency_scalar(Ok(Vec::new())).is_err());
        assert!(latency_scalar(Err(anyhow::anyhow!("timeout")))
            .unwrap_err()
            .contains("timeout"));
        assert!(latency_scalar(Ok(vec![bucket("1", 1.0), bucket("2", 2.0)])).is_err());
        for value in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(latency_scalar(Ok(vec![bucket("1", value)])).is_err());
        }
    }
}

#[cfg(test)]
mod latency_scope_tests {
    use super::{latency_queries, LatencyFamily};
    #[test]
    fn detail_queries_never_merge_same_service_across_namespaces() {
        for model in ["same-service", "ds4-proxy"] {
            let first = latency_queries(
                "tenant-a",
                model,
                if model == "ds4-proxy" {
                    LatencyFamily::Proxy
                } else {
                    LatencyFamily::Vllm
                },
            );
            let second = latency_queries(
                "tenant-b",
                model,
                if model == "ds4-proxy" {
                    LatencyFamily::Proxy
                } else {
                    LatencyFamily::Vllm
                },
            );
            assert_eq!(
                first.iter().filter(|q| !q.is_empty()).count(),
                if model == "ds4-proxy" { 7 } else { 10 }
            );
            assert_ne!(first, second);
            for query in first.into_iter().filter(|q| !q.is_empty()) {
                assert!(query.contains("namespace=\"tenant-a\""));
                assert!(query.contains(&format!("service=\"{model}\"")));
                assert!(query.contains("[5m]"));
            }
        }
    }
    #[test]
    fn remote_label_characters_cannot_change_the_selector() {
        let model = "model\"} or vector(999) #\n\\";
        let encoded = serde_json::to_string(model).unwrap();
        for query in latency_queries("ns", model, LatencyFamily::Vllm) {
            assert!(query.contains(&format!("service={encoded}")));
            assert!(!query.contains('\n'));
            assert!(query.contains("namespace=\"ns\""));
        }
    }
}

#[cfg(test)]
mod performance_scope_tests {
    use super::performance_query_text;

    #[test]
    fn namespace_binding_preserves_filters_and_scopes_both_ratio_operands() {
        let template = "sum(rate(vllm:prefix_cache_hits_total{namespace=__LMD_NAMESPACE__,service=\"model\"}[5m])) / sum(rate(vllm:prefix_cache_queries_total{namespace=__LMD_NAMESPACE__,service=\"model\"}[5m]))";
        let query = performance_query_text(template, "tenant-a");
        assert_eq!(query.matches("namespace=\"tenant-a\"").count(), 2);
        assert_eq!(query.matches("service=\"model\"").count(), 2);
        assert!(!query.contains("__LMD_NAMESPACE__"));
        assert_ne!(query, performance_query_text(template, "tenant-b"));
        let escaped = performance_query_text(template, "bad\"\\\n");
        assert!(!escaped.contains('\n'));
        assert_eq!(
            escaped
                .matches(&format!(
                    "namespace={}",
                    serde_json::to_string("bad\"\\\n").unwrap()
                ))
                .count(),
            2
        );
    }
}

#[cfg(test)]
mod engine_collision_tests {
    use super::*;
    fn sample(value: f64) -> Series {
        Series {
            labels: BTreeMap::new(),
            value,
        }
    }
    #[test]
    fn engine_merge_omits_collisions_and_keeps_disjoint_services() {
        let mut left =
            BTreeMap::from([("shared".into(), sample(10.0)), ("gpu".into(), sample(2.0))]);
        let right = BTreeMap::from([("shared".into(), sample(20.0)), ("npu".into(), sample(3.0))]);
        let mut warnings = Vec::new();
        merge_engine_metrics(&mut left, right, "queue", &mut warnings);
        assert!(!left.contains_key("shared"));
        assert_eq!(left["gpu"].value, 2.0);
        assert_eq!(left["npu"].value, 3.0);
        assert!(warnings[0].contains("shared queue"));
    }
    #[test]
    fn conflicting_row_fields_never_recover_by_last_writer_wins() {
        let mut seen = std::collections::HashSet::new();
        let mut warnings = Vec::new();
        assert_eq!(
            unique_performance_value(&mut seen, "model", "tps", 0.0, &mut warnings),
            0.0
        );
        for value in [0.0, 10.0, 20.0] {
            assert!(
                unique_performance_value(&mut seen, "model", "tps", value, &mut warnings).is_nan()
            );
        }
        assert_eq!(
            unique_performance_value(&mut seen, "model", "req", 2.0, &mut warnings),
            2.0
        );
        assert_eq!(
            unique_performance_value(&mut seen, "other", "tps", 3.0, &mut warnings),
            3.0
        );
    }
    #[test]
    fn a_vendor_percentile_is_not_presented_as_a_mixed_percentile() {
        let mut warnings = Vec::new();
        assert!(single_engine_percentile(0.1, 0.9, &mut warnings).is_nan());
        assert_eq!(single_engine_percentile(0.1, f64::NAN, &mut warnings), 0.1);
        assert_eq!(single_engine_percentile(f64::NAN, 0.9, &mut warnings), 0.9);
        assert!(single_engine_percentile(f64::NAN, f64::NAN, &mut warnings).is_nan());
        assert_eq!(warnings.len(), 1);
    }
}

#[cfg(test)]
mod family_detail_tests {
    use super::*;
    #[test]
    fn furiosa_detail_uses_observed_family_without_guessing_e2e() {
        let queries = latency_queries(
            "tenant",
            "service-without-vendor-name",
            LatencyFamily::Furiosa,
        );
        assert_eq!(queries.iter().filter(|q| !q.is_empty()).count(), 6);
        assert!(queries[..3].iter().all(String::is_empty));
        assert!(queries[9].is_empty());
        assert!(queries[3].contains("furiosa_llm_time_to_first_token_seconds_bucket"));
        assert!(queries[6].contains("furiosa_llm_inter_token_latency_seconds_bucket"));
        assert!(queries.iter().all(|q| !q.contains("vllm:")));
        let vllm = latency_queries("tenant", "ds4-proxy", LatencyFamily::Vllm);
        assert!(
            vllm[0].contains("vllm:"),
            "model name must not choose the engine"
        );
    }
    #[test]
    fn missing_or_ambiguous_family_stops_before_network_io() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for target in [
            PerformanceTarget::default(),
            PerformanceTarget {
                service: "model".into(),
                families: [LatencyFamily::Vllm, LatencyFamily::Furiosa].into(),
            },
        ] {
            let detail = runtime.block_on(perf_detail(
                "invalid-host-no-network",
                "ns",
                "display",
                &target,
            ));
            assert!(detail.e2e.iter().all(|v| v.is_nan()));
            assert!(detail.warnings[0].contains("ambiguous"));
        }
    }
}

#[cfg(test)]
mod proxy_identity_tests {
    use super::*;
    #[test]
    fn proxy_services_keep_their_actual_names_and_independent_targets() {
        let mut targets = BTreeMap::new();
        let query = "sum by (service)(rate(ds4_proxy_requests_total{namespace=\"ns\"}[5m]))";
        for service in ["ds4-inject-proxy", "second-proxy"] {
            record_metric_family(&mut targets, service, query);
            assert_eq!(targets[service].service, service);
            assert_eq!(targets[service].families, [LatencyFamily::Proxy].into());
            let detail = latency_queries("ns", service, LatencyFamily::Proxy);
            assert!(detail[0].contains(&format!("service=\"{service}\"")));
        }
        assert!(!targets.contains_key("ds4-proxy"));
        record_metric_family(&mut targets, "second-proxy", "vllm:request_success_total");
        assert_eq!(targets["second-proxy"].families.len(), 2);
        assert_eq!(targets["ds4-inject-proxy"].families.len(), 1);
    }
}
