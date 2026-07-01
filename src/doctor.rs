//! `lmd-top --doctor` — Prometheus 메트릭 전수조사 + 갭 분석.
//! (1) 감지된 exporter(job) (2) lmd-top 이 읽는 메트릭의 존재/부재 + 부재 시 영향
//! (3) 미사용 가속기 메트릭(=새 신호 후보). "왜 이 뷰가 비었나"를 한 번에 진단.

use crate::collect::Config;
use crate::prom;
use std::collections::BTreeSet;

/// lmd-top 이 의존하는 메트릭: (family, metric, 부재 시 영향).
/// 새 collector 를 추가하면 여기에도 등록 → doctor 가 자동으로 커버리지 추적.
const DEPS: &[(&str, &str, &str)] = &[
    // NVIDIA GPU (DCGM)
    ("NVIDIA GPU (DCGM)", "DCGM_FI_DEV_GPU_UTIL", "GPU util unavailable"),
    ("NVIDIA GPU (DCGM)", "DCGM_FI_DEV_GPU_TEMP", "GPU temp unavailable"),
    ("NVIDIA GPU (DCGM)", "DCGM_FI_DEV_POWER_USAGE", "GPU power unavailable"),
    ("NVIDIA GPU (DCGM)", "DCGM_FI_DEV_FB_USED", "GPU mem used unavailable (unified-mem falls back to host)"),
    ("NVIDIA GPU (DCGM)", "DCGM_FI_DEV_FB_TOTAL", "GPU mem total unavailable (unified-mem falls back to host)"),
    // Rebellions RBLN
    ("Rebellions RBLN", "RBLN_DEVICE_STATUS:UTILIZATION", "RBLN util unavailable"),
    ("Rebellions RBLN", "RBLN_DEVICE_STATUS:TEMPERATURE", "RBLN temp unavailable"),
    ("Rebellions RBLN", "RBLN_DEVICE_STATUS:CARD_POWER", "RBLN power unavailable"),
    ("Rebellions RBLN", "RBLN_DEVICE_STATUS:DRAM_USED", "RBLN mem used unavailable"),
    ("Rebellions RBLN", "RBLN_DEVICE_STATUS:DRAM_TOTAL", "RBLN mem total unavailable"),
    ("Rebellions RBLN", "RBLN_DEVICE_STATUS:HEALTH", "RBLN health unavailable"),
    // Furiosa RNGD
    ("Furiosa RNGD", "furiosa_npu_core_utilization", "RNGD util unavailable"),
    ("Furiosa RNGD", "furiosa_npu_hw_temperature", "RNGD temp unavailable"),
    ("Furiosa RNGD", "furiosa_npu_hw_power", "RNGD power unavailable"),
    ("Furiosa RNGD", "furiosa_npu_dram_usage", "RNGD mem used unavailable"),
    ("Furiosa RNGD", "furiosa_npu_dram_total", "RNGD mem total unavailable"),
    ("Furiosa RNGD", "furiosa_npu_alive", "RNGD liveness unavailable"),
    ("Furiosa RNGD", "furiosa_npu_throttling_events_count", "RNGD throttle detection unavailable"),
    // host (node-exporter)
    ("Host (node)", "node_load1", "node load unavailable"),
    ("Host (node)", "node_memory_MemTotal_bytes", "node/unified mem total unavailable"),
    ("Host (node)", "node_memory_MemAvailable_bytes", "node/unified mem used unavailable"),
    ("Host (node)", "node_cpu_seconds_total", "host CPU% unavailable"),
    // vLLM (model server)
    ("vLLM (model server)", "vllm:num_requests_running", "Models run/wait empty"),
    ("vLLM (model server)", "vllm:num_requests_waiting", "Models run/wait empty"),
    ("vLLM (model server)", "vllm:kv_cache_usage_perc", "KV% empty"),
    ("vLLM (model server)", "vllm:generation_tokens_total", "tok/s empty"),
    ("vLLM (model server)", "vllm:request_success_total", "Perf req/s empty"),
    ("vLLM (model server)", "vllm:time_to_first_token_seconds_bucket", "TTFT empty"),
    ("vLLM (model server)", "vllm:e2e_request_latency_seconds_bucket", "E2E latency empty"),
    ("vLLM (model server)", "vllm:request_queue_time_seconds_bucket", "QUEUE p95 empty"),
    ("vLLM (model server)", "vllm:request_prefill_time_seconds_bucket", "PREFILL(P) p95 empty"),
    ("vLLM (model server)", "vllm:request_decode_time_seconds_bucket", "DECODE(D) p95 empty"),
    ("vLLM (model server)", "vllm:num_preemptions_total", "preemption rate empty"),
    // EPP / InferencePool
    ("EPP / InferencePool", "inference_pool_ready_pods", "EPP pools empty (EPP not in path or not scraped)"),
    ("EPP / InferencePool", "inference_pool_average_queue_size", "pool queue empty"),
    ("EPP / InferencePool", "inference_pool_average_kv_cache_utilization", "pool KV empty"),
    ("EPP / InferencePool", "inference_pool_per_pod_queue_size", "per-pod queue distribution empty"),
    ("EPP / InferencePool", "inference_extension_flow_control_pool_saturation", "pool saturation empty"),
    ("EPP / InferencePool", "inference_extension_scheduler_attempts_total", "routing distribution empty"),
    ("EPP / InferencePool", "inference_extension_prefix_indexer_size", "prefix-cache index size empty"),
];

/// 가속기 신호를 담은 메트릭 family 접두 — "미사용(=새 신호 후보)" 탐지용.
const ACCEL_PREFIXES: &[&str] = &["DCGM_FI_DEV_", "furiosa_npu_", "RBLN_DEVICE_STATUS:"];

pub async fn run(cfg: &Config) {
    println!("lmd-top doctor · prometheus {} · ns {}\n", cfg.prom, cfg.ns);

    let names = match prom::label_values(&cfg.prom, "__name__").await {
        Ok(n) => n,
        Err(e) => {
            println!("✗ cannot reach Prometheus at {} — {}", cfg.prom, e);
            println!("  check LMD_PROM (host:port, plain HTTP) and network reachability.");
            return;
        }
    };
    let present: BTreeSet<&str> = names.iter().map(|s| s.as_str()).collect();

    // exporters (job 라벨)
    match prom::label_values(&cfg.prom, "job").await {
        Ok(jobs) => {
            let accel_jobs: Vec<&String> = jobs
                .iter()
                .filter(|j| {
                    let l = j.to_lowercase();
                    l.contains("dcgm") || l.contains("furiosa") || l.contains("rbln") || l.contains("node") || l.contains("gpu")
                })
                .collect();
            println!("exporters (accelerator/host jobs): {}", if accel_jobs.is_empty() { "(none detected)".to_string() } else { accel_jobs.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("  ") });
        }
        Err(_) => println!("exporters: (job label unavailable)"),
    }
    println!("total metric names in Prometheus: {}\n", names.len());

    // 커버리지: family 별로 present/absent + 부재 영향
    println!("metric coverage (what lmd-top reads):");
    let mut fam = "";
    let (mut have, mut miss) = (0usize, 0usize);
    let mut affected: Vec<&str> = Vec::new();
    for (family, metric, impact) in DEPS {
        if *family != fam {
            println!("  {}", family);
            fam = family;
        }
        if present.contains(metric) {
            have += 1;
            println!("    ✓ {}", metric);
        } else {
            miss += 1;
            affected.push(impact);
            println!("    ✗ {:<48} → {}", metric, impact);
        }
    }
    println!("\nsummary: {}/{} expected metrics present, {} missing.", have, have + miss, miss);

    // 미사용 가속기 메트릭(= 새 신호 후보) — DEPS 에 없지만 가속기 family 접두를 가진 것들
    let known: BTreeSet<&str> = DEPS.iter().map(|(_, m, _)| *m).collect();
    let mut candidates: Vec<&str> = names
        .iter()
        .map(|s| s.as_str())
        .filter(|n| ACCEL_PREFIXES.iter().any(|p| n.starts_with(p)) && !known.contains(n))
        .collect();
    candidates.sort_unstable();
    if candidates.is_empty() {
        println!("\nunused accelerator metrics: (none)");
    } else {
        println!("\nunused accelerator metrics present ({} — candidate new signals to wire):", candidates.len());
        for n in &candidates {
            println!("    · {}", n);
        }
    }
}
