//! Snapshot 도메인 타입 + prometheus/kubectl 에서 수집하는 로직.
//! 각 소스는 독립적으로 실패해도 전체를 막지 않음(warnings 에 누적, 부재 필드는 None/빈값).

use crate::kube;
use crate::prom::{self, Series};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct Config {
    pub prom: String,
    pub ns: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prom: std::env::var("LMD_PROM")
                .unwrap_or_else(|_| "192.0.2.5:30090".to_string()),
            ns: std::env::var("LMD_NS").unwrap_or_else(|_| "llm-serving".to_string()),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum AccelKind {
    Gpu,
    Rbln,
    Rngd,
}
impl AccelKind {
    pub fn label(&self) -> &'static str {
        match self {
            AccelKind::Gpu => "A100",
            AccelKind::Rbln => "RBLN",
            AccelKind::Rngd => "RNGD",
        }
    }
}

#[derive(Clone)]
pub struct Accel {
    pub kind: AccelKind,
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
}

#[derive(Clone)]
pub struct NodeInfo {
    pub name: String,
    pub load1: f64,
    pub mem_used_gb: f64,
    pub mem_total_gb: f64,
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
    pub accel: String, // 어떤 가속기/노드에서 도는지(파드 노드/가속기 추정)
    pub running: Option<f64>,
    pub waiting: Option<f64>,
    pub tps: Option<f64>,
    pub kv: Option<f64>,   // vllm:gpu_cache_usage_perc (0..1)
    pub ttft: Option<f64>, // TTFT p95 (s)
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
    pub routes: Vec<Route>,
    pub objectives: Vec<Objective>,
    pub decisions: Vec<(String, f64)>, // (pod, 라우팅 픽 횟수) — 트래픽이 EPP 경유 시
    pub autoscalers: Vec<Autoscale>,
    pub epp: Option<EppCfg>,
    pub epp_in_path: bool, // HTTPRoute backend 중 InferencePool 이 있는가(없으면 EPP 우회)
    pub prefix_idx: f64,   // inference_extension_prefix_indexer_size
    pub gw_addr: String,
    pub gw_ok: bool,
    pub warnings: Vec<String>,
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

fn to_gb(v: f64) -> f64 {
    if v > 1.0e9 {
        v / 1.0e9
    } else if v > 1.0e3 {
        v / 1.0e3
    } else {
        v
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

/// vLLM 모델 메트릭 묶음 (model_name 키).
struct Vllm {
    run: BTreeMap<String, Series>,
    wait: BTreeMap<String, Series>,
    tps: BTreeMap<String, Series>,
    kv: BTreeMap<String, Series>,
    ttft: BTreeMap<String, Series>,
}

/// 모델 deploy 가 어느 가속기/노드에서 도는지 추정(가속기 busy_model 라벨이 파드명 ⊇ deploy명).
fn accel_for(accels: &[Accel], deploy: &str) -> String {
    let mut kind = "";
    let mut node = "";
    let mut n = 0;
    for a in accels {
        if !a.busy_model.is_empty() && a.busy_model.starts_with(deploy) {
            kind = a.kind.label();
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

pub async fn collect(cfg: &Config) -> Snapshot {
    let mut snap = Snapshot::default();
    let mut warn = Vec::new();
    snap.ts = now_secs();

    // ---------- Accelerators ----------
    // Furiosa (key=uuid, util=core 평균)
    let f_util = q(cfg, "avg by (uuid,device,hostname) (furiosa_npu_core_utilization)", &mut warn).await;
    let f_temp = map_by(q(cfg, "max by (uuid) (furiosa_npu_hw_temperature)", &mut warn).await, "uuid");
    let f_pow = map_by(q(cfg, "max by (uuid) (furiosa_npu_hw_power)", &mut warn).await, "uuid");
    let f_du = map_by(q(cfg, "max by (uuid) (furiosa_npu_dram_usage)", &mut warn).await, "uuid");
    let f_dt = map_by(q(cfg, "max by (uuid) (furiosa_npu_dram_total)", &mut warn).await, "uuid");
    let f_alive = map_by(q(cfg, "max by (uuid) (furiosa_npu_alive)", &mut warn).await, "uuid");
    let f_thr = map_by(q(cfg, "max by (uuid) (furiosa_npu_throttling_events_count)", &mut warn).await, "uuid");
    for s in &f_util {
        let uuid = s.l("uuid");
        snap.accel.push(Accel {
            kind: AccelKind::Rngd,
            id: s.l("device").to_string(),
            node: s.l("hostname").to_string(),
            util: norm_pct(s.value),
            mem_used_gb: f_du.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            mem_total_gb: f_dt.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            temp: f_temp.get(uuid).map(|x| x.value).unwrap_or(0.0),
            power: f_pow.get(uuid).map(|x| x.value).unwrap_or(0.0),
            busy_model: String::new(),
            alive: f_alive.get(uuid).map(|x| x.value > 0.0).unwrap_or(true),
            throttle: f_thr.get(uuid).map(|x| x.value).unwrap_or(0.0),
        });
    }

    // RBLN (per-device series, key=uuid)
    let r_util = q(cfg, "RBLN_DEVICE_STATUS:UTILIZATION", &mut warn).await;
    let r_temp = map_by(q(cfg, "RBLN_DEVICE_STATUS:TEMPERATURE", &mut warn).await, "uuid");
    let r_pow = map_by(q(cfg, "RBLN_DEVICE_STATUS:CARD_POWER", &mut warn).await, "uuid");
    let r_du = map_by(q(cfg, "RBLN_DEVICE_STATUS:DRAM_USED", &mut warn).await, "uuid");
    let r_dt = map_by(q(cfg, "RBLN_DEVICE_STATUS:DRAM_TOTAL", &mut warn).await, "uuid");
    let r_health = map_by(q(cfg, "RBLN_DEVICE_STATUS:HEALTH", &mut warn).await, "uuid");
    for s in &r_util {
        let uuid = s.l("uuid");
        let pod = s.l("exported_pod");
        snap.accel.push(Accel {
            kind: AccelKind::Rbln,
            id: s.l("name").to_string(),
            node: s.l("node").to_string(),
            util: norm_pct(s.value),
            mem_used_gb: r_du.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            mem_total_gb: r_dt.get(uuid).map(|x| to_gb(x.value)).unwrap_or(0.0),
            temp: r_temp.get(uuid).map(|x| x.value).unwrap_or(0.0),
            power: r_pow.get(uuid).map(|x| x.value).unwrap_or(0.0),
            busy_model: pod.to_string(),
            alive: r_health.get(uuid).map(|x| x.value > 0.0).unwrap_or(true),
            throttle: 0.0,
        });
    }

    // NVIDIA DCGM (현재 보통 부재 — nvidia.com/gpu:0)
    let g_util = q(cfg, "DCGM_FI_DEV_GPU_UTIL", &mut warn).await;
    let g_mu = map_by(q(cfg, "DCGM_FI_DEV_FB_USED", &mut warn).await, "gpu");
    let g_temp = map_by(q(cfg, "DCGM_FI_DEV_GPU_TEMP", &mut warn).await, "gpu");
    let g_pow = map_by(q(cfg, "DCGM_FI_DEV_POWER_USAGE", &mut warn).await, "gpu");
    for s in &g_util {
        let gpu = s.l("gpu");
        snap.accel.push(Accel {
            kind: AccelKind::Gpu,
            id: format!("gpu{}", gpu),
            node: s.l("Hostname").to_string(),
            util: norm_pct(s.value),
            mem_used_gb: g_mu.get(gpu).map(|x| x.value / 1024.0).unwrap_or(0.0), // MiB→GB 근사
            mem_total_gb: 80.0,
            temp: g_temp.get(gpu).map(|x| x.value).unwrap_or(0.0),
            power: g_pow.get(gpu).map(|x| x.value).unwrap_or(0.0),
            busy_model: String::new(),
            alive: true,
            throttle: 0.0,
        });
    }
    // 정렬: kind, node, id
    snap.accel.sort_by(|a, b| {
        (a.kind as u8, &a.node, &a.id).cmp(&(b.kind as u8, &b.node, &b.id))
    });

    // ---------- Nodes ----------
    let node_ip = node_ip_map(cfg, &mut warn).await; // ip -> name
    let n_load = q(cfg, "node_load1", &mut warn).await;
    let n_mt = map_by(q(cfg, "node_memory_MemTotal_bytes", &mut warn).await, "instance");
    let n_ma = map_by(q(cfg, "node_memory_MemAvailable_bytes", &mut warn).await, "instance");
    for s in &n_load {
        let inst = s.l("instance");
        let ip = inst.split(':').next().unwrap_or(inst);
        let name = node_ip
            .get(ip)
            .cloned()
            .unwrap_or_else(|| ip.to_string());
        let mt = n_mt.get(inst).map(|x| x.value).unwrap_or(0.0);
        let ma = n_ma.get(inst).map(|x| x.value).unwrap_or(0.0);
        snap.nodes.push(NodeInfo {
            name,
            load1: s.value,
            mem_used_gb: to_gb(mt - ma),
            mem_total_gb: to_gb(mt),
        });
    }
    snap.nodes.sort_by(|a, b| a.name.cmp(&b.name));

    // ---------- EPP pools ----------
    let p_ready = q(cfg, "inference_pool_ready_pods", &mut warn).await;
    let p_q = map_by(q(cfg, "inference_pool_average_queue_size", &mut warn).await, "name");
    let p_kv = map_by(
        q(cfg, "inference_pool_average_kv_cache_utilization", &mut warn).await,
        "name",
    );
    let p_sat = map_by(
        q(cfg, "inference_extension_flow_control_pool_saturation", &mut warn).await,
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
        run: map_by(q(cfg, "vllm:num_requests_running", &mut warn).await, "model_name"),
        wait: map_by(q(cfg, "vllm:num_requests_waiting", &mut warn).await, "model_name"),
        tps: map_by(
            q(cfg, "sum by (model_name) (rate(vllm:generation_tokens_total[1m]))", &mut warn).await,
            "model_name",
        ),
        kv: map_by(q(cfg, "vllm:gpu_cache_usage_perc", &mut warn).await, "model_name"),
        ttft: map_by(
            q(
                cfg,
                "histogram_quantile(0.95, sum by (model_name,le) (rate(vllm:time_to_first_token_seconds_bucket[5m])))",
                &mut warn,
            )
            .await,
            "model_name",
        ),
    };

    // ---------- kube: deployments / pods / routes / gateway / epp ----------
    collect_kube(cfg, &mut snap, &vllm, &mut warn).await;

    snap.warnings = warn;
    snap
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

async fn node_ip_map(cfg: &Config, warn: &mut Vec<String>) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    match kube::get_json(&["get", "nodes", "-o", "json"]).await {
        Ok(v) => {
            if let Some(items) = v["items"].as_array() {
                for it in items {
                    let name = it["metadata"]["name"].as_str().unwrap_or("").to_string();
                    if let Some(addrs) = it["status"]["addresses"].as_array() {
                        for a in addrs {
                            if a["type"] == "InternalIP" {
                                if let Some(ip) = a["address"].as_str() {
                                    m.insert(ip.to_string(), name.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(e) => warn.push(format!("kube nodes: {}", e)),
    }
    let _ = cfg;
    m
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
                    snap.models.push(ModelRow {
                        route: route_for(&name),
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

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
