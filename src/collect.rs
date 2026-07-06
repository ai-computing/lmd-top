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
    pub model: String, // 실제 모델(예: GB10/A100/H100) — 메트릭 자동 감지, 없으면 ""
    pub id: String,    // rbln0 / npu0 / gpu0
    pub node: String,
    pub util: f64, // 0..100
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
        if self.model.is_empty() {
            self.kind.label()
        } else {
            self.model.as_str()
        }
    }

}

/// 통합 메모리(Grace 계열 superchip: GB10/GH200/GB200/GB300) 여부 — 별도 VRAM 없이 호스트와 공유.
fn is_unified(model: &str) -> bool {
    let m = model.to_uppercase();
    m.starts_with("GB10")
        || m.starts_with("GH200")
        || m.starts_with("GB200")
        || m.starts_with("GB300")
}

#[derive(Clone, Default)]
pub struct NodeInfo {
    pub name: String,
    pub load1: f64,
    pub mem_used_gb: f64,
    pub mem_total_gb: f64,
    pub cpu_pct: f64,
    pub disk_used_gb: f64, // 루트 파일시스템(mountpoint="/")
    pub disk_total_gb: f64,
    pub ready: bool,
    pub cordoned: bool,
    pub pressure: bool, // Memory/Disk/PID pressure 중 하나라도
    pub version: String,
    pub npu: String, // NPU 드라이버/SDK 요약(노드 라벨 기반, 예: "RNGD drv2026.3.0 · RBLN drv3.0.0"). 컴파일 가능 노드 판별용.
}

#[derive(Clone, Default)]
pub struct Pool {
    pub name: String,
    pub ready: f64,
    pub queue: f64,
    pub kv: f64,
    pub sat: f64,
    pub selector: String, // app=vllm-rbln-llama31-8b
    pub epp: String,      // endpointPickerRef (EPP service)
    pub ep_ready: i64,    // selector 매칭 파드 중 ready
    pub ep_total: i64,    // selector 매칭 파드 총수
}

#[derive(Clone)]
pub struct Route {
    pub path: String,
    pub backend: String,
    pub kind: String,  // Service | InferencePool
    pub route: String, // 소속 HTTPRoute 이름(편집 대상 지정용)
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
    pub accel: String,  // 어떤 가속기/노드에서 도는지(파드 노드/가속기 추정)
    pub running: Option<f64>,
    pub waiting: Option<f64>,
    pub tps: Option<f64>,
    pub kv: Option<f64>,   // vllm:gpu_cache_usage_perc (0..1)
    pub ttft: Option<f64>, // TTFT p95 (s)
}

/// 공유 모델 스토어 인벤토리 항목(디스커버리 CronJob → model-inventory ConfigMap).
/// 배포 여부와 무관하게 "스토어에 존재하는" HF 원본/NPU 컴파일본.
#[derive(Clone, Default)]
pub struct StoredModel {
    pub repo: String,         // org/name (HF id) 또는 로컬 식별자
    pub family: String,       // 트리 그룹 키(정규화)
    pub revision: String,     // HF revision 또는 "-"
    pub format: String,       // hf | rbln | furiosa
    pub compiled_for: String, // 컴파일 타깃/옵션(예: RBLN-CA22-tp4-s8192) 또는 "-"
    pub size: String,         // du -sh 결과
    pub path: String,         // 스토어 내 상대 경로
}

/// 컴파일 Job 진행 상태(Deploy 뷰 '진행 중 컴파일' 패널). `compile-*` Job 을 요약.
#[derive(Clone, Default)]
pub struct CompileJob {
    pub name: String,               // Job 이름(compile-{모델}-{타깃})
    pub model: String,              // 사람이 읽는 모델(이름에서 추출)
    pub vendor: String,             // RBLN | RNGD | -
    pub target: String,             // 옵션 요약(tp/pp/seq 등, 이름의 타깃 부분)
    pub status: String,             // Running | Complete | Failed | Pending
    pub age_secs: u64,              // 시작(없으면 생성) 후 경과 초
    pub duration_secs: Option<u64>, // 완료된 경우 소요 초
    pub phase: String,              // 진행 힌트(파드 로그 마지막 줄 또는 상태)
    pub progress: Option<f32>,      // 로그에서 파싱한 진행률 0.0~1.0(없으면 indeterminate 바)
}

/// Store 뷰용 — 모델이 "어디에(경로/볼륨)" 저장되고 "어떤 옵션으로" 컴파일/서빙되는지.
/// deploy 컨테이너 spec(command/args/env/volumeMounts)에서 추출(휴리스틱).
#[derive(Clone, Default)]
pub struct ModelArtifact {
    pub model: String,               // deploy 이름(=변형 이름)
    pub family: String,              // 트리 그룹 키(모델 계열, source/name 에서 정규화)
    pub engine: String,              // 추론 엔진
    pub node: String,                // 저장/구동 노드(파드 스케줄 노드)
    pub image: String,               // 컨테이너 이미지
    pub source: String,              // 모델 소스: HF id / --model 경로 / MODEL_ID
    pub mount: String,               // 저장 위치: "mountPath ← PVC/hostPath/emptyDir"
    pub opts: Vec<(String, String)>, // 컴파일/서빙 옵션(TP·PP·max-len·batch·dtype·quant·NPU bucket 등)
}

#[derive(Clone)]
pub struct EventRow {
    pub typ: String, // Normal | Warning
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
    pub age_secs: u64, // 생성 후 경과(초) — 배포 시작 시각 표시용. 0=미상.
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
    pub artifacts: Vec<ModelArtifact>, // Store 뷰: 모델 저장 위치 + 컴파일/서빙 옵션
    pub stored: Vec<StoredModel>, // 공유 스토어 인벤토리(model-inventory ConfigMap) — 배포 무관
    pub compiles: Vec<CompileJob>, // 진행/최근 컴파일 Job(compile-*) — Deploy 뷰 모니터 패널
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
    pub node_alloc: std::collections::BTreeMap<String, std::collections::BTreeMap<String, i64>>, // node → resource → 할당(파드 requests)
    pub warnings: Vec<String>,
    pub prom_ok: bool, // Prometheus 도달 가능 여부(false 면 "가속기 없음"이 아니라 연결 문제)
}

impl Snapshot {
    /// 서빙 중(ready>0)인 모델 수 — 진단·요약·헤더가 공유하는 파생 지표.
    pub fn serving_count(&self) -> usize {
        self.models.iter().filter(|m| m.ready > 0).count()
    }
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
        Perf {
            req_rate: n,
            err_rate: n,
            tps: n,
            prefix_hit: n,
            ttft_p95: n,
            e2e_p95: n,
        }
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
            req: n,
            tps: n,
            ttft_p95: n,
            tpot_p95: n,
            e2e_p95: n,
            in_tok_p95: n,
            out_tok_p95: n,
            queue_p95: n,
            prefill_p95: n,
            decode_p95: n,
            preempt: n,
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

/// prom::query 결과에 q() 와 동일한 warn 처리 — tokio::join! 로 병렬 조회 후 결과 해소용.
fn resolve(
    r: Result<Vec<Series>, anyhow::Error>,
    promql: &str,
    warn: &mut Vec<String>,
) -> Vec<Series> {
    match r {
        Ok(v) => v,
        Err(e) => {
            warn.push(format!("prom[{}]: {}", short(promql), e));
            Vec::new()
        }
    }
}

/// 단일 스칼라 결과(첫 값) — 없으면 NaN. (join! 병렬용, warn 없음)
async fn qs1(prom_base: &str, promql: &str) -> f64 {
    prom::query(prom_base, promql)
        .await
        .ok()
        .and_then(|v| v.first().map(|s| s.value))
        .unwrap_or(f64::NAN)
}

/// Perf 드릴다운(선택 모델) 온디맨드 상세 — 구간별 p50/p95/p99 + E2E 지연 버킷 분포(히스토그램).
#[derive(Clone, Default)]
pub struct PerfDetail {
    pub model: String,
    pub e2e: [f64; 3], // p50/p95/p99 (s)
    pub ttft: [f64; 3],
    pub tpot: [f64; 3],
    pub buckets: Vec<(f64, f64)>, // (le 상한 s, 해당 구간 rate) — 누적차 분포
}

/// 선택 모델의 지연 분포를 프로메테우스에서 즉석 조회(Enter 시). vLLM/ds4-proxy 엔진 구분.
pub async fn perf_detail(prom: &str, model: &str) -> PerfDetail {
    // (E2E metric, TTFT metric, TPOT metric, selector)
    let (e2e_m, ttft_m, tpot_m, sel) = if model == "ds4-proxy" {
        (
            "ds4_proxy_request_duration_seconds",
            "ds4_proxy_ttft_seconds",
            "",
            String::new(),
        )
    } else {
        (
            "vllm:e2e_request_latency_seconds",
            "vllm:time_to_first_token_seconds",
            "vllm:request_time_per_output_token_seconds",
            format!("{{service=\"{}\"}}", model),
        )
    };
    let q = |base: &str, quant: f64| {
        format!(
            "histogram_quantile({}, sum by (le)(rate({}_bucket{}[5m])))",
            quant, base, sel
        )
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
        async {
            if has_tpot {
                qs1(prom, &qp50).await
            } else {
                f64::NAN
            }
        },
        async {
            if has_tpot {
                qs1(prom, &qp95).await
            } else {
                f64::NAN
            }
        },
        async {
            if has_tpot {
                qs1(prom, &qp99).await
            } else {
                f64::NAN
            }
        },
        prom::query(prom, &qbuckets),
    );
    // 누적 버킷 → 구간별 분포(le 오름차순, 인접 차분).
    let mut cum: Vec<(f64, f64)> = buckets
        .unwrap_or_default()
        .iter()
        .filter_map(|s| {
            let le = s.l("le");
            let up = if le == "+Inf" {
                f64::INFINITY
            } else {
                le.parse::<f64>().ok()?
            };
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
/// pod spec 의 여러 컨테이너 중 모델 서버 컨테이너 선택(프록시/사이드카 제외).
/// llm-d/게이트웨이는 프록시+모델서버 사이드카 패턴이 흔해 [0] 이 프록시일 수 있음.
fn model_container(spec: &serde_json::Value) -> &serde_json::Value {
    let arr = match spec["containers"].as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return &spec["containers"][0],
    };
    let is_sidecar = |c: &serde_json::Value| {
        let n = c["name"].as_str().unwrap_or("").to_lowercase();
        let img = c["image"].as_str().unwrap_or("").to_lowercase();
        [
            "proxy", "sidecar", "istio", "envoy", "-router", "gateway", "dcgm", "exporter",
            "vector", "fluent", "otel",
        ]
        .iter()
        .any(|k| n.contains(k) || img.contains(k))
    };
    let looks_server = |c: &serde_json::Value| {
        let mut t = String::new();
        for key in ["command", "args"] {
            if let Some(a) = c[key].as_array() {
                for x in a {
                    if let Some(s) = x.as_str() {
                        t.push_str(&s.to_lowercase());
                        t.push(' ');
                    }
                }
            }
        }
        let img = c["image"].as_str().unwrap_or("").to_lowercase();
        [
            "vllm",
            "sglang",
            "furiosa",
            "ollama",
            "--model",
            "optimum",
            "text-generation",
        ]
        .iter()
        .any(|k| t.contains(k) || img.contains(k))
    };
    if let Some(c) = arr.iter().find(|c| looks_server(c)) {
        return c;
    }
    if let Some(c) = arr.iter().find(|c| !is_sidecar(c)) {
        return c;
    }
    &arr[0]
}

fn detect_engine(d: &serde_json::Value, accel: &str) -> String {
    let c = model_container(&d["spec"]["template"]["spec"]);
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
        if rbln {
            "vLLM-RBLN".into()
        } else {
            "vLLM".into()
        }
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

/// deploy 컨테이너 spec(command/args/env/volumeMounts)에서 모델 저장 위치 + 컴파일/서빙 옵션 추출.
fn model_artifact(d: &serde_json::Value, name: &str, engine: &str) -> ModelArtifact {
    let pod = &d["spec"]["template"]["spec"];
    let c = model_container(pod);
    let image = c["image"].as_str().unwrap_or("").to_string();

    let mut toks: Vec<String> = Vec::new();
    for key in ["command", "args"] {
        if let Some(arr) = c[key].as_array() {
            for x in arr {
                if let Some(s) = x.as_str() {
                    toks.push(s.to_string());
                }
            }
        }
    }
    let mut env: Vec<(String, String)> = Vec::new();
    if let Some(arr) = c["env"].as_array() {
        for e in arr {
            if let (Some(n), Some(v)) = (e["name"].as_str(), e["value"].as_str()) {
                env.push((n.to_string(), v.to_string()));
            }
        }
    }
    // `--key value` / `--key=value` 값 추출.
    let arg_val = |flag: &str| -> Option<String> {
        for (i, t) in toks.iter().enumerate() {
            if let Some(rest) = t.strip_prefix(flag) {
                if let Some(v) = rest.strip_prefix('=') {
                    return Some(v.to_string());
                }
                if rest.is_empty() {
                    return toks.get(i + 1).cloned();
                }
            }
        }
        None
    };
    let env_val = |keys: &[&str]| -> Option<String> {
        env.iter()
            .find(|(n, _)| keys.iter().any(|k| n.eq_ignore_ascii_case(k)))
            .map(|(_, v)| v.clone())
    };

    // HF id(`org/model`) 또는 경로처럼 보이는 토큰만(셸 스크립트 인자 `sh -c "…"` 는 공백/개행 포함 → 제외).
    let looks_like_model = |t: &&String| -> bool {
        !t.starts_with('-')
            && t.contains('/')
            && !t
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, ';' | '#' | '&' | '|' | '='))
    };
    let source = arg_val("--model")
        .or_else(|| arg_val("--model-path"))
        .or_else(|| {
            env_val(&[
                "MODEL_ID",
                "HF_MODEL_ID",
                "MODEL_PATH",
                "MODEL",
                "HF_MODEL",
                "SERVED_MODEL_NAME",
            ])
        })
        .or_else(|| toks.iter().find(looks_like_model).cloned())
        .unwrap_or_default();

    // 저장 위치: model/hf/cache/weight/rbln/npu 힌트가 있는 volumeMount 우선, 없으면 첫 번째.
    let mut mount = String::new();
    if let Some(vms) = c["volumeMounts"].as_array() {
        let pick = vms
            .iter()
            .find(|m| {
                let p = m["mountPath"].as_str().unwrap_or("").to_lowercase();
                let n = m["name"].as_str().unwrap_or("").to_lowercase();
                [
                    "model", "hf", "cache", "data", "weight", "ckpt", "rbln", "npu",
                ]
                .iter()
                .any(|h| p.contains(h) || n.contains(h))
            })
            .or_else(|| vms.first());
        if let Some(m) = pick {
            let mp = m["mountPath"].as_str().unwrap_or("").to_string();
            let vn = m["name"].as_str().unwrap_or("");
            let backing = pod["volumes"]
                .as_array()
                .and_then(|vs| vs.iter().find(|v| v["name"].as_str() == Some(vn)))
                .map(|v| {
                    if let Some(pvc) = v["persistentVolumeClaim"]["claimName"].as_str() {
                        format!("PVC:{}", pvc)
                    } else if let Some(hp) = v["hostPath"]["path"].as_str() {
                        format!("host:{}", hp)
                    } else if v.get("emptyDir").is_some() {
                        "emptyDir".into()
                    } else if let Some(cm) = v["configMap"]["name"].as_str() {
                        format!("cm:{}", cm)
                    } else {
                        "vol".into()
                    }
                })
                .unwrap_or_default();
            mount = if backing.is_empty() {
                mp
            } else {
                format!("{} ← {}", mp, backing)
            };
        }
    }

    // 컴파일/서빙 옵션.
    let mut opts: Vec<(String, String)> = Vec::new();
    let mut push = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            if !v.is_empty() {
                opts.push((k.into(), v));
            }
        }
    };
    // 병렬화(공통): TP/PP/DP.
    push(
        "tp",
        arg_val("--tensor-parallel-size")
            .or_else(|| arg_val("-tp"))
            .or_else(|| env_val(&["TENSOR_PARALLEL_SIZE", "RBLN_TENSOR_PARALLEL_SIZE"])),
    );
    push(
        "pp",
        arg_val("--pipeline-parallel-size").or_else(|| arg_val("-pp")),
    );
    push(
        "dp",
        arg_val("--data-parallel-size").or_else(|| arg_val("-dp")),
    );
    // 길이/배치(NPU 는 컴파일 시 고정되는 값 — RBLN max_seq_len / Furiosa bucket).
    push(
        "max-len",
        arg_val("--max-model-len")
            .or_else(|| arg_val("--max-seq-len"))
            .or_else(|| env_val(&["RBLN_MAX_SEQ_LEN", "MAX_SEQ_LEN"])),
    );
    push("max-seqs", arg_val("--max-num-seqs"));
    push(
        "batch",
        arg_val("--max-num-batched-tokens")
            .or_else(|| env_val(&["BATCH_SIZE", "MAX_BATCH_SIZE", "RBLN_BATCH_SIZE"])),
    );
    push(
        "bucket",
        arg_val("--bucket-config")
            .or_else(|| arg_val("--prefill-buckets"))
            .or_else(|| arg_val("--decode-buckets")),
    ); // Furiosa RNGD
       // 정밀도/양자화.
    push("dtype", arg_val("--dtype").or_else(|| env_val(&["DTYPE"])));
    push(
        "quant",
        arg_val("--quantization").or_else(|| env_val(&["QUANTIZATION", "RBLN_QUANTIZATION"])),
    );
    push("kv-dtype", arg_val("--kv-cache-dtype"));
    push("block", arg_val("--block-size"));
    // 디바이스/타깃 NPU.
    push("device", arg_val("--device"));
    push("devices", arg_val("--devices")); // Furiosa PE 지정
    push("npu", arg_val("--npu").or_else(|| env_val(&["RBLN_NPU"]))); // RBLN 타깃 칩(예: RBLN-CA22)
    push("gpu-mem", arg_val("--gpu-memory-utilization"));
    // NPU 특화 env / args (RBLN·Furiosa·compile).
    for (n, v) in &env {
        let nu = n.to_uppercase();
        if (nu.starts_with("RBLN_")
            || nu.starts_with("FURIOSA_")
            || nu.contains("COMPILE")
            || nu.contains("NPU"))
            && opts.len() < 14
            && !v.is_empty()
        {
            opts.push((n.clone(), v.clone()));
        }
    }
    for t in &toks {
        if (t.starts_with("--rbln") || t.starts_with("--furiosa") || t.starts_with("--compile"))
            && opts.len() < 14
        {
            let (k, v) = match t.split_once('=') {
                Some((a, b)) => (a.to_string(), b.to_string()),
                None => (t.clone(), "✓".into()),
            };
            opts.push((k.trim_start_matches("--").into(), v));
        }
    }

    let family = model_family(&source, name);
    ModelArtifact {
        model: name.to_string(),
        family,
        engine: engine.to_string(),
        node: String::new(),
        image,
        source,
        mount,
        opts,
    }
}

/// 변형(양자화/정밀도/HW/엔진) 태그를 벗겨 모델 "베이스" 이름만 남김.
fn strip_variant_tags(s: &str) -> String {
    let mut b = s.to_lowercase();
    for pre in ["vllm-", "sglang-", "ollama-", "furiosa-", "k-"] {
        if let Some(x) = b.strip_prefix(pre) {
            b = x.to_string();
        }
    }
    for suf in [
        "-instruct",
        "-chat",
        "-awq",
        "-gptq",
        "-fp8",
        "-bf16",
        "-fp16",
        "-int8",
        "-int4",
        "-w4a16",
        "-w8a8",
        "-nvfp4a16",
        "-nvfp4",
        "-mxfp4",
        "-rbln",
        "-gb10",
        "-npu",
        "-cpu",
        "-gpu",
        "-llm-d",
        "-modelservice",
        "-decode",
        "-prefill",
        "-proxy",
        "-server",
        "-n2",
        "-n3",
        "-v2",
        "-hf",
    ] {
        b = b.replace(suf, "");
    }
    b.trim_matches(|c| c == '-' || c == '_' || c == '.')
        .to_string()
}

/// 트리 그룹 키(모델 계열) — 표준 정체성. 우선순위: HF repo id(org/name) > 경로 leaf > deploy 이름.
/// 같은 모델의 여러 배포/컴파일본(다른 TP·양자화·노드)이 한 계열로 묶이도록 정규화.
fn model_family(source: &str, name: &str) -> String {
    // 1) HF id: "org/Name" — org 유지(중복 방지) + name 부분 변형태그 제거.
    if source.contains('/')
        && !source.starts_with('/')
        && !source.chars().any(|c| c.is_whitespace())
    {
        let mut it = source.rsplitn(2, '/');
        let leaf = it.next().unwrap_or(source);
        let org = it.next();
        let base = strip_variant_tags(leaf);
        let base = if base.is_empty() {
            leaf.to_lowercase()
        } else {
            base
        };
        return match org {
            Some(o) if !o.is_empty() => format!("{}/{}", o.to_lowercase(), base),
            _ => base,
        };
    }
    // 2) 로컬 경로: leaf 디렉터리 이름.
    if source.starts_with('/') {
        let leaf = source.rsplit('/').find(|s| !s.is_empty()).unwrap_or(source);
        let b = strip_variant_tags(leaf);
        if !b.is_empty() {
            return b;
        }
    }
    // 3) deploy 이름.
    let b = strip_variant_tags(name);
    if b.is_empty() {
        name.to_string()
    } else {
        b
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
        prom::query(
            p,
            "avg by (uuid,device,hostname) (furiosa_npu_core_utilization)"
        ),
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
    util.iter()
        .map(|s| {
            let uuid = s.l("uuid");
            Accel {
                kind: AccelKind::Rngd,
                model: String::new(),
                id: s.l("device").to_string(),
                node: s.l("hostname").to_string(),
                util: norm_pct(s.value),
                mem_used_gb: du.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
                mem_total_gb: dt.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
                temp: temp.get(uuid).map(|x| x.value).unwrap_or(0.0),
                power: pow.get(uuid).map(|x| x.value).unwrap_or(0.0),
                busy_model: String::new(),
                alive: alive.get(uuid).map(|x| x.value > 0.0).unwrap_or(true),
                throttle: thr.get(uuid).map(|x| x.value).unwrap_or(0.0),
                unified_mem: false,
                mem_bw: f64::NAN,
                clock_mhz: f64::NAN,
                mem_temp: f64::NAN,
                energy_mj: f64::NAN,
            }
        })
        .collect()
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
    util.iter()
        .map(|s| {
            let uuid = s.l("uuid");
            Accel {
                kind: AccelKind::Rbln,
                model: String::new(),
                id: s.l("name").to_string(),
                node: s.l("node").to_string(),
                util: norm_pct(s.value),
                mem_used_gb: du.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
                mem_total_gb: dt.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
                temp: temp.get(uuid).map(|x| x.value).unwrap_or(0.0),
                power: pow.get(uuid).map(|x| x.value).unwrap_or(0.0),
                busy_model: s.l("exported_pod").to_string(),
                alive: health.get(uuid).map(|x| x.value == 0.0).unwrap_or(true),
                throttle: 0.0,
                unified_mem: false,
                mem_bw: f64::NAN,
                clock_mhz: f64::NAN,
                mem_temp: f64::NAN,
                energy_mj: f64::NAN,
            }
        })
        .collect()
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
    util.iter()
        .map(|s| {
            let gpu = s.l("gpu");
            let model = gpu_model(s.l("modelName"));
            let unified = is_unified(&model);
            Accel {
                kind: AccelKind::Gpu,
                model,
                id: format!("gpu{}", gpu),
                node: s.l("Hostname").to_string(),
                util: norm_pct(s.value),
                mem_used_gb: mu.get(gpu).map(|x| x.value / 1024.0).unwrap_or(0.0),
                mem_total_gb: mt.get(gpu).map(|x| x.value / 1024.0).unwrap_or(0.0),
                temp: temp.get(gpu).map(|x| x.value).unwrap_or(0.0),
                power: pow.get(gpu).map(|x| x.value).unwrap_or(0.0),
                busy_model: s.l("exported_pod").to_string(),
                alive: true,
                throttle: 0.0,
                unified_mem: unified,
                mem_bw: bw.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
                clock_mhz: clk.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
                mem_temp: mtemp.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
                energy_mj: energy.get(gpu).map(|x| x.value).unwrap_or(f64::NAN),
            }
        })
        .collect()
}

async fn collect_nodes(p: &str) -> Vec<NodeInfo> {
    let fs_size_q = format!("{}{{mountpoint=\"/\"}}", metrics::NODE_FS_SIZE);
    let fs_avail_q = format!("{}{{mountpoint=\"/\"}}", metrics::NODE_FS_AVAIL);
    let (load, mt, ma, cpu, ds, da, node_res) = tokio::join!(
        prom::query(p, metrics::NODE_LOAD1),
        prom::query(p, "node_memory_MemTotal_bytes"),
        prom::query(p, "node_memory_MemAvailable_bytes"),
        prom::query(
            p,
            "100 - (avg by (instance)(rate(node_cpu_seconds_total{mode=\"idle\"}[1m])) * 100)"
        ),
        prom::query(p, &fs_size_q),
        prom::query(p, &fs_avail_q),
        node_kube(),
    );
    let (node_ip, node_meta) = node_res;
    let load = load.unwrap_or_default();
    let n_mt = map_by(mt.unwrap_or_default(), "instance");
    let n_ma = map_by(ma.unwrap_or_default(), "instance");
    let n_cpu = map_by(cpu.unwrap_or_default(), "instance");
    let n_ds = map_by(ds.unwrap_or_default(), "instance");
    let n_da = map_by(da.unwrap_or_default(), "instance");
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
        let dsz = n_ds.get(inst.as_str()).map(|x| x.value).unwrap_or(0.0);
        let dav = n_da.get(inst.as_str()).map(|x| x.value).unwrap_or(0.0);
        let meta = node_meta.get(&name).cloned().unwrap_or_default();
        nodes.push(NodeInfo {
            load1: load_by.get(&name).copied().unwrap_or(f64::NAN),
            mem_used_gb: to_gb(mt - ma),
            mem_total_gb: to_gb(mt),
            cpu_pct: n_cpu
                .get(inst.as_str())
                .map(|x| x.value)
                .unwrap_or(f64::NAN),
            disk_used_gb: to_gb(dsz - dav),
            disk_total_gb: to_gb(dsz),
            ready: meta.0,
            cordoned: meta.1,
            pressure: meta.2,
            version: meta.3,
            name,
            npu: String::new(), // collect_node_npu 가 라벨에서 채움(full tier)
        });
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
}

/// fast tier: 소스별 수집기 4개를 병렬 실행 후 합침. util/mem 반응성을 위해 collect()에서 분리.
pub async fn collect_fast(cfg: &Config) -> (Vec<Accel>, Vec<NodeInfo>) {
    let p = &cfg.prom;
    let (fu, rb, gpu, nodes) = tokio::join!(
        collect_furiosa(p),
        collect_rbln(p),
        collect_gpu(p),
        collect_nodes(p)
    );
    let mut accel = fu;
    accel.extend(rb);
    accel.extend(gpu);
    accel.sort_by(|a, b| (a.kind as u8, &a.node, &a.id).cmp(&(b.kind as u8, &b.node, &b.id)));
    // 통합 메모리(GB10 등): 별도 VRAM 없음 → 노드(호스트) 메모리 풀로 backfill.
    for a in accel
        .iter_mut()
        .filter(|a| a.unified_mem && a.mem_total_gb <= 0.0)
    {
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

    // Prometheus 도달성 프로브 — 실패 시 빈 테이블을 "장비 없음"이 아니라 연결 문제로 구분.
    snap.prom_ok = prom::query(&cfg.prom, "vector(1)").await.is_ok();
    if !snap.prom_ok {
        warn.push(format!("Prometheus unreachable at {}", cfg.prom));
    }

    // 가속기 + 노드는 fast tier(collect_fast) 재사용 — 중복 제거
    let (accel, nodes) = collect_fast(cfg).await;
    snap.accel = accel;
    snap.nodes = nodes;
    collect_node_npu(&mut snap.nodes).await; // 노드 라벨에서 NPU 드라이버/SDK 요약 채움

    // ---------- EPP pools (독립 쿼리 — 병렬 배칭으로 순차 라운드트립 제거) ----------
    let dec_q = "sum by (pod_name) (inference_extension_scheduler_attempts_total)";
    let pidx_q = "max(inference_extension_prefix_indexer_size)";
    let (r_ready, r_q, r_kv, r_sat, r_dec, r_pidx, r_ppq) = tokio::join!(
        prom::query(&cfg.prom, metrics::POOL_READY),
        prom::query(&cfg.prom, metrics::POOL_QUEUE),
        prom::query(&cfg.prom, metrics::POOL_KV),
        prom::query(&cfg.prom, metrics::POOL_SAT),
        prom::query(&cfg.prom, dec_q),
        prom::query(&cfg.prom, pidx_q),
        prom::query(&cfg.prom, metrics::POOL_PER_POD_QUEUE),
    );
    let p_ready = resolve(r_ready, metrics::POOL_READY, &mut warn);
    let p_q = map_by(resolve(r_q, metrics::POOL_QUEUE, &mut warn), "name");
    let p_kv = map_by(resolve(r_kv, metrics::POOL_KV, &mut warn), "name");
    let p_sat = map_by(resolve(r_sat, metrics::POOL_SAT, &mut warn), "name");
    // 라우팅 결정 분배 (트래픽이 EPP 경유 시 채워짐)
    let dec = resolve(r_dec, dec_q, &mut warn);
    for s in &dec {
        let pod = s.l("pod_name");
        if !pod.is_empty() {
            snap.decisions.push((pod.to_string(), s.value));
        }
    }
    snap.decisions
        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let pidx = resolve(r_pidx, pidx_q, &mut warn);
    snap.prefix_idx = pidx.first().map(|s| s.value).unwrap_or(f64::NAN);

    // per-pod 큐 깊이(요청 분배)
    for s in &resolve(r_ppq, metrics::POOL_PER_POD_QUEUE, &mut warn) {
        let pod = s.l("model_server_pod");
        if !pod.is_empty() {
            snap.pod_queues.push((pod.to_string(), s.value));
        }
    }
    snap.pod_queues
        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Perf: 구간별 지연 percentile·토큰분포·처리량 (EPP 정책용). 전부 graceful(NaN).
    let pp = &cfg.prom;
    // vLLM 네이티브 + Furiosa-LLM(K-EXAONE) 을 클러스터 전역으로 합산(엔진 무관 서빙 건강).
    let (req_v, err_rate, tps_v, prefix_hit, ttft_v, e2e_p95, req_f, tps_f, ttft_f) = tokio::join!(
        qs1(pp, "sum(rate(vllm:request_success_total[1m]))"),
        qs1(pp, "sum(rate(vllm:request_success_total{finished_reason=\"abort\"}[1m]))"),
        qs1(pp, "sum(rate(vllm:generation_tokens_total[1m]))"),
        qs1(pp, "sum(rate(vllm:prefix_cache_hits_total[5m])) / sum(rate(vllm:prefix_cache_queries_total[5m]))"),
        qs1(pp, "histogram_quantile(0.95, sum by (le)(rate(vllm:time_to_first_token_seconds_bucket[1m])))"),
        qs1(pp, "histogram_quantile(0.95, sum by (le)(rate(vllm:e2e_request_latency_seconds_bucket[1m])))"),
        qs1(pp, "sum(rate(furiosa_llm_request_success_total[1m]))"),
        qs1(pp, "sum(rate(furiosa_llm_generation_tokens_total[1m]))"),
        qs1(pp, "histogram_quantile(0.95, sum by (le)(rate(furiosa_llm_time_to_first_token_seconds_bucket[1m])))"),
    );
    // throughput 는 가산(NaN=0 취급, 둘 다 NaN 이면 NaN); TTFT 는 존재하는 값 우선(엔진 혼합 p95 는 근사).
    let nadd = |a: f64, b: f64| {
        if a.is_nan() && b.is_nan() {
            f64::NAN
        } else {
            (if a.is_nan() { 0.0 } else { a }) + (if b.is_nan() { 0.0 } else { b })
        }
    };
    let nor = |a: f64, b: f64| if a.is_nan() { b } else { a };
    snap.perf = Perf {
        req_rate: nadd(req_v, req_f),
        err_rate,
        tps: nadd(tps_v, tps_f),
        prefix_hit,
        ttft_p95: nor(ttft_v, ttft_f),
        e2e_p95,
    };

    // per-model 성능 — service(=Deployment 이름) 기준 병합. Models 뷰와 동일 키.
    // 여기선 pm 만 채우고, 런칭된 모델 seed 는 collect_kube(모델 목록) 이후에 좌조인(아래).
    let mut pm: BTreeMap<String, PerfRow> = BTreeMap::new();
    // per-model 지연/토큰/처리량 — 17개 독립 쿼리를 병렬 배칭(순차 라운드트립 제거).
    // (promql, 필드 setter). setter 는 캡처 없는 클로저라 fn 포인터로 강제.
    type Set = fn(&mut PerfRow, f64);
    let specs: &[(&str, Set)] = &[
        ("sum by (service)(rate(vllm:request_success_total[1m]))", |r, v| r.req = v),
        ("sum by (service)(rate(vllm:generation_tokens_total[1m]))", |r, v| r.tps = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:time_to_first_token_seconds_bucket[1m])))", |r, v| r.ttft_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_time_per_output_token_seconds_bucket[1m])))", |r, v| r.tpot_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:e2e_request_latency_seconds_bucket[1m])))", |r, v| r.e2e_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_prompt_tokens_bucket[1m])))", |r, v| r.in_tok_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_generation_tokens_bucket[1m])))", |r, v| r.out_tok_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_queue_time_seconds_bucket[1m])))", |r, v| r.queue_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_prefill_time_seconds_bucket[1m])))", |r, v| r.prefill_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_decode_time_seconds_bucket[1m])))", |r, v| r.decode_p95 = v),
        ("sum by (service)(rate(vllm:num_preemptions_total[1m]))", |r, v| r.preempt = v),
        // Furiosa-LLM (K-EXAONE 등): furiosa_llm_* 로 노출, 같은 service 조인키. TPOT≈inter-token-latency.
        ("sum by (service)(rate(furiosa_llm_request_success_total[1m]))", |r, v| r.req = v),
        ("sum by (service)(rate(furiosa_llm_generation_tokens_total[1m]))", |r, v| r.tps = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_time_to_first_token_seconds_bucket[1m])))", |r, v| r.ttft_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_inter_token_latency_seconds_bucket[1m])))", |r, v| r.tpot_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_request_prompt_tokens_bucket[1m])))", |r, v| r.in_tok_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_request_generation_tokens_bucket[1m])))", |r, v| r.out_tok_p95 = v),
    ];
    let qs: Vec<&str> = specs.iter().map(|(q, _)| *q).collect();
    let results = prom::query_all(&cfg.prom, &qs).await;
    for ((promql, set), r) in specs.iter().zip(results) {
        for s in &resolve(r, promql, &mut warn) {
            let m = s.l("service");
            if !m.is_empty() {
                set(
                    pm.entry(m.to_string()).or_insert_with(|| PerfRow::new(m)),
                    s.value,
                );
            }
        }
    }

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
    let mut vllm = Vllm {
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
    // Furiosa-LLM(K-EXAONE): furiosa_llm_* 를 같은 service 키로 병합(한 service 는 둘 중 하나만 노출 → 충돌 없음).
    vllm.run.extend(map_by(
        q(
            cfg,
            "sum by (service) (furiosa_llm_num_requests_running)",
            &mut warn,
        )
        .await,
        "service",
    ));
    vllm.wait.extend(map_by(
        q(
            cfg,
            "sum by (service) (furiosa_llm_num_requests_waiting)",
            &mut warn,
        )
        .await,
        "service",
    ));
    vllm.tps.extend(map_by(
        q(
            cfg,
            "sum by (service) (rate(furiosa_llm_generation_tokens_total[1m]))",
            &mut warn,
        )
        .await,
        "service",
    ));
    vllm.kv.extend(map_by(
        q(
            cfg,
            "max by (service) (furiosa_llm_kv_cache_usage_percent)",
            &mut warn,
        )
        .await,
        "service",
    ));
    vllm.ttft.extend(map_by(q(cfg, "histogram_quantile(0.95, sum by (service,le) (rate(furiosa_llm_time_to_first_token_seconds_bucket[1m])))", &mut warn).await, "service"));

    // ---------- kube: deployments / pods / routes / gateway / epp ----------
    collect_kube(cfg, &mut snap, &vllm, &mut warn).await;

    // per-model perf: 런칭된 모든 모델(snap.models)을 seed 로 → 트래픽 없어도 표(–)에 표시.
    // 배포명↔service 는 Models 뷰와 동일한 match_model 로 해석, pm(메트릭)을 좌조인.
    {
        let mut rows: Vec<PerfRow> = Vec::new();
        for m in &snap.models {
            let key = match_model(&m.name, &vllm.run);
            let mut row = key
                .and_then(|k| pm.remove(&k))
                .unwrap_or_else(|| PerfRow::new(&m.name));
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
    collect_inventory(&mut snap.inventory, &mut snap.node_alloc, &mut warn).await;

    snap.warnings = warn;
    snap
}

const ACCEL_RESOURCES: [&str; 3] = ["nvidia.com/gpu", "rebellions.ai/ATOM", "furiosa.ai/rngd"];

/// 노드 allocatable 합계 − 파드 requests 합계 = 여유. (가속기 resource별)
async fn collect_inventory(
    inv: &mut Vec<(String, i64, i64)>,
    node_alloc: &mut BTreeMap<String, BTreeMap<String, i64>>,
    warn: &mut Vec<String>,
) {
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
                            if let Some(q) = a
                                .get(r)
                                .and_then(|x| x.as_str())
                                .and_then(|s| s.parse::<i64>().ok())
                            {
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
                let node = p["spec"]["nodeName"].as_str().unwrap_or("").to_string();
                if let Some(cs) = p["spec"]["containers"].as_array() {
                    for c in cs {
                        if let Some(req) = c["resources"]["requests"].as_object() {
                            for r in ACCEL_RESOURCES {
                                if let Some(q) = req
                                    .get(r)
                                    .and_then(|x| x.as_str())
                                    .and_then(|s| s.parse::<i64>().ok())
                                {
                                    *used.get_mut(r).unwrap() += q;
                                    if !node.is_empty() {
                                        *node_alloc
                                            .entry(node.clone())
                                            .or_default()
                                            .entry(r.to_string())
                                            .or_insert(0) += q;
                                    }
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
    if let Ok(v) = kube::get_json(&["get", "nodes", "-o", "json"]).await {
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
                let ver = it["status"]["nodeInfo"]["kubeletVersion"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let cordoned = it["spec"]["unschedulable"].as_bool().unwrap_or(false);
                let (mut ready, mut pressure) = (false, false);
                if let Some(cs) = it["status"]["conditions"].as_array() {
                    for c in cs {
                        let t = c["type"].as_str().unwrap_or("");
                        let st = c["status"] == "True";
                        match t {
                            "Ready" => ready = st,
                            "MemoryPressure" | "DiskPressure" | "PIDPressure" if st => {
                                pressure = true
                            }
                            _ => {}
                        }
                    }
                }
                meta.insert(name, (ready, cordoned, pressure, ver));
            }
        }
    }
    (ip, meta)
}

async fn collect_kube(cfg: &Config, snap: &mut Snapshot, vllm: &Vllm, warn: &mut Vec<String>) {
    // routes: path -> backend (+ kind: Service|InferencePool)
    if let Ok(v) = kube::get_json(&["get", "httproute", "-n", &cfg.ns, "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for r in items {
                let route_name = r["metadata"]["name"].as_str().unwrap_or("").to_string();
                if let Some(rules) = r["spec"]["rules"].as_array() {
                    for rule in rules {
                        let backend = rule["backendRefs"][0]["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let kind = rule["backendRefs"][0]["kind"]
                            .as_str()
                            .unwrap_or("Service")
                            .to_string();
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
                                        route: route_name.clone(),
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
                    snap.artifacts.push(model_artifact(d, &name, &engine));
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
                    let age_secs = p["metadata"]["creationTimestamp"]
                        .as_str()
                        .and_then(parse_k8s_ts)
                        .map(|s| now_secs().saturating_sub(s))
                        .unwrap_or(0);
                    snap.pods.push(PodRow {
                        name,
                        phase,
                        ready: format!("{}/{}", readyc, total),
                        node,
                        restarts,
                        age_secs,
                    });
                }
            }
        }
        Err(e) => warn.push(format!("kube pods: {}", e)),
    }
    snap.pods.sort_by(|a, b| a.name.cmp(&b.name));

    collect_events(cfg, snap).await;
    collect_gateway(cfg, snap).await;

    // EPP config (ConfigMap)
    if let Ok(v) =
        kube::get_json(&["get", "cm", "llmd-router-epp", "-n", &cfg.ns, "-o", "json"]).await
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
                let epp = ip["spec"]["endpointPickerRef"]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
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
                    if let Ok(pj) = kube::get_json(&[
                        "get", "pods", "-n", &cfg.ns, "-l", &selector, "-o", "json",
                    ])
                    .await
                    {
                        if let Some(ps) = pj["items"].as_array() {
                            ep_total = ps.len() as i64;
                            for p in ps {
                                let ready = p["status"]["containerStatuses"]
                                    .as_array()
                                    .map(|cs| {
                                        !cs.is_empty()
                                            && cs
                                                .iter()
                                                .all(|c| c["ready"].as_bool().unwrap_or(false))
                                    })
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
    if let Ok(v) = kube::get_json(&["get", "inferenceobjective", "-n", &cfg.ns, "-o", "json"]).await
    {
        if let Some(items) = v["items"].as_array() {
            for o in items {
                snap.objectives.push(Objective {
                    name: o["metadata"]["name"].as_str().unwrap_or("").to_string(),
                    priority: o["spec"]["priority"].as_i64().unwrap_or(0),
                    pool: o["spec"]["poolRef"]["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }
    }
    snap.objectives
        .sort_by_key(|o| std::cmp::Reverse(o.priority));

    // 오토스케일링 (KEDA ScaledObject + 상태)
    if let Ok(v) = kube::get_json(&["get", "scaledobject", "-n", &cfg.ns, "-o", "json"]).await {
        if let Some(items) = v["items"].as_array() {
            for so in items {
                let target = so["spec"]["scaleTargetRef"]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
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
                    .map(|ts| {
                        ts.iter()
                            .filter_map(|t| t["type"].as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                let replicas = snap
                    .models
                    .iter()
                    .find(|m| m.name == target)
                    .map(|m| m.ready)
                    .unwrap_or(0);
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

    collect_stored(cfg, snap).await;
    collect_compiles(cfg, snap).await;

    // artifact 저장/구동 노드 — 가속기 busy_model 우선, 없으면 파드 스케줄 노드(분리 차용으로 먼저 계산).
    let nodes: Vec<String> = snap
        .artifacts
        .iter()
        .map(|a| {
            snap.accel
                .iter()
                .find(|x| !x.busy_model.is_empty() && x.busy_model.starts_with(&a.model))
                .map(|x| x.node.clone())
                .or_else(|| {
                    snap.pods
                        .iter()
                        .find(|p| p.name.starts_with(&a.model))
                        .map(|p| p.node.clone())
                })
                .unwrap_or_default()
        })
        .collect();
    for (a, n) in snap.artifacts.iter_mut().zip(nodes) {
        a.node = n;
    }
}

/// 노드 라벨에서 NPU 드라이버/SDK 존재·버전을 읽어 NodeInfo.npu 채움.
/// 컴파일은 해당 NPU 드라이버가 설치된 노드에서만 가능 → 타깃 선택·경고에 사용.
async fn collect_node_npu(nodes: &mut [NodeInfo]) {
    let Ok(v) = kube::get_json(&["get", "nodes", "-o", "json"]).await else {
        return;
    };
    let Some(items) = v["items"].as_array() else {
        return;
    };
    for n in items {
        let name = n["metadata"]["name"].as_str().unwrap_or("");
        let Some(node) = nodes.iter_mut().find(|x| x.name == name) else {
            continue;
        };
        let l = &n["metadata"]["labels"];
        let mut parts = Vec::new();
        if let Some(prod) = l["furiosa.ai/npu.product"].as_str() {
            let drv = l["furiosa.ai/driver.version"].as_str().unwrap_or("?");
            parts.push(format!("{} drv{}", prod.to_uppercase(), drv));
        }
        if l["rebellions.ai/npu.present"].as_str() == Some("true")
            || l.get("rebellions.ai/npu.product").is_some()
        {
            let prod = l["rebellions.ai/npu.product"].as_str().unwrap_or("RBLN");
            let drv = l["rebellions.ai/driver-version.full"]
                .as_str()
                .unwrap_or("?");
            let installed = l["rebellions.ai/npu.driver.status"].as_str() == Some("installed");
            parts.push(format!(
                "{} drv{}{}",
                prod,
                drv,
                if installed { "" } else { "(!drv)" }
            ));
        }
        node.npu = parts.join(" · ");
    }
}

/// k8s/llm-d 이벤트(최근 40) → snap.events. collect_kube 에서 분리된 leaf 파서.
async fn collect_events(cfg: &Config, snap: &mut Snapshot) {
    if let Ok(v) = kube::get_json(&[
        "get",
        "events",
        "-n",
        &cfg.ns,
        "--sort-by=.lastTimestamp",
        "-o",
        "json",
    ])
    .await
    {
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
}

/// Gateway 주소/상태(Programmed) → snap.gw_addr/gw_ok.
async fn collect_gateway(cfg: &Config, snap: &mut Snapshot) {
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
}

/// 공유 스토어 인벤토리(model-inventory ConfigMap) → snap.stored. 없으면(미배포) 조용히 빈 값.
async fn collect_stored(cfg: &Config, snap: &mut Snapshot) {
    if let Ok(v) =
        kube::get_json(&["get", "cm", "model-inventory", "-n", &cfg.ns, "-o", "json"]).await
    {
        if let Some(txt) = v["data"]["inventory"].as_str() {
            for line in txt.lines() {
                let l = line.trim();
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                let f: Vec<&str> = l.split('|').map(|s| s.trim()).collect();
                if f.len() >= 6 {
                    snap.stored.push(StoredModel {
                        family: model_family(f[0], f[0]),
                        repo: f[0].into(),
                        revision: f[1].into(),
                        format: f[2].into(),
                        compiled_for: f[3].into(),
                        size: f[4].into(),
                        path: f[5].into(),
                    });
                }
            }
        }
    }
}

/// k8s RFC3339 타임스탬프("2026-07-03T12:34:56Z")를 epoch 초로. 실패 시 None.
/// chrono 의존 없이 고정 포맷만 파싱(k8s 는 항상 UTC 'Z').
fn parse_k8s_ts(s: &str) -> Option<u64> {
    let s = s.trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, mo, da): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let mut t = time.split(':');
    let (h, mi, se): (i64, i64, i64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.split('.').next()?.parse().ok()?,
    );
    // days-from-civil (Howard Hinnant) — 1970-01-01 기준 일수.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se;
    if secs < 0 {
        None
    } else {
        Some(secs as u64)
    }
}

/// 진행/최근 컴파일 Job(`compile-*`) → snap.compiles. Job 없으면 조용히 빈 값.
/// 상태·경과·(완료 시)소요 + 활성 Job 은 파드 로그 마지막 줄을 진행 힌트로.
async fn collect_compiles(cfg: &Config, snap: &mut Snapshot) {
    let Ok(v) = kube::get_json(&["get", "jobs", "-n", &cfg.ns, "-o", "json"]).await else {
        return;
    };
    let Some(items) = v["items"].as_array() else {
        return;
    };
    let now = now_secs();
    for j in items {
        let name = j["metadata"]["name"].as_str().unwrap_or("");
        if !name.starts_with("compile-") {
            continue;
        }
        // 이름 = compile-{model}-{target}. target 은 "-rbln-" 또는 "-rngd-" 부터.
        let rest = &name["compile-".len()..];
        let (model, target, vendor) = if let Some(i) = rest.find("-rbln-") {
            (
                rest[..i].to_string(),
                rest[i + 1..].to_string(),
                "RBLN".to_string(),
            )
        } else if let Some(i) = rest.find("-rngd-") {
            (
                rest[..i].to_string(),
                rest[i + 1..].to_string(),
                "RNGD".to_string(),
            )
        } else {
            (rest.to_string(), String::new(), "-".to_string())
        };
        let st = &j["status"];
        let succeeded = st["succeeded"].as_i64().unwrap_or(0) >= 1;
        let active = st["active"].as_i64().unwrap_or(0) >= 1;
        let failed_cond = st["conditions"]
            .as_array()
            .map(|c| {
                c.iter()
                    .any(|x| x["type"] == "Failed" && x["status"] == "True")
            })
            .unwrap_or(false);
        let status = if succeeded {
            "Complete"
        } else if failed_cond {
            "Failed"
        } else if active {
            "Running"
        } else {
            "Pending"
        }
        .to_string();
        let start = st["startTime"].as_str().and_then(parse_k8s_ts).or_else(|| {
            j["metadata"]["creationTimestamp"]
                .as_str()
                .and_then(parse_k8s_ts)
        });
        let completion = st["completionTime"]
            .as_str()
            .and_then(parse_k8s_ts)
            .or_else(|| {
                st["conditions"]
                    .as_array()
                    .and_then(|c| {
                        c.iter()
                            .find(|x| x["type"] == "Failed")
                            .and_then(|x| x["lastTransitionTime"].as_str())
                    })
                    .and_then(parse_k8s_ts)
            });
        let age_secs = start.map(|s| now.saturating_sub(s)).unwrap_or(0);
        let duration_secs = match (start, completion) {
            (Some(s), Some(c)) if c >= s => Some(c - s),
            _ => None,
        };
        // 진행 힌트: 활성 Job 은 파드 로그 마지막 비어있지 않은 줄. 완료/실패는 상태로 대체.
        let phase = if active {
            let pod = snap
                .pods
                .iter()
                .find(|p| p.name.starts_with(name))
                .map(|p| p.name.clone());
            match pod {
                Some(p) => kube::last_log_line(&cfg.ns, &p)
                    .await
                    .unwrap_or_else(|| "starting…".to_string()),
                None => "starting…".to_string(),
            }
        } else if status == "Complete" {
            "COMPILE_DONE".to_string()
        } else if status == "Failed" {
            "failed — see logs".to_string()
        } else {
            "pending…".to_string()
        };
        // 진행률: 완료=1.0, 실행 중이면 로그 줄에서 %/스텝 파싱(있으면), 없으면 None(indeterminate).
        let progress = if status == "Complete" {
            Some(1.0)
        } else if active {
            parse_progress(&phase)
        } else {
            None
        };
        snap.compiles.push(CompileJob {
            name: name.to_string(),
            model,
            vendor,
            target,
            status,
            age_secs,
            duration_secs,
            phase,
            progress,
        });
    }
    // 진행 중 → 최근 순: Running 먼저, 그다음 나이 어린 것.
    snap.compiles.sort_by(|a, b| {
        let rank = |s: &str| match s {
            "Running" => 0,
            "Pending" => 1,
            "Failed" => 2,
            _ => 3,
        };
        rank(&a.status)
            .cmp(&rank(&b.status))
            .then(a.age_secs.cmp(&b.age_secs))
    });
}

/// 컴파일 로그 한 줄에서 진행률(0.0~1.0)을 best-effort 파싱. 정규식 없이 순수 스캔.
/// 우선순위: (1) "NN%" / "NN.N%"  (2) "N/M" 스텝(양쪽 모두 숫자일 때만). 없으면 None(indeterminate).
/// tqdm("45%|██| 45/100")·"[3/8]"·"step 3 of 8"(→ "3/8" 아님, %/슬래시만) 등을 커버.
fn parse_progress(line: &str) -> Option<f32> {
    let b = line.as_bytes();
    // 1) '%' 앞의 숫자(공백 건너뜀).
    for (i, &c) in b.iter().enumerate() {
        if c == b'%' {
            // i-1 부터 역방향으로 숫자/'.' 수집.
            let mut j = i;
            while j > 0 && (b[j - 1].is_ascii_digit() || b[j - 1] == b'.') {
                j -= 1;
            }
            if j < i {
                if let Ok(v) = line[j..i].parse::<f32>() {
                    if (0.0..=100.0).contains(&v) {
                        return Some((v / 100.0).clamp(0.0, 1.0));
                    }
                }
            }
        }
    }
    // 2) "N/M" — '/' 양쪽이 모두 연속 숫자일 때만(경로/날짜 오검출 방지).
    for (i, &c) in b.iter().enumerate() {
        if c == b'/' {
            let mut l = i;
            while l > 0 && b[l - 1].is_ascii_digit() {
                l -= 1;
            }
            let mut r = i + 1;
            while r < b.len() && b[r].is_ascii_digit() {
                r += 1;
            }
            if l < i && r > i + 1 {
                let n: f32 = line[l..i].parse().unwrap_or(0.0);
                let m: f32 = line[i + 1..r].parse().unwrap_or(0.0);
                if m > 0.0 && n <= m {
                    return Some((n / m).clamp(0.0, 1.0));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod compile_progress_tests {
    use super::parse_progress;

    #[test]
    fn parses_percent() {
        assert_eq!(parse_progress("Compiling model... 45%"), Some(0.45));
        assert_eq!(parse_progress("progress 7.5% done"), Some(0.075));
        assert_eq!(parse_progress("100%|████████| complete"), Some(1.0));
    }

    #[test]
    fn parses_step_ratio() {
        assert_eq!(parse_progress("[3/8] lowering graph"), Some(0.375));
        assert_eq!(parse_progress("layer 5/5"), Some(1.0));
    }

    #[test]
    fn ignores_paths_and_plain_text() {
        assert_eq!(parse_progress("writing to /hf-cache/model"), None);
        assert_eq!(parse_progress("starting compilation…"), None);
        assert_eq!(parse_progress("loading weights"), None);
    }

    #[test]
    fn percent_wins_over_ratio() {
        // tqdm 형태: 퍼센트를 우선.
        assert_eq!(parse_progress("42%|███ | 42/100 [00:10<00:14]"), Some(0.42));
    }
}

fn match_model(deploy: &str, v_run: &BTreeMap<String, Series>) -> Option<String> {
    let norm = |s: &str| {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_normalizes_hf_and_paths() {
        // HF id: org 유지 + 변형태그(quant/instruct) 제거, 소문자.
        assert_eq!(
            model_family("Qwen/Qwen2.5-0.5B-Instruct", "vllm-qwen05b-gb10"),
            "qwen/qwen2.5-0.5b"
        );
        assert_eq!(
            model_family("furiosa-ai/K-EXAONE-236B-A23B-NVFP4A16", "k-exaone-236b"),
            "furiosa-ai/exaone-236b-a23b"
        );
        // 로컬 경로: leaf 디렉터리.
        assert_eq!(model_family("/models/exaone35", "vllm-exaone"), "exaone35");
        // 소스 없음(셸 래퍼 등): deploy 이름에서 엔진/HW 접사 제거.
        assert_eq!(model_family("", "vllm-v4flash"), "v4flash");
    }

    #[test]
    fn same_model_different_builds_share_family() {
        // 같은 HF 모델의 두 배포(다른 양자화/HW)는 한 family 로 묶임.
        let a = model_family("meta-llama/Llama-3.1-8B-Instruct", "llama31-rbln");
        let b = model_family("meta-llama/Llama-3.1-8B-Instruct-FP8", "llama31-gpu");
        assert_eq!(a, b);
        assert_eq!(a, "meta-llama/llama-3.1-8b");
    }

    #[test]
    fn strip_variant_tags_drops_hw_and_precision() {
        assert_eq!(strip_variant_tags("vllm-koni-rbln"), "koni");
        assert_eq!(strip_variant_tags("Model-BF16-Instruct"), "model");
    }

    #[test]
    fn to_gb_and_norm_pct() {
        assert_eq!(to_gb(2.0e9), 2.0);
        assert_eq!(to_gb(f64::NAN), 0.0); // NaN → 0
        assert_eq!(norm_pct(0.82), 82.0); // 0..1 비율 → %
        assert_eq!(norm_pct(82.0), 82.0); // 이미 % 면 그대로
        assert_eq!(norm_pct(f64::NAN), 0.0);
        assert_eq!(norm_pct(0.0), 0.0); // 0 은 0 (비율 확대 안 함)
    }

    #[test]
    fn match_model_fuzzy() {
        let mut m: BTreeMap<String, Series> = BTreeMap::new();
        m.insert(
            "koni-llama3.1-8b".into(),
            Series {
                labels: BTreeMap::new(),
                value: 1.0,
            },
        );
        // 비영숫자 무시 매칭: "koni-llama31-8b-rbln" ⊃ "konillama318b"
        assert_eq!(
            match_model("koni-llama31-8b-rbln", &m).as_deref(),
            Some("koni-llama3.1-8b")
        );
        assert_eq!(match_model("totally-different", &m), None);
    }

    #[test]
    fn parse_epp_scorers() {
        let yaml = "schedulingProfiles:\n  - name: default\n    plugins:\n      - pluginRef: kv-cache-scorer\n        weight: 2\n      - pluginRef: queue-scorer\n        weight: 1\n";
        let cfg = parse_epp(yaml).expect("parses");
        assert_eq!(cfg.profile, "default");
        assert_eq!(cfg.scorers.len(), 2);
        assert_eq!(cfg.scorers[0], ("kv-cache-scorer".to_string(), 2.0));
        // 프로파일 없는 입력은 graceful — scorers 빈 EppCfg.
        assert_eq!(parse_epp("42").map(|c| c.scorers.len()), Some(0));
    }

    #[test]
    fn model_container_skips_sidecar() {
        // 프록시 사이드카가 [0], vLLM 모델서버가 [1] → 모델서버 선택.
        let spec = serde_json::json!({
            "containers": [
                { "name": "proxy", "image": "envoyproxy/envoy:v1", "args": [] },
                { "name": "server", "image": "vllm/vllm-openai", "args": ["--model", "/models/x"] }
            ]
        });
        let c = model_container(&spec);
        assert_eq!(c["name"].as_str(), Some("server"));
    }
}
