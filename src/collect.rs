//! Snapshot 도메인 타입 + prometheus/kubectl 에서 수집하는 로직.
//! 각 소스는 독립적으로 실패해도 전체를 막지 않음(warnings 에 누적, 부재 필드는 None/빈값).

use crate::config::Config;
use crate::kube;
use crate::metrics;
use crate::prom::{self, Series};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AccelKind {
    Gpu,
    Rbln,
    Rngd,
    /// A device class declared at runtime (see `accel::packs`). The payload is the pack's slot
    /// in the registry, so `accel::by_kind` stays a bijection — a declared accelerator gets
    /// its own class rather than borrowing an existing one's colour and capabilities.
    Other(u16),
}
impl AccelKind {
    /// 벤더 계열 라벨(fallback). GPU 는 모델이 다양하므로 generic "GPU" — 실제 모델은 Accel.model.
    pub fn label(&self) -> &'static str {
        crate::accel::by_kind(*self).label
    }

    /// Stable sort rank: built-in classes keep their declaration order, declared ones follow.
    pub fn rank(&self) -> u16 {
        match self {
            AccelKind::Gpu => 0,
            AccelKind::Rbln => 1,
            AccelKind::Rngd => 2,
            AccelKind::Other(slot) => 3 + slot,
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
    /// Is this device throttling? `None` when the hardware reports no throttling signal at
    /// all or when its current sample is unavailable. Neither means "not throttling".
    pub fn throttling(&self) -> Option<bool> {
        crate::accel::by_kind(self.kind)
            .caps
            .throttle
            .then_some(self.throttle)
            .filter(|value| value.is_finite())
            .map(|value| value > 0.0)
    }

    /// Does this accelerator report cumulative energy (so session Wh is meaningful)?
    pub fn reports_energy(&self) -> bool {
        crate::accel::by_kind(self.kind).caps.energy
    }

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
    pub identity: Option<crate::resource_guard::Identity>,
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
    /// Pressure conditions currently True, by name — `pressure` says whether, this says which.
    pub pressure_kinds: Vec<String>,
    /// Devices the node declares per accelerator family (labels + allocatable), from the
    /// Kubernetes API. Compared against the exporters to tell "absent" from "not reporting".
    pub devices: Vec<DeviceExpectation>,
}

/// One accelerator family's presence on a node, as Kubernetes sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceExpectation {
    /// Pack label (`RBLN`, `RNGD`, `GPU`) — the same key `AccelKind::label` yields.
    pub label: &'static str,
    /// Count from the feature-discovery label. Survives device-plugin/exporter eviction.
    pub declared: u32,
    /// `status.allocatable[resource_key]`; `None` when the node does not list the resource.
    pub allocatable: Option<u32>,
}

/// Read every pack's declared device count off one node object.
fn device_expectations(node: &serde_json::Value) -> Vec<DeviceExpectation> {
    let labels = &node["metadata"]["labels"];
    let allocatable = &node["status"]["allocatable"];
    crate::accel::packs()
        .iter()
        .filter_map(|pack| {
            let declared = labels[pack.scheduling.count_label?]
                .as_str()?
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)?;
            let allocatable = allocatable[pack.scheduling.resource_key]
                .as_str()
                .and_then(|v| v.trim().parse::<u32>().ok());
            Some(DeviceExpectation {
                label: pack.label,
                declared,
                allocatable,
            })
        })
        .collect()
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
    pub configuration: Option<crate::routing::Configuration>,
    pub match_summary: String,
    pub backend_summary: String,
    pub single_local_backend: bool,
    pub identity: Option<crate::resource_guard::Identity>,
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
    /// Toolchain that produced this build, as the scan read it out of the artifact itself
    /// (`optimum-rbln=0.10.2`, `furiosa-compiler=2026.3.0`), or `-` when unknown.
    ///
    /// This is the one fact that identifies *why* a build works when an identical-looking
    /// rebuild does not — see docs/RBLN-COMPILE-INCIDENT.md, where the answer was a version
    /// recorded inside a working artifact and nowhere else.
    pub built_with: String,
}

/// Shared store capacity, as measured inside the discovery scan.
///
/// A PVC's `status.capacity` reports the *requested* size, which on this cluster's SMB backend
/// is unrelated to what is actually free. Only a pod with the claim mounted can measure it, and
/// the discovery scan already is one — so it publishes `df` alongside the inventory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StoreCapacity {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub avail_bytes: u64,
}

impl StoreCapacity {
    /// Used share of the filesystem, 0-100. Uses used/(used+avail) rather than used/total:
    /// reserved blocks make those differ, and the second is what the operator can act on.
    pub fn used_pct(&self) -> f64 {
        let usable = self.used_bytes.saturating_add(self.avail_bytes);
        if usable == 0 {
            return 0.0;
        }
        self.used_bytes as f64 * 100.0 / usable as f64
    }
}

/// 컴파일 Job 진행 상태(Deploy 뷰 '진행 중 컴파일' 패널). `compile-*` Job 을 요약.
#[derive(Clone, Default)]
pub struct CompileJob {
    pub identity: Option<crate::resource_guard::Identity>,
    /// Namespace/name/UID identifies an execution, including retries under the same name.
    pub execution_id: String,
    pub name: String,               // Job 이름(compile-{모델}-{타깃})
    pub model: String,              // 사람이 읽는 모델(이름에서 추출)
    pub vendor: String,             // RBLN | RNGD | -
    pub target: String,             // 옵션 요약(tp/pp/seq 등, 이름의 타깃 부분)
    pub status: String,             // Running | Complete | Failed | Pending
    pub age_secs: u64,              // 시작(없으면 생성) 후 경과 초
    pub duration_secs: Option<u64>, // 완료된 경우 소요 초
    pub phase: String,              // 진행 힌트(파드 로그 마지막 줄 또는 상태)
    pub progress: Option<f32>,      // 로그에서 파싱한 진행률 0.0~1.0(없으면 indeterminate 바)
    /// Classified cause when the Job failed — replaces the old "see logs", and is what gets
    /// written to history before `ttlSecondsAfterFinished` removes the Job.
    pub failure: Option<crate::diagnose::Failure>,
    /// Compile options read off the Job's environment (tp, max-len, attn…).
    pub options: std::collections::BTreeMap<String, String>,
    /// HuggingFace source id from the Job's MODEL_ID env, for history keying.
    pub source: String,
    /// Toolchain versions the build ran against, reported by the recipe.
    pub toolchain: std::collections::BTreeMap<String, String>,
}

/// Store 뷰용 — 모델이 "어디에(경로/볼륨)" 저장되고 "어떤 옵션으로" 컴파일/서빙되는지.
/// deploy 컨테이너 spec(command/args/env/volumeMounts)에서 추출(휴리스틱).
#[derive(Clone, Default)]
pub struct ModelArtifact {
    pub model: String,  // deploy 이름(=변형 이름)
    pub family: String, // 트리 그룹 키(모델 계열, source/name 에서 정규화)
    pub engine: String, // 추론 엔진
    pub node: String,   // 저장/구동 노드(파드 스케줄 노드)
    pub image: String,  // 컨테이너 이미지
    pub source: String, // 모델 소스: HF id / --model 경로 / MODEL_ID
    pub mount: String,  // 저장 위치: "mountPath ← PVC/hostPath/emptyDir"
    /// Node-local directory backing the model volume, when it is a `hostPath`.
    ///
    /// Structured rather than scraped back out of `mount`, which is a display string. The
    /// artifacts actually serving on this cluster are hand-compiled hostPath directories, and
    /// reading anything out of one means running a pod on `node` with this path mounted.
    pub host_path: Option<String>,
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
    pub identity: Option<crate::resource_guard::Identity>,
    pub name: String,
    pub phase: String,
    pub ready: String,
    pub node: String,
    pub restarts: i64,
    pub age_secs: u64, // 생성 후 경과(초) — 배포 시작 시각 표시용. 0=미상.
}

pub use crate::epp_config::EppCfg;

pub use crate::setup_probe::SetupProbe;

/// LoRA training run (npu-lora Job, `app.kubernetes.io/part-of=npu-lora`), joined with the
/// trainer's own `npu_lora_*` metrics. Metric fields are None until the first step is scraped.
#[derive(Clone, Default, Debug)]
pub struct TrainRun {
    pub job: String,
    pub run: String,
    pub model: String,
    pub status: String, // Running | Complete | Failed | Pending
    pub age_secs: u64,
    pub step: Option<f64>,
    pub steps_planned: Option<f64>,
    pub loss: Option<f64>,
    pub tokens_per_s: Option<f64>,
    pub out: String,
}

impl TrainRun {
    /// 0.0..=1.0 once both the step and the plan are known.
    pub fn progress(&self) -> Option<f64> {
        match (self.step, self.steps_planned) {
            (Some(s), Some(p)) if p > 0.0 => Some((s / p).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct Snapshot {
    /// Completion time of the full collection; fast updates do not modify it.
    pub ts: u64,
    pub sources: crate::quality::Sources,
    pub stale_after_secs: u64,
    pub nodes: Vec<NodeInfo>,
    pub accel: Vec<Accel>,
    pub pools: Vec<Pool>,
    pub deployment_versions: BTreeMap<String, crate::resource_guard::Identity>,
    pub models: Vec<ModelRow>,
    pub workloads: Vec<crate::workload::Identity>,
    pub artifacts: Vec<ModelArtifact>, // Store 뷰: 모델 저장 위치 + 컴파일/서빙 옵션
    pub stored: Vec<StoredModel>, // 공유 스토어 인벤토리(model-inventory ConfigMap) — 배포 무관
    /// 공유 스토어 여유 용량(디스커버리 스캔이 df 로 실측). None=미관측(구버전 스캔 등).
    pub store_capacity: Option<StoreCapacity>,
    pub store_scanned_at: Option<u64>,
    pub compiles: Vec<CompileJob>, // 진행/최근 컴파일 Job(compile-*) — Deploy 뷰 모니터 패널
    pub trainings: Vec<TrainRun>,  // LoRA 학습 Job(npu-lora) + npu_lora_* 지표
    pub pods: Vec<PodRow>,
    pub events: Vec<EventRow>,
    pub routes: Vec<Route>,
    pub objectives: Vec<Objective>,
    pub decisions: Vec<(String, f64)>, // (pod, 라우팅 픽 횟수) — 트래픽이 EPP 경유 시
    pub pod_queues: Vec<(String, f64)>, // per-pod 큐 깊이(요청 분배)
    pub endpoint_scores: Vec<(String, f64)>, // per-pod EPP composite score(epp-score-observer 플러그인). 없으면 빈 값.
    pub perf: Perf,
    pub service_bindings: BTreeMap<String, Vec<String>>,
    pub deployment_pods: BTreeMap<String, Vec<String>>,
    pub service_names: Vec<String>,
    pub pool_bindings: BTreeMap<String, Vec<String>>,
    pub perf_targets: BTreeMap<String, PerformanceTarget>,
    pub perf_rows: Vec<PerfRow>, // 모델(=하드웨어)별 성능

    pub autoscalers: Vec<Autoscale>,
    pub epp: Option<EppCfg>,
    pub epp_in_path: bool, // observed InferencePool backend configuration; not runtime traversal
    pub prefix_idx: f64,   // inference_extension_prefix_indexer_size
    pub gw_addr: String,
    pub gw_ok: bool,
    pub inventory: Vec<(String, i64, i64)>, // (가속기 resource, allocatable total, 사용중 requests)
    pub node_capacity: BTreeMap<String, BTreeMap<String, i64>>, // observed Kubernetes allocatable
    pub node_alloc: std::collections::BTreeMap<String, std::collections::BTreeMap<String, i64>>, // node → resource → 할당(파드 requests)
    pub warnings: Vec<String>,
    pub prom_ok: bool, // Prometheus 도달 가능 여부(false 면 "가속기 없음"이 아니라 연결 문제)
    pub setup: SetupProbe, // 플랫폼 부트스트랩 전제조건(Setup 뷰)
    pub pvcs: Vec<String>, // 대상 ns 에 실존하는 PVC 이름들 — prefetch/compile 대상 검증에 사용(빈 벡터=미관측)
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
    v / 1.0e9
}

fn short(s: &str) -> String {
    s.chars().take(28).collect()
}

/// Surface a failed kubectl collector as a warning. A missing CRD ("the server doesn't have a
/// resource type") is a normal cluster shape, not a fault, so it stays quiet — everything else
/// (RBAC denial, unreachable API server, missing kubectl) must be visible, because an empty view
/// and a permission error otherwise look identical (REG-08 / BUG-09).
fn kube_warn(label: &str, e: &anyhow::Error, warn: &mut Vec<String>) {
    let m = e.to_string();
    if m.contains("doesn't have a resource type")
        || m.contains("could not find the requested resource")
    {
        return;
    }
    let reason = m.lines().last().unwrap_or("unknown error").trim();
    warn.push(format!("kube[{}]: {}", label, reason));
}

/// prom::query 결과를 warn 으로 해소 — join!/query_all 로 병렬 조회한 뒤 사용.
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

use crate::latency::{
    merge_engine_metrics, performance_query, performance_query_text, record_metric_family,
    single_engine_percentile, unique_performance_value,
};
pub use crate::latency::{perf_detail, LatencyFamily, PerfDetail, PerformanceTarget};

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
    if let Some(c) = arr
        .iter()
        .find(|c| crate::workload::container(c).0.is_model())
    {
        return c;
    }
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
    let identity = crate::workload::classify(d);
    if identity.engine == "vLLM" && accel.contains("RBLN") {
        "vLLM-RBLN".into()
    } else {
        identity.engine
    }
}

/// deploy 컨테이너 spec(command/args/env/volumeMounts)에서 모델 저장 위치 + 컴파일/서빙 옵션 추출.
/// Host directory backing a volume, if it is a `hostPath`.
fn volume_host_path<'a>(pod: &'a serde_json::Value, vol_name: &str) -> Option<&'a str> {
    pod["volumes"]
        .as_array()?
        .iter()
        .find(|v| v["name"].as_str() == Some(vol_name))?["hostPath"]["path"]
        .as_str()
}

/// Is this mount plainly a library/system directory rather than a model?
///
/// Judged on the *backing* path, not the mount point: the RBLN deployments here mount
/// `/usr/lib` at `/host-rbln-lib`, so the container-side path looks model-ish and the host side
/// does not. Python package directories under a home directory are libraries too.
fn is_system_mount(mount_path: &str, backing: Option<&str>) -> bool {
    const SYS_PREFIXES: [&str; 9] = [
        "/usr/", "/lib", "/bin", "/sbin", "/etc/", "/dev/", "/proc", "/sys/", "/run/",
    ];
    let sys = |p: &str| {
        SYS_PREFIXES.iter().any(|q| p.starts_with(q))
            || matches!(p, "/usr" | "/lib" | "/bin" | "/sbin" | "/dev")
            || p.contains("/site-packages")
            || p.contains("/dist-packages")
    };
    sys(mount_path) || backing.is_some_and(sys)
}

/// Choose the volumeMount that holds the model, by scoring rather than first-match.
///
/// First-match was wrong on this cluster: every RBLN deployment has a volume *named*
/// `host-rbln-lib`, the hint "rbln" matched its name, and it sorts before the volume actually
/// named `model` — so lmd-top reported the model as living in `/usr/lib` for eight of sixteen
/// deployments. Scoring with system mounts disqualified picks the model directory instead.
fn pick_model_mount<'a>(
    vms: &'a [serde_json::Value],
    pod: &serde_json::Value,
) -> Option<&'a serde_json::Value> {
    // Strong: names a model or its artifacts. Medium: a vendor build directory. Weak: a cache
    // that may or may not be the model's.
    const STRONG: [&str; 7] = [
        "model",
        "artifact",
        "ckpt",
        "checkpoint",
        "weight",
        "gguf",
        "safetensor",
    ];
    const MEDIUM: [&str; 4] = ["rbln", "furiosa", "fxb", "compiled"];
    const WEAK: [&str; 5] = ["hf", "cache", "data", "npu", "store"];

    let score = |m: &serde_json::Value| -> i32 {
        let mp = m["mountPath"].as_str().unwrap_or("").to_lowercase();
        let vn = m["name"].as_str().unwrap_or("");
        let backing = volume_host_path(pod, vn);
        if is_system_mount(&mp, backing) {
            return -1;
        }
        let hay = format!(
            "{} {} {}",
            mp,
            vn.to_lowercase(),
            backing.unwrap_or("").to_lowercase()
        );
        let hits = |set: &[&str]| set.iter().filter(|h| hay.contains(**h)).count() as i32;
        hits(&STRONG) * 3 + hits(&MEDIUM) * 2 + hits(&WEAK)
    };

    // Highest score wins. Ties keep document order, which preserves the old behaviour for the
    // cases that were already right -- max_by_key returns the *last* maximum, so the index is
    // reversed to make the first one win. If everything is disqualified we still fall back to
    // the first mount, so the field says something rather than nothing.
    vms.iter()
        .enumerate()
        .filter(|(_, m)| score(m) >= 0)
        .max_by_key(|(i, m)| (score(m), std::cmp::Reverse(*i)))
        .map(|(_, m)| m)
        .or_else(|| vms.first())
}

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

    // 저장 위치: 어느 volumeMount 가 "모델"인지 점수로 고른다.
    let mut mount = String::new();
    let mut host_path: Option<String> = None;
    if let Some(vms) = c["volumeMounts"].as_array() {
        let pick = pick_model_mount(vms, pod);
        if let Some(m) = pick {
            let mp = m["mountPath"].as_str().unwrap_or("").to_string();
            let vn = m["name"].as_str().unwrap_or("");
            host_path = pod["volumes"]
                .as_array()
                .and_then(|vs| vs.iter().find(|v| v["name"].as_str() == Some(vn)))
                .and_then(|v| v["hostPath"]["path"].as_str())
                .map(|p| p.to_string());
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
        host_path,
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
pub(crate) fn model_family(source: &str, name: &str) -> String {
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

/// Collect every device of one accelerator family, driven by its [`Pack`].
///
/// Replaces three near-identical per-vendor collectors. Each one queried its own metric names,
/// read its own label spelling, and normalised its own units — which is where the unit-guessing
/// bug lived (BUG-04) and where a fourth accelerator would have meant a fourth copy. The pack
/// declares metric, unit, aggregation, label spelling and capabilities; this walks them.
async fn collect_pack(prom: &str, pack: &'static crate::accel::Pack) -> (Vec<Accel>, Vec<String>) {
    use crate::accel::{Agg, Field, Health};

    // One query per declared series, all in parallel. Aggregation is pushed into PromQL so a
    // per-core series (Furiosa) collapses the way the pack asks.
    let queries: Vec<String> = pack
        .series
        .iter()
        .map(|sp| {
            let by = std::iter::once(pack.labels.key)
                .chain([pack.labels.id, pack.labels.node])
                .chain(pack.labels.model)
                .chain(pack.labels.busy)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(",");
            let f = match sp.agg {
                Agg::Max => "max",
                Agg::Avg => "avg",
                Agg::Sum => "sum",
            };
            format!("{} by ({}) ({})", f, by, sp.metric)
        })
        .collect();
    let refs: Vec<&str> = queries.iter().map(String::as_str).collect();
    let results = prom::query_all(prom, &refs).await;

    // The utilisation series enumerates the devices; the rest are joined onto it by key label.
    let util_idx = pack
        .series
        .iter()
        .position(|sp| sp.field == Field::Util)
        .expect("every pack declares a Util series");
    let mut errors = Vec::new();
    let mut by_field: BTreeMap<Field, BTreeMap<String, Series>> = BTreeMap::new();
    for (sp, r) in pack.series.iter().zip(results) {
        let rows = resolve(r, sp.metric, &mut errors);
        by_field.insert(sp.field, map_by(rows, pack.labels.key));
    }
    let Some(util) = by_field.get(&Field::Util) else {
        return (Vec::new(), errors);
    };

    // Read a field for one device, already normalised to the UI's unit.
    let read = |field: Field, key: &str| -> Option<f64> {
        let sp = pack.series_for(field)?;
        let v = by_field.get(&field)?.get(key)?.value;
        Some(sp.unit.normalise(v))
    };
    // Fields this accelerator does not report stay NaN — distinguishable from a zero reading.
    let read_or_nan = |field: Field, key: &str| read(field, key).unwrap_or(f64::NAN);

    let mut out: Vec<Accel> = Vec::new();
    for (key, sample) in util {
        let model = pack
            .labels
            .model
            .map(|l| gpu_model(sample.l(l)))
            .unwrap_or_default();
        let unified = pack.caps.unified_memory || is_unified(&model);
        let id = {
            let raw = sample.l(pack.labels.id);
            // DCGM's device label is a bare index; the others already carry a device name.
            if pack.kind == AccelKind::Gpu {
                format!("gpu{}", raw)
            } else {
                raw.to_string()
            }
        };
        let alive = match pack.caps.health {
            Some(Health::NonZeroIsAlive) => read(Field::Health, key).map(|v| v > 0.0),
            Some(Health::ZeroIsHealthy) => read(Field::Health, key).map(|v| v == 0.0),
            // No health series: a device that is being scraped is present.
            None => None,
        }
        .unwrap_or(true);
        out.push(Accel {
            kind: pack.kind,
            model,
            id,
            node: sample.l(pack.labels.node).to_string(),
            util: pack.series[util_idx].unit.normalise(sample.value),
            mem_used_gb: read_or_nan(Field::MemUsed, key),
            mem_total_gb: read_or_nan(Field::MemTotal, key),
            temp: read_or_nan(Field::Temp, key),
            power: read_or_nan(Field::Power, key),
            busy_model: pack
                .labels
                .busy
                .map(|l| sample.l(l).to_string())
                .unwrap_or_default(),
            alive,
            throttle: read_or_nan(Field::Throttle, key),
            unified_mem: unified,
            mem_bw: read_or_nan(Field::MemBandwidth, key),
            clock_mhz: read_or_nan(Field::ClockMhz, key),
            mem_temp: read_or_nan(Field::MemTemp, key),
            energy_mj: read_or_nan(Field::Energy, key),
        });
    }
    (out, errors)
}

async fn collect_nodes(p: &str) -> (Vec<NodeInfo>, Vec<String>) {
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
    let mut errors = Vec::new();
    let (node_ip, node_meta) = node_res.unwrap_or_else(|e| {
        errors.push(format!("kube nodes: {e}"));
        Default::default()
    });
    let load = resolve(load, metrics::NODE_LOAD1, &mut errors);
    let n_mt = map_by(
        resolve(mt, "node_memory_MemTotal_bytes", &mut errors),
        "instance",
    );
    let n_ma = map_by(
        resolve(ma, "node_memory_MemAvailable_bytes", &mut errors),
        "instance",
    );
    let n_cpu = map_by(
        resolve(cpu, "node_cpu_seconds_total", &mut errors),
        "instance",
    );
    let n_ds = map_by(
        resolve(ds, "node_filesystem_size_bytes", &mut errors),
        "instance",
    );
    let n_da = map_by(
        resolve(da, "node_filesystem_avail_bytes", &mut errors),
        "instance",
    );
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
        let mt = n_mt.get(inst.as_str()).map(|x| x.value).unwrap_or(f64::NAN);
        let ma = n_ma.get(inst.as_str()).map(|x| x.value).unwrap_or(f64::NAN);
        let dsz = n_ds.get(inst.as_str()).map(|x| x.value).unwrap_or(f64::NAN);
        let dav = n_da.get(inst.as_str()).map(|x| x.value).unwrap_or(f64::NAN);
        let meta = node_meta.get(&name).cloned().unwrap_or_default();
        nodes.push(NodeInfo {
            identity: meta.identity,
            load1: load_by.get(&name).copied().unwrap_or(f64::NAN),
            mem_used_gb: to_gb(mt - ma),
            mem_total_gb: to_gb(mt),
            cpu_pct: n_cpu
                .get(inst.as_str())
                .map(|x| x.value)
                .unwrap_or(f64::NAN),
            disk_used_gb: to_gb(dsz - dav),
            disk_total_gb: to_gb(dsz),
            ready: meta.ready,
            cordoned: meta.cordoned,
            pressure: !meta.pressure.is_empty(),
            pressure_kinds: meta.pressure,
            version: meta.version,
            devices: meta.devices,
            name,
            npu: String::new(), // collect_node_npu 가 라벨에서 채움(full tier)
        });
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    (nodes, errors)
}

/// fast tier: 소스별 수집기 4개를 병렬 실행 후 합침. util/mem 반응성을 위해 collect()에서 분리.
pub async fn collect_fast(cfg: &Config) -> crate::snapshot::FastSnapshot {
    let mut result = crate::snapshot::FastSnapshot::default();
    let handles: Vec<_> = crate::accel::packs()
        .iter()
        .copied()
        .map(|pack| {
            let prom = cfg.prom.clone();
            (
                format!("accel/{}", pack.id),
                tokio::spawn(async move {
                    let result = collect_pack(&prom, pack).await;
                    (now_secs(), result)
                }),
            )
        })
        .collect();
    let node_prom = cfg.prom.clone();
    let node_handle = tokio::spawn(async move {
        let result = collect_nodes(&node_prom).await;
        (now_secs(), result)
    });
    for (key, handle) in handles {
        let (at, (devices, errors)) = handle.await.unwrap_or_else(|e| {
            (
                now_secs(),
                (vec![], vec![format!("collector task failed: {e}")]),
            )
        });
        result.sources.insert(
            key,
            crate::quality::SourceStatus::observed(at, devices.len(), &errors),
        );
        result.accel.extend(devices);
    }
    let (at, (nodes, errors)) = node_handle.await.unwrap_or_else(|e| {
        (
            now_secs(),
            (vec![], vec![format!("node collector task failed: {e}")]),
        )
    });
    result.sources.insert(
        "nodes".into(),
        crate::quality::SourceStatus::observed(at, nodes.len(), &errors),
    );
    result.nodes = nodes;
    result
        .accel
        .sort_by(|a, b| (a.kind.rank(), &a.node, &a.id).cmp(&(b.kind.rank(), &b.node, &b.id)));
    for a in result
        .accel
        .iter_mut()
        .filter(|a| a.unified_mem && (!a.mem_total_gb.is_finite() || a.mem_total_gb <= 0.0))
    {
        if let Some(n) = result.nodes.iter().find(|n| n.name == a.node) {
            a.mem_used_gb = n.mem_used_gb;
            a.mem_total_gb = n.mem_total_gb;
        }
    }
    result
}

/// Pod-only views cannot safely choose among multiple series for one pod.
fn unique_endpoint_metrics(
    series: &[Series],
    label: &str,
    metric: &str,
    nonnegative: bool,
    warnings: &mut Vec<String>,
) -> Vec<(String, f64)> {
    let mut values: BTreeMap<String, Option<f64>> = BTreeMap::new();
    for sample in series {
        let pod = sample.l(label);
        if pod.is_empty() {
            continue;
        }
        let value = (sample.value.is_finite() && (!nonnegative || sample.value >= 0.0))
            .then_some(sample.value);
        match values.entry(pod.to_string()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(value);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    values
        .into_iter()
        .filter_map(|(pod, value)| match value {
            Some(value) => Some((pod, value)),
            None => {
                warnings.push(format!(
                    "{metric}: ambiguous or invalid series for {pod}; observation omitted"
                ));
                None
            }
        })
        .collect()
}

fn endpoint_observation(
    result: anyhow::Result<Vec<Series>>,
    label: &str,
    metric: &str,
    nonnegative: bool,
) -> (Vec<(String, f64)>, crate::quality::SourceStatus) {
    let mut errors = Vec::new();
    let series = resolve(result, metric, &mut errors);
    let values = unique_endpoint_metrics(&series, label, metric, nonnegative, &mut errors);
    let status = crate::quality::SourceStatus::observed(now_secs(), values.len(), &errors);
    (values, status)
}

/// Exporter-owned namespace labels may be renamed by Prometheus on collision
/// with scrape labels. Prefer that endpoint identity over the EPP scrape target.
fn epp_endpoint_selector(metric: &str, namespace: &str) -> String {
    let ns = serde_json::to_string(namespace).unwrap();
    format!("({metric}{{exported_namespace={ns}}} or {metric}{{exported_namespace=\"\",namespace={ns}}})")
}

fn epp_scrape_selector(metric: &str, namespace: &str) -> String {
    format!(
        "{metric}{{namespace={}}}",
        serde_json::to_string(namespace).unwrap()
    )
}

fn normalize_score_identity(mut series: Vec<Series>) -> Vec<Series> {
    for sample in &mut series {
        if let Some(pod) = sample.labels.get("exported_pod").cloned() {
            sample.labels.insert("pod".into(), pod);
        }
    }
    series
}

pub async fn collect(cfg: &Config) -> Snapshot {
    let mut snap = Snapshot::default();
    let mut warn = Vec::new();
    snap.ts = now_secs();
    snap.stale_after_secs = cfg.interval_full.saturating_mul(3).max(10);

    // Prometheus 도달성 프로브 — 실패 시 빈 테이블을 "장비 없음"이 아니라 연결 문제로 구분.
    // 프로브는 2초·무재시도이며, 실패 시 서킷을 열어 이후 쿼리들이 네트워크를 건드리지 않게 한다.
    // (서킷 없이는 도달 불가 클러스터의 collect 가 92초 걸려 첫 화면까지 프리즈됐다 — BUG-01.)
    snap.prom_ok = prom::probe(&cfg.prom).await;
    if !snap.prom_ok {
        warn.push(format!(
            "Prometheus unreachable at {} — metric columns stay empty (check LMD_PROM)",
            cfg.prom
        ));
    }

    // 가속기 + 노드는 fast tier(collect_fast) 재사용 — 중복 제거
    snap.merge_fast(collect_fast(cfg).await);
    collect_node_npu(&mut snap.nodes).await; // 노드 라벨에서 NPU 드라이버/SDK 요약 채움

    // ---------- EPP pools (독립 쿼리 — 병렬 배칭으로 순차 라운드트립 제거) ----------
    let dec_q = format!(
        "sum by (pod_name) {}",
        epp_endpoint_selector("inference_extension_scheduler_attempts_total", &cfg.ns)
    );
    let pidx_q = format!(
        "max({})",
        epp_scrape_selector("inference_extension_prefix_indexer_size", &cfg.ns)
    );
    let ready_q = epp_scrape_selector(metrics::POOL_READY, &cfg.ns);
    let queue_q = epp_scrape_selector(metrics::POOL_QUEUE, &cfg.ns);
    let kv_q = epp_scrape_selector(metrics::POOL_KV, &cfg.ns);
    let sat_q = epp_scrape_selector(metrics::POOL_SAT, &cfg.ns);
    let ppq_q = epp_scrape_selector(metrics::POOL_PER_POD_QUEUE, &cfg.ns);
    let score_q = epp_endpoint_selector(metrics::EPP_ENDPOINT_SCORE, &cfg.ns);
    let (r_ready, r_q, r_kv, r_sat, r_dec, r_pidx, r_ppq, r_score) = tokio::join!(
        prom::query(&cfg.prom, &ready_q),
        prom::query(&cfg.prom, &queue_q),
        prom::query(&cfg.prom, &kv_q),
        prom::query(&cfg.prom, &sat_q),
        prom::query(&cfg.prom, &dec_q),
        prom::query(&cfg.prom, &pidx_q),
        prom::query(&cfg.prom, &ppq_q),
        prom::query(&cfg.prom, &score_q),
    );
    let p_ready = resolve(r_ready, metrics::POOL_READY, &mut warn);
    let p_q = map_by(resolve(r_q, metrics::POOL_QUEUE, &mut warn), "name");
    let p_kv = map_by(resolve(r_kv, metrics::POOL_KV, &mut warn), "name");
    let p_sat = map_by(resolve(r_sat, metrics::POOL_SAT, &mut warn), "name");
    // 라우팅 결정 분배 (트래픽이 EPP 경유 시 채워짐)
    let (decisions, status) = endpoint_observation(r_dec, "pod_name", &dec_q, true);
    snap.decisions = decisions;
    if let Some(error) = &status.error {
        warn.push(error.clone());
    }
    snap.sources.insert("epp_decisions".into(), status);
    snap.decisions
        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let pidx = resolve(r_pidx, &pidx_q, &mut warn);
    snap.prefix_idx = pidx.first().map(|s| s.value).unwrap_or(f64::NAN);

    // per-pod 큐 깊이(요청 분배)
    let (queues, status) =
        endpoint_observation(r_ppq, "model_server_pod", metrics::POOL_PER_POD_QUEUE, true);
    snap.pod_queues = queues;
    if let Some(error) = &status.error {
        warn.push(error.clone());
    }
    snap.sources.insert("epp_queues".into(), status);
    snap.pod_queues
        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // per-pod EPP composite score (epp-score-observer 플러그인 배포 시 채워짐; 없으면 빈 값 → '–')
    let (scores, status) = endpoint_observation(
        r_score.map(normalize_score_identity),
        "pod",
        metrics::EPP_ENDPOINT_SCORE,
        false,
    );
    snap.endpoint_scores = scores;
    if let Some(error) = &status.error {
        warn.push(error.clone());
    }
    snap.sources.insert("epp_scores".into(), status);

    // Perf: 구간별 지연 percentile·토큰분포·처리량 (EPP 정책용). 전부 graceful(NaN).
    let pp = &cfg.prom;
    // vLLM 네이티브 + Furiosa-LLM(K-EXAONE) 을 선택 namespace 안에서 합산(엔진 무관 서빙 건강).
    let (req_v, err_rate, tps_v, prefix_hit, ttft_v, e2e_p95, req_f, tps_f, ttft_f) = tokio::join!(
        performance_query(pp, &cfg.ns, "sum(rate(vllm:request_success_total{namespace=__LMD_NAMESPACE__}[1m]))"),
        performance_query(pp, &cfg.ns, "sum(rate(vllm:request_success_total{namespace=__LMD_NAMESPACE__,finished_reason=\"abort\"}[1m]))"),
        performance_query(pp, &cfg.ns, "sum(rate(vllm:generation_tokens_total{namespace=__LMD_NAMESPACE__}[1m]))"),
        performance_query(pp, &cfg.ns, "sum(rate(vllm:prefix_cache_hits_total{namespace=__LMD_NAMESPACE__}[5m])) / sum(rate(vllm:prefix_cache_queries_total{namespace=__LMD_NAMESPACE__}[5m]))"),
        performance_query(pp, &cfg.ns, "histogram_quantile(0.95, sum by (le)(rate(vllm:time_to_first_token_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))"),
        performance_query(pp, &cfg.ns, "histogram_quantile(0.95, sum by (le)(rate(vllm:e2e_request_latency_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))"),
        performance_query(pp, &cfg.ns, "sum(rate(furiosa_llm_request_success_total{namespace=__LMD_NAMESPACE__}[1m]))"),
        performance_query(pp, &cfg.ns, "sum(rate(furiosa_llm_generation_tokens_total{namespace=__LMD_NAMESPACE__}[1m]))"),
        performance_query(pp, &cfg.ns, "histogram_quantile(0.95, sum by (le)(rate(furiosa_llm_time_to_first_token_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))"),
    );
    let req_v = resolve(req_v, "req_v", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let err_rate = resolve(err_rate, "err_rate", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let tps_v = resolve(tps_v, "tps_v", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let prefix_hit = resolve(prefix_hit, "prefix_hit", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let ttft_v = resolve(ttft_v, "ttft_v", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let e2e_p95 = resolve(e2e_p95, "e2e_p95", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let req_f = resolve(req_f, "req_f", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let tps_f = resolve(tps_f, "tps_f", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    let ttft_f = resolve(ttft_f, "ttft_f", &mut warn)
        .first()
        .map_or(f64::NAN, |s| s.value);
    // throughput 는 가산(NaN=0 취급, 둘 다 NaN 이면 NaN); TTFT 는 단일 엔진 근거일 때만 표시(혼합 p95 합성 불가).
    let nadd = |a: f64, b: f64| {
        if a.is_nan() && b.is_nan() {
            f64::NAN
        } else {
            (if a.is_nan() { 0.0 } else { a }) + (if b.is_nan() { 0.0 } else { b })
        }
    };
    snap.perf = Perf {
        req_rate: nadd(req_v, req_f),
        err_rate,
        tps: nadd(tps_v, tps_f),
        prefix_hit,
        ttft_p95: single_engine_percentile(ttft_v, ttft_f, &mut warn),
        e2e_p95,
    };

    // per-model 성능 — service(=Deployment 이름) 기준 병합. Models 뷰와 동일 키.
    // 여기선 pm 만 채우고, 런칭된 모델 seed 는 collect_kube(모델 목록) 이후에 좌조인(아래).
    let mut pm: BTreeMap<String, PerfRow> = BTreeMap::new();
    // per-Service latency/token/throughput queries run concurrently across engine families.
    // (promql, 필드 setter). setter 는 캡처 없는 클로저라 fn 포인터로 강제.
    type Set = fn(&mut PerfRow, f64);
    let specs: &[(&str, &str, Set)] = &[
        ("sum by (service)(rate(vllm:request_success_total{namespace=__LMD_NAMESPACE__}[1m]))", "req", |r, v| r.req = v),
        ("sum by (service)(rate(vllm:generation_tokens_total{namespace=__LMD_NAMESPACE__}[1m]))", "tps", |r, v| r.tps = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:time_to_first_token_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "ttft_p95", |r, v| r.ttft_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_time_per_output_token_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "tpot_p95", |r, v| r.tpot_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:e2e_request_latency_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "e2e_p95", |r, v| r.e2e_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_prompt_tokens_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "in_tok_p95", |r, v| r.in_tok_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_generation_tokens_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "out_tok_p95", |r, v| r.out_tok_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_queue_time_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "queue_p95", |r, v| r.queue_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_prefill_time_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "prefill_p95", |r, v| r.prefill_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(vllm:request_decode_time_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "decode_p95", |r, v| r.decode_p95 = v),
        ("sum by (service)(rate(vllm:num_preemptions_total{namespace=__LMD_NAMESPACE__}[1m]))", "preempt", |r, v| r.preempt = v),
        // Furiosa-LLM (K-EXAONE 등): furiosa_llm_* 로 노출, 같은 service 조인키. TPOT≈inter-token-latency.
        ("sum by (service)(rate(furiosa_llm_request_success_total{namespace=__LMD_NAMESPACE__}[1m]))", "req", |r, v| r.req = v),
        ("sum by (service)(rate(furiosa_llm_generation_tokens_total{namespace=__LMD_NAMESPACE__}[1m]))", "tps", |r, v| r.tps = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_time_to_first_token_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "ttft_p95", |r, v| r.ttft_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_inter_token_latency_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "tpot_p95", |r, v| r.tpot_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_request_prompt_tokens_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "in_tok_p95", |r, v| r.in_tok_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(furiosa_llm_request_generation_tokens_bucket{namespace=__LMD_NAMESPACE__}[1m])))", "out_tok_p95", |r, v| r.out_tok_p95 = v),
        // Proxy metrics are per Service, never a fixed synthetic model name.
        ("sum by (service)(rate(ds4_proxy_requests_total{namespace=__LMD_NAMESPACE__}[5m]))", "req", |r, v| r.req = v),
        ("sum by (service)(rate(ds4_proxy_output_tokens_total{namespace=__LMD_NAMESPACE__}[5m]))", "tps", |r, v| r.tps = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(ds4_proxy_ttft_seconds_bucket{namespace=__LMD_NAMESPACE__}[5m])))", "ttft_p95", |r, v| r.ttft_p95 = v),
        ("histogram_quantile(0.95, sum by (service,le)(rate(ds4_proxy_request_duration_seconds_bucket{namespace=__LMD_NAMESPACE__}[5m])))", "e2e_p95", |r, v| r.e2e_p95 = v),
    ];
    let scoped: Vec<String> = specs
        .iter()
        .map(|(q, _, _)| performance_query_text(q, &cfg.ns))
        .collect();
    let qs: Vec<&str> = scoped.iter().map(String::as_str).collect();
    let results = prom::query_all(&cfg.prom, &qs).await;
    let mut seen_values = std::collections::HashSet::new();
    for ((promql, field, set), r) in specs.iter().zip(results) {
        for s in &resolve(r, promql, &mut warn) {
            let m = s.l("service");
            if !m.is_empty() {
                record_metric_family(&mut snap.perf_targets, m, promql);
                let value =
                    unique_performance_value(&mut seen_values, m, field, s.value, &mut warn);
                if !value.is_finite() {
                    // A cross-engine total must not retain double-counted or invalid rows.
                    match *field {
                        "req" => snap.perf.req_rate = f64::NAN,
                        "tps" => snap.perf.tps = f64::NAN,
                        _ => {}
                    }
                }
                set(
                    pm.entry(m.to_string()).or_insert_with(|| PerfRow::new(m)),
                    value,
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
    // vLLM 네이티브와 Furiosa-LLM 을 service 키로 병합하되 충돌 값은 제외.
    // 10개 쿼리는 반드시 병렬(query_all) — 직렬 .await 였을 때 Prometheus 장애 시 이 블록만
    // 120초 규모로 늘어나 collect 전체를 마비시켰다(BUG-01).
    let vllm_qs: [&str; 10] = [
        "sum by (service) (vllm:num_requests_running{namespace=__LMD_NAMESPACE__})",
        "sum by (service) (vllm:num_requests_waiting{namespace=__LMD_NAMESPACE__})",
        "sum by (service) (rate(vllm:generation_tokens_total{namespace=__LMD_NAMESPACE__}[1m]))",
        "max by (service) (vllm:kv_cache_usage_perc{namespace=__LMD_NAMESPACE__})",
        "histogram_quantile(0.95, sum by (service,le) (rate(vllm:time_to_first_token_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))",
        "sum by (service) (furiosa_llm_num_requests_running{namespace=__LMD_NAMESPACE__})",
        "sum by (service) (furiosa_llm_num_requests_waiting{namespace=__LMD_NAMESPACE__})",
        "sum by (service) (rate(furiosa_llm_generation_tokens_total{namespace=__LMD_NAMESPACE__}[1m]))",
        "max by (service) (furiosa_llm_kv_cache_usage_percent{namespace=__LMD_NAMESPACE__})",
        "histogram_quantile(0.95, sum by (service,le) (rate(furiosa_llm_time_to_first_token_seconds_bucket{namespace=__LMD_NAMESPACE__}[1m])))",
    ];
    let scoped: Vec<String> = vllm_qs
        .iter()
        .map(|q| performance_query_text(q, &cfg.ns))
        .collect();
    let refs: Vec<&str> = scoped.iter().map(String::as_str).collect();
    let mut vres = prom::query_all(&cfg.prom, &refs).await.into_iter();
    let mut next = |warn: &mut Vec<String>, i: usize| {
        map_by(
            resolve(
                vres.next().unwrap_or_else(|| Ok(Vec::new())),
                vllm_qs[i],
                warn,
            ),
            "service",
        )
    };
    let mut vllm = Vllm {
        // service 라벨 = Deployment 이름(예: gemma4-rbln) → deploy 와 정확히 join.
        run: next(&mut warn, 0),
        wait: next(&mut warn, 1),
        tps: next(&mut warn, 2),
        kv: next(&mut warn, 3),
        ttft: next(&mut warn, 4),
    };
    let other = next(&mut warn, 5);
    merge_engine_metrics(&mut vllm.run, other, "run", &mut warn);
    let other = next(&mut warn, 6);
    merge_engine_metrics(&mut vllm.wait, other, "wait", &mut warn);
    let other = next(&mut warn, 7);
    merge_engine_metrics(&mut vllm.tps, other, "tps", &mut warn);
    let other = next(&mut warn, 8);
    merge_engine_metrics(&mut vllm.kv, other, "kv", &mut warn);
    let other = next(&mut warn, 9);
    merge_engine_metrics(&mut vllm.ttft, other, "ttft", &mut warn);

    // ---------- kube: deployments / pods / routes / gateway / epp ----------
    collect_kube(cfg, &mut snap, &vllm, &mut warn).await;

    // per-model perf: 런칭된 모든 모델(snap.models)을 seed 로 → 트래픽 없어도 표(–)에 표시.
    // Join deployment rows only through declared Service selector relationships.
    {
        let mut rows: Vec<PerfRow> = Vec::new();
        for m in &snap.models {
            let key = crate::service_binding::metric_service(&m.name, &snap.service_bindings, &pm);
            if let Some(target) = key.as_ref().and_then(|k| snap.perf_targets.get(k)).cloned() {
                snap.perf_targets.insert(m.name.clone(), target);
            }
            let mut row = key
                .and_then(|k| pm.remove(&k))
                .unwrap_or_else(|| PerfRow::new(&m.name));
            row.model = m.name.clone(); // 표시명을 배포명으로 통일(Models 와 일관)
            rows.push(row);
        }
        for mut row in pm.into_values() {
            // Keep unbound Service observations distinct from Deployment identities.
            let label = format!("service/{}", row.model);
            if let Some(target) = snap.perf_targets.get(&row.model).cloned() {
                snap.perf_targets.insert(label.clone(), target);
            }
            row.model = label;
            rows.push(row);
        }
        snap.perf_rows = rows;
    }

    let perf_errors: Vec<String> = warn
        .iter()
        .filter(|w| w.starts_with("prom["))
        .cloned()
        .collect();
    snap.sources.insert(
        "performance".into(),
        crate::quality::SourceStatus::observed(
            now_secs(),
            snap.perf_rows
                .iter()
                .filter(|r| r.req.is_finite() || r.tps.is_finite() || r.ttft_p95.is_finite())
                .count(),
            &perf_errors,
        ),
    );

    // ---------- 가속기 재고(런처 솔버용) ----------
    let mut inventory_errors = Vec::new();
    collect_inventory(
        &mut snap.inventory,
        &mut snap.node_alloc,
        &mut snap.node_capacity,
        &mut inventory_errors,
    )
    .await;

    snap.sources.insert(
        "inventory".into(),
        crate::quality::SourceStatus::observed(
            now_secs(),
            snap.node_capacity.len(),
            &inventory_errors,
        ),
    );
    warn.extend(inventory_errors);

    // ---------- 플랫폼 부트스트랩 전제조건(Setup 뷰) ----------
    snap.setup = crate::setup_probe::collect(&cfg.ns).await;
    // BUG-08: `kubectl get deploy -n <missing-ns>` returns success + an empty list, so no collector
    // can tell "nothing deployed" from "wrong namespace". The Setup probe already knows — say so,
    // rather than leaving the user with silently empty views (ENV-03).
    if snap.setup.probed && !snap.setup.ns_exists {
        warn.push(format!(
            "namespace '{}' does not exist — every view stays empty (check LMD_NS)",
            cfg.ns
        ));
    }

    // ---------- 대상 ns 실존 PVC 목록(prefetch/compile 대상 검증용) ----------
    // 없는 PVC 로 Job 을 만들면 pod 가 영구 Pending(FailedScheduling)이 되므로, 폼이 실존 PVC 만 제시하고
    // submit 이 검증하도록 이름을 수집. 미도달/비어있으면 빈 벡터(검증 skip → 오탐 방지).
    if let Some(list) = kube::get_jsonpath(
        &["get", "pvc", "-n", &cfg.ns],
        "{range .items[*]}{.metadata.name}{\"\\n\"}{end}",
    )
    .await
    {
        snap.pvcs = list.split_whitespace().map(|s| s.to_string()).collect();
    }

    // History: serving observations feed the compile-option advisor.
    record_serving(&snap);

    snap.ts = now_secs();
    let errors: Vec<String> = warn
        .iter()
        .cloned()
        .chain(snap.sources.values().filter_map(|s| s.error.clone()))
        .collect();
    snap.sources.insert(
        "full".into(),
        crate::quality::SourceStatus::observed(
            snap.ts,
            snap.models.len() + snap.nodes.len(),
            &errors,
        ),
    );
    snap.warnings = warn;
    snap
}

/// 노드 allocatable 합계 − 파드 requests 합계 = 여유. (가속기 resource별)
async fn collect_inventory(
    inv: &mut Vec<(String, i64, i64)>,
    node_alloc: &mut BTreeMap<String, BTreeMap<String, i64>>,
    node_capacity: &mut BTreeMap<String, BTreeMap<String, i64>>,
    warn: &mut Vec<String>,
) {
    inv.clear();
    node_alloc.clear();
    node_capacity.clear();
    let (nodes, pods) = tokio::join!(
        kube::get_json(&["get", "nodes", "-o", "json"]),
        kube::get_json(&["get", "pods", "-A", "-o", "json"]),
    );
    match nodes.and_then(|nodes| pods.and_then(|pods| crate::inventory::from_json(&nodes, &pods))) {
        Ok((totals, allocations, capacity)) => {
            *inv = totals;
            *node_alloc = allocations;
            *node_capacity = capacity;
        }
        Err(error) => warn.push(format!("inventory unavailable: {error}")),
    }
}

/// What the Kubernetes API says about a node, independent of Prometheus.
#[derive(Clone, Default)]
struct NodeMeta {
    ready: bool,
    cordoned: bool,
    /// Pressure conditions currently True (`DiskPressure`, …).
    pressure: Vec<String>,
    version: String,
    identity: Option<crate::resource_guard::Identity>,
    devices: Vec<DeviceExpectation>,
}

async fn node_kube() -> anyhow::Result<(BTreeMap<String, String>, BTreeMap<String, NodeMeta>)> {
    let value = kube::get_json(&["get", "nodes", "-o", "json"]).await?;
    parse_node_list(&value)
}

fn parse_node_list(
    v: &serde_json::Value,
) -> anyhow::Result<(BTreeMap<String, String>, BTreeMap<String, NodeMeta>)> {
    let mut ip = BTreeMap::new();
    let mut meta = BTreeMap::new();
    let items = v["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("node response has no items array"))?;
    for it in items {
        let name = it["metadata"]["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("node response contains an unnamed node"))?
            .to_string();
        if meta.contains_key(&name) {
            anyhow::bail!("node response contains duplicate node {name}");
        }
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
        let (mut ready, mut pressure) = (false, Vec::new());
        if let Some(cs) = it["status"]["conditions"].as_array() {
            for c in cs {
                let t = c["type"].as_str().unwrap_or("");
                let st = c["status"] == "True";
                match t {
                    "Ready" => ready = st,
                    "MemoryPressure" | "DiskPressure" | "PIDPressure" if st => {
                        pressure.push(t.to_string())
                    }
                    _ => {}
                }
            }
        }
        meta.insert(
            name,
            NodeMeta {
                ready,
                cordoned,
                pressure,
                version: ver,
                identity: crate::resource_guard::Identity::read(it),
                devices: device_expectations(it),
            },
        );
    }
    Ok((ip, meta))
}

/// A malformed list is a failed observation, never an authoritative empty list.
fn validate_resource_list(value: &serde_json::Value) -> anyhow::Result<()> {
    let items = value["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("resource response has no items array"))?;
    let mut names = std::collections::BTreeSet::new();
    for item in items {
        let name = item["metadata"]["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| anyhow::anyhow!("resource list contains an unnamed object"))?;
        let namespace = item["metadata"]["namespace"].as_str().unwrap_or("");
        if !names.insert((namespace, name)) {
            anyhow::bail!("resource list contains a duplicate object identity");
        }
    }
    Ok(())
}

async fn tracked_json(
    snap: &mut Snapshot,
    source: &str,
    args: &[&str],
) -> anyhow::Result<serde_json::Value> {
    let result = kube::get_json(args).await.and_then(|value| {
        validate_resource_list(&value)?;
        Ok(value)
    });
    let mut status = match &result {
        Ok(v) => crate::quality::SourceStatus::observed(
            now_secs(),
            v["items"].as_array().map_or(0, Vec::len),
            &[],
        ),
        Err(e) => crate::quality::SourceStatus::observed(now_secs(), 0, &[e.to_string()]),
    };
    if let Err(e) = &result {
        if e.to_string().contains("doesn't have a resource type") {
            status = crate::quality::SourceStatus::observed(now_secs(), 0, &[]);
            status.unsupported = true;
        }
    }
    snap.sources.insert(source.into(), status);
    result
}

async fn collect_kube(cfg: &Config, snap: &mut Snapshot, vllm: &Vllm, warn: &mut Vec<String>) {
    // routes: path -> backend (+ kind: Service|InferencePool)
    let r = tracked_json(
        snap,
        "routes",
        &["get", "httproute", "-n", &cfg.ns, "-o", "json"],
    )
    .await;
    if let Err(e) = &r {
        kube_warn("httproute", e, warn);
    }
    if let Ok(v) = r {
        (snap.routes, snap.epp_in_path) = crate::routing::rows(&v);
    }
    let deployments = tracked_json(
        snap,
        "workloads",
        &["get", "deploy", "-n", &cfg.ns, "-o", "json"],
    )
    .await;
    let services = tracked_json(
        snap,
        "services",
        &["get", "services", "-n", &cfg.ns, "-o", "json"],
    )
    .await;
    if let Ok(services) = &services {
        snap.service_names = crate::service_binding::names_in_namespace(services, &cfg.ns);
    }
    let pods = tracked_json(snap, "pods", &["get", "pods", "-n", &cfg.ns, "-o", "json"]).await;
    let replicasets = tracked_json(
        snap,
        "replicasets",
        &["get", "replicasets", "-n", &cfg.ns, "-o", "json"],
    )
    .await;
    let slices = tracked_json(
        snap,
        "endpointslices",
        &[
            "get",
            "endpointslices.discovery.k8s.io",
            "-n",
            &cfg.ns,
            "-o",
            "json",
        ],
    )
    .await;
    if let (Ok(deployments), Ok(pods), Ok(replicasets)) = (&deployments, &pods, &replicasets) {
        snap.deployment_pods =
            crate::service_binding::deployment_pods(deployments, pods, replicasets);
    }
    if let (Ok(services), Ok(deployments), Ok(pods), Ok(replicasets), Ok(slices)) =
        (&services, &deployments, &pods, &replicasets, &slices)
    {
        snap.service_bindings = crate::service_binding::verified_bindings(
            services,
            deployments,
            pods,
            replicasets,
            slices,
        );
    }
    for (name, result) in [
        ("services", &services),
        ("replicasets", &replicasets),
        ("endpointslices", &slices),
    ] {
        if let Err(error) = result {
            warn.push(format!(
                "kube {name}: {error}; model metric bindings unavailable"
            ));
        }
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
    match &deployments {
        Ok(v) => {
            if let Some(items) = v["items"].as_array() {
                for d in items {
                    let name = d["metadata"]["name"].as_str().unwrap_or("").to_string();
                    if let Some(version) = crate::resource_guard::Identity::read(d) {
                        snap.deployment_versions.insert(name.clone(), version);
                    }
                    let identity = crate::workload::classify(d);
                    let is_model = identity.role.is_model();
                    snap.workloads.push(identity);
                    if !is_model {
                        continue;
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
                    // Match only a unique observed Service with a declared selector relationship.
                    let mn = crate::service_binding::metric_service(
                        &name,
                        &snap.service_bindings,
                        &vllm.run,
                    );
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
    match &pods {
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
                        identity: crate::resource_guard::Identity::read(p),
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

    collect_events(cfg, snap, warn).await;
    collect_gateway(cfg, snap, warn).await;

    // Configuration presence and successful parsing are separate from runtime health.
    let epp_result = kube::get_json(&["get", "cm", "llmd-router-epp", "-n", &cfg.ns, "-o", "json"])
        .await
        .and_then(|value| {
            let text = kube::cm_data(&value, "default-plugins.yaml")
                .ok_or_else(|| anyhow::anyhow!("EPP ConfigMap has no default-plugins.yaml"))?;
            crate::epp_config::parse(text).ok_or_else(|| {
                anyhow::anyhow!(
                    "EPP configuration has an invalid or unsupported scheduling profile"
                )
            })
        });
    let epp_status = match epp_result {
        Ok(config) => {
            snap.epp = Some(config);
            crate::quality::SourceStatus::observed(now_secs(), 1, &[])
        }
        Err(error) => {
            warn.push(format!("EPP configuration: {error}"));
            crate::quality::SourceStatus::observed(now_secs(), 0, &[error.to_string()])
        }
    };
    snap.sources.insert("epp_config".into(), epp_status);

    // InferencePool 스펙(selector / EPP / endpoints) → prom Pool 과 병합
    let r = tracked_json(
        snap,
        "pools",
        &["get", "inferencepool", "-n", &cfg.ns, "-o", "json"],
    )
    .await;
    if let Err(e) = &r {
        kube_warn("inferencepool", e, warn);
    }
    if let Ok(v) = r {
        if let (Ok(deployments), Ok(pods), Ok(replicasets)) = (&deployments, &pods, &replicasets) {
            snap.pool_bindings =
                crate::service_binding::pool_bindings(&v, deployments, pods, replicasets);
        }
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
    let r = kube::get_json(&["get", "inferenceobjective", "-n", &cfg.ns, "-o", "json"]).await;
    if let Err(e) = &r {
        kube_warn("inferenceobjective", e, warn);
    }
    if let Ok(v) = r {
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
    let r = kube::get_json(&["get", "scaledobject", "-n", &cfg.ns, "-o", "json"]).await;
    if let Err(e) = &r {
        kube_warn("scaledobject", e, warn);
    }
    if let Ok(v) = r {
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

    collect_stored(cfg, snap, warn).await;
    collect_compiles(cfg, snap, warn).await;
    collect_trainings(cfg, snap, warn).await;

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
            parts.push(format!("Furiosa {} drv{}", prod.to_uppercase(), drv));
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
                "Rebellions {} drv{}{}",
                prod,
                drv,
                if installed { "" } else { "(!drv)" }
            ));
        }
        node.npu = parts.join(" · ");
    }
}

/// k8s/llm-d 이벤트(최근 40) → snap.events. collect_kube 에서 분리된 leaf 파서.
async fn collect_events(cfg: &Config, snap: &mut Snapshot, warn: &mut Vec<String>) {
    let result = tracked_json(
        snap,
        "events",
        &[
            "get",
            "events",
            "-n",
            &cfg.ns,
            "--sort-by=.lastTimestamp",
            "-o",
            "json",
        ],
    )
    .await;
    if let Err(error) = &result {
        warn.push(format!("kube events: {error}"));
    }
    if let Ok(v) = result {
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
async fn collect_gateway(cfg: &Config, snap: &mut Snapshot, warn: &mut Vec<String>) {
    let r = tracked_json(
        snap,
        "gateway",
        &["get", "gateway", "-n", &cfg.ns, "-o", "json"],
    )
    .await;
    if let Err(e) = &r {
        kube_warn("gateway", e, warn);
    }
    if let Ok(v) = r {
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

/// Shared-store scan data has its own observation status; failed reads retain prior data.
async fn collect_stored(cfg: &Config, snap: &mut Snapshot, warn: &mut Vec<String>) {
    let result = kube::get_json(&["get", "cm", "model-inventory", "-n", &cfg.ns, "-o", "json"])
        .await
        .and_then(|value| {
            let text = value["data"]["inventory"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("model-inventory has no inventory data"))?;
            let rows = checked_inventory(text)?;
            let capacity = value["data"]["capacity"]
                .as_str()
                .and_then(parse_store_capacity);
            let scanned_at = value["data"]["scanned_at"]
                .as_str()
                .and_then(|text| parse_scan_time(text, now_secs()));
            Ok((rows, capacity, scanned_at))
        });
    let status = match result {
        Ok((rows, capacity, scanned_at)) => {
            let status = crate::quality::SourceStatus::observed(now_secs(), rows.len(), &[]);
            snap.stored = rows;
            snap.store_capacity = capacity;
            snap.store_scanned_at = scanned_at;
            status
        }
        Err(error) => {
            warn.push(format!("model store: {error}"));
            crate::quality::SourceStatus::observed(now_secs(), 0, &[error.to_string()])
        }
    };
    snap.sources.insert("model_store".into(), status);
}

fn parse_scan_time(text: &str, now: u64) -> Option<u64> {
    text.trim()
        .parse::<u64>()
        .ok()
        .filter(|at| *at > 0 && *at <= now)
}

fn checked_inventory(text: &str) -> anyhow::Result<Vec<StoredModel>> {
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split('|').map(str::trim).collect();
        if fields.len() < 6 || fields[0].is_empty() || fields[2].is_empty() || fields[5].is_empty()
        {
            anyhow::bail!("invalid model inventory row {}", index + 1);
        }
    }
    Ok(parse_inventory(text))
}

/// Parse the scan's `"<total> <used> <avail>"` byte triple. None when absent or malformed —
/// a scan predating the capacity measurement writes nothing, and the view simply omits it.
fn parse_store_capacity(txt: &str) -> Option<StoreCapacity> {
    let mut it = txt.split_whitespace();
    let (t, u, a) = (it.next()?, it.next()?, it.next()?);
    let (total_bytes, used_bytes, avail_bytes): (u64, u64, u64) =
        (t.parse().ok()?, u.parse().ok()?, a.parse().ok()?);
    // Reserved blocks may make used + available smaller than total. Values larger
    // than total, overflow or trailing fields cannot describe this scan format.
    let accounted = used_bytes.checked_add(avail_bytes)?;
    if total_bytes == 0 || accounted > total_bytes || it.next().is_some() {
        return None;
    }
    Some(StoreCapacity {
        total_bytes,
        used_bytes,
        avail_bytes,
    })
}

/// Parse the discovery scan's pipe-separated inventory.
///
/// Format: `repo | revision | format | compiled_for | size | path [| built_with]`. The last
/// column is a later addition, so a cluster still running the older CronJob is read as before
/// rather than losing its inventory to a format bump.
fn parse_inventory(txt: &str) -> Vec<StoredModel> {
    let mut out = Vec::new();
    {
        {
            for line in txt.lines() {
                let l = line.trim();
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                let f: Vec<&str> = l.split('|').map(|s| s.trim()).collect();
                if f.len() >= 6 {
                    out.push(StoredModel {
                        family: model_family(f[0], f[0]),
                        repo: f[0].into(),
                        revision: f[1].into(),
                        format: f[2].into(),
                        compiled_for: f[3].into(),
                        size: f[4].into(),
                        path: f[5].into(),
                        // Provenance is a later addition to the scan's output. A cluster
                        // running the older discovery CronJob publishes six fields, and must
                        // keep working rather than losing its whole inventory to a format bump.
                        built_with: f
                            .get(6)
                            .map(|x| x.trim())
                            .filter(|x| !x.is_empty())
                            .unwrap_or("-")
                            .to_string(),
                    });
                }
            }
        }
    }
    out
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

/// npu-lora training Jobs → snap.trainings, joined with the trainer's Prometheus metrics by
/// the `run` label (the Job's `npu-lora/run` label carries the same value).
async fn collect_trainings(cfg: &Config, snap: &mut Snapshot, warn: &mut Vec<String>) {
    let jobs = match kube::get_json(&[
        "get",
        "jobs",
        "-n",
        &cfg.ns,
        "-l",
        "app.kubernetes.io/part-of=npu-lora",
        "-o",
        "json",
    ])
    .await
    {
        Ok(v) => v,
        Err(e) => {
            warn.push(format!("kube npu-lora jobs: {e}"));
            return;
        }
    };
    let mut metric: BTreeMap<(&str, String), f64> = BTreeMap::new();
    if snap.prom_ok {
        const M: [&str; 4] = [
            "npu_lora_step",
            "npu_lora_steps_planned",
            "npu_lora_loss",
            "npu_lora_tokens_per_second",
        ];
        let rs = prom::query_all(&cfg.prom, &M).await;
        for (m, r) in M.iter().zip(rs) {
            for sr in r.unwrap_or_default() {
                metric.insert((m, sr.l("run").to_string()), sr.value);
            }
        }
    }
    let now = now_secs();
    for j in jobs["items"].as_array().into_iter().flatten() {
        let md = &j["metadata"];
        let st = &j["status"];
        let run = md["labels"]["npu-lora/run"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let status = if st["succeeded"].as_u64().unwrap_or(0) > 0 {
            "Complete"
        } else if st["failed"].as_u64().unwrap_or(0) > 0 {
            "Failed"
        } else if st["active"].as_u64().unwrap_or(0) > 0 {
            "Running"
        } else {
            "Pending"
        };
        let started = st["startTime"]
            .as_str()
            .or(md["creationTimestamp"].as_str())
            .unwrap_or("");
        let get = |m: &'static str| metric.get(&(m, run.clone())).copied();
        snap.trainings.push(TrainRun {
            job: md["name"].as_str().unwrap_or("").to_string(),
            model: md["annotations"]["npu-lora/model"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            out: md["annotations"]["npu-lora/out"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            status: status.to_string(),
            age_secs: parse_k8s_ts(started)
                .map(|t| now.saturating_sub(t))
                .unwrap_or(0),
            step: get("npu_lora_step"),
            steps_planned: get("npu_lora_steps_planned"),
            loss: get("npu_lora_loss"),
            tokens_per_s: get("npu_lora_tokens_per_second"),
            run,
        });
    }
    snap.trainings.sort_by_key(|t| t.age_secs);
}

/// 진행/최근 컴파일 Job(`compile-*`)과 프리페치 Job(`prefetch-*`) → snap.compiles. 없으면 빈 값.
/// 상태·경과·(완료 시)소요 + 활성 Job 은 파드 로그 마지막 줄을 진행 힌트로.
async fn collect_compiles(cfg: &Config, snap: &mut Snapshot, warn: &mut Vec<String>) {
    let result = tracked_json(snap, "jobs", &["get", "jobs", "-n", &cfg.ns, "-o", "json"]).await;
    let v = match result {
        Ok(value) => value,
        Err(error) => {
            warn.push(format!("kube jobs: {error}"));
            return;
        }
    };
    let Some(items) = v["items"].as_array() else {
        return;
    };
    let now = now_secs();
    let recorded = crate::history::recorded_ids();
    let pods = kube::get_json(&["get", "pods", "-n", &cfg.ns, "-o", "json"])
        .await
        .ok();
    for j in items {
        let name = j["metadata"]["name"].as_str().unwrap_or("");
        let is_prefetch = name.starts_with("prefetch-");
        if !name.starts_with("compile-") && !is_prefetch {
            continue;
        }
        let execution_id = crate::history::job_execution_id(&cfg.ns, &j["metadata"]);
        // 이름 = compile-{model}-{target}(target 은 "-rbln-"/"-rngd-" 부터) 또는 prefetch-{model}-hf.
        let (model, target, vendor) = if is_prefetch {
            let rest = name.strip_prefix("prefetch-").unwrap_or(name);
            let rest = rest.strip_suffix("-hf").unwrap_or(rest);
            (
                rest.to_string(),
                "download".to_string(),
                "prefetch".to_string(),
            )
        } else {
            let rest = &name["compile-".len()..];
            if let Some(i) = rest.find("-rbln-") {
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
            }
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
        // Compile options and source come off the Job's own environment, so history records
        // what was actually built rather than re-deriving it from the name.
        let (options, source) = job_options(j);
        let pod = pods
            .as_ref()
            .and_then(|v| v["items"].as_array())
            .and_then(|items| {
                items.iter().find(|p| {
                    p["metadata"]["ownerReferences"]
                        .as_array()
                        .is_some_and(|owners| {
                            owners.iter().any(|o| {
                                o["kind"] == "Job"
                                    && o["uid"].as_str().is_some()
                                    && o["uid"] == j["metadata"]["uid"]
                            })
                        })
                })
            })
            .and_then(|p| p["metadata"]["name"].as_str())
            .map(str::to_string);

        // Read the log once for any terminal job we have not recorded yet: a failure needs its
        // cause classified before `ttlSecondsAfterFinished` removes the Job, and a *success*
        // needs its toolchain captured — a version skew is only visible against a build that
        // worked. Already-recorded jobs are skipped, so this is not a per-tick cost.
        let terminal = status == "Complete" || status == "Failed";
        let (failure, toolchain) =
            if terminal && !execution_id.is_empty() && !recorded.contains(&execution_id) {
                let log = match &pod {
                    Some(p) => kube::log_tail(&cfg.ns, p, 120).await.unwrap_or_default(),
                    None => String::new(),
                };
                let tools = crate::diagnose::toolchain(&log);
                let fail = if status == "Failed" {
                    let exit = match &pod {
                        Some(p) => kube::pod_exit_code(&cfg.ns, p).await,
                        None => None,
                    };
                    Some(crate::diagnose::classify(&log, exit))
                } else {
                    None
                };
                (fail, tools)
            } else {
                (None, BTreeMap::new())
            };

        // 진행 힌트: 활성 Job 은 파드 로그 마지막 비어있지 않은 줄. 완료/실패는 상태로 대체.
        let phase = if active {
            match &pod {
                Some(p) => kube::last_log_line(&cfg.ns, p)
                    .await
                    .unwrap_or_else(|| "progress unavailable (no log observation)".to_string()),
                None => "progress unavailable (no pod observation)".to_string(),
            }
        } else if status == "Complete" {
            "COMPILE_DONE".to_string()
        } else if let Some(f) = &failure {
            f.line()
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
            identity: crate::resource_guard::Identity::read(j),
            execution_id,
            name: name.to_string(),
            model,
            vendor,
            target,
            status,
            age_secs,
            duration_secs,
            phase,
            progress,
            failure,
            options,
            source,
            toolchain,
        });
    }
    record_finished_jobs(&snap.compiles);
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

/// Compile options and HF source from a Job's container environment.
///
/// Read from the Job rather than parsed back out of its name: the name is a lossy summary
/// (`rbln-ca22-tp4-s8192` omits batch, attn and quantisation), and history needs the full set
/// to be able to recommend one.
fn job_options(job: &serde_json::Value) -> (BTreeMap<String, String>, String) {
    let mut opts = BTreeMap::new();
    let mut source = String::new();
    let envs = job["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for e in envs {
        let (Some(k), Some(v)) = (e["name"].as_str(), e["value"].as_str()) else {
            continue;
        };
        // Map the recipe's environment names back onto form option keys.
        let key = match k {
            "MODEL_ID" | "SOURCE" => {
                source = v.to_string();
                continue;
            }
            "RBLN_TENSOR_PARALLEL_SIZE" | "TENSOR_PARALLEL_SIZE" | "TP" => "tp",
            "PIPELINE_PARALLEL_SIZE" | "PP" => "pp",
            "RBLN_MAX_SEQ_LEN" | "MAX_SEQ_LEN_TO_CAPTURE" | "MAX_LEN" => "max-len",
            "RBLN_BATCH_SIZE" | "BUCKET_BATCH_SIZE" => "batch",
            "RBLN_ATTN_IMPL" => "attn",
            "RBLN_KVCACHE_PARTITION_LEN" => "kvpart",
            "RBLN_NPU" => "npu",
            "RBLN_QUANTIZATION" | "USE_ACTIVATION_DQ" => "quant",
            "PAGED_ATTENTION_BLOCK_SIZE" => "block",
            "PREFILL_CHUNK_SIZE" => "chunk",
            _ => continue,
        };
        opts.insert(key.to_string(), v.to_string());
    }
    (opts, source)
}

/// Record how a serving deployment is actually performing, so the advisor can prefer option
/// sets that served well over ones that merely compiled.
///
/// Only under real load (`tps > 0`) — an idle deployment says nothing about its options. At
/// most one record per deployment per hour, since this runs on every collect tick.
fn record_serving(snap: &Snapshot) {
    let with_traffic: Vec<&PerfRow> = snap
        .perf_rows
        .iter()
        .filter(|r| r.tps.is_finite() && r.tps > 0.0)
        .collect();
    if with_traffic.is_empty() {
        return;
    }
    let hour = now_secs() / 3600;
    let known = crate::history::recorded_ids();
    for row in with_traffic {
        let id = format!("serve:{}:{}", row.model, hour);
        if known.contains(&id) {
            continue;
        }
        // Options and source come from the serving artifact discovered on the deployment.
        let art = snap.artifacts.iter().find(|a| a.model == row.model);
        let options: BTreeMap<String, String> = art
            .map(|a| a.opts.iter().cloned().collect())
            .unwrap_or_default();
        let vendor = art
            .map(|a| crate::app::App::vendor_for_engine(&a.engine).to_string())
            .unwrap_or_else(|| "gpu".to_string());
        let model = art
            .map(|a| {
                if a.source.is_empty() {
                    a.model.clone()
                } else {
                    a.source.clone()
                }
            })
            .unwrap_or_else(|| row.model.clone());
        crate::history::append(&crate::history::Record {
            ts: now_secs(),
            kind: "serve".into(),
            id,
            model,
            vendor,
            options,
            outcome: crate::history::Outcome::Ok,
            duration_secs: None,
            failure_kind: String::new(),
            detail: String::new(),
            tps: Some(row.tps),
            ttft_p95: Some(row.ttft_p95).filter(|v| v.is_finite()),
            toolchain: BTreeMap::new(),
        });
    }
}

/// Write a history record for each newly-finished compile Job.
///
/// Deduplicated by Job execution identity, so a Job that stays visible across many ticks is recorded once,
/// and the record outlives the Job itself — which is the whole point.
fn record_finished_jobs(jobs: &[CompileJob]) {
    let terminal: Vec<&CompileJob> = jobs
        .iter()
        .filter(|j| j.status == "Complete" || j.status == "Failed")
        .collect();
    if terminal.is_empty() {
        return;
    }
    let known = crate::history::recorded_ids();
    for j in terminal {
        if j.execution_id.is_empty() || known.contains(&j.execution_id) {
            continue;
        }
        let ok = j.status == "Complete";
        crate::history::append(&crate::history::Record {
            ts: now_secs(),
            kind: if j.name.starts_with("prefetch-") {
                "prefetch".into()
            } else {
                "compile".into()
            },
            id: j.execution_id.clone(),
            // Prefer the Job's own MODEL_ID. Falling back to the name means undoing the
            // DNS-safe encoding (`org--name`), or the family key would not match anything.
            model: if j.source.is_empty() {
                j.model.replacen("--", "/", 1)
            } else {
                j.source.clone()
            },
            vendor: crate::accel::by_id(&j.vendor)
                .map(|p| p.id.to_string())
                .unwrap_or_else(|| j.vendor.to_lowercase()),
            options: j.options.clone(),
            outcome: if ok {
                crate::history::Outcome::Ok
            } else {
                crate::history::Outcome::Fail
            },
            duration_secs: j.duration_secs,
            failure_kind: j
                .failure
                .as_ref()
                .map(|f| f.kind.to_string())
                .unwrap_or_default(),
            detail: j.failure.as_ref().map(|f| f.line()).unwrap_or_default(),
            tps: None,
            ttft_p95: None,
            toolchain: j.toolchain.clone(),
        });
    }
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

    /// The volume layouts of this cluster's real Deployments, verbatim.
    ///
    /// BUG: every RBLN deployment mounts /usr/lib at /host-rbln-lib from a volume *named*
    /// `host-rbln-lib`. The old first-match picker matched "rbln" in that name and reported the
    /// model as living in /usr/lib for eight of sixteen deployments.
    #[test]
    fn model_mount_is_the_model_not_the_vendor_libraries() {
        /// (volume name, mountPath, hostPath or "" for an emptyDir).
        type Mount = (&'static str, &'static str, &'static str);
        /// (deployment, its mounts, the volume the picker must choose).
        type Case = (&'static str, &'static [Mount], &'static str);
        let cases: [Case; 4] = [
            (
                "gemma4-rbln",
                &[
                    (
                        "host-local-pkgs",
                        "/home/user/.local/lib/python3.10/site-packages",
                        "/home/user/.local/lib/python3.10/site-packages",
                    ),
                    (
                        "host-sys-local-pkgs",
                        "/host-sys-local-pkgs",
                        "/usr/local/lib/python3.10/dist-packages",
                    ),
                    (
                        "host-sys-pkgs",
                        "/host-sys-pkgs",
                        "/usr/lib/python3/dist-packages",
                    ),
                    ("host-libs", "/host-libs", "/usr/lib/x86_64-linux-gnu"),
                    ("host-rbln-lib", "/host-rbln-lib", "/usr/lib"),
                    ("host-rbln-bin", "/host-rbln-bin", "/usr/bin"),
                    (
                        "model",
                        "/model",
                        "/home/user/rbln-gemma4-26b-a4b-tp4-s8192",
                    ),
                    ("shm", "/dev/shm", ""),
                ],
                "model",
            ),
            (
                "sw-atom-decode-b1",
                &[
                    (
                        "local-pkgs",
                        "/home/user/.local/lib/python3.10/site-packages",
                        "/home/user/.local/lib/python3.10/site-packages",
                    ),
                    ("libs", "/host-libs", "/usr/lib/x86_64-linux-gnu"),
                    ("rbln-lib", "/host-rbln-lib", "/usr/lib"),
                    ("rbln-bin", "/host-rbln-bin", "/usr/bin"),
                    (
                        "app",
                        "/app",
                        "/home/user/workspace/01_pd_disag/rngd_llmd",
                    ),
                    ("hf", "/hf", "/home/user/.cache/huggingface"),
                    ("dshm", "/dev/shm", "/dev/shm"),
                    ("artifacts", "/artifacts", "/home/user/rbln-llama8b-pd"),
                ],
                "artifacts",
            ),
            (
                // No artifact directory at all: the HF cache is the best available answer.
                "hetero-pd-gateway",
                &[
                    ("rbln-lib", "/host-rbln-lib", "/usr/lib"),
                    (
                        "app",
                        "/app",
                        "/home/user/workspace/01_pd_disag/rngd_llmd",
                    ),
                    ("hf", "/hf", "/home/user/.cache/huggingface"),
                    ("dshm", "/dev/shm", "/dev/shm"),
                    ("cfg", "/cfg", ""),
                ],
                "hf",
            ),
            (
                "ds4-v4flash-gb10",
                &[
                    ("ds4bin", "/ds4", "/home/user/ds4"),
                    ("gguf", "/gguf", "/home/user/model-cache/ds4-gguf"),
                    ("kv", "/tmp/ds4-kv", ""),
                    ("dshm", "/dev/shm", ""),
                ],
                "gguf",
            ),
        ];
        for (deploy, mounts, want) in cases {
            let vms: Vec<serde_json::Value> = mounts
                .iter()
                .map(|(n, mp, _)| serde_json::json!({ "name": n, "mountPath": mp }))
                .collect();
            let vols: Vec<serde_json::Value> = mounts
                .iter()
                .map(|(n, _, hp)| {
                    if hp.is_empty() {
                        serde_json::json!({ "name": n, "emptyDir": {} })
                    } else {
                        serde_json::json!({ "name": n, "hostPath": { "path": hp } })
                    }
                })
                .collect();
            let pod = serde_json::json!({ "volumes": vols });
            let got = pick_model_mount(&vms, &pod).expect("a mount");
            assert_eq!(
                got["name"].as_str(),
                Some(want),
                "{}: picked {:?}",
                deploy,
                got["name"]
            );
        }
    }

    /// A pod whose every mount is a system directory must still report something, and a pod
    /// with no mounts at all must not panic.
    #[test]
    fn model_mount_degrades_without_a_candidate() {
        let vms = vec![
            serde_json::json!({ "name": "libs", "mountPath": "/host-libs" }),
            serde_json::json!({ "name": "bin", "mountPath": "/host-bin" }),
        ];
        let pod = serde_json::json!({ "volumes": [
            { "name": "libs", "hostPath": { "path": "/usr/lib" } },
            { "name": "bin", "hostPath": { "path": "/usr/bin" } },
        ] });
        assert_eq!(
            pick_model_mount(&vms, &pod).map(|m| m["name"].as_str()),
            Some(Some("libs")),
            "falls back to the first mount rather than reporting nothing"
        );
        assert!(pick_model_mount(&[], &pod).is_none());
    }

    /// The system-mount judgement is on the backing path, since the container-side path can
    /// look perfectly model-ish.
    #[test]
    fn system_mounts_are_judged_by_where_they_come_from() {
        assert!(is_system_mount("/host-rbln-lib", Some("/usr/lib")));
        assert!(is_system_mount(
            "/model",
            Some("/usr/lib/python3/dist-packages")
        ));
        assert!(is_system_mount(
            "/home/user/.local/lib/python3.10/site-packages",
            None
        ));
        assert!(is_system_mount("/dev/shm", None));
        // A real model directory under a home directory is not a system mount.
        assert!(!is_system_mount(
            "/model",
            Some("/home/user/rbln-gemma4-26b-a4b-tp4-s8192")
        ));
        assert!(!is_system_mount("/model-cache", None));
    }

    /// The exact string the cluster's scan publishes, and the ways it can be absent.
    #[test]
    fn store_capacity_parses_and_degrades() {
        // Measured on the real store: 52.3T total, 6.4T used, 45.9T available.
        let c = parse_store_capacity("57504801730560 7038588125184 50466213605376")
            .expect("the live format must parse");
        assert_eq!(c.total_bytes, 57_504_801_730_560);
        assert_eq!(c.used_bytes, 7_038_588_125_184);
        assert_eq!(c.avail_bytes, 50_466_213_605_376);
        // df -h reports 12% used; used/(used+avail) must land there.
        assert!(
            (c.used_pct() - 12.24).abs() < 0.1,
            "used_pct was {}",
            c.used_pct()
        );

        // A scan predating the measurement writes an empty file, and older scans write no key.
        assert!(parse_store_capacity("").is_none());
        assert!(parse_store_capacity("   \n").is_none());
        // Malformed input must not become a misleading zero-capacity bar.
        for invalid in [
            "100 90 20",
            "100 101 0",
            "100 0 101",
            "100 10 90 trailing",
            "18446744073709551615 18446744073709551615 1",
            "100 -1 10",
        ] {
            assert!(parse_store_capacity(invalid).is_none(), "{invalid}");
        }
        assert!(
            parse_store_capacity("100 40 50").is_some(),
            "reserved blocks are valid"
        );
        assert!(parse_store_capacity("1 2").is_none(), "too few fields");
        assert!(parse_store_capacity("a b c").is_none(), "non-numeric");
        assert!(
            parse_store_capacity("0 0 0").is_none(),
            "zero total says nothing"
        );
    }

    /// A full store must not divide by zero, and a store with no reserved blocks must read 100%.
    #[test]
    fn store_capacity_pct_edges() {
        let full = StoreCapacity {
            total_bytes: 100,
            used_bytes: 100,
            avail_bytes: 0,
        };
        assert!((full.used_pct() - 100.0).abs() < f64::EPSILON);
        let empty = StoreCapacity {
            total_bytes: 100,
            used_bytes: 0,
            avail_bytes: 0,
        };
        assert_eq!(empty.used_pct(), 0.0, "must not divide by zero");
    }

    /// The exact bytes the cluster's discovery CronJob publishes today, plus the seventh
    /// column it publishes after the provenance change. Both must parse.
    #[test]
    fn inventory_parses_with_and_without_provenance() {
        // Six columns: what a cluster running the older scan emits.
        let old = "\
# repo | revision | format | compiled_for | size | path
furiosa-ai/Qwen3-4B-FP8 | - | furiosa | rngd-tp8-pp1-s8192 | 109M | compiled/furiosa-ai--Qwen3-4B-FP8/furiosa/rngd-tp8-pp1-s8192/
";
        let rows = parse_inventory(old);
        assert_eq!(rows.len(), 1, "comment line must be skipped");
        assert_eq!(rows[0].repo, "furiosa-ai/Qwen3-4B-FP8");
        assert_eq!(rows[0].size, "109M");
        assert_eq!(
            rows[0].built_with, "-",
            "an older inventory must read as unknown provenance, not drop the row"
        );

        // Seven columns, both vendors, as the updated scan emits them.
        let new = "\
# repo | revision | format | compiled_for | size | path | built_with
furiosa-ai/Qwen3-4B-FP8 | - | furiosa | rngd-tp8-pp1-s8192 | 109M | compiled/a/furiosa/x/ | furiosa-compiler=2026.3.0
KISTI-KONI/KONI | - | rbln | RBLN-CA22-tp4-s8192 | 20G | compiled/b/rbln/y/ | optimum-rbln=0.10.2
meta-llama/L | main | hf | - | 16G | hub/models--meta-llama--L/snapshots/abc/ | -
";
        let rows = parse_inventory(new);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].built_with, "furiosa-compiler=2026.3.0");
        assert_eq!(rows[1].built_with, "optimum-rbln=0.10.2");
        assert_eq!(
            rows[2].built_with, "-",
            "source weights have no compiler, and an explicit - must stay unknown"
        );
        // The added column must not disturb the columns before it.
        assert_eq!(rows[1].compiled_for, "RBLN-CA22-tp4-s8192");
        assert_eq!(rows[1].path, "compiled/b/rbln/y/");
    }

    /// A blank seventh column (a scan that found no version) must read as unknown rather than
    /// as an empty string that would render as a gap.
    #[test]
    fn inventory_treats_a_blank_provenance_column_as_unknown() {
        let rows = parse_inventory("org/n | - | rbln | tp4 | 1G | compiled/x/rbln/tp4/ |  \n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].built_with, "-");
    }

    #[test]
    fn to_gb_handles_missing() {
        assert_eq!(to_gb(2.0e9), 2.0);
        assert!(to_gb(f64::NAN).is_nan()); // Missing must never become measured zero
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

#[cfg(test)]
mod node_list_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn malformed_node_list_is_not_a_successful_empty_observation() {
        for value in [
            json!({}),
            json!({"items":null}),
            json!({"items":[{}]}),
            json!({"items":[{"metadata":{"name":"a"}},{"metadata":{"name":"a"}}]}),
        ] {
            assert!(parse_node_list(&value).is_err());
        }
        let (addresses, nodes) = parse_node_list(&json!({"items":[]})).unwrap();
        assert!(addresses.is_empty() && nodes.is_empty());
    }
    #[test]
    fn node_observation_retains_identity_and_scheduling_conditions() {
        let value = json!({"items":[{"metadata":{"name":"worker","uid":"uid-1","resourceVersion":"42"},
            "spec":{"unschedulable":true},"status":{"conditions":[{"type":"Ready","status":"True"}],
            "addresses":[{"type":"InternalIP","address":"192.0.2.1"}]}}]});
        let (addresses, nodes) = parse_node_list(&value).unwrap();
        assert_eq!(addresses["192.0.2.1"], "worker");
        let node = &nodes["worker"];
        assert!(node.ready && node.cordoned);
        assert_eq!(node.identity.as_ref().unwrap().uid, "uid-1");
        assert_eq!(node.identity.as_ref().unwrap().version, "42");
    }
    #[test]
    fn node_declares_devices_from_labels_even_when_the_plugin_is_gone() {
        // npu-node-1 during the 2026-10-14 DiskPressure eviction: feature-discovery labels stay,
        // the RBLN device plugin is evicted so ATOM drops out of allocatable entirely.
        let value = json!({"items":[{"metadata":{"name":"npu-node-1","labels":{
                "rebellions.ai/npu.count":"4","furiosa.ai/npu.count":"3","unrelated.count":"9"}},
            "status":{"allocatable":{"furiosa.ai/rngd":"3","cpu":"64"},
                "conditions":[{"type":"Ready","status":"True"},{"type":"DiskPressure","status":"True"}]}}]});
        let (_, nodes) = parse_node_list(&value).unwrap();
        let node = &nodes["npu-node-1"];
        assert_eq!(node.pressure, vec!["DiskPressure".to_string()]);
        let rbln = node.devices.iter().find(|d| d.label == "RBLN").unwrap();
        assert_eq!((rbln.declared, rbln.allocatable), (4, None));
        let rngd = node.devices.iter().find(|d| d.label == "RNGD").unwrap();
        assert_eq!((rngd.declared, rngd.allocatable), (3, Some(3)));
        assert_eq!(node.devices.len(), 2);
    }
}

#[cfg(test)]
mod resource_list_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn endpoint_query_failure_and_successful_empty_have_different_quality() {
        let (_, failed) =
            endpoint_observation(Err(anyhow::anyhow!("offline")), "pod", "score", false);
        let (_, empty) = endpoint_observation(Ok(vec![]), "pod", "score", false);
        assert_eq!(failed.state(now_secs(), 30), crate::quality::Quality::Error);
        assert_eq!(empty.state(now_secs(), 30), crate::quality::Quality::Empty);
    }
    #[test]
    fn endpoint_metric_collisions_do_not_choose_arbitrary_series() {
        let series = |pod: &str, value| Series {
            labels: [("pod".into(), pod.into())].into(),
            value,
        };
        let mut warnings = Vec::new();
        let values = unique_endpoint_metrics(
            &[
                series("ambiguous", 1.0),
                series("ambiguous", 2.0),
                series("invalid", f64::NAN),
                series("valid", 0.0),
            ],
            "pod",
            "queue",
            true,
            &mut warnings,
        );
        assert_eq!(values, vec![("valid".into(), 0.0)]);
        assert_eq!(warnings.len(), 2);
    }
    #[test]
    fn scan_time_rejects_unknown_and_future_observations() {
        for text in ["", "bad", "0", "101", "-1"] {
            assert_eq!(parse_scan_time(text, 100), None);
        }
        assert_eq!(parse_scan_time(" 90\n", 100), Some(90));
    }
    #[test]
    fn malformed_inventory_does_not_silently_drop_models() {
        assert!(checked_inventory("").unwrap().is_empty());
        assert!(checked_inventory("# scan header\n").unwrap().is_empty());
        assert_eq!(
            checked_inventory("org/model | rev | rbln | tp1 | 1G | compiled/model")
                .unwrap()
                .len(),
            1
        );
        for bad in [
            "partial row",
            "org/model | rev | rbln | tp1 | 1G | ",
            " | rev | rbln | tp1 | 1G | path",
        ] {
            assert!(checked_inventory(bad).is_err());
        }
    }
    #[test]
    fn resource_lists_distinguish_bad_payloads_from_empty_and_namespaced_lists() {
        for value in [
            json!({"kind":"Status","status":"Failure"}),
            json!({"items":{}}),
            json!({"items":[null]}),
            json!({"items":[{"metadata":{"name":""}}]}),
            json!({"items":[{"metadata":{"name":"same"}},{"metadata":{"name":"same"}}]}),
        ] {
            assert!(validate_resource_list(&value).is_err());
        }
        assert!(validate_resource_list(&json!({"items":[]})).is_ok());
        assert!(validate_resource_list(&json!({"items":[
            {"metadata":{"name":"same","namespace":"a"}},
            {"metadata":{"name":"same","namespace":"b"}}
        ]}))
        .is_ok());
    }
}

#[cfg(test)]
mod epp_identity_tests {
    use super::*;
    #[test]
    fn scrape_target_pod_does_not_replace_endpoint_identity() {
        let samples = ["model-a", "model-b"]
            .into_iter()
            .map(|pod| Series {
                labels: BTreeMap::from([
                    ("pod".into(), "epp-worker".into()),
                    ("exported_pod".into(), pod.into()),
                ]),
                value: 0.5,
            })
            .collect();
        let (values, status) =
            endpoint_observation(Ok(normalize_score_identity(samples)), "pod", "score", false);
        assert_eq!(values, [("model-a".into(), 0.5), ("model-b".into(), 0.5)]);
        assert!(status.error.is_none());
        let original = Series {
            labels: BTreeMap::from([("pod".into(), "native-model".into())]),
            value: 0.0,
        };
        assert_eq!(
            normalize_score_identity(vec![original])[0].l("pod"),
            "native-model"
        );
    }
    #[test]
    fn endpoint_namespace_prefers_exported_identity_and_pool_uses_scrape_scope() {
        assert_eq!(epp_endpoint_selector("metric", "tenant-a"), "(metric{exported_namespace=\"tenant-a\"} or metric{exported_namespace=\"\",namespace=\"tenant-a\"})");
        assert_eq!(
            epp_scrape_selector("pool", "tenant-a"),
            "pool{namespace=\"tenant-a\"}"
        );
        assert_ne!(
            epp_endpoint_selector("metric", "tenant-a"),
            epp_endpoint_selector("metric", "tenant-b")
        );
        assert!(!epp_endpoint_selector("metric", "bad\"\n").contains('\n'));
    }
}
