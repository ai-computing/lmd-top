//! Agent-facing machine-readable state (`--snapshot --json`).
//! Exports a curated, stable schema so an AI agent can understand state and available actions without screen scraping.
//! Schema decoupled from the internal Snapshot → internal refactors don't break the contract. Managed via schema version.

use crate::app::{diagnose, snapshot_alerts, Action, Sev};
use crate::collect::Snapshot;
use crate::config::Config;
use serde::Serialize;

/// Normalize NaN → null (None) so the agent clearly distinguishes "no value".
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
    compiles: Vec<Compile>,
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
    unified_mem: bool, // true → mem is the host-shared unified pool (GB10, etc.)
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
    npu: Option<String>, // NPU driver/SDK summary (label-based) — identifies compile-capable nodes
}

/// Deploy compile variant (artifact) — model source, storage location, compile/serving options.
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

/// Shared store inventory (deployment-agnostic) — HF originals / NPU-compiled builds.
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

/// In-progress/recent compile Jobs (compile-*) — status, elapsed time, progress hints.
#[derive(Serialize)]
struct Compile {
    name: String,
    model: String,
    vendor: String,
    target: String,
    status: String,
    age_s: u64,
    duration_s: Option<u64>,
    phase: String,
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
    requires_confirmation: bool, // y/n confirmation in the UI
}

fn build(s: &Snapshot, cfg: &Config) -> AgentState {
    // Cluster summary (same aggregation as summary_bar).
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
    let serving = s.serving_count();
    let (disk_u, disk_t): (f64, f64) = s.nodes.iter().fold((0.0, 0.0), |(u, t), nd| {
        (u + nd.disk_used_gb, t + nd.disk_total_gb)
    });

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
            disk_used_gb: if nd.disk_total_gb > 0.0 {
                Some(nd.disk_used_gb)
            } else {
                None
            },
            disk_total_gb: if nd.disk_total_gb > 0.0 {
                Some(nd.disk_total_gb)
            } else {
                None
            },
            load1: opt(nd.load1),
            npu: if nd.npu.is_empty() {
                None
            } else {
                Some(nd.npu.clone())
            },
        })
        .collect();

    let artifacts = s
        .artifacts
        .iter()
        .map(|a| Artifact {
            model: a.model.clone(),
            family: a.family.clone(),
            engine: a.engine.clone(),
            node: if a.node.is_empty() {
                None
            } else {
                Some(a.node.clone())
            },
            source: if a.source.is_empty() {
                None
            } else {
                Some(a.source.clone())
            },
            storage: if a.mount.is_empty() {
                None
            } else {
                Some(a.mount.clone())
            },
            compile_opts: a.opts.iter().cloned().collect(),
        })
        .collect();

    let noneify = |v: &str| {
        if v.is_empty() || v == "-" {
            None
        } else {
            Some(v.to_string())
        }
    };
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

    let compiles = s
        .compiles
        .iter()
        .map(|c| Compile {
            name: c.name.clone(),
            model: c.model.clone(),
            vendor: c.vendor.clone(),
            target: c.target.clone(),
            status: c.status.clone(),
            age_s: c.age_secs,
            duration_s: c.duration_secs,
            phase: c.phase.clone(),
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
            busy_model: if a.busy_model.is_empty() {
                None
            } else {
                Some(a.busy_model.clone())
            },
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
            route: if m.route.is_empty() {
                None
            } else {
                Some(m.route.clone())
            },
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
            epp: if p.epp.is_empty() {
                None
            } else {
                Some(p.epp.clone())
            },
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
        .map(|a| Alrt {
            severity: sev_str(a.sev),
            key: a.key,
            message: a.msg,
        })
        .collect();

    let mut actions: Vec<Act> = Vec::new();
    let mut push_action = |id: String, label: String, action: Action, confirm: bool| {
        actions.push(Act {
            id,
            label,
            risk: action.risk_label(),
            requires_confirmation: confirm,
        });
    };
    for m in &s.models {
        push_action(
            format!("yaml:model:{}", m.name),
            format!("show live YAML for {}", m.name),
            Action::Yaml,
            false,
        );
        push_action(
            format!("logs:model:{}", m.name),
            format!("tail logs for {}", m.name),
            Action::Logs,
            false,
        );
        push_action(
            format!("restart:{}", m.name),
            format!("rollout restart {}", m.name),
            Action::Restart,
            true,
        );
        if m.desired > 0 {
            push_action(
                format!("stop:{}", m.name),
                format!("stop {} (scale to 0)", m.name),
                Action::Stop,
                true,
            );
        }
        {
            let target = if m.desired == 0 { 1 } else { 0 };
            push_action(
                format!("scale:{}:{}", m.name, target),
                format!("scale {} → {} replica(s)", m.name, target),
                Action::Scale,
                true,
            );
        }
    }
    for r in &s.routes {
        push_action(
            format!("route-rename:{}:{}", r.route, r.path),
            format!("rename route {} in {}", r.path, r.route),
            Action::RouteRename,
            true,
        );
        push_action(
            format!("route-retarget:{}:{}", r.route, r.path),
            format!("retarget route {} in {}", r.path, r.route),
            Action::RouteRetarget,
            true,
        );
        push_action(
            format!("route-delete:{}:{}", r.route, r.path),
            format!("delete route {} from {}", r.path, r.route),
            Action::RouteDelete,
            true,
        );
    }
    for c in &s.compiles {
        push_action(
            format!("logs:compile:{}", c.name),
            format!("tail logs for compile job {}", c.name),
            Action::Logs,
            false,
        );
        push_action(
            format!("delete-job:{}", c.name),
            format!("delete compile job {}", c.name),
            Action::DeleteJob,
            true,
        );
    }

    AgentState {
        schema: "lmd-top/agent-state/v2",
        ts: s.ts,
        namespace: cfg.ns.clone(),
        prometheus: cfg.prom.clone(),
        gateway: Gw {
            addr: s.gw_addr.clone(),
            ok: s.gw_ok,
        },
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
        compiles,
        pools,
        per_model_perf,
        diagnosis,
        alerts,
        actions,
    }
}

/// Print the pretty JSON state tree to stdout.
/// Agent state as a JSON string (a seam for tests/contract verification). emit_json prints this.
pub fn to_json(snap: &Snapshot, cfg: &Config) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&build(snap, cfg))
}

pub fn emit_json(snap: &Snapshot, cfg: &Config) {
    match to_json(snap, cfg) {
        Ok(s) => println!("{}", s),
        Err(e) => eprintln!("lmd-top: json serialize error: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_contract_schema_and_keys() {
        // Synthetic snapshot → agent JSON → verify the contract (schema version, core keys). Cluster-agnostic.
        let mut snap = Snapshot::default();
        snap.nodes.push(crate::collect::NodeInfo {
            name: "node-a".into(),
            load1: 0.5,
            mem_used_gb: 10.0,
            mem_total_gb: 100.0,
            cpu_pct: 20.0,
            disk_used_gb: 5.0,
            disk_total_gb: 50.0,
            ready: true,
            cordoned: false,
            pressure: false,
            version: "v1.30".into(),
            npu: "RNGD drv2026.3.0".into(),
        });
        let cfg = Config::default();
        let s = to_json(&snap, &cfg).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&s).expect("valid json");
        // Contract: schema version v2, top-level keys, node.npu field exposed.
        assert_eq!(v["schema"], "lmd-top/agent-state/v2");
        for k in ["cluster", "nodes", "artifacts", "stored", "models"] {
            assert!(v.get(k).is_some(), "top-level key '{}' present", k);
        }
        assert_eq!(v["nodes"][0]["name"], "node-a");
        assert_eq!(v["nodes"][0]["npu"], "RNGD drv2026.3.0");
    }
}
