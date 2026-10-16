//! Apply full/fast collector results under the snapshot lock. No I/O in this module.
use crate::collect::{Accel, NodeInfo, Snapshot};
use crate::quality::{Quality, Sources};

#[derive(Default)]
pub struct FastSnapshot {
    pub accel: Vec<Accel>,
    pub nodes: Vec<NodeInfo>,
    pub sources: Sources,
}

impl Snapshot {
    pub fn source_updated(&self, previous: &Self, key: &str) -> bool {
        match self.sources.get(key) {
            Some(status) => status.newer_sample_than(previous.sources.get(key)),
            // Compatibility for deterministic fixtures and synthetic --cast snapshots.
            None => self.ts != previous.ts,
        }
    }
    pub fn source_report(&self, now: u64) -> String {
        let mut lines =
            vec!["Source                   State         Samples  Last success".to_string()];
        for (key, source) in &self.sources {
            let state =
                format!("{:?}", source.state(now, self.stale_after_secs.max(10))).to_lowercase();
            let age = source
                .last_success_at
                .map(|at| format!("{}s ago", now.saturating_sub(at)))
                .unwrap_or_else(|| "never".into());
            lines.push(format!(
                "{key:<24} {state:<13} {:>7}  {age}",
                source.samples
            ));
            if let Some(error) = &source.error {
                lines.push(format!("  {error}"));
            }
        }
        if self.sources.is_empty() {
            lines.push("No source observations yet.".into());
        }
        lines.push("\nFailed refreshes retain previous values and their original timestamps.\nEmpty means a successful query returned no samples; unsupported means the API is absent.\nThis report is a snapshot; reopen it to refresh.".into());
        lines.join("\n")
    }
    pub fn degraded_sources(&self, now: u64) -> Vec<(&str, &crate::quality::SourceStatus)> {
        self.sources
            .iter()
            .filter(|(_, s)| {
                matches!(
                    s.state(now, self.stale_after_secs.max(10)),
                    Quality::Stale | Quality::Error
                )
            })
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }
    pub fn merge_fast(&mut self, incoming: FastSnapshot) {
        for (key, status) in incoming.sources {
            let previous = self.sources.get(&key);
            let accept = previous.is_none_or(|p| {
                p.attempted_at <= status.attempted_at
                    && (status.error.is_none() || p.last_success_at.is_none())
            });
            if accept {
                if key == "nodes" {
                    let mut nodes = incoming.nodes.clone();
                    for node in &mut nodes {
                        if node.npu.is_empty() {
                            if let Some(old) = self.nodes.iter().find(|old| old.name == node.name) {
                                node.npu.clone_from(&old.npu);
                            }
                        }
                    }
                    self.nodes = nodes;
                } else if let Some(id) = key.strip_prefix("accel/") {
                    if let Some(pack) = crate::accel::by_id(id) {
                        self.accel.retain(|a| a.kind != pack.kind);
                        self.accel.extend(
                            incoming
                                .accel
                                .iter()
                                .filter(|a| a.kind == pack.kind)
                                .cloned(),
                        );
                    }
                }
            }
            self.sources.insert(key, status.after(previous));
        }
        self.accel
            .sort_by(|a, b| (a.kind.rank(), &a.node, &a.id).cmp(&(b.kind.rank(), &b.node, &b.id)));
        // Deliberately do not touch self.ts: this refresh says nothing about models/perf/routes.
    }
    pub fn merge_full(&mut self, mut incoming: Snapshot) {
        if incoming.ts < self.ts {
            return;
        }
        // Start with existing fast data, merge the full collector's fast observations using
        // their own timestamps. A slow full collection cannot roll back newer device samples.
        let fast = FastSnapshot {
            accel: std::mem::take(&mut incoming.accel),
            nodes: std::mem::take(&mut incoming.nodes),
            sources: incoming
                .sources
                .iter()
                .filter(|(k, _)| k.as_str() == "nodes" || k.starts_with("accel/"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        self.merge_fast(fast);
        incoming.accel.clone_from(&self.accel);
        incoming.nodes.clone_from(&self.nodes);
        for (key, status) in &self.sources {
            if key == "nodes" || key.starts_with("accel/") {
                incoming.sources.insert(key.clone(), status.clone());
            }
        }
        for key in [
            "workloads",
            "pods",
            "routes",
            "pools",
            "performance",
            "inventory",
            "events",
            "gateway",
            "jobs",
            "epp_config",
            "model_store",
            "epp_decisions",
            "epp_queues",
            "epp_scores",
        ] {
            let keep = incoming.sources.get(key).is_some_and(|s| s.error.is_some())
                && self
                    .sources
                    .get(key)
                    .is_some_and(|s| s.last_success_at.is_some());
            if keep {
                match key {
                    "workloads" => {
                        incoming.models.clone_from(&self.models);
                        incoming
                            .deployment_versions
                            .clone_from(&self.deployment_versions);
                        incoming.workloads.clone_from(&self.workloads);
                        incoming.artifacts.clone_from(&self.artifacts);
                    }
                    "inventory" => {
                        incoming.inventory.clone_from(&self.inventory);
                        incoming.node_alloc.clone_from(&self.node_alloc);
                        incoming.node_capacity.clone_from(&self.node_capacity);
                    }
                    "epp_decisions" => incoming.decisions.clone_from(&self.decisions),
                    "epp_queues" => incoming.pod_queues.clone_from(&self.pod_queues),
                    "epp_scores" => incoming.endpoint_scores.clone_from(&self.endpoint_scores),
                    "model_store" => {
                        incoming.stored.clone_from(&self.stored);
                        incoming.store_capacity = self.store_capacity;
                        incoming.store_scanned_at = self.store_scanned_at;
                    }
                    "epp_config" => incoming.epp.clone_from(&self.epp),
                    "jobs" => incoming.compiles.clone_from(&self.compiles),
                    "events" => incoming.events.clone_from(&self.events),
                    "gateway" => {
                        incoming.gw_addr.clone_from(&self.gw_addr);
                        incoming.gw_ok = self.gw_ok;
                    }
                    "pods" => incoming.pods.clone_from(&self.pods),
                    "routes" => {
                        incoming.routes.clone_from(&self.routes);
                        incoming.epp_in_path = self.epp_in_path;
                    }
                    "pools" => incoming.pools.clone_from(&self.pools),
                    "performance" => {
                        incoming.perf = self.perf.clone();
                        incoming.perf_rows.clone_from(&self.perf_rows);
                        incoming.perf_targets.clone_from(&self.perf_targets);
                    }
                    _ => unreachable!(),
                }
            }
            if let Some(status) = incoming.sources.get_mut(key) {
                *status = status.clone().after(self.sources.get(key));
            }
        }
        if let Some(status) = incoming.sources.get_mut("full") {
            *status = status.clone().after(self.sources.get("full"));
        }
        *self = incoming;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::SourceStatus;
    #[test]
    fn inventory_failure_retains_original_evidence_until_successful_empty() {
        let mut snapshot = Snapshot::default();
        snapshot
            .node_capacity
            .insert("n1".into(), [("nvidia.com/gpu".into(), 4)].into());
        snapshot
            .node_alloc
            .insert("n1".into(), [("nvidia.com/gpu".into(), 2)].into());
        snapshot
            .sources
            .insert("inventory".into(), SourceStatus::observed(100, 1, &[]));
        let mut failed = Snapshot {
            ts: 105,
            ..Default::default()
        };
        failed.sources.insert(
            "inventory".into(),
            SourceStatus::observed(105, 0, &["forbidden".into()]),
        );
        snapshot.merge_full(failed);
        assert_eq!(snapshot.node_capacity["n1"]["nvidia.com/gpu"], 4);
        assert_eq!(snapshot.node_alloc["n1"]["nvidia.com/gpu"], 2);
        assert_eq!(snapshot.sources["inventory"].last_success_at, Some(100));
        assert_eq!(snapshot.sources["inventory"].state(105, 30), Quality::Stale);
        let mut empty = Snapshot {
            ts: 106,
            ..Default::default()
        };
        empty
            .sources
            .insert("inventory".into(), SourceStatus::observed(106, 0, &[]));
        snapshot.merge_full(empty);
        assert!(snapshot.node_capacity.is_empty());
        assert!(snapshot.node_alloc.is_empty());
        assert_eq!(snapshot.sources["inventory"].state(106, 30), Quality::Empty);
    }

    fn nodes(at: u64, error: bool, names: &[&str]) -> FastSnapshot {
        FastSnapshot {
            nodes: names
                .iter()
                .map(|n| NodeInfo {
                    name: (*n).into(),
                    ..Default::default()
                })
                .collect(),
            sources: [(
                "nodes".into(),
                SourceStatus::observed(
                    at,
                    names.len(),
                    &if error {
                        vec!["offline".into()]
                    } else {
                        vec![]
                    },
                ),
            )]
            .into(),
            ..Default::default()
        }
    }
    #[test]
    fn fast_refresh_does_not_refresh_full_data_and_failure_retains_rows() {
        let mut snap = Snapshot {
            ts: 100,
            ..Default::default()
        };
        snap.merge_fast(nodes(101, false, &["node-a"]));
        snap.merge_fast(nodes(102, true, &[]));
        assert_eq!(snap.ts, 100);
        assert_eq!(snap.nodes[0].name, "node-a");
        assert_eq!(snap.sources["nodes"].last_success_at, Some(101));
        snap.merge_fast(nodes(103, false, &[]));
        assert!(
            snap.nodes.is_empty(),
            "successful empty response must clear deleted resources"
        );
    }
    #[test]
    fn slow_full_collection_never_rolls_back_newer_fast_data() {
        let mut current = Snapshot {
            ts: 100,
            ..Default::default()
        };
        current.merge_fast(nodes(110, false, &["new"]));
        let old = nodes(105, false, &["old"]);
        current.merge_full(Snapshot {
            ts: 111,
            nodes: old.nodes,
            sources: old.sources,
            ..Default::default()
        });
        assert_eq!(current.nodes[0].name, "new");
        assert_eq!(current.sources["nodes"].last_success_at, Some(110));
    }
    #[test]
    fn api_failure_preserves_workloads_without_claiming_new_sample() {
        let mut current = Snapshot {
            ts: 100,
            sources: [("workloads".into(), SourceStatus::observed(100, 1, &[]))].into(),
            ..Default::default()
        };
        current.artifacts.push(crate::collect::ModelArtifact {
            model: "model-a".into(),
            ..Default::default()
        });
        let next = Snapshot {
            ts: 110,
            sources: [(
                "workloads".into(),
                SourceStatus::observed(110, 0, &["Forbidden".into()]),
            )]
            .into(),
            ..Default::default()
        };
        current.merge_full(next);
        assert_eq!(current.artifacts[0].model, "model-a");
        assert_eq!(current.sources["workloads"].last_success_at, Some(100));
    }
}

#[cfg(test)]
mod additional_source_tests {
    use super::*;
    use crate::quality::SourceStatus;

    #[test]
    fn endpoint_source_failure_preserves_only_its_own_observation() {
        let mut current = Snapshot {
            decisions: vec![("p".into(), 4.0)],
            pod_queues: vec![("p".into(), 2.0)],
            endpoint_scores: vec![("p".into(), 0.5)],
            ..Default::default()
        };
        for key in ["epp_decisions", "epp_queues", "epp_scores"] {
            current
                .sources
                .insert(key.into(), SourceStatus::observed(100, 1, &[]));
        }
        let mut next = Snapshot::default();
        next.sources.insert(
            "epp_decisions".into(),
            SourceStatus::observed(101, 0, &["offline".into()]),
        );
        for key in ["epp_queues", "epp_scores"] {
            next.sources
                .insert(key.into(), SourceStatus::observed(101, 0, &[]));
        }
        current.merge_full(next);
        assert_eq!(current.decisions, vec![("p".into(), 4.0)]);
        assert_eq!(current.sources["epp_decisions"].last_success_at, Some(100));
        assert!(current.pod_queues.is_empty() && current.endpoint_scores.is_empty());
    }

    #[test]
    fn events_and_gateway_survive_failure_and_clear_on_valid_empty_recovery() {
        let mut current = Snapshot {
            store_scanned_at: Some(90),
            gw_addr: "observed-gateway".into(),
            gw_ok: true,
            ..Default::default()
        };
        current.stored.push(crate::collect::StoredModel {
            path: "compiled/model".into(),
            ..Default::default()
        });
        current.store_capacity = Some(crate::collect::StoreCapacity {
            total_bytes: 100,
            used_bytes: 40,
            avail_bytes: 60,
        });
        current.epp = Some(
            crate::epp_config::parse("schedulingProfiles: [{name: observed, plugins: []}]")
                .unwrap(),
        );
        current.events.push(crate::collect::EventRow {
            typ: "Warning".into(),
            reason: "FailedScheduling".into(),
            object: "Pod/test".into(),
            message: "Insufficient accelerator".into(),
            count: 1,
        });
        current.compiles.push(crate::collect::CompileJob {
            name: "compile-test".into(),
            status: "Running".into(),
            ..Default::default()
        });
        for key in ["events", "gateway", "jobs", "model_store", "epp_config"] {
            current
                .sources
                .insert(key.into(), SourceStatus::observed(100, 1, &[]));
        }
        let mut failed = Snapshot::default();
        for key in ["events", "gateway", "jobs", "model_store", "epp_config"] {
            failed.sources.insert(
                key.into(),
                SourceStatus::observed(101, 0, &["request failed".into()]),
            );
        }
        current.merge_full(failed);
        assert_eq!(current.stored[0].path, "compiled/model");
        assert_eq!(current.store_capacity.unwrap().avail_bytes, 60);
        assert_eq!(current.store_scanned_at, Some(90));
        assert_eq!(current.epp.as_ref().unwrap().profile, "observed");
        assert_eq!(current.events.len(), 1);
        assert_eq!(current.compiles[0].name, "compile-test");
        assert_eq!(current.gw_addr, "observed-gateway");
        assert!(current.gw_ok);
        for key in ["events", "gateway", "jobs", "model_store", "epp_config"] {
            assert_eq!(current.sources[key].last_success_at, Some(100));
            assert!(current.sources[key].error.is_some());
        }
        let mut recovered = Snapshot::default();
        for key in ["events", "gateway", "jobs", "model_store", "epp_config"] {
            recovered
                .sources
                .insert(key.into(), SourceStatus::observed(102, 0, &[]));
        }
        current.merge_full(recovered);
        assert!(
            current.stored.is_empty() && current.store_capacity.is_none() && current.epp.is_none()
        );
        assert!(current.store_scanned_at.is_none());
        assert!(current.compiles.is_empty());
        assert!(current.events.is_empty() && current.gw_addr.is_empty() && !current.gw_ok);
        for key in ["events", "gateway", "jobs", "model_store", "epp_config"] {
            assert_eq!(current.sources[key].last_success_at, Some(102));
            assert!(current.sources[key].error.is_none());
        }
    }
}
