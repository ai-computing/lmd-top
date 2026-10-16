//! Agent-facing machine-readable state (`--snapshot --json`).
//! Exports a curated, stable schema so an AI agent can understand state and available actions without screen scraping.
//! Schema decoupled from the internal Snapshot → internal refactors don't break the contract. Managed via schema version.

use crate::app::{diagnose, snapshot_alerts, Action, Sev};
use crate::collect::Snapshot;
use crate::config::Config;
use serde::Serialize;

/// Normalize NaN → null (None) so the agent clearly distinguishes "no value".
fn opt(v: f64) -> Option<f64> {
    if !v.is_finite() {
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
    sources: crate::quality::Sources,
    source_states: std::collections::BTreeMap<String, crate::quality::Quality>,
    warnings: Vec<String>,
    kubernetes_context: Option<String>,
    namespace: String,
    prometheus: String,
    gateway: Gw,
    epp_in_path: bool,
    route_configuration: crate::routing::Evidence,
    routes: Vec<Route>,
    epp_profiles: Vec<EppProfile>,
    cluster: Cluster,
    store_scanned_at: Option<u64>,
    resource_inventory: ResourceInventory,
    nodes: Vec<Node>,
    accelerators: Vec<Acc>,
    models: Vec<Mdl>,
    workloads: Vec<crate::workload::Identity>,
    pods: Vec<Pod>,
    artifacts: Vec<Artifact>,
    /// Shared model store free space, or null when the discovery scan does not report it.
    store: Option<StoreCap>,
    stored: Vec<Stored>,
    compiles: Vec<Compile>,
    /// LoRA training runs (npu-lora) with their latest step/loss/throughput.
    trainings: Vec<Train>,
    pools: Vec<Pl>,
    service_bindings: std::collections::BTreeMap<String, Vec<String>>,
    pool_bindings: std::collections::BTreeMap<String, Vec<String>>,
    per_model_perf: Vec<Pm>,
    diagnosis: Diag,
    alerts: Vec<Alrt>,
    actions: Vec<Act>,
}

#[derive(Serialize)]
struct Route {
    name: String,
    path: String,
    uid: Option<String>,
    resource_version: Option<String>,
    configuration: Option<crate::routing::Configuration>,
    verified_deployments: Vec<String>,
}

#[derive(Serialize)]
struct EppProfile {
    name: String,
    scorers: Vec<EppScorer>,
    configured_picker: Option<String>,
    unclassified_plugins: Vec<String>,
}
#[derive(Serialize)]
struct EppScorer {
    plugin_ref: String,
    weight: f64,
}

/// Raw Kubernetes evidence, separate from utilization and scheduling guarantees.
#[derive(Serialize)]
struct ResourceInventory {
    state: crate::quality::Quality,
    last_success_at: Option<u64>,
    allocatable_by_node:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, i64>>,
    requested_by_node: std::collections::BTreeMap<String, std::collections::BTreeMap<String, i64>>,
    totals: Vec<ResourceTotal>,
}

#[derive(Serialize)]
struct ResourceTotal {
    resource: String,
    allocatable: i64,
    requested: i64,
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
    busy: Option<usize>,
    util_avg_pct: Option<f64>,
    vram_used_gb: Option<f64>,
    vram_total_gb: Option<f64>,
    disk_used_gb: Option<f64>,
    disk_total_gb: Option<f64>,
    power_w: Option<f64>,
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
    service: Option<String>,
    metric_families: Vec<crate::collect::LatencyFamily>,
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
    /// Toolchain the artifact records having been built with, when the scan could read it.
    built_with: Option<String>,
}

/// Shared model store capacity as measured inside the discovery scan (`df`), not the PVC's
/// requested size — on an SMB backend those are unrelated.
#[derive(Serialize)]
struct StoreCap {
    total_bytes: u64,
    used_bytes: u64,
    avail_bytes: u64,
    used_pct: f64,
}

/// Serving pods in the namespace — the layer an agent needs to reason about a restart, a
/// pending scheduling failure, or a crash loop. The TUI has always had a Pods view; the agent
/// state did not, so anything automated was blind below the Deployment.
#[derive(Serialize)]
struct Pod {
    name: String,
    phase: String,
    /// Container readiness as the API reports it, e.g. "1/1".
    ready: String,
    node: String,
    restarts: i64,
    /// Seconds since creation, or null when the timestamp could not be parsed.
    age_secs: Option<u64>,
}

/// LoRA training runs (npu-lora Jobs) joined with their npu_lora_* metrics.
#[derive(Serialize)]
struct Train {
    job: String,
    run: String,
    model: String,
    status: String,
    age_s: u64,
    step: Option<f64>,
    steps_planned: Option<f64>,
    progress: Option<f64>,
    loss: Option<f64>,
    tokens_per_s: Option<f64>,
    out: String,
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
    // Empty or incomplete populations do not establish measured totals.
    let sum = crate::measurement::complete_sum;
    let util_sum = sum(s.accel.iter().map(|a| opt(a.util)));
    let busy = crate::measurement::busy_count(s.accel.iter().map(|a| a.util));
    let serving = s.serving_count();

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
            built_with: noneify(&m.built_with),
        })
        .collect();

    let store = s.store_capacity.map(|c| StoreCap {
        total_bytes: c.total_bytes,
        used_bytes: c.used_bytes,
        avail_bytes: c.avail_bytes,
        used_pct: (c.used_pct() * 10.0).round() / 10.0,
    });

    let pods = s
        .pods
        .iter()
        .map(|p| Pod {
            name: p.name.clone(),
            phase: p.phase.clone(),
            ready: p.ready.clone(),
            node: p.node.clone(),
            restarts: p.restarts,
            // 0 is the collector's "unknown", and an agent must not read that as "just created".
            age_secs: (p.age_secs > 0).then_some(p.age_secs),
        })
        .collect();

    let trainings = s
        .trainings
        .iter()
        .map(|t| Train {
            job: t.job.clone(),
            run: t.run.clone(),
            model: t.model.clone(),
            status: t.status.clone(),
            age_s: t.age_secs,
            step: t.step,
            steps_planned: t.steps_planned,
            progress: t.progress(),
            loss: t.loss,
            tokens_per_s: t.tokens_per_s,
            out: t.out.clone(),
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
            service: s.perf_targets.get(&r.model).map(|t| t.service.clone()),
            metric_families: s
                .perf_targets
                .get(&r.model)
                .map(|t| t.families.iter().copied().collect())
                .unwrap_or_default(),
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
        push_action(
            format!("rollback:{}", m.name),
            format!("rollout undo {} (previous revision)", m.name),
            Action::Rollback,
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
        sources: s.sources.clone(),
        source_states: s
            .sources
            .iter()
            .map(|(key, status)| {
                (
                    key.clone(),
                    status.state(crate::collect::now_secs(), s.stale_after_secs.max(10)),
                )
            })
            .collect(),
        warnings: s.warnings.clone(),
        kubernetes_context: crate::kube_session::context().map(str::to_owned),
        store_scanned_at: s.store_scanned_at,
        resource_inventory: ResourceInventory {
            state: s
                .sources
                .get("inventory")
                .map(|status| status.state(crate::collect::now_secs(), s.stale_after_secs.max(10)))
                .unwrap_or(crate::quality::Quality::Unknown),
            last_success_at: s
                .sources
                .get("inventory")
                .and_then(|status| status.last_success_at),
            allocatable_by_node: s.node_capacity.clone(),
            requested_by_node: s.node_alloc.clone(),
            totals: s
                .inventory
                .iter()
                .map(|(resource, allocatable, requested)| ResourceTotal {
                    resource: resource.clone(),
                    allocatable: *allocatable,
                    requested: *requested,
                })
                .collect(),
        },
        namespace: cfg.ns.clone(),
        prometheus: cfg.prom.clone(),
        gateway: Gw {
            addr: s.gw_addr.clone(),
            ok: s.gw_ok,
        },
        epp_in_path: s.epp_in_path,
        route_configuration: crate::routing::evidence(s),
        routes: s
            .routes
            .iter()
            .map(|route| Route {
                name: route.route.clone(),
                path: route.path.clone(),
                uid: route.identity.as_ref().map(|id| id.uid.clone()),
                resource_version: route.identity.as_ref().map(|id| id.version.clone()),
                configuration: route.configuration.clone(),
                verified_deployments: crate::routing::owners(s, route)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            })
            .collect(),
        epp_profiles: s
            .epp
            .as_ref()
            .map(|config| {
                config
                    .entries
                    .iter()
                    .map(|profile| EppProfile {
                        name: profile.profile.clone(),
                        scorers: profile
                            .scorers
                            .iter()
                            .map(|(name, weight)| EppScorer {
                                plugin_ref: name.clone(),
                                weight: *weight,
                            })
                            .collect(),
                        configured_picker: (!profile.picker_refs.is_empty())
                            .then(|| profile.picker_refs.join(", ")),
                        unclassified_plugins: profile.other_plugins.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        cluster: Cluster {
            nodes: s.nodes.len(),
            accelerators: s.accel.len(),
            busy,
            util_avg_pct: opt(util_sum / s.accel.len() as f64),
            vram_used_gb: opt(crate::measurement::complete_sum(
                s.accel.iter().map(|a| opt(a.mem_used_gb)),
            )),
            vram_total_gb: opt(crate::measurement::complete_sum(
                s.accel.iter().map(|a| opt(a.mem_total_gb)),
            )),
            disk_used_gb: opt(crate::measurement::complete_sum(
                s.nodes.iter().map(|n| opt(n.disk_used_gb)),
            )),
            disk_total_gb: opt(crate::measurement::complete_sum(
                s.nodes.iter().map(|n| opt(n.disk_total_gb)),
            )),
            power_w: opt(crate::measurement::complete_sum(
                s.accel.iter().map(|a| opt(a.power)),
            )),
            models_serving: serving,
            models_total: s.models.len(),
            req_per_s: opt(s.perf.req_rate),
            ttft_p95_s: opt(s.perf.ttft_p95),
        },
        nodes,
        accelerators,
        models,
        workloads: s.workloads.clone(),
        artifacts,
        pods,
        store,
        stored,
        compiles,
        trainings,
        pools,
        service_bindings: s.service_bindings.clone(),
        pool_bindings: s.pool_bindings.clone(),
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
    fn cluster_totals_distinguish_empty_partial_and_measured_zero() {
        let mut snap = Snapshot::default();
        let cluster = |s: &Snapshot| -> serde_json::Value {
            serde_json::from_str::<serde_json::Value>(&to_json(s, &Config::default()).unwrap())
                .unwrap()["cluster"]
                .clone()
        };
        let empty = cluster(&snap);
        for field in [
            "busy",
            "util_avg_pct",
            "vram_used_gb",
            "vram_total_gb",
            "power_w",
            "disk_used_gb",
            "disk_total_gb",
        ] {
            assert!(empty[field].is_null(), "{field}");
        }
        snap.accel.push(crate::collect::Accel {
            kind: crate::collect::AccelKind::Gpu,
            model: "gpu".into(),
            id: "0".into(),
            node: "node".into(),
            util: 0.0,
            mem_used_gb: 0.0,
            mem_total_gb: 80.0,
            temp: 30.0,
            power: 0.0,
            busy_model: String::new(),
            alive: true,
            throttle: f64::NAN,
            unified_mem: false,
            mem_bw: f64::NAN,
            clock_mhz: f64::NAN,
            mem_temp: f64::NAN,
            energy_mj: f64::NAN,
        });
        assert_eq!(cluster(&snap)["power_w"], 0.0);
        assert_eq!(cluster(&snap)["busy"], 0);
        let mut missing = snap.accel[0].clone();
        missing.util = f64::NAN;
        missing.power = f64::NAN;
        missing.mem_used_gb = f64::NAN;
        snap.accel.push(missing);
        let partial = cluster(&snap);
        for field in ["busy", "util_avg_pct", "power_w", "vram_used_gb"] {
            assert!(partial[field].is_null());
        }
        assert_eq!(partial["vram_total_gb"], 160.0);
    }

    #[test]
    fn routes_export_structured_matches_and_all_backends_without_runtime_claims() {
        let object = serde_json::json!({"items":[{"metadata":{"name":"route","namespace":"ns","uid":"uid","resourceVersion":"9"},
        "spec":{"rules":[{"matches":[{"method":"POST"}],"backendRefs":[
            {"name":"primary","weight":90}, {"name":"canary","namespace":"other","weight":10}
        ]}]}}]});
        let mut snap = Snapshot::default();
        (snap.routes, snap.epp_in_path) = crate::routing::rows(&object);
        let value: serde_json::Value =
            serde_json::from_str(&to_json(&snap, &Config::default()).unwrap()).unwrap();
        let route = &value["routes"][0];
        assert_eq!(route["uid"], "uid");
        assert_eq!(route["resource_version"], "9");
        assert_eq!(route["path"], "/");
        assert_eq!(route["configuration"]["rule_index"], 0);
        assert_eq!(route["configuration"]["implicit_path"], true);
        assert_eq!(route["configuration"]["match_conditions"]["method"], "POST");
        assert_eq!(
            route["configuration"]["backend_refs"],
            object["items"][0]["spec"]["rules"][0]["backendRefs"]
        );
        assert_eq!(route["verified_deployments"], serde_json::json!([]));
        assert_eq!(value["route_configuration"]["state"], "unknown");
    }

    #[test]
    fn epp_json_exports_all_profiles_without_inventing_default_picker() {
        let snap = Snapshot {
            epp: crate::epp_config::parse("schedulingProfiles:\n- name: one\n  plugins: [{pluginRef: queue-scorer, weight: 2}]\n- name: two\n  plugins: [{pluginRef: max-score-picker}, {pluginRef: custom-filter}]\n"),
            ..Default::default()
        };
        let value: serde_json::Value =
            serde_json::from_str(&to_json(&snap, &Config::default()).unwrap()).unwrap();
        let profiles = value["epp_profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0]["scorers"][0]["weight"], 2.0);
        assert!(profiles[0]["configured_picker"].is_null());
        assert_eq!(profiles[1]["configured_picker"], "max-score-picker");
        assert!(profiles[1]["scorers"].as_array().unwrap().is_empty());
        assert_eq!(
            profiles[1]["unclassified_plugins"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn inventory_json_preserves_zero_and_marks_retained_data_stale() {
        let mut snap = Snapshot::default();
        snap.node_capacity
            .insert("n1".into(), [("nvidia.com/gpu".into(), 4)].into());
        snap.node_alloc
            .insert("n1".into(), [("nvidia.com/gpu".into(), 0)].into());
        snap.inventory.push(("nvidia.com/gpu".into(), 4, 0));
        let status = crate::quality::SourceStatus::observed(100, 1, &[]);
        snap.sources.insert("inventory".into(), status);
        let value: serde_json::Value =
            serde_json::from_str(&to_json(&snap, &Config::default()).unwrap()).unwrap();
        let inv = &value["resource_inventory"];
        assert_eq!(inv["state"], "stale");
        assert_eq!(inv["last_success_at"], 100);
        assert_eq!(inv["allocatable_by_node"]["n1"]["nvidia.com/gpu"], 4);
        assert_eq!(inv["requested_by_node"]["n1"]["nvidia.com/gpu"], 0);
        assert_eq!(inv["totals"][0]["requested"], 0);
        let empty: serde_json::Value =
            serde_json::from_str(&to_json(&Snapshot::default(), &Config::default()).unwrap())
                .unwrap();
        assert_eq!(empty["resource_inventory"]["state"], "unknown");
        assert!(empty["resource_inventory"]["last_success_at"].is_null());
        assert!(empty["resource_inventory"]["requested_by_node"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn collection_failure_is_visible_without_claiming_freshness() {
        let good = crate::quality::SourceStatus::observed(100, 2, &[]);
        let failed = crate::quality::SourceStatus::observed(110, 0, &["Forbidden".into()])
            .after(Some(&good));
        let snap = Snapshot {
            ts: 110,
            sources: [("workloads".into(), failed)].into(),
            ..Default::default()
        };
        let value: serde_json::Value =
            serde_json::from_str(&to_json(&snap, &Config::default()).unwrap()).unwrap();
        assert_eq!(value["sources"]["workloads"]["last_success_at"], 100);
        assert_eq!(value["source_states"]["workloads"], "stale");
        assert!(value["diagnosis"]
            .to_string()
            .contains("collection degraded"));
        assert_eq!(opt(f64::INFINITY), None);
        assert_eq!(opt(f64::NAN), None);
    }

    #[test]
    fn json_contract_schema_and_keys() {
        // Synthetic snapshot → agent JSON → verify the contract (schema version, core keys). Cluster-agnostic.
        let mut snap = Snapshot::default();
        snap.nodes.push(crate::collect::NodeInfo {
            identity: None,
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
            npu: "Furiosa RNGD drv2026.3.0".into(),
        });
        let cfg = Config::default();
        let s = to_json(&snap, &cfg).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&s).expect("valid json");
        // Contract: schema version v2, top-level keys, node.npu field exposed.
        assert_eq!(v["schema"], "lmd-top/agent-state/v2");
        assert_eq!(v["route_configuration"]["state"], "unknown");
        assert!(v["route_configuration"]["inference_pool_backend_observed"].is_null());
        assert_eq!(v["route_configuration"]["runtime_verified"], false);
        for k in ["cluster", "nodes", "artifacts", "stored", "models", "pods"] {
            assert!(v.get(k).is_some(), "top-level key '{}' present", k);
        }
        assert_eq!(v["nodes"][0]["name"], "node-a");
        assert_eq!(v["nodes"][0]["npu"], "Furiosa RNGD drv2026.3.0");
    }

    /// Pods reach the agent with the fields an automation needs to act on them, and an
    /// unknown age is null rather than a zero that reads as "just started".
    #[test]
    fn pods_are_exported_with_unknown_age_as_null() {
        use crate::collect::PodRow;
        let mut snap = Snapshot::default();
        snap.pods.push(PodRow {
            identity: None,
            name: "vllm-rbln-koni-abc123".into(),
            phase: "Running".into(),
            ready: "1/1".into(),
            node: "npu-node-1".into(),
            restarts: 2,
            age_secs: 3600,
        });
        snap.pods.push(PodRow {
            identity: None,
            name: "no-timestamp".into(),
            phase: "Pending".into(),
            ready: "0/1".into(),
            node: "-".into(),
            restarts: 0,
            age_secs: 0, // the collector's "unknown"
        });
        let s = to_json(&snap, &Config::default()).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&s).expect("valid json");
        assert_eq!(v["pods"].as_array().map(|a| a.len()), Some(2));
        assert_eq!(v["pods"][0]["name"], "vllm-rbln-koni-abc123");
        assert_eq!(v["pods"][0]["phase"], "Running");
        assert_eq!(v["pods"][0]["ready"], "1/1");
        assert_eq!(v["pods"][0]["node"], "npu-node-1");
        assert_eq!(v["pods"][0]["restarts"], 2);
        assert_eq!(v["pods"][0]["age_secs"], 3600);
        assert!(
            v["pods"][1]["age_secs"].is_null(),
            "an unparsed timestamp must be null, not 0"
        );
    }
}
