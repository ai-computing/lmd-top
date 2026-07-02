//! Agent-facing machine-readable state (`--snapshot --json`).
//! 화면 파싱 없이 AI agent 가 상태·가능 액션을 이해하도록 큐레이트된 안정 스키마를 내보낸다.
//! 내부 Snapshot 과 분리된 스키마 → 내부 리팩터가 계약을 깨지 않음. schema 버전으로 관리.

use crate::app::{diagnose, snapshot_alerts, Sev};
use crate::collect::Snapshot;
use crate::config::Config;
use serde::Serialize;

/// NaN → null(None) 로 정규화(agent 가 "값 없음"을 명확히 구분).
fn opt(v: f64) -> Option<f64> {
    if v.is_nan() {
        None
    } else {
        Some(v)
    }
}

fn sev_str(s: Sev) -> &'static str {
    match s {
        Sev::Bad => "bad",
        Sev::Warn => "warn",
    }
}

#[derive(Serialize)]
struct AgentState {
    schema: &'static str,
    ts: u64,
    namespace: String,
    prometheus: String,
    gateway: Gw,
    epp_in_path: bool,
    cluster: Cluster,
    nodes: Vec<Node>,
    accelerators: Vec<Acc>,
    models: Vec<Mdl>,
    artifacts: Vec<Artifact>,
    stored: Vec<Stored>,
    pools: Vec<Pl>,
    per_model_perf: Vec<Pm>,
    diagnosis: Diag,
    alerts: Vec<Alrt>,
    actions: Vec<Act>,
}

#[derive(Serialize)]
struct Gw {
    addr: String,
    ok: bool,
}

#[derive(Serialize)]
struct Cluster {
    nodes: usize,
    accelerators: usize,
    busy: usize,
    util_avg_pct: f64,
    vram_used_gb: f64,
    vram_total_gb: f64,
    disk_used_gb: f64,
    disk_total_gb: f64,
    power_w: f64,
    models_serving: usize,
    models_total: usize,
    req_per_s: Option<f64>,
    ttft_p95_s: Option<f64>,
}

#[derive(Serialize)]
struct Acc {
    kind: String,  // vendor family: GPU / RBLN / RNGD
    model: String, // detected model: GB10 / A100 / …
    id: String,
    node: String,
    util_pct: f64,
    mem_used_gb: f64,
    mem_total_gb: f64,
    unified_mem: bool, // true → mem is the host-shared unified pool (GB10 등)
    mem_bw_pct: Option<f64>, // DCGM MEM_COPY_UTIL — memory bandwidth pressure
    clock_mhz: Option<f64>,
    mem_temp_c: Option<f64>,
    temp_c: f64,
    power_w: f64,
    alive: bool,
    throttling: bool,
    busy_model: Option<String>,
}

#[derive(Serialize)]
struct Mdl {
    name: String,
    engine: String,
    accel: String,
    ready: i64,
    desired: i64,
    running: Option<f64>,
    waiting: Option<f64>,
    kv_pct: Option<f64>,
    tps: Option<f64>,
    ttft_p95_s: Option<f64>,
    route: Option<String>,
    status: String,
}

#[derive(Serialize)]
struct Pl {
    name: String,
    ep_ready: i64,
    ep_total: i64,
    queue: Option<f64>,
    kv: Option<f64>,
    saturation: Option<f64>,
    epp: Option<String>,
}

#[derive(Serialize)]
struct Pm {
    model: String,
    req_per_s: Option<f64>,
    tps: Option<f64>,
    ttft_p95_s: Option<f64>,
    queue_p95_s: Option<f64>,
    prefill_p95_s: Option<f64>,
    decode_p95_s: Option<f64>,
    tpot_p95_s: Option<f64>,
    e2e_p95_s: Option<f64>,
    preempt_per_s: Option<f64>,
}

#[derive(Serialize)]
struct Node {
    name: String,
    ready: bool,
    cordoned: bool,
    cpu_pct: Option<f64>,
    mem_used_gb: f64,
    mem_total_gb: f64,
    disk_used_gb: Option<f64>,
    disk_total_gb: Option<f64>,
    load1: Option<f64>,
}

/// Deploy 컴파일 변형(아티팩트) — 모델 소스·저장 위치·컴파일/서빙 옵션.
#[derive(Serialize)]
struct Artifact {
    model: String,
    family: String,
    engine: String,
    node: Option<String>,
    source: Option<String>,
    storage: Option<String>,
    compile_opts: std::collections::BTreeMap<String, String>,
}

/// 공유 스토어 인벤토리(배포 무관) — HF 원본/NPU 컴파일본.
#[derive(Serialize)]
struct Stored {
    repo: String,
    family: String,
    format: String,
    compiled_for: Option<String>,
    revision: Option<String>,
    size: String,
    path: String,
}

#[derive(Serialize)]
struct Diag {
    message: String,
    severity: &'static str, // ok / warn / bad
}

#[derive(Serialize)]
struct Alrt {
    severity: &'static str,
    key: String,
    message: String,
}

#[derive(Serialize)]
struct Act {
    id: String,
    label: String,
    risk: &'static str,          // permission mode required
    requires_confirmation: bool, // UI 에서 y/n 확인
}

fn build(s: &Snapshot, cfg: &Config) -> AgentState {
    // 클러스터 요약(summary_bar 와 동일 집계).
    let (mut busy, mut util_sum, mut mu, mut mt, mut pw) = (0usize, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for a in &s.accel {
        if a.util > 5.0 {
            busy += 1;
        }
        util_sum += a.util;
        mu += a.mem_used_gb;
        mt += a.mem_total_gb;
        pw += a.power;
    }
    let n = s.accel.len().max(1);
    let serving = s.models.iter().filter(|m| m.ready > 0).count();
    let (disk_u, disk_t): (f64, f64) = s.nodes.iter().fold((0.0, 0.0), |(u, t), nd| (u + nd.disk_used_gb, t + nd.disk_total_gb));

    let nodes = s
        .nodes
        .iter()
        .map(|nd| Node {
            name: nd.name.clone(),
            ready: nd.ready,
            cordoned: nd.cordoned,
            cpu_pct: opt(nd.cpu_pct),
            mem_used_gb: nd.mem_used_gb,
            mem_total_gb: nd.mem_total_gb,
            disk_used_gb: if nd.disk_total_gb > 0.0 { Some(nd.disk_used_gb) } else { None },
            disk_total_gb: if nd.disk_total_gb > 0.0 { Some(nd.disk_total_gb) } else { None },
            load1: opt(nd.load1),
        })
        .collect();

    let artifacts = s
        .artifacts
        .iter()
        .map(|a| Artifact {
            model: a.model.clone(),
            family: a.family.clone(),
            engine: a.engine.clone(),
            node: if a.node.is_empty() { None } else { Some(a.node.clone()) },
            source: if a.source.is_empty() { None } else { Some(a.source.clone()) },
            storage: if a.mount.is_empty() { None } else { Some(a.mount.clone()) },
            compile_opts: a.opts.iter().cloned().collect(),
        })
        .collect();

    let noneify = |v: &str| if v.is_empty() || v == "-" { None } else { Some(v.to_string()) };
    let stored = s
        .stored
        .iter()
        .map(|m| Stored {
            repo: m.repo.clone(),
            family: m.family.clone(),
            format: m.format.clone(),
            compiled_for: noneify(&m.compiled_for),
            revision: noneify(&m.revision),
            size: m.size.clone(),
            path: m.path.clone(),
        })
        .collect();

    let accelerators = s
        .accel
        .iter()
        .map(|a| Acc {
            kind: a.kind.label().to_string(),
            model: a.disp().to_string(),
            id: a.id.clone(),
            node: a.node.clone(),
            util_pct: a.util,
            mem_used_gb: a.mem_used_gb,
            mem_total_gb: a.mem_total_gb,
            unified_mem: a.unified_mem,
            mem_bw_pct: opt(a.mem_bw),
            clock_mhz: opt(a.clock_mhz),
            mem_temp_c: opt(a.mem_temp),
            temp_c: a.temp,
            power_w: a.power,
            alive: a.alive,
            throttling: a.throttle > 0.0,
            busy_model: if a.busy_model.is_empty() { None } else { Some(a.busy_model.clone()) },
        })
        .collect();

    let models = s
        .models
        .iter()
        .map(|m| Mdl {
            name: m.name.clone(),
            engine: m.engine.clone(),
            accel: m.accel.clone(),
            ready: m.ready,
            desired: m.desired,
            running: m.running,
            waiting: m.waiting,
            kv_pct: m.kv.map(|x| x * 100.0),
            tps: m.tps,
            ttft_p95_s: m.ttft,
            route: if m.route.is_empty() { None } else { Some(m.route.clone()) },
            status: m.status.clone(),
        })
        .collect();

    let pools = s
        .pools
        .iter()
        .map(|p| Pl {
            name: p.name.clone(),
            ep_ready: p.ep_ready,
            ep_total: p.ep_total,
            queue: opt(p.queue),
            kv: opt(p.kv),
            saturation: opt(p.sat),
            epp: if p.epp.is_empty() { None } else { Some(p.epp.clone()) },
        })
        .collect();

    let per_model_perf = s
        .perf_rows
        .iter()
        .map(|r| Pm {
            model: r.model.clone(),
            req_per_s: opt(r.req),
            tps: opt(r.tps),
            ttft_p95_s: opt(r.ttft_p95),
            queue_p95_s: opt(r.queue_p95),
            prefill_p95_s: opt(r.prefill_p95),
            decode_p95_s: opt(r.decode_p95),
            tpot_p95_s: opt(r.tpot_p95),
            e2e_p95_s: opt(r.e2e_p95),
            preempt_per_s: opt(r.preempt),
        })
        .collect();

    let (dmsg, dsev) = diagnose(s);
    let diagnosis = Diag {
        message: dmsg,
        severity: match dsev {
            None => "ok",
            Some(Sev::Warn) => "warn",
            Some(Sev::Bad) => "bad",
        },
    };

    let alerts = snapshot_alerts(s)
        .into_iter()
        .map(|a| Alrt { severity: sev_str(a.sev), key: a.key, message: a.msg })
        .collect();

    // 가능한 액션: 모델별 scale 토글(권한 admin, 확인 필요) — UI 의 `s` 와 동일.
    let actions = s
        .models
        .iter()
        .map(|m| {
            let target = if m.desired == 0 { 1 } else { 0 };
            Act {
                id: format!("scale:{}:{}", m.name, target),
                label: format!("scale {} → {} replica(s)", m.name, target),
                risk: "admin",
                requires_confirmation: true,
            }
        })
        .collect();

    AgentState {
        schema: "lmd-top/agent-state/v2",
        ts: s.ts,
        namespace: cfg.ns.clone(),
        prometheus: cfg.prom.clone(),
        gateway: Gw { addr: s.gw_addr.clone(), ok: s.gw_ok },
        epp_in_path: s.epp_in_path,
        cluster: Cluster {
            nodes: s.nodes.len(),
            accelerators: s.accel.len(),
            busy,
            util_avg_pct: util_sum / n as f64,
            vram_used_gb: mu,
            vram_total_gb: mt,
            disk_used_gb: disk_u,
            disk_total_gb: disk_t,
            power_w: pw,
            models_serving: serving,
            models_total: s.models.len(),
            req_per_s: opt(s.perf.req_rate),
            ttft_p95_s: opt(s.perf.ttft_p95),
        },
        nodes,
        accelerators,
        models,
        artifacts,
        stored,
        pools,
        per_model_perf,
        diagnosis,
        alerts,
        actions,
    }
}

/// stdout 으로 pretty JSON 상태 트리 출력.
pub fn emit_json(snap: &Snapshot, cfg: &Config) {
    match serde_json::to_string_pretty(&build(snap, cfg)) {
        Ok(s) => println!("{}", s),
        Err(e) => eprintln!("lmd-top: json serialize error: {}", e),
    }
}
