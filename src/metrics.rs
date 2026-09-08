//! Single source of truth for metric names.
//! collect (gathering) and doctor (full survey) reference the same constants → names can never drift.
//! To add/change a metric, edit only this one place.

// Accelerator metric names live in the accelerator packs (src/accel/), one record per
// family, together with their unit, aggregation and label spelling. They were listed here as
// well, and the two lists drifted: four metrics collect reads were missing from DEPS, so
// --doctor skipped them in its coverage table and then advertised them as "unused candidates
// to wire" (BUG-10). Deriving the accelerator half of DEPS from the packs makes that
// impossible — there is only one list now.

// ── host (node-exporter) ─────────────────────────────
pub const NODE_LOAD1: &str = "node_load1";
pub const NODE_MEM_TOTAL: &str = "node_memory_MemTotal_bytes";
pub const NODE_MEM_AVAIL: &str = "node_memory_MemAvailable_bytes";
pub const NODE_CPU_SECONDS: &str = "node_cpu_seconds_total";
pub const NODE_FS_SIZE: &str = "node_filesystem_size_bytes";
pub const NODE_FS_AVAIL: &str = "node_filesystem_avail_bytes";

// ── vLLM (model server) ──────────────────────────────
pub const VLLM_REQ_SUCCESS: &str = "vllm:request_success_total";
pub const VLLM_GEN_TOKENS: &str = "vllm:generation_tokens_total";
pub const VLLM_TTFT_BUCKET: &str = "vllm:time_to_first_token_seconds_bucket";
pub const VLLM_E2E_BUCKET: &str = "vllm:e2e_request_latency_seconds_bucket";
pub const VLLM_QUEUE_BUCKET: &str = "vllm:request_queue_time_seconds_bucket";
pub const VLLM_PREFILL_BUCKET: &str = "vllm:request_prefill_time_seconds_bucket";
pub const VLLM_DECODE_BUCKET: &str = "vllm:request_decode_time_seconds_bucket";
pub const VLLM_RUNNING: &str = "vllm:num_requests_running";
pub const VLLM_WAITING: &str = "vllm:num_requests_waiting";
pub const VLLM_KV: &str = "vllm:kv_cache_usage_perc";
pub const VLLM_PREEMPT: &str = "vllm:num_preemptions_total";

// ── EPP / InferencePool ──────────────────────────────
pub const POOL_READY: &str = "inference_pool_ready_pods";
pub const POOL_QUEUE: &str = "inference_pool_average_queue_size";
pub const POOL_KV: &str = "inference_pool_average_kv_cache_utilization";
pub const POOL_PER_POD_QUEUE: &str = "inference_pool_per_pod_queue_size";
pub const POOL_SAT: &str = "inference_extension_flow_control_pool_saturation";
pub const SCHED_ATTEMPTS: &str = "inference_extension_scheduler_attempts_total";
pub const PREFIX_IDX: &str = "inference_extension_prefix_indexer_size";
// per-endpoint composite scheduler score — exposed by the epp-score-observer picker plugin
// (see contrib/epp-score-observer). Absent until that plugin is deployed → column shows '–'.
pub const EPP_ENDPOINT_SCORE: &str = "epp_endpoint_score";

/// doctor coverage targets: (family, metric, impact when absent). Same constants as the metrics collect reads.
/// Metric coverage targets that are *not* accelerator-specific: host, model server, EPP.
/// The accelerator half comes from the packs — see [`coverage`].
pub const DEPS: &[(&str, &str, &str)] = &[
    ("Host (node)", NODE_LOAD1, "node load unavailable"),
    (
        "Host (node)",
        NODE_MEM_TOTAL,
        "node/unified mem total unavailable",
    ),
    (
        "Host (node)",
        NODE_MEM_AVAIL,
        "node/unified mem used unavailable",
    ),
    ("Host (node)", NODE_CPU_SECONDS, "host CPU% unavailable"),
    ("Host (node)", NODE_FS_SIZE, "node disk usage unavailable"),
    ("vLLM (model server)", VLLM_RUNNING, "Models run/wait empty"),
    ("vLLM (model server)", VLLM_WAITING, "Models run/wait empty"),
    ("vLLM (model server)", VLLM_KV, "KV% empty"),
    ("vLLM (model server)", VLLM_GEN_TOKENS, "tok/s empty"),
    ("vLLM (model server)", VLLM_REQ_SUCCESS, "Perf req/s empty"),
    ("vLLM (model server)", VLLM_TTFT_BUCKET, "TTFT empty"),
    ("vLLM (model server)", VLLM_E2E_BUCKET, "E2E latency empty"),
    ("vLLM (model server)", VLLM_QUEUE_BUCKET, "QUEUE p95 empty"),
    (
        "vLLM (model server)",
        VLLM_PREFILL_BUCKET,
        "PREFILL(P) p95 empty",
    ),
    (
        "vLLM (model server)",
        VLLM_DECODE_BUCKET,
        "DECODE(D) p95 empty",
    ),
    ("vLLM (model server)", VLLM_PREEMPT, "preemption rate empty"),
    (
        "EPP / InferencePool",
        POOL_READY,
        "EPP pools empty (EPP not in path or not scraped)",
    ),
    ("EPP / InferencePool", POOL_QUEUE, "pool queue empty"),
    ("EPP / InferencePool", POOL_KV, "pool KV empty"),
    (
        "EPP / InferencePool",
        POOL_PER_POD_QUEUE,
        "per-pod queue distribution empty",
    ),
    ("EPP / InferencePool", POOL_SAT, "pool saturation empty"),
    (
        "EPP / InferencePool",
        SCHED_ATTEMPTS,
        "routing distribution empty",
    ),
    (
        "EPP / InferencePool",
        PREFIX_IDX,
        "prefix-cache index size empty",
    ),
];

/// Everything `--doctor` checks: the accelerator packs' declared series, then the shared
/// host/model-server/EPP metrics. Single source of truth — a metric a collector reads is by
/// construction a metric doctor checks, and vice versa.
pub fn coverage() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut out: Vec<(&str, &str, &str)> = Vec::new();
    for pack in crate::accel::PACKS {
        for series in pack.series {
            out.push((pack.family, series.metric, series.missing));
        }
    }
    out.extend_from_slice(DEPS);
    out
}

/// Metric names any collector reads — used to tell a genuinely unwired signal from one that
/// is merely absent from a hand-maintained list.
pub fn known_metrics() -> std::collections::BTreeSet<&'static str> {
    coverage().into_iter().map(|(_, m, _)| m).collect()
}

/// Family prefixes for detecting "unused accelerator metrics (= new signal candidates)".
pub const ACCEL_PREFIXES: &[&str] = &["DCGM_FI_DEV_", "furiosa_npu_", "RBLN_DEVICE_STATUS:"];

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-10 cannot recur by construction — but pin the property, so a future change that
    /// reintroduces a second hand-written list fails here rather than in the doctor output.
    #[test]
    fn coverage_includes_every_series_the_packs_declare() {
        let covered = known_metrics();
        for pack in crate::accel::PACKS {
            for series in pack.series {
                assert!(
                    covered.contains(series.metric),
                    "{} declares {} but doctor would not check it",
                    pack.id,
                    series.metric
                );
            }
        }
        // And the shared metrics still make it through.
        assert!(covered.contains(VLLM_RUNNING) && covered.contains(POOL_READY));
    }

    #[test]
    fn coverage_has_no_duplicates() {
        let mut seen = std::collections::BTreeSet::new();
        for (_, metric, _) in coverage() {
            assert!(seen.insert(metric), "duplicate coverage entry: {}", metric);
        }
    }
}
