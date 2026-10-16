//! Pure Kubernetes accelerator capacity and effective reservation accounting.
//! Collection, timestamps and UI policy remain with their callers.
use std::collections::BTreeMap;

const ACCEL_RESOURCES: [&str; 3] = ["nvidia.com/gpu", "rebellions.ai/ATOM", "furiosa.ai/rngd"];

pub type InventoryData = (
    Vec<(String, i64, i64)>,
    BTreeMap<String, BTreeMap<String, i64>>,
    BTreeMap<String, BTreeMap<String, i64>>,
);

/// Commit reservation evidence only after both complete lists have been validated.
pub fn from_json(
    nodes: &serde_json::Value,
    pods: &serde_json::Value,
) -> anyhow::Result<InventoryData> {
    let nodes = nodes["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("node list has no items array"))?;
    let pods = pods["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("pod list has no items array"))?;
    let mut total = BTreeMap::new();
    let mut used = BTreeMap::new();
    let mut allocations = BTreeMap::new();
    let mut capacity = BTreeMap::new();
    for resource in ACCEL_RESOURCES {
        total.insert(resource, 0_i64);
        used.insert(resource, 0_i64);
    }
    for node in nodes {
        let name = node["metadata"]["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("node missing name"))?;
        let allocatable = node["status"]["allocatable"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("node {name} missing allocatable"))?;
        let mut zeros = BTreeMap::new();
        let mut counts = BTreeMap::new();
        for resource in ACCEL_RESOURCES {
            let count = allocatable
                .get(resource)
                .map(accelerator_quantity)
                .transpose()?
                .unwrap_or(0);
            let sum = total.get_mut(resource).unwrap();
            *sum = sum
                .checked_add(count)
                .ok_or_else(|| anyhow::anyhow!("allocatable overflow"))?;
            zeros.insert(resource.to_string(), 0_i64);
            counts.insert(resource.to_string(), count);
        }
        allocations.insert(name.to_string(), zeros);
        capacity.insert(name.to_string(), counts);
    }
    for pod in pods {
        if matches!(
            pod["status"]["phase"].as_str(),
            Some("Succeeded" | "Failed")
        ) {
            continue;
        }
        let node = pod["spec"]["nodeName"].as_str().unwrap_or("");
        for resource in ACCEL_RESOURCES {
            let count = pod_accelerator_request(pod, resource)?;
            let sum = used.get_mut(resource).unwrap();
            *sum = sum
                .checked_add(count)
                .ok_or_else(|| anyhow::anyhow!("request overflow"))?;
            if !node.is_empty() {
                let per_node = allocations
                    .get_mut(node)
                    .ok_or_else(|| anyhow::anyhow!("pod references unobserved node {node}"))?;
                let sum = per_node.get_mut(resource).unwrap();
                *sum = sum
                    .checked_add(count)
                    .ok_or_else(|| anyhow::anyhow!("node request overflow"))?;
            }
        }
    }

    Ok((
        ACCEL_RESOURCES
            .iter()
            .map(|r| (r.to_string(), total[r], used[r]))
            .collect(),
        allocations,
        capacity,
    ))
}

fn accelerator_quantity(value: &serde_json::Value) -> anyhow::Result<i64> {
    value
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| value.as_i64())
        .filter(|n| *n >= 0)
        .ok_or_else(|| anyhow::anyhow!("invalid accelerator quantity: {value}"))
}

/// Peak concurrent request: normal containers plus persistent init sidecars, or
/// any initialization stage plus the sidecars already started, then pod overhead.
fn pod_accelerator_request(pod: &serde_json::Value, resource: &str) -> anyhow::Result<i64> {
    let add = |a: i64, b: i64| {
        a.checked_add(b)
            .ok_or_else(|| anyhow::anyhow!("pod request overflow"))
    };
    let request = |container: &serde_json::Value| {
        container["resources"]["requests"]
            .get(resource)
            .or_else(|| container["resources"]["limits"].get(resource))
            .map(accelerator_quantity)
            .transpose()
            .map(|q| q.unwrap_or(0))
    };
    let containers = pod["spec"]["containers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("pod missing containers"))?;
    let mut app = 0_i64;
    for container in containers {
        app = add(app, request(container)?)?;
    }
    let mut sidecars = 0_i64;
    let mut init_peak = 0_i64;
    if let Some(init) = pod["spec"].get("initContainers") {
        let init = init
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("invalid initContainers"))?;
        for container in init {
            let count = request(container)?;
            let concurrent = add(sidecars, count)?;
            init_peak = init_peak.max(concurrent);
            if container["restartPolicy"].as_str() == Some("Always") {
                sidecars = concurrent;
            }
        }
    }
    let overhead = pod["spec"]["overhead"]
        .get(resource)
        .map(accelerator_quantity)
        .transpose()?
        .unwrap_or(0);
    add(add(app, sidecars)?.max(init_peak), overhead)
}

#[cfg(test)]
mod inventory_evidence_tests {
    use super::from_json;
    use serde_json::json;

    /// Opt-in captured API evidence; no kubectl invocation or cluster mutation.
    #[test]
    #[ignore = "set LMD_INVENTORY_FIXTURE to a directory with nodes.json and pods.json"]
    fn captured_cluster_inventory() {
        let dir = std::path::PathBuf::from(
            std::env::var("LMD_INVENTORY_FIXTURE").expect("fixture directory"),
        );
        let read = |name| {
            serde_json::from_slice(&std::fs::read(dir.join(name)).expect("read fixture"))
                .expect("JSON fixture")
        };
        let (totals, allocations, capacity) =
            from_json(&read("nodes.json"), &read("pods.json")).expect("valid inventory");
        assert_eq!(allocations.len(), capacity.len());
        for (resource, total, requested) in totals {
            assert_eq!(total, capacity.values().map(|r| r[&resource]).sum::<i64>());
            assert!(requested >= allocations.values().map(|r| r[&resource]).sum::<i64>());
            println!(
                "{resource}: allocatable={total}, requested={requested}, nodes={}",
                capacity.len()
            );
        }
    }

    #[test]
    fn effective_requests_follow_init_and_sidecar_lifetimes() {
        let regular = |n| json!({"resources":{"requests":{"nvidia.com/gpu": n}}});
        let sidecar =
            |n| json!({"restartPolicy":"Always", "resources":{"requests":{"nvidia.com/gpu": n}}});
        for (init, expected) in [
            (vec![], 3),
            (vec![regular(8), regular(5)], 8),
            (vec![sidecar(2), regular(8)], 10),
            (vec![regular(8), sidecar(2)], 8),
            (vec![sidecar(2), sidecar(4)], 9),
        ] {
            let pod =
                json!({"spec":{"containers":[regular(1), regular(2)], "initContainers":init}});
            assert_eq!(
                super::pod_accelerator_request(&pod, "nvidia.com/gpu").unwrap(),
                expected
            );
        }
        let pod = json!({"spec":{"containers":[{"resources":{"limits":{"nvidia.com/gpu":"2"}}}],"overhead":{"nvidia.com/gpu":"1"}}});
        assert_eq!(
            super::pod_accelerator_request(&pod, "nvidia.com/gpu").unwrap(),
            3
        );
        let overflow = json!({"spec":{"containers":[regular(i64::MAX), regular(1)]}});
        assert!(super::pod_accelerator_request(&overflow, "nvidia.com/gpu").is_err());
    }

    #[test]
    fn zero_reservations_require_two_valid_lists() {
        let nodes = json!({"items":[{"metadata":{"name":"n1"},"status":{"allocatable":{"nvidia.com/gpu":"4"}}}]});
        let (totals, allocations, capacity) = from_json(&nodes, &json!({"items":[]})).unwrap();
        assert_eq!(allocations["n1"]["nvidia.com/gpu"], 0);
        assert_eq!(capacity["n1"]["nvidia.com/gpu"], 4);
        assert!(totals.contains(&("nvidia.com/gpu".into(), 4, 0)));
        assert!(from_json(&nodes, &json!({"message":"forbidden"})).is_err());
        assert!(from_json(&json!({}), &json!({"items":[]})).is_err());
    }

    #[test]
    fn requests_are_counted_and_invalid_evidence_is_rejected() {
        let nodes = json!({"items":[{"metadata":{"name":"n1"},"status":{"allocatable":{"nvidia.com/gpu":4}}}]});
        let mut pods = json!({"items":[{"spec":{"nodeName":"n1","containers":[{"resources":{"requests":{"nvidia.com/gpu":"2"}}}]},"status":{"phase":"Running"}}]});
        assert_eq!(
            from_json(&nodes, &pods).unwrap().1["n1"]["nvidia.com/gpu"],
            2
        );
        pods["items"][0]["status"]["phase"] = json!("Succeeded");
        assert_eq!(
            from_json(&nodes, &pods).unwrap().1["n1"]["nvidia.com/gpu"],
            0
        );
        pods["items"][0]["status"]["phase"] = json!("Running");
        pods["items"][0]["spec"]["containers"][0]["resources"]["requests"]["nvidia.com/gpu"] =
            json!("invalid");
        assert!(from_json(&nodes, &pods).is_err());
    }
}
