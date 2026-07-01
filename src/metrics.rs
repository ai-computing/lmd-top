//! 메트릭 이름 단일 출처(single source of truth).
//! collect(수집)와 doctor(전수조사)가 같은 상수를 참조 → 이름이 절대 어긋나지 않음.
//! 메트릭 추가/변경은 여기 한 곳만 고치면 됨.

// ── NVIDIA DCGM ──────────────────────────────────────
pub const DCGM_GPU_UTIL: &str = "DCGM_FI_DEV_GPU_UTIL";
pub const DCGM_GPU_TEMP: &str = "DCGM_FI_DEV_GPU_TEMP";
pub const DCGM_POWER: &str = "DCGM_FI_DEV_POWER_USAGE";
pub const DCGM_FB_USED: &str = "DCGM_FI_DEV_FB_USED";
pub const DCGM_FB_TOTAL: &str = "DCGM_FI_DEV_FB_TOTAL";
pub const DCGM_MEM_COPY_UTIL: &str = "DCGM_FI_DEV_MEM_COPY_UTIL";
pub const DCGM_SM_CLOCK: &str = "DCGM_FI_DEV_SM_CLOCK";
pub const DCGM_MEM_TEMP: &str = "DCGM_FI_DEV_MEMORY_TEMP";
pub const DCGM_ENERGY: &str = "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION";

// ── Rebellions RBLN (recording rules) ────────────────
pub const RBLN_UTIL: &str = "RBLN_DEVICE_STATUS:UTILIZATION";
pub const RBLN_TEMP: &str = "RBLN_DEVICE_STATUS:TEMPERATURE";
pub const RBLN_POWER: &str = "RBLN_DEVICE_STATUS:CARD_POWER";
pub const RBLN_DRAM_USED: &str = "RBLN_DEVICE_STATUS:DRAM_USED";
pub const RBLN_DRAM_TOTAL: &str = "RBLN_DEVICE_STATUS:DRAM_TOTAL";
pub const RBLN_HEALTH: &str = "RBLN_DEVICE_STATUS:HEALTH";

// ── Furiosa RNGD ─────────────────────────────────────
pub const FURIOSA_UTIL: &str = "furiosa_npu_core_utilization";
pub const FURIOSA_TEMP: &str = "furiosa_npu_hw_temperature";
pub const FURIOSA_POWER: &str = "furiosa_npu_hw_power";
pub const FURIOSA_DRAM_USED: &str = "furiosa_npu_dram_usage";
pub const FURIOSA_DRAM_TOTAL: &str = "furiosa_npu_dram_total";
pub const FURIOSA_ALIVE: &str = "furiosa_npu_alive";
pub const FURIOSA_THROTTLE: &str = "furiosa_npu_throttling_events_count";

// ── host (node-exporter) ─────────────────────────────
pub const NODE_LOAD1: &str = "node_load1";
pub const NODE_MEM_TOTAL: &str = "node_memory_MemTotal_bytes";
pub const NODE_MEM_AVAIL: &str = "node_memory_MemAvailable_bytes";
pub const NODE_CPU_SECONDS: &str = "node_cpu_seconds_total";

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


/// doctor 커버리지 대상: (family, metric, 부재 시 영향). collect 가 읽는 메트릭과 동일 상수.
pub const DEPS: &[(&str, &str, &str)] = &[
    ("NVIDIA GPU (DCGM)", DCGM_GPU_UTIL, "GPU util unavailable"),
    ("NVIDIA GPU (DCGM)", DCGM_GPU_TEMP, "GPU temp unavailable"),
    ("NVIDIA GPU (DCGM)", DCGM_POWER, "GPU power unavailable"),
    ("NVIDIA GPU (DCGM)", DCGM_FB_USED, "GPU mem used unavailable (unified-mem falls back to host)"),
    ("NVIDIA GPU (DCGM)", DCGM_FB_TOTAL, "GPU mem total unavailable (unified-mem falls back to host)"),
    ("Rebellions RBLN", RBLN_UTIL, "RBLN util unavailable"),
    ("Rebellions RBLN", RBLN_TEMP, "RBLN temp unavailable"),
    ("Rebellions RBLN", RBLN_POWER, "RBLN power unavailable"),
    ("Rebellions RBLN", RBLN_DRAM_USED, "RBLN mem used unavailable"),
    ("Rebellions RBLN", RBLN_DRAM_TOTAL, "RBLN mem total unavailable"),
    ("Rebellions RBLN", RBLN_HEALTH, "RBLN health unavailable"),
    ("Furiosa RNGD", FURIOSA_UTIL, "RNGD util unavailable"),
    ("Furiosa RNGD", FURIOSA_TEMP, "RNGD temp unavailable"),
    ("Furiosa RNGD", FURIOSA_POWER, "RNGD power unavailable"),
    ("Furiosa RNGD", FURIOSA_DRAM_USED, "RNGD mem used unavailable"),
    ("Furiosa RNGD", FURIOSA_DRAM_TOTAL, "RNGD mem total unavailable"),
    ("Furiosa RNGD", FURIOSA_ALIVE, "RNGD liveness unavailable"),
    ("Furiosa RNGD", FURIOSA_THROTTLE, "RNGD throttle detection unavailable"),
    ("Host (node)", NODE_LOAD1, "node load unavailable"),
    ("Host (node)", NODE_MEM_TOTAL, "node/unified mem total unavailable"),
    ("Host (node)", NODE_MEM_AVAIL, "node/unified mem used unavailable"),
    ("Host (node)", NODE_CPU_SECONDS, "host CPU% unavailable"),
    ("vLLM (model server)", VLLM_RUNNING, "Models run/wait empty"),
    ("vLLM (model server)", VLLM_WAITING, "Models run/wait empty"),
    ("vLLM (model server)", VLLM_KV, "KV% empty"),
    ("vLLM (model server)", VLLM_GEN_TOKENS, "tok/s empty"),
    ("vLLM (model server)", VLLM_REQ_SUCCESS, "Perf req/s empty"),
    ("vLLM (model server)", VLLM_TTFT_BUCKET, "TTFT empty"),
    ("vLLM (model server)", VLLM_E2E_BUCKET, "E2E latency empty"),
    ("vLLM (model server)", VLLM_QUEUE_BUCKET, "QUEUE p95 empty"),
    ("vLLM (model server)", VLLM_PREFILL_BUCKET, "PREFILL(P) p95 empty"),
    ("vLLM (model server)", VLLM_DECODE_BUCKET, "DECODE(D) p95 empty"),
    ("vLLM (model server)", VLLM_PREEMPT, "preemption rate empty"),
    ("EPP / InferencePool", POOL_READY, "EPP pools empty (EPP not in path or not scraped)"),
    ("EPP / InferencePool", POOL_QUEUE, "pool queue empty"),
    ("EPP / InferencePool", POOL_KV, "pool KV empty"),
    ("EPP / InferencePool", POOL_PER_POD_QUEUE, "per-pod queue distribution empty"),
    ("EPP / InferencePool", POOL_SAT, "pool saturation empty"),
    ("EPP / InferencePool", SCHED_ATTEMPTS, "routing distribution empty"),
    ("EPP / InferencePool", PREFIX_IDX, "prefix-cache index size empty"),
];

/// "미사용 가속기 메트릭(=새 신호 후보)" 탐지용 family 접두.
pub const ACCEL_PREFIXES: &[&str] = &["DCGM_FI_DEV_", "furiosa_npu_", "RBLN_DEVICE_STATUS:"];
