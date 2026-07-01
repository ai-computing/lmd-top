//! Snapshot 도메인 타입 + prometheus/kubectl 에서 수집하는 로직.
//! 각 소스는 독립적으로 실패해도 전체를 막지 않음(warnings 에 누적, 부재 필드는 None/빈값).

use crate::config::Config;
use crate::kube;
use crate::metrics;
use crate::prom::{self, Series};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq)]
pub enum AccelKind {
    Gpu,
    Rbln,
    Rngd,
}
impl AccelKind {
    /// 벤더 계열 라벨(fallback). GPU 는 모델이 다양하므로 generic "GPU" — 실제 모델은 Accel.model.
    pub fn label(&self) -> &'static str {
        match self {
            AccelKind::Gpu => "GPU",
            AccelKind::Rbln => "RBLN",
            AccelKind::Rngd => "RNGD",
        }
    }
}

/// DCGM modelName("NVIDIA GB10", "NVIDIA A100-SXM4-40GB" 등) → 짧은 모델 토큰("GB10"/"A100").
/// 벤더 접두 제거 후 첫 토큰(공백/'-' 기준). 빈 값이면 빈 문자열.
fn gpu_model(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let s = s
        .strip_prefix("NVIDIA ")
        .or_else(|| s.strip_prefix("Nvidia "))
        .unwrap_or(s);
    s.split([' ', '-']).next().unwrap_or(s).to_string()
}

#[derive(Clone)]
pub struct Accel {
    pub kind: AccelKind,
    pub model: String,     // 실제 모델(예: GB10/A100/H100) — 메트릭 자동 감지, 없으면 ""
    pub id: String,        // rbln0 / npu0 / gpu0
    pub node: String,
    pub util: f64,         // 0..100
    pub mem_used_gb: f64,
    pub mem_total_gb: f64,
    pub temp: f64,
    pub power: f64,
    pub busy_model: String, // exported_pod 등
    pub alive: bool,        // furiosa_npu_alive / RBLN HEALTH
    pub throttle: f64,      // furiosa throttling events (>0 = 스로틀링)
    pub unified_mem: bool,  // GB10/GH200 등 CPU·GPU 통합 메모리 → mem 은 호스트(노드) 풀
    pub mem_bw: f64,        // DCGM MEM_COPY_UTIL % (메모리 대역폭 압박), NaN=미지원
    pub clock_mhz: f64,     // DCGM SM_CLOCK MHz, NaN=미지원
    pub mem_temp: f64,      // DCGM MEMORY_TEMP °C, NaN=미지원
    pub energy_mj: f64,     // DCGM TOTAL_ENERGY_CONSUMPTION 누적(mJ), NaN=미지원
}
impl Accel {
    /// 표시용 계열/모델 라벨 — 감지된 모델이 있으면 그것, 없으면 벤더 라벨.
    pub fn disp(&self) -> &str {
        if self.model.is_empty() { self.kind.label() } else { self.model.as_str() }
    }
}

/// 통합 메모리(Grace 계열 superchip: GB10/GH200/GB200/GB300) 여부 — 별도 VRAM 없이 호스트와 공유.
fn is_unified(model: &str) -> bool {
    let m = model.to_uppercase();
    m.starts_with("GB10") || m.starts_with("GH200") || m.starts_with("GB200") || m.starts_with("GB300")
}

#[derive(Clone, Default)]
pub struct NodeInfo {
    pub name: String,
    pub load1: f64,
    pub mem_used_gb: f64,
    pub mem_total_gb: f64,
    pub cpu_pct: f64,
    pub ready: bool,
    pub cordoned: bool,
    pub pressure: bool, // Memory/Disk/PID pressure 중 하나라도
    pub version: String,
}

#[derive(Clone, Default)]
pub struct Pool {
    pub name: String,
    pub ready: f64,
    pub queue: f64,
    pub kv: f64,
    pub sat: f64,
    pub selector: String,    // app=vllm-rbln-llama31-8b
    pub epp: String,         // endpointPickerRef (EPP service)
    pub ep_ready: i64,       // selector 매칭 파드 중 ready
    pub ep_total: i64,       // selector 매칭 파드 총수
}

#[derive(Clone)]
pub struct Route {
    pub path: String,
    pub backend: String,
    pub kind: String, // Service | InferencePool
}

#[derive(Clone)]
pub struct Objective {
    pub name: String,
    pub priority: i64,
    pub pool: String,
}

#[derive(Clone)]
pub struct ModelRow {
    pub name: String,
    pub ready: i64,
    pub desired: i64,
    pub status: String,
    pub route: String,
    pub engine: String, // 추론 엔진(vLLM/SGLang/vLLM-RBLN/Ollama/Furiosa/custom)
    pub accel: String, // 어떤 가속기/노드에서 도는지(파드 노드/가속기 추정)
    pub running: Option<f64>,
    pub waiting: Option<f64>,
    pub tps: Option<f64>,
    pub kv: Option<f64>,   // vllm:gpu_cache_usage_perc (0..1)
    pub ttft: Option<f64>, // TTFT p95 (s)
}

#[derive(Clone)]
pub struct EventRow {
    pub typ: String,    // Normal | Warning
    pub reason: String,
    pub object: String, // kind/name
    pub message: String,
    pub count: i64,
}

#[derive(Clone)]
pub struct PodRow {
    pub name: String,
    pub phase: String,
    pub ready: String,
    pub node: String,
    pub restarts: i64,
}

#[derive(Clone)]
pub struct EppCfg {
    pub profile: String,
    pub scorers: Vec<(String, f64)>,
    pub picker: String,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub ts: u64,
    pub nodes: Vec<NodeInfo>,
    pub accel: Vec<Accel>,
    pub pools: Vec<Pool>,
    pub models: Vec<ModelRow>,
    pub pods: Vec<PodRow>,
    pub events: Vec<EventRow>,
    pub routes: Vec<Route>,
    pub objectives: Vec<Objective>,
    pub decisions: Vec<(String, f64)>, // (pod, 라우팅 픽 횟수) — 트래픽이 EPP 경유 시
    pub pod_queues: Vec<(String, f64)>, // per-pod 큐 깊이(요청 분배)
    pub perf: Perf,
    pub perf_rows: Vec<PerfRow>, // 모델(=하드웨어)별 성능

    pub autoscalers: Vec<Autoscale>,
    pub epp: Option<EppCfg>,
    pub epp_in_path: bool, // HTTPRoute backend 중 InferencePool 이 있는가(없으면 EPP 우회)
    pub prefix_idx: f64,   // inference_extension_prefix_indexer_size
    pub gw_addr: String,
    pub gw_ok: bool,
    pub inventory: Vec<(String, i64, i64)>, // (가속기 resource, allocatable total, 사용중 requests)
    pub warnings: Vec<String>,
}

/// EPP 정책 수립용 성능 지표(구간별 지연 percentile·토큰분포·처리량). 값 없으면 NaN.
/// 클러스터 전역 성능 요약(throughput 라인 + timeline 용). 상세는 per-model(perf_rows)/drill-down.
#[derive(Clone)]
pub struct Perf {
    pub req_rate: f64,
    pub err_rate: f64,
    pub tps: f64,
    pub prefix_hit: f64,
    pub ttft_p95: f64,
    pub e2e_p95: f64,
}
impl Default for Perf {
    fn default() -> Self {
        let n = f64::NAN;
        Perf { req_rate: n, err_rate: n, tps: n, prefix_hit: n, ttft_p95: n, e2e_p95: n }
    }
}

/// 모델(=특정 하드웨어 배치)별 성능 행. 값 없으면 NaN.
#[derive(Clone)]
pub struct PerfRow {
    pub model: String,
    pub req: f64,
    pub tps: f64,
    pub ttft_p95: f64,
    pub tpot_p95: f64,
    pub e2e_p95: f64,
    pub in_tok_p95: f64,
    pub out_tok_p95: f64,
    pub queue_p95: f64,   // request_queue_time — 스케줄링 대기
    pub prefill_p95: f64, // request_prefill_time — prefill(P) 구간
    pub decode_p95: f64,  // request_decode_time — decode(D) 구간
    pub preempt: f64,     // num_preemptions rate — KV/메모리 스래싱
}
impl PerfRow {
    fn new(model: &str) -> Self {
        let n = f64::NAN;
        PerfRow {
            model: model.to_string(),
            req: n, tps: n, ttft_p95: n, tpot_p95: n, e2e_p95: n, in_tok_p95: n, out_tok_p95: n,
            queue_p95: n, prefill_p95: n, decode_p95: n, preempt: n,
        }
    }
}

#[derive(Clone)]
pub struct Autoscale {
    pub target: String, // scaleTargetRef (deployment)
    pub min: i64,
    pub max: i64,
    pub replicas: i64,
    pub ready: bool,
    pub active: bool,
    pub triggers: String,
}

/// 메모리 바이트 → GB. RBLN/Furiosa/node DRAM 모두 bytes 단위(실측 확인).
fn to_gb(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v / 1.0e9
    }
}

async fn q(cfg: &Config, promql: &str, warn: &mut Vec<String>) -> Vec<Series> {
    match prom::query(&cfg.prom, promql).await {
        Ok(v) => v,
        Err(e) => {
            warn.push(format!("prom[{}]: {}", short(promql), e));
            Vec::new()
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(28).collect()
}

/// 단일 스칼라 결과(첫 값) — 없으면 NaN. (join! 병렬용, warn 없음)
async fn qs1(prom_base: &str, promql: &str) -> f64 {
    prom::query(prom_base, promql).await.ok().and_then(|v| v.first().map(|s| s.value)).unwrap_or(f64::NAN)
}

/// Perf 드릴다운(선택 모델) 온디맨드 상세 — 구간별 p50/p95/p99 + E2E 지연 버킷 분포(히스토그램).
#[derive(Clone, Default)]
pub struct PerfDetail {
    pub model: String,
    pub e2e: [f64; 3],   // p50/p95/p99 (s)
    pub ttft: [f64; 3],
    pub tpot: [f64; 3],
    pub buckets: Vec<(f64, f64)>, // (le 상한 s, 해당 구간 rate) — 누적차 분포
}

/// 선택 모델의 지연 분포를 프로메테우스에서 즉석 조회(Enter 시). vLLM/ds4-proxy 엔진 구분.
pub async fn perf_detail(prom: &str, model: &str) -> PerfDetail {
    // (E2E metric, TTFT metric, TPOT metric, selector)
    let (e2e_m, ttft_m, tpot_m, sel) = if model == "ds4-proxy" {
        ("ds4_proxy_request_duration_seconds", "ds4_proxy_ttft_seconds", "", String::new())
    } else {
        (
            "vllm:e2e_request_latency_seconds",
            "vllm:time_to_first_token_seconds",
            "vllm:request_time_per_output_token_seconds",
            format!("{{service=\"{}\"}}", model),
        )
    };
    let q = |base: &str, quant: f64| {
        format!("histogram_quantile({}, sum by (le)(rate({}_bucket{}[5m])))", quant, base, sel)
    };
    let has_tpot = !tpot_m.is_empty();
    // 쿼리 문자열을 먼저 바인딩(참조가 join 전체에서 살아있도록).
    let (qe50, qe95, qe99) = (q(e2e_m, 0.5), q(e2e_m, 0.95), q(e2e_m, 0.99));
    let (qt50, qt95, qt99) = (q(ttft_m, 0.5), q(ttft_m, 0.95), q(ttft_m, 0.99));
    let (qp50, qp95, qp99) = (q(tpot_m, 0.5), q(tpot_m, 0.95), q(tpot_m, 0.99));
    let qbuckets = format!("sum by (le)(rate({}_bucket{}[5m]))", e2e_m, sel);
    let (e50, e95, e99, t50, t95, t99, p50, p95, p99, buckets) = tokio::join!(
        qs1(prom, &qe50),
        qs1(prom, &qe95),
        qs1(prom, &qe99),
        qs1(prom, &qt50),
        qs1(prom, &qt95),
        qs1(prom, &qt99),
        async { if has_tpot { qs1(prom, &qp50).await } else { f64::NAN } },
        async { if has_tpot { qs1(prom, &qp95).await } else { f64::NAN } },
        async { if has_tpot { qs1(prom, &qp99).await } else { f64::NAN } },
        prom::query(prom, &qbuckets),
    );
    // 누적 버킷 → 구간별 분포(le 오름차순, 인접 차분).
    let mut cum: Vec<(f64, f64)> = buckets
        .unwrap_or_default()
        .iter()
        .filter_map(|s| {
            let le = s.l("le");
            let up = if le == "+Inf" { f64::INFINITY } else { le.parse::<f64>().ok()? };
            Some((up, s.value))
        })
        .collect();
    cum.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut dist = Vec::new();
    let mut prev = 0.0;
    for (le, c) in &cum {
        let cnt = (c - prev).max(0.0);
        dist.push((*le, cnt));
        prev = *c;
    }
    PerfDetail {
        model: model.to_string(),
        e2e: [e50, e95, e99],
        ttft: [t50, t95, t99],
        tpot: [p50, p95, p99],
        buckets: dist,
    }
}

/// vLLM 모델 메트릭 묶음 (model_name 키).
struct Vllm {
    run: BTreeMap<String, Series>,
    wait: BTreeMap<String, Series>,
    tps: BTreeMap<String, Series>,
    kv: BTreeMap<String, Series>,
    ttft: BTreeMap<String, Series>,
}

/// deploy 컨테이너 command/args/image 로 추론 엔진 추정.
fn detect_engine(d: &serde_json::Value, accel: &str) -> String {
    let c = &d["spec"]["template"]["spec"]["containers"][0];
    let mut t = String::new();
    for key in ["command", "args"] {
        if let Some(arr) = c[key].as_array() {
            for x in arr {
                if let Some(s) = x.as_str() {
                    t.push_str(s);
                    t.push(' ');
                }
            }
        }
    }
    let t = t.to_lowercase();
    let img = c["image"].as_str().unwrap_or("").to_lowercase();
    let rbln = accel.contains("RBLN") || t.contains("rbln");
    if t.contains("sglang") {
        "SGLang".into()
    } else if t.contains("vllm") {
        if rbln { "vLLM-RBLN".into() } else { "vLLM".into() }
    } else if t.contains("ollama") || img.contains("ollama") {
        "Ollama".into()
    } else if t.contains("furiosa") {
        "Furiosa-LLM".into()
    } else if rbln {
        "custom(RBLN)".into()
    } else {
        "custom".into()
    }
}

/// 모델 deploy 가 어느 가속기/노드에서 도는지 추정(가속기 busy_model 라벨이 파드명 ⊇ deploy명).
fn accel_for(accels: &[Accel], deploy: &str) -> String {
    let mut kind = "";
    let mut node = "";
    let mut n = 0;
    for a in accels {
        if !a.busy_model.is_empty() && a.busy_model.starts_with(deploy) {
            kind = a.disp(); // 감지된 모델(GB10 등), 없으면 벤더 라벨
            node = &a.node;
            n += 1;
        }
    }
    if n > 0 {
        format!("{}×{} {}", kind, n, node)
    } else {
        "–".to_string()
    }
}

/// uuid 등 key 기준으로 first-match value 맵 (Vec 소유 → 수명 단순화).
fn map_by(series: Vec<Series>, key: &str) -> BTreeMap<String, Series> {
    let mut m = BTreeMap::new();
    for s in series {
        let k = s.l(key).to_string();
        m.entry(k).or_insert(s);
    }
    m
}

/// 소스별 가속기 수집기 — 각자 자기 메트릭만 join! → 메트릭 추가 시 해당 함수만 수정(위치 튜플 결합 제거).
async fn collect_furiosa(p: &str) -> Vec<Accel> {
    let (util, temp, pow, du, dt, alive, thr) = tokio::join!(
        prom::query(p, "avg by (uuid,device,hostname) (furiosa_npu_core_utilization)"),
        prom::query(p, "max by (uuid) (furiosa_npu_hw_temperature)"),
        prom::query(p, "max by (uuid) (furiosa_npu_hw_power)"),
        prom::query(p, "max by (uuid) (furiosa_npu_dram_usage)"),
        prom::query(p, "max by (uuid) (furiosa_npu_dram_total)"),
        prom::query(p, "max by (uuid) (furiosa_npu_alive)"),
        prom::query(p, "max by (uuid) (furiosa_npu_throttling_events_count)"),
    );
    let util = util.unwrap_or_default();
    let temp = map_by(temp.unwrap_or_default(), "uuid");
    let pow = map_by(pow.unwrap_or_default(), "uuid");
    let du = map_by(du.unwrap_or_default(), "uuid");
    let dt = map_by(dt.unwrap_or_default(), "uuid");
    let alive = map_by(alive.unwrap_or_default(), "uuid");
    let thr = map_by(thr.unwrap_or_default(), "uuid");
    util.iter().map(|s| {
        let uuid = s.l("uuid");
        Accel {
            kind: AccelKind::Rngd, model: String::new(),
            id: s.l("device").to_string(), node: s.l("hostname").to_string(),
            util: norm_pct(s.value),
            mem_used_gb: du.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            mem_total_gb: dt.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            temp: temp.get(uuid).map(|x| x.value).unwrap_or(0.0),
            power: pow.get(uuid).map(|x| x.value).unwrap_or(0.0),
            busy_model: String::new(),
            alive: alive.get(uuid).map(|x| x.value > 0.0).unwrap_or(true),
            throttle: thr.get(uuid).map(|x| x.value).unwrap_or(0.0),
            unified_mem: false, mem_bw: f64::NAN, clock_mhz: f64::NAN, mem_temp: f64::NAN, energy_mj: f64::NAN,
        }
    }).collect()
}

async fn collect_rbln(p: &str) -> Vec<Accel> {
    let (util, temp, pow, du, dt, health) = tokio::join!(
        prom::query(p, metrics::RBLN_UTIL),
        prom::query(p, metrics::RBLN_TEMP),
        prom::query(p, metrics::RBLN_POWER),
        prom::query(p, metrics::RBLN_DRAM_USED),
        prom::query(p, metrics::RBLN_DRAM_TOTAL),
        prom::query(p, metrics::RBLN_HEALTH),
    );
    let util = util.unwrap_or_default();
    let temp = map_by(temp.unwrap_or_default(), "uuid");
    let pow = map_by(pow.unwrap_or_default(), "uuid");
    let du = map_by(du.unwrap_or_default(), "uuid");
    let dt = map_by(dt.unwrap_or_default(), "uuid");
    let health = map_by(health.unwrap_or_default(), "uuid");
    util.iter().map(|s| {
        let uuid = s.l("uuid");
        Accel {
            kind: AccelKind::Rbln, model: String::new(),
            id: s.l("name").to_string(), node: s.l("node").to_string(),
            util: norm_pct(s.value),
            mem_used_gb: du.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            mem_total_gb: dt.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            temp: temp.get(uuid).map(|x| x.value).unwrap_or(0.0),
            power: pow.get(uuid).map(|x| x.value).unwrap_or(0.0),
            busy_model: s.l("exported_pod").to_string(),
            alive: health.get(uuid).map(|x| x.value == 0.0).unwrap_or(true),
            throttle: 0.0,
            unified_mem: false, mem_bw: f64::NAN, clock_mhz: f64::NAN, mem_temp: f64::NAN, energy_mj: f64::NAN,
        }
    }).collect()
}

/// NVIDIA DCGM — 모델명/총메모리/대역폭/클럭/에너지 자동 감지.
async fn collect_gpu(p: &str) -> Vec<Accel> {
    let (util, mu, mt, temp, pow, bw, clk, mtemp, energy) = tokio::join!(
        prom::query(p, metrics::DCGM_GPU_UTIL),
        prom::query(p, metrics::DCGM_FB_USED),
        prom::query(p, metrics::DCGM_FB_TOTAL),
        prom::query(p, metrics::DCGM_GPU_TEMP),
        prom::query(p, metrics::DCGM_POWER),
        prom::query(p, metrics::DCGM_MEM_COPY_UTIL),
        prom::query(p, metrics::DCGM_SM_CLOCK),
        prom::query(p, metrics::DCGM_MEM_TEMP),
        prom::query(p, metrics::DCGM_ENERGY),
    );
    let util = util.unwrap_or_default();
    let mu = map_by(mu.unwrap_or_default(), "gpu");
    let mt = map_by(mt.unwrap_or_default(), "gpu");
    let temp = map_by(temp.unwrap_or_default(), "gpu");
    let pow = map_by(pow.unwrap_or_default(), "gpu");
    let bw = map_by(bw.unwrap_or_default(), "gpu");
    let clk = map_by(clk.unwrap_or_default(), "gpu");
    let mtemp = map_by(mtemp.unwrap_or_default(), "gpu");
    let energy = map_by(energy.unwrap_or_default(), "gpu");
    util.iter().map(|s| {
        let gpu = s.l("gpu");
        let model = gpu_model(s.l("modelName"));
        let unified = is_unified(&model);
        Accel {
            kind: AccelKind::Gpu, model,
            id: format!("gpu{}", gpu), node: s.l("Hostname").to_string(),
            util: norm_pct(s.value),
            mem_used_gb: mu.get(gpu).map(|x| x.value / 1024.0).unwrap_or(0.0),
            mem_total_gb: mt.get(gpu).map(|x| x.value / 1024.0).unwrap_or(0.0),
            temp: temp.get(gpu).map(|x| x.value).unwrap_or(0.0),
            power: pow.get(gpu).map(|x| x.value).unwrap_or(0.0),
            busy_model: s.l("exported_pod").to_string(),
            alive: true, throttle: 0.0, unified_mem: unified,
            mem_bw: bw.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
            clock_mhz: clk.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
            mem_temp: mtemp.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
            energy_mj: energy.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
        }
    }).collect()
}

async fn collect_nodes(p: &str) -> Vec<NodeInfo> {
    let (load, mt, ma, cpu, node_res) = tokio::join!(
        prom::query(p, metrics::NODE_LOAD1),
        prom::query(p, "node_memory_MemTotal_bytes"),
        prom::query(p, "node_memory_MemAvailable_bytes"),
        prom::query(p, "100 - (avg by (instance)(rate(node_cpu_seconds_total{mode=\"idle\"}[1m])) * 100)"),
        node_kube(),
    );
    let (node_ip, node_meta) = node_res;
    let load = load.unwrap_or_default();
    let n_mt = map_by(mt.unwrap_or_default(), "instance");
    let n_ma = map_by(ma.unwrap_or_default(), "instance");
    let n_cpu = map_by(cpu.unwrap_or_default(), "instance");
    let resolve = |inst: &str| -> String {
        let ip = inst.split(':').next().unwrap_or(inst);
        node_ip.get(ip).cloned().unwrap_or_else(|| ip.to_string())
    };
    let mut load_by: BTreeMap<String, f64> = BTreeMap::new();
    let mut inst_by: BTreeMap<String, String> = BTreeMap::new();
    for s in &load {
        let name = resolve(s.l("instance"));
        inst_by.insert(name.clone(), s.l("instance").to_string());
        load_by.insert(name, s.value);
    }
    let mut names: Vec<String> = node_meta.keys().cloned().collect();
    if names.is_empty() {
        names = load_by.keys().cloned().collect();
    }
    let mut nodes: Vec<NodeInfo> = Vec::new();
    for name in names {
        let inst = inst_by.get(&name).cloned().unwrap_or_default();
        let mt = n_mt.get(inst.as_str()).map(|x| x.value).unwrap_or(0.0);
        let ma = n_ma.get(inst.as_str()).map(|x| x.value).unwrap_or(0.0);
        let meta = node_meta.get(&name).cloned().unwrap_or_default();
        nodes.push(NodeInfo {
            load1: load_by.get(&name).copied().unwrap_or(f64::NAN),
            mem_used_gb: to_gb(mt - ma), mem_total_gb: to_gb(mt),
            cpu_pct: n_cpu.get(inst.as_str()).map(|x| x.value).unwrap_or(f64::NAN),
            ready: meta.0, cordoned: meta.1, pressure: meta.2, version: meta.3, name,
        });
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
}

/// fast tier: 소스별 수집기 4개를 병렬 실행 후 합침. util/mem 반응성을 위해 collect()에서 분리.
pub async fn collect_fast(cfg: &Config) -> (Vec<Accel>, Vec<NodeInfo>) {
    let p = &cfg.prom;
    let (fu, rb, gpu, nodes) = tokio::join!(collect_furiosa(p), collect_rbln(p), collect_gpu(p), collect_nodes(p));
    let mut accel = fu;
    accel.extend(rb);
    accel.extend(gpu);
    accel.sort_by(|a, b| (a.kind as u8, &a.node, &a.id).cmp(&(b.kind as u8, &b.node, &b.id)));
    // 통합 메모리(GB10 등): 별도 VRAM 없음 → 노드(호스트) 메모리 풀로 backfill.
    for a in accel.iter_mut().filter(|a| a.unified_mem && a.mem_total_gb <= 0.0) {
        if let Some(n) = nodes.iter().find(|n| n.name == a.node) {
            a.mem_used_gb = n.mem_used_gb;
            a.mem_total_gb = n.mem_total_gb;
        }
    }
    (accel, nodes)
}

pub async fn collect(cfg: &Config) -> Snapshot {
    let mut snap = Snapshot::default();
    let mut warn = Vec::new();
    snap.ts = now_secs();

    // 가속기 + 노드는 fast tier(collect_fast) 재사용 — 중복 제거
    let (accel, nodes) = collect_fast(cfg).await;
    snap.accel = accel;
    snap.nodes = nodes;

    // ---------- EPP pools ----------
    let p_ready = q(cfg, metrics::POOL_READY, &mut warn).await;
    let p_q = map_by(q(cfg, metrics::POOL_QUEUE, &mut warn).await, "name");
    let p_kv = map_by(
        q(cfg, metrics::POOL_KV, &mut warn).await,
        "name",
    );
    let p_sat = map_by(
        q(cfg, metrics::POOL_SAT, &mut warn).await,
        "name",
    );
    // 라우팅 결정 분배 (트래픽이 EPP 경유 시 채워짐)
    let dec = q(cfg, "sum by (pod_name) (inference_extension_scheduler_attempts_total)", &mut warn).await;
    for s in &dec {
        let pod = s.l("pod_name");
        if !pod.is_empty() {
            snap.decisions.push((pod.to_string(), s.value));
        }
    }
    snap.decisions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let pidx = q(cfg, "max(inference_extension_prefix_indexer_size)", &mut warn).await;
    snap.prefix_idx = pidx.first().map(|s| s.value).unwrap_or(f64::NAN);

    // per-pod 큐 깊이(요청 분배)
    for s in &q(cfg, metrics::POOL_PER_POD_QUEUE, &mut warn).await {
        let pod = s.l("model_server_pod");
        if !pod.is_empty() {
            snap.pod_queues.push((pod.to_string(), s.value));
        }
    }
    snap.pod_queues.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Perf: 구간별 지연 percentile·토큰분포·처리량 (EPP 정책용). 전부 graceful(NaN).
    let pp = &cfg.prom;
    // 전부 vLLM 네이티브 메트릭(EPP 우회 여부와 무관하게 model-server 에서 export).
    let (req_rate, err_rate, tps, prefix_hit, ttft_p95, e2e_p95) = tokio::join!(
        qs1(pp, "sum(rate(vllm:request_success_total[1m]))"),
        qs1(pp, "sum(rate(vllm:request_success_total{finished_reason=\"abort\"}[1m]))"),
        qs1(pp, "sum(rate(vllm:generation_tokens_total[1m]))"),
        qs1(pp, "sum(rate(vllm:prefix_cache_hits_total[5m])) / sum(rate(vllm:prefix_cache_queries_total[5m]))"),
        qs1(pp, "histogram_quantile(0.95, sum by (le)(rate(vllm:time_to_first_token_seconds_bucket[1m])))"),
        qs1(pp, "histogram_quantile(0.95, sum by (le)(rate(vllm:e2e_request_latency_seconds_bucket[1m])))"),
    );
    snap.perf = Perf { req_rate, err_rate, tps, prefix_hit, ttft_p95, e2e_p95 };

    // per-model 성능 — service(=Deployment 이름) 기준 병합. Models 뷰와 동일 키.
    // 여기선 pm 만 채우고, 런칭된 모델 seed 는 collect_kube(모델 목록) 이후에 좌조인(아래).
    let mut pm: BTreeMap<String, PerfRow> = BTreeMap::new();
    macro_rules! merge {
        ($promql:expr, $field:ident) => {
            for s in &q(cfg, $promql, &mut warn).await {
                let m = s.l("service").to_string();
                if !m.is_empty() {
                    pm.entry(m.clone()).or_insert_with(|| PerfRow::new(&m)).$field = s.value;
                }
            }
        };
    }
    merge!("sum by (service)(rate(vllm:request_success_total[1m]))", req);
    merge!("sum by (service)(rate(vllm:generation_tokens_total[1m]))", tps);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:time_to_first_token_seconds_bucket[1m])))", ttft_p95);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_time_per_output_token_seconds_bucket[1m])))", tpot_p95);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:e2e_request_latency_seconds_bucket[1m])))", e2e_p95);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_prompt_tokens_bucket[1m])))", in_tok_p95);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_generation_tokens_bucket[1m])))", out_tok_p95);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_queue_time_seconds_bucket[1m])))", queue_p95);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_prefill_time_seconds_bucket[1m])))", prefill_p95);
    merge!("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_decode_time_seconds_bucket[1m])))", decode_p95);
    merge!("sum by (service)(rate(vllm:num_preemptions_total[1m]))", preempt);

    for s in &p_ready {
        let name = s.l("name");
        snap.pools.push(Pool {
            name: name.to_string(),
            ready: s.value,
            queue: p_q.get(name).map(|x| x.value).unwrap_or(f64::NAN),
            kv: p_kv.get(name).map(|x| x.value).unwrap_or(f64::NAN),
            sat: p_sat.get(name).map(|x| x.value).unwrap_or(f64::NAN),
            ..Default::default()
        });
    }

    // vLLM model metrics (모델 서버에 vLLM ServiceMonitor + 트래픽 있을 때 채워짐)
    let vllm = Vllm {
        // service 라벨 = Deployment 이름(예: gemma4-rbln) → deploy 와 정확히 join.
        run: map_by(q(cfg, "sum by (service) (vllm:num_requests_running)", &mut warn).await, "service"),
        wait: map_by(q(cfg, "sum by (service) (vllm:num_requests_waiting)", &mut warn).await, "service"),
        tps: map_by(
            q(cfg, "sum by (service) (rate(vllm:generation_tokens_total[1m]))", &mut warn).await,
            "service",
        ),
        kv: map_by(q(cfg, "max by (service) (vllm:kv_cache_usage_perc)", &mut warn).await, "service"),
        ttft: map_by(
            q(
                cfg,
                "histogram_quantile(0.95, sum by (service,le) (rate(vllm:time_to_first_token_seconds_bucket[1m])))",
                &mut warn,
            )
            .await,
            "service",
        ),
    };

    // ---------- kube: deployments / pods / routes / gateway / epp ----------
    collect_kube(cfg, &mut snap, &vllm, &mut warn).await;

    // per-model perf: 런칭된 모든 모델(snap.models)을 seed 로 → 트래픽 없어도 표(–)에 표시.
    // 배포명↔service 는 Models 뷰와 동일한 match_model 로 해석, pm(메트릭)을 좌조인.
    {
        let mut rows: Vec<PerfRow> = Vec::new();
        for m in &snap.models {
            let key = match_model(&m.name, &vllm.run);
            let mut row = key.and_then(|k| pm.remove(&k)).unwrap_or_else(|| PerfRow::new(&m.name));
            row.model = m.name.clone(); // 표시명을 배포명으로 통일(Models 와 일관)
            rows.push(row);
        }
        rows.extend(pm.into_values()); // 모델 목록엔 없지만 메트릭만 있는 잔여
        snap.perf_rows = rows;
    }

    // 커스텀 엔진 어댑터: ds4-inject-proxy 는 vLLM 이 아니라 `ds4_proxy_*` 로 노출 →
    // vllm:* 쿼리로는 안 잡힘. 프록시가 백엔드 라벨 없이 집계하므로 집계 perf 행 1개로 반영.
    // (저트래픽이라 5m 윈도로 안정화. 일반화는 로드맵의 선언적 collector.)
    {
        let pp = &cfg.prom;
        let (req, tok, ttft, e2e) = tokio::join!(
            qs1(pp, "sum(rate(ds4_proxy_requests_total[5m]))"),
            qs1(pp, "sum(rate(ds4_proxy_output_tokens_total[5m]))"),
            qs1(pp, "histogram_quantile(0.95, sum by (le)(rate(ds4_proxy_ttft_seconds_bucket[5m])))"),
            qs1(pp, "histogram_quantile(0.95, sum by (le)(rate(ds4_proxy_request_duration_seconds_bucket[5m])))"),
        );
        if !req.is_nan() || !ttft.is_nan() || !e2e.is_nan() {
            let mut r = PerfRow::new("ds4-proxy");
            r.req = req;
            r.tps = tok;
            r.ttft_p95 = ttft;
            r.e2e_p95 = e2e;
            snap.perf_rows.push(r);
        }
    }

    // ---------- 가속기 재고(런처 솔버용) ----------
    collect_inventory(&mut snap.inventory, &mut warn).await;

    snap.warnings = warn;
    snap
}

const ACCEL_RESOURCES: [&str; 3] = ["nvidia.com/gpu", "rebellions.ai/ATOM", "furiosa.ai/rngd"];

/// 노드 allocatable 합계 − 파드 requests 합계 = 여유. (가속기 resource별)
async fn collect_inventory(inv: &mut Vec<(String, i64, i64)>, warn: &mut Vec<String>) {
    let mut total: BTreeMap<String, i64> = BTreeMap::new();
    let mut used: BTreeMap<String, i64> = BTreeMap::new();
    for r in ACCEL_RESOURCES {
        total.insert(r.to_string(), 0);
        used.insert(r.to_string(), 0);
    }
    // 노드 allocatable
    match kube::get_json(&["get", "nodes", "-o", "json"]).await {
        Ok(v) => {
            if let Some(items) = v["items"].as_array() {
                for n in items {
                    if let Some(a) = n["status"]["allocatable"].as_object() {
                        for r in ACCEL_RESOURCES {
                            if let Some(q) = a.get(r).and_then(|x| x.as_str()).and_then(|s| s.parse::<i64>().ok()) {
                                *total.get_mut(r).unwrap() += q;
                            }
                        }
                    }
                }
            }
        }
        Err(e) => warn.push(format!("inv nodes: {}", e)),
    }
    // 파드 requests (비종료 파드, 전 네임스페이스)
    if let Ok(v) = kube::get_json(&["get", "pods", "-A", "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for p in items {
                let phase = p["status"]["phase"].as_str().unwrap_or("");
                if phase == "Succeeded" || phase == "Failed" {
                    continue;
                }
                if let Some(cs) = p["spec"]["containers"].as_array() {
                    for c in cs {
                        if let Some(req) = c["resources"]["requests"].as_object() {
                            for r in ACCEL_RESOURCES {
                                if let Some(q) = req.get(r).and_then(|x| x.as_str()).and_then(|s| s.parse::<i64>().ok()) {
                                    *used.get_mut(r).unwrap() += q;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    for r in ACCEL_RESOURCES {
        inv.push((r.to_string(), total[r], used[r]));
    }
}

fn norm_pct(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else if v <= 1.0 && v > 0.0 {
        v * 100.0
    } else {
        v
    }
}

type NodeMeta = (bool, bool, bool, String); // (ready, cordoned, pressure, version)

async fn node_kube() -> (BTreeMap<String, String>, BTreeMap<String, NodeMeta>) {
    let mut ip = BTreeMap::new();
    let mut meta = BTreeMap::new();
    match kube::get_json(&["get", "nodes", "-o", "json"]).await {
        Ok(v) => {
            if let Some(items) = v["items"].as_array() {
                for it in items {
                    let name = it["metadata"]["name"].as_str().unwrap_or("").to_string();
                    if let Some(addrs) = it["status"]["addresses"].as_array() {
                        for a in addrs {
                            if a["type"] == "InternalIP" {
                                if let Some(x) = a["address"].as_str() {
                                    ip.insert(x.to_string(), name.clone());
                                }
                            }
                        }
                    }
                    let ver = it["status"]["nodeInfo"]["kubeletVersion"].as_str().unwrap_or("").to_string();
                    let cordoned = it["spec"]["unschedulable"].as_bool().unwrap_or(false);
                    let (mut ready, mut pressure) = (false, false);
                    if let Some(cs) = it["status"]["conditions"].as_array() {
                        for c in cs {
                            let t = c["type"].as_str().unwrap_or("");
                            let st = c["status"] == "True";
                            match t {
                                "Ready" => ready = st,
                                "MemoryPressure" | "DiskPressure" | "PIDPressure" => {
                                    if st {
                                        pressure = true
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    meta.insert(name, (ready, cordoned, pressure, ver));
                }
            }
        }
        Err(_) => {}
    }
    (ip, meta)
}

async fn collect_kube(cfg: &Config, snap: &mut Snapshot, vllm: &Vllm, warn: &mut Vec<String>) {
    // routes: path -> backend (+ kind: Service|InferencePool)
    if let Ok(v) = kube::get_json(&["get", "httproute", "-n", &cfg.ns, "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for r in items {
                if let Some(rules) = r["spec"]["rules"].as_array() {
                    for rule in rules {
                        let backend = rule["backendRefs"][0]["name"].as_str().unwrap_or("").to_string();
                        let kind = rule["backendRefs"][0]["kind"].as_str().unwrap_or("Service").to_string();
                        if kind == "InferencePool" {
                            snap.epp_in_path = true;
                        }
                        if let Some(matches) = rule["matches"].as_array() {
                            for m in matches {
                                if let Some(p) = m["path"]["value"].as_str() {
                                    snap.routes.push(Route {
                                        path: p.to_string(),
                                        backend: backend.clone(),
                                        kind: kind.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        warn.push("kube httproute".into());
    }
    let route_for = |backend: &str| -> String {
        snap.routes
            .iter()
            .filter(|r| r.backend == backend)
            .map(|r| r.path.clone())
            .collect::<Vec<_>>()
            .join(",")
    };

    // deployments → models
    match kube::get_json(&["get", "deploy", "-n", &cfg.ns, "-o", "json"]).await {
        Ok(v) => {
            if let Some(items) = v["items"].as_array() {
                for d in items {
                    let name = d["metadata"]["name"].as_str().unwrap_or("").to_string();
                    if name.contains("epp") || name.contains("router") {
                        continue; // 라우터는 모델 아님
                    }
                    let desired = d["spec"]["replicas"].as_i64().unwrap_or(0);
                    let ready = d["status"]["readyReplicas"].as_i64().unwrap_or(0);
                    let status = if desired == 0 {
                        "○ Scaled-0".to_string()
                    } else if ready >= desired {
                        "● Running".to_string()
                    } else {
                        "◐ Pending".to_string()
                    };
                    // vllm 메트릭 fuzzy 매칭 (model_name ⊃/⊂ deploy 토큰)
                    let mn = match_model(name.as_str(), &vllm.run);
                    let accel = accel_for(&snap.accel, &name);
                    let engine = detect_engine(d, &accel);
                    snap.models.push(ModelRow {
                        route: route_for(&name),
                        engine,
                        accel,
                        running: mn.as_ref().and_then(|k| vllm.run.get(k)).map(|x| x.value),
                        waiting: mn.as_ref().and_then(|k| vllm.wait.get(k)).map(|x| x.value),
                        tps: mn.as_ref().and_then(|k| vllm.tps.get(k)).map(|x| x.value),
                        kv: mn.as_ref().and_then(|k| vllm.kv.get(k)).map(|x| x.value),
                        ttft: mn.as_ref().and_then(|k| vllm.ttft.get(k)).map(|x| x.value),
                        name,
                        ready,
                        desired,
                        status,
                    });
                }
            }
        }
        Err(e) => warn.push(format!("kube deploy: {}", e)),
    }
    snap.models.sort_by(|a, b| a.name.cmp(&b.name));

    // pods
    match kube::get_json(&["get", "pods", "-n", &cfg.ns, "-o", "json"]).await {
        Ok(v) => {
            if let Some(items) = v["items"].as_array() {
                for p in items {
                    let name = p["metadata"]["name"].as_str().unwrap_or("").to_string();
                    let phase = p["status"]["phase"].as_str().unwrap_or("?").to_string();
                    let node = p["spec"]["nodeName"].as_str().unwrap_or("-").to_string();
                    let (mut rc, mut total, mut readyc, mut restarts) = (0, 0, 0, 0i64);
                    if let Some(cs) = p["status"]["containerStatuses"].as_array() {
                        total = cs.len();
                        for c in cs {
                            if c["ready"].as_bool().unwrap_or(false) {
                                readyc += 1;
                            }
                            restarts += c["restartCount"].as_i64().unwrap_or(0);
                            rc += 1;
                        }
                    }
                    let _ = rc;
                    snap.pods.push(PodRow {
                        name,
                        phase,
                        ready: format!("{}/{}", readyc, total),
                        node,
                        restarts,
                    });
                }
            }
        }
        Err(e) => warn.push(format!("kube pods: {}", e)),
    }
    snap.pods.sort_by(|a, b| a.name.cmp(&b.name));

    // events (최근순) — llm-d/k8s 이벤트 통합
    if let Ok(v) = kube::get_json(&["get", "events", "-n", &cfg.ns, "--sort-by=.lastTimestamp", "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for e in items.iter().rev().take(40) {
                let obj = format!(
                    "{}/{}",
                    e["involvedObject"]["kind"].as_str().unwrap_or(""),
                    e["involvedObject"]["name"].as_str().unwrap_or("")
                );
                snap.events.push(EventRow {
                    typ: e["type"].as_str().unwrap_or("Normal").to_string(),
                    reason: e["reason"].as_str().unwrap_or("").to_string(),
                    object: obj,
                    message: e["message"].as_str().unwrap_or("").to_string(),
                    count: e["count"].as_i64().unwrap_or(1),
                });
            }
        }
    }

    // gateway
    if let Ok(v) = kube::get_json(&["get", "gateway", "-n", &cfg.ns, "-o", "json"]).await {
        if let Some(g) = v["items"].as_array().and_then(|a| a.first()) {
            snap.gw_addr = g["status"]["addresses"][0]["value"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if let Some(conds) = g["status"]["conditions"].as_array() {
                snap.gw_ok = conds
                    .iter()
                    .any(|c| c["type"] == "Programmed" && c["status"] == "True");
            }
        }
    }

    // EPP config (ConfigMap)
    if let Ok(v) = kube::get_json(&["get", "cm", "llmd-router-epp", "-n", &cfg.ns, "-o", "json"]).await
    {
        if let Some(text) = kube::cm_data(&v, "default-plugins.yaml") {
            snap.epp = parse_epp(text);
        }
    }

    // InferencePool 스펙(selector / EPP / endpoints) → prom Pool 과 병합
    if let Ok(v) = kube::get_json(&["get", "inferencepool", "-n", &cfg.ns, "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for ip in items {
                let name = ip["metadata"]["name"].as_str().unwrap_or("").to_string();
                let epp = ip["spec"]["endpointPickerRef"]["name"].as_str().unwrap_or("").to_string();
                let mut sel = Vec::new();
                if let Some(ml) = ip["spec"]["selector"]["matchLabels"].as_object() {
                    for (k, val) in ml {
                        if let Some(s) = val.as_str() {
                            sel.push(format!("{}={}", k, s));
                        }
                    }
                }
                let selector = sel.join(",");
                let (mut ep_total, mut ep_ready) = (0i64, 0i64);
                if !selector.is_empty() {
                    if let Ok(pj) =
                        kube::get_json(&["get", "pods", "-n", &cfg.ns, "-l", &selector, "-o", "json"]).await
                    {
                        if let Some(ps) = pj["items"].as_array() {
                            ep_total = ps.len() as i64;
                            for p in ps {
                                let ready = p["status"]["containerStatuses"]
                                    .as_array()
                                    .map(|cs| !cs.is_empty() && cs.iter().all(|c| c["ready"].as_bool().unwrap_or(false)))
                                    .unwrap_or(false);
                                if ready {
                                    ep_ready += 1;
                                }
                            }
                        }
                    }
                }
                if let Some(pool) = snap.pools.iter_mut().find(|p| p.name == name) {
                    pool.selector = selector;
                    pool.epp = epp;
                    pool.ep_total = ep_total;
                    pool.ep_ready = ep_ready;
                } else {
                    snap.pools.push(Pool {
                        name,
                        selector,
                        epp,
                        ep_total,
                        ep_ready,
                        ready: f64::NAN,
                        queue: f64::NAN,
                        kv: f64::NAN,
                        sat: f64::NAN,
                    });
                }
            }
        }
    }

    // InferenceObjective (SLO priority)
    if let Ok(v) = kube::get_json(&["get", "inferenceobjective", "-n", &cfg.ns, "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for o in items {
                snap.objectives.push(Objective {
                    name: o["metadata"]["name"].as_str().unwrap_or("").to_string(),
                    priority: o["spec"]["priority"].as_i64().unwrap_or(0),
                    pool: o["spec"]["poolRef"]["name"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }
    snap.objectives.sort_by(|a, b| b.priority.cmp(&a.priority));

    // 오토스케일링 (KEDA ScaledObject + 상태)
    if let Ok(v) = kube::get_json(&["get", "scaledobject", "-n", &cfg.ns, "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for so in items {
                let target = so["spec"]["scaleTargetRef"]["name"].as_str().unwrap_or("").to_string();
                let min = so["spec"]["minReplicaCount"].as_i64().unwrap_or(0);
                let max = so["spec"]["maxReplicaCount"].as_i64().unwrap_or(0);
                let conds = so["status"]["conditions"].as_array();
                let cond = |t: &str| {
                    conds
                        .and_then(|c| c.iter().find(|x| x["type"] == t))
                        .map(|x| x["status"] == "True")
                        .unwrap_or(false)
                };
                let triggers = so["spec"]["triggers"]
                    .as_array()
                    .map(|ts| ts.iter().filter_map(|t| t["type"].as_str()).collect::<Vec<_>>().join(","))
                    .unwrap_or_default();
                let replicas = snap.models.iter().find(|m| m.name == target).map(|m| m.ready).unwrap_or(0);
                snap.autoscalers.push(Autoscale {
                    target,
                    min,
                    max,
                    replicas,
                    ready: cond("Ready"),
                    active: cond("Active"),
                    triggers,
                });
            }
        }
    }
}

fn match_model(deploy: &str, v_run: &BTreeMap<String, Series>) -> Option<String> {
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let dn = norm(deploy);
    for k in v_run.keys() {
        let kn = norm(k);
        if !kn.is_empty() && (dn.contains(&kn) || kn.contains(&dn)) {
            return Some(k.clone());
        }
    }
    None
}

fn parse_epp(text: &str) -> Option<EppCfg> {
    let v: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    let prof = &v["schedulingProfiles"][0];
    let profile = prof["name"].as_str().unwrap_or("default").to_string();
    let mut scorers = Vec::new();
    if let Some(plugins) = prof["plugins"].as_sequence() {
        for p in plugins {
            let name = p["pluginRef"].as_str().unwrap_or("").to_string();
            let w = p["weight"].as_f64().unwrap_or(1.0);
            if !name.is_empty() {
                scorers.push((name, w));
            }
        }
    }
    Some(EppCfg {
        profile,
        scorers,
        picker: "max-score-picker (default)".to_string(),
    })
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
