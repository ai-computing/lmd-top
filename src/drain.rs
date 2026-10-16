//! Plan routing-label exclusion from observed selectors, without guessing label names.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const ORIGINAL: &str = "lmd-top.io/drain-original-labels";
#[derive(Debug)]
pub struct Plan {
    pub uid: String,
    pub original: BTreeMap<String, String>,
    pub changed: BTreeMap<String, String>,
    pub services: Vec<String>,
}
fn selector(resource: &Value, pool: bool) -> Result<BTreeMap<String, String>> {
    let spec = &resource["spec"]["selector"];
    if pool
        && spec
            .get("matchExpressions")
            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
    {
        bail!("InferencePool selector expressions are unsupported; drain aborted");
    }
    let labels = if pool {
        spec.get("matchLabels").unwrap_or(spec)
    } else {
        spec
    };
    if labels.is_null() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_value(labels.clone()).map_err(|_| anyhow!("invalid routing selector"))
}
pub fn plan(pod: &Value, services: &Value, pools: &Value) -> Result<Plan> {
    if pod["metadata"]["annotations"].get(ORIGINAL).is_some() {
        bail!("pod already has a drain record")
    }
    let uid = pod["metadata"]["uid"]
        .as_str()
        .ok_or_else(|| anyhow!("pod UID missing"))?;
    let labels = &pod["metadata"]["labels"];
    let mut plan = Plan {
        uid: uid.into(),
        original: BTreeMap::new(),
        changed: BTreeMap::new(),
        services: vec![],
    };
    for (resources, pool) in [(services, false), (pools, true)] {
        for resource in resources["items"]
            .as_array()
            .ok_or_else(|| anyhow!("routing resources unavailable"))?
        {
            let selector = selector(resource, pool)?;
            // Selectorless Services do not automatically select this Pod.
            if selector.is_empty() {
                if pool {
                    bail!("empty InferencePool selector; drain aborted")
                } else {
                    continue;
                }
            }
            if !selector
                .iter()
                .all(|(k, v)| labels[k].as_str() == Some(v.as_str()))
            {
                continue;
            }
            let (key, old) = selector.iter().next().unwrap();
            plan.original.insert(key.clone(), old.clone());
            // UID-derived value fits the label limit even when the original uses all 63 chars.
            let changed = format!(
                "lmd-drained-{}",
                uid.chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(40)
                    .collect::<String>()
            );
            if changed == *old {
                bail!("pod is already excluded by a drain label")
            }
            plan.changed.insert(key.clone(), changed);
            if !pool {
                plan.services.push(
                    resource["metadata"]["name"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Service name missing"))?
                        .into(),
                );
            }
        }
    }
    if plan.changed.is_empty() {
        bail!("no matching Service or InferencePool selector; drain aborted")
    }
    Ok(plan)
}
pub fn pointer(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
impl Plan {
    pub fn patch(&self, pod: &Value) -> Result<Value> {
        let version = pod["metadata"]["resourceVersion"]
            .as_str()
            .ok_or_else(|| anyhow!("pod resourceVersion missing"))?;
        let mut patch = vec![
            json!({"op":"test","path":"/metadata/uid","value":self.uid}),
            json!({"op":"test","path":"/metadata/resourceVersion","value":version}),
        ];
        for (key, value) in &self.changed {
            patch.push(json!({"op":"replace","path":format!("/metadata/labels/{}",pointer(key)),"value":value}));
        }
        if !pod["metadata"]["annotations"].is_object() {
            patch.push(json!({"op":"add","path":"/metadata/annotations","value":{}}));
        }
        patch.push(
            json!({"op":"add","path":format!("/metadata/annotations/{}",pointer(ORIGINAL)),
            "value":serde_json::to_string(&self.original)?}),
        );
        Ok(json!(patch))
    }
    pub fn rollback(&self) -> Value {
        let mut patch = vec![json!({"op":"test","path":"/metadata/uid","value":self.uid})];
        for (key, value) in &self.changed {
            patch.push(json!({"op":"test","path":format!("/metadata/labels/{}",pointer(key)),"value":value}));
        }
        for (key, value) in &self.original {
            patch.push(json!({"op":"replace","path":format!("/metadata/labels/{}",pointer(key)),"value":value}));
        }
        patch.push(
            json!({"op":"remove","path":format!("/metadata/annotations/{}",pointer(ORIGINAL))}),
        );
        json!(patch)
    }
    pub fn services_detached(&self, slices: &Value) -> Result<bool> {
        let items = slices["items"]
            .as_array()
            .ok_or_else(|| anyhow!("EndpointSlice list unavailable"))?;
        Ok(!items.iter().any(|s| {
            self.services
                .iter()
                .any(|name| s["metadata"]["labels"]["kubernetes.io/service-name"] == *name)
                && s["endpoints"]
                    .as_array()
                    .is_some_and(|eps| eps.iter().any(|ep| ep["targetRef"]["uid"] == self.uid))
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_actual_selectors_not_app_priority() {
        let pod = json!({"metadata":{"uid":"123", "resourceVersion":"7", "labels":{"app":"display", "model":"serve", "role":"decode"}}});
        let services =
            json!({"items":[{"metadata":{"name":"svc"},"spec":{"selector":{"role":"decode"}}}]});
        let pools = json!({"items":[{"spec":{"selector":{"matchLabels":{"model":"serve"}}}}]});
        let p = plan(&pod, &services, &pools).unwrap();
        assert!(!p.changed.contains_key("app"));
        assert!(p.changed.contains_key("model") && p.changed.contains_key("role"));
        assert_eq!(p.patch(&pod).unwrap()[1]["value"], "7");
        assert_eq!(p.rollback()[0]["value"], "123");
        let slices = json!({"items":[{"metadata":{"labels":{"kubernetes.io/service-name":"svc"}}, "endpoints":[{"targetRef":{"uid":"123"}}]}]});
        assert!(!p.services_detached(&slices).unwrap());
        assert!(p.services_detached(&json!({"items":[]})).unwrap());
    }
    #[test]
    fn refuses_unknown_routing_and_records_original_long_values() {
        let pod = json!({"metadata":{"uid":"abc", "labels":{"app":"x".repeat(63)}}});
        let empty = json!({"items":[]});
        assert!(plan(&pod, &empty, &empty).is_err());
        let svc = json!({"items":[{"metadata":{"name":"svc"},"spec":{"selector":{"app":"x".repeat(63)}}}]});
        let p = plan(&pod, &svc, &empty).unwrap();
        assert_eq!(p.original["app"].len(), 63);
        assert!(p.changed["app"].len() <= 63);
    }
}
