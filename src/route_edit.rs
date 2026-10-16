//! Pure HTTPRoute edits. Never discard sibling matches or silently collapse weighted backends.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

fn rules(route: &mut Value) -> Result<&mut Vec<Value>> {
    route["spec"]["rules"]
        .as_array_mut()
        .ok_or_else(|| anyhow!("HTTPRoute has no rules"))
}
fn selected(route: &Value, path: &str) -> Result<usize> {
    let rules = route["spec"]["rules"]
        .as_array()
        .ok_or_else(|| anyhow!("HTTPRoute has no rules"))?;
    if path == "/"
        && rules.iter().any(|rule| {
            rule["matches"].as_array().is_none_or(|matches| {
                matches.is_empty()
                    || matches
                        .iter()
                        .any(|m| m["path"]["value"].as_str().is_none())
            })
        })
    {
        bail!("implicit root path requires explicit HTTPRoute editing")
    }
    let hits: Vec<_> = rules
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r["matches"]
                .as_array()
                .is_some_and(|ms| ms.iter().any(|m| m["path"]["value"] == path))
        })
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [i] => {
            let matches = rules[*i]["matches"].as_array().unwrap();
            if matches
                .iter()
                .filter(|m| m["path"]["value"] == path)
                .count()
                != 1
            {
                bail!("path {path} matches multiple conditions in one rule; edit the HTTPRoute explicitly")
            }
            Ok(*i)
        }
        [] => bail!("path {path} not found"),
        _ => bail!("path {path} matches multiple rules; edit the HTTPRoute explicitly"),
    }
}
pub fn rename(route: &mut Value, old: &str, new: &str) -> Result<()> {
    if !new.starts_with('/') {
        bail!("route path must start with /")
    }
    let i = selected(route, old)?;
    for m in rules(route)?[i]["matches"].as_array_mut().unwrap() {
        if m["path"]["value"] == old {
            m["path"]["value"] = json!(new);
        }
    }
    Ok(())
}
pub fn delete(route: &mut Value, path: &str) -> Result<()> {
    let i = selected(route, path)?;
    let rules = rules(route)?;
    let matches = rules[i]["matches"].as_array_mut().unwrap();
    matches.retain(|m| m["path"]["value"] != path);
    if matches.is_empty() {
        rules.remove(i);
    }
    Ok(())
}

pub fn retarget(
    route: &mut Value,
    path: &str,
    backend: &str,
    kind: &str,
    service: Option<&Value>,
) -> Result<()> {
    if !matches!(kind, "Service" | "InferencePool") {
        bail!("unsupported backend kind: {kind}")
    }
    let i = selected(route, path)?;
    let original = &route["spec"]["rules"][i];
    let refs = original["backendRefs"]
        .as_array()
        .ok_or_else(|| anyhow!("rule has no backends"))?;
    if refs.len() != 1 {
        bail!("weighted/multiple backends require explicit HTTPRoute editing")
    }
    // Splitting a named rule would duplicate its name or silently detach policies
    // targeting that name. Require an explicit policy-aware edit instead.
    let splitting = original["matches"].as_array().unwrap().len() > 1;
    if splitting && original.get("name").is_some() {
        bail!("splitting a named rule requires explicit HTTPRoute and attached-policy review")
    }
    if splitting && route["spec"]["rules"].as_array().unwrap().len() >= 16 {
        bail!("splitting this rule would exceed the HTTPRoute limit of 16 rules; edit explicitly")
    }
    let mut replacement = original.clone();
    let reference = &mut replacement["backendRefs"][0];
    if !reference.is_object() {
        bail!("invalid backend reference")
    }
    if kind == "Service" {
        let ports: Vec<_> = service
            .and_then(|s| s["spec"]["ports"].as_array())
            .ok_or_else(|| anyhow!("target Service ports unavailable"))?
            .iter()
            .filter(|p| p["protocol"].as_str().unwrap_or("TCP") == "TCP")
            .filter_map(|p| p["port"].as_u64())
            .collect();
        let old = reference["port"].as_u64().filter(|p| ports.contains(p));
        let port = old
            .or_else(|| (ports.len() == 1).then(|| ports[0]))
            .ok_or_else(|| {
                anyhow!("target Service has ambiguous or missing TCP port; edit explicitly")
            })?;
        reference["port"] = json!(port);
    } else {
        reference.as_object_mut().unwrap().remove("port");
    }
    // The picker lists local resources; do not inherit an old cross-namespace target.
    reference.as_object_mut().unwrap().remove("namespace");
    reference["group"] = json!(if kind == "InferencePool" {
        "inference.networking.k8s.io"
    } else {
        ""
    });
    reference["kind"] = json!(kind);
    reference["name"] = json!(backend);
    let rules = rules(route)?;
    let matches = rules[i]["matches"].as_array_mut().unwrap();
    let chosen: Vec<_> = matches
        .iter()
        .filter(|m| m["path"]["value"] == path)
        .cloned()
        .collect();
    matches.retain(|m| m["path"]["value"] != path);
    replacement["matches"] = json!(chosen);
    if matches.is_empty() {
        rules[i] = replacement;
    } else {
        rules.insert(i + 1, replacement);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route() -> Value {
        json!({"spec":{"rules":[{"matches":[
        {"path":{"value":"/a"}}, {"path":{"value":"/b"}}],
        "filters":[{"type":"RequestHeaderModifier"}],
        "backendRefs":[{"name":"old", "kind":"Service", "port":8000, "weight":3}]}]}})
    }
    #[test]
    fn implicit_root_cannot_redirect_an_edit_to_an_explicit_root_sibling() {
        let original = json!({"spec":{"rules":[{}, {"matches":[{"path":{"value":"/"}}], "backendRefs":[{"name":"service"}]}]}});
        for action in ["rename", "delete", "retarget"] {
            let mut edited = original.clone();
            let result = match action {
                "rename" => rename(&mut edited, "/", "/new"),
                "delete" => delete(&mut edited, "/"),
                _ => retarget(&mut edited, "/", "pool", "InferencePool", None),
            };
            assert!(result.unwrap_err().to_string().contains("implicit root"));
            assert_eq!(edited, original);
        }
    }

    #[test]
    fn delete_preserves_sibling_path() {
        let mut r = route();
        delete(&mut r, "/a").unwrap();
        assert_eq!(r["spec"]["rules"].as_array().unwrap().len(), 1);
        assert_eq!(
            r["spec"]["rules"][0]["matches"],
            json!([{"path":{"value":"/b"}}])
        );
        delete(&mut r, "/b").unwrap();
        assert_eq!(r["spec"]["rules"], json!([]));
    }
    #[test]
    fn named_rule_splits_are_rejected_without_losing_policy_attachment_identity() {
        let mut r = route();
        r["spec"]["rules"][0]["name"] = json!("policy-target");
        let before = r.clone();
        assert!(retarget(&mut r, "/a", "new", "InferencePool", None)
            .unwrap_err()
            .to_string()
            .contains("attached-policy"));
        assert_eq!(r, before);
        r["spec"]["rules"][0]["matches"] = json!([{"path":{"value":"/a"}}]);
        retarget(&mut r, "/a", "new", "InferencePool", None).unwrap();
        assert_eq!(r["spec"]["rules"][0]["name"], "policy-target");
        assert_eq!(r["spec"]["rules"][0]["backendRefs"][0]["name"], "new");
    }

    #[test]
    fn splitting_respects_rule_capacity_without_rejecting_in_place_updates() {
        let mut r = route();
        for i in 1..16 {
            r["spec"]["rules"]
                .as_array_mut()
                .unwrap()
                .push(json!({"matches":[{"path":{"value":format!("/other-{i}")}}]}));
        }
        let before = r.clone();
        assert!(retarget(&mut r, "/a", "new", "InferencePool", None)
            .unwrap_err()
            .to_string()
            .contains("16 rules"));
        assert_eq!(r, before);
        r["spec"]["rules"][0]["matches"] = json!([{"path":{"value":"/a"}}]);
        retarget(&mut r, "/a", "new", "InferencePool", None).unwrap();
        assert_eq!(r["spec"]["rules"].as_array().unwrap().len(), 16);
    }

    #[test]
    fn retarget_splits_matches_preserving_filters_weight_and_port() {
        let mut r = route();
        let service = json!({"spec":{"ports":[{"port":9000}]}});
        retarget(&mut r, "/a", "new", "Service", Some(&service)).unwrap();
        assert_eq!(r["spec"]["rules"][0]["backendRefs"][0]["name"], "old");
        assert_eq!(r["spec"]["rules"][0]["matches"][0]["path"]["value"], "/b");
        let new = &r["spec"]["rules"][1];
        assert_eq!(new["backendRefs"][0]["port"], 9000);
        assert_eq!(new["backendRefs"][0]["weight"], 3);
        assert_eq!(new["filters"], r["spec"]["rules"][0]["filters"]);
    }
    #[test]
    fn ambiguous_ports_and_weighted_backends_are_rejected_without_changes() {
        let mut r = route();
        let before = r.clone();
        let service = json!({"spec":{"ports":[{"port":80},{"port":443}]}});
        assert!(retarget(&mut r, "/a", "new", "Service", Some(&service)).is_err());
        assert_eq!(r, before);
        r["spec"]["rules"][0]["backendRefs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"canary"}));
        let before = r.clone();
        assert!(retarget(&mut r, "/a", "new", "InferencePool", None).is_err());
        assert_eq!(r, before);
    }
    #[test]
    fn identical_paths_with_distinct_match_conditions_cannot_be_bulk_edited() {
        for second in [
            json!({"path":{"value":"/a", "type":"Exact"}}),
            json!({"path":{"value":"/a"}, "method":"POST"}),
            json!({"path":{"value":"/a"}, "headers":[{"name":"tenant", "value":"other"}]}),
            json!({"path":{"value":"/a"}, "queryParams":[{"name":"version", "value":"2"}]}),
        ] {
            let mut original = route();
            original["spec"]["rules"][0]["matches"]
                .as_array_mut()
                .unwrap()
                .push(second);
            for action in ["rename", "delete", "retarget"] {
                let mut edited = original.clone();
                let result = match action {
                    "rename" => rename(&mut edited, "/a", "/new"),
                    "delete" => delete(&mut edited, "/a"),
                    _ => retarget(&mut edited, "/a", "new", "InferencePool", None),
                };
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("multiple conditions"));
                assert_eq!(edited, original, "{action} must preserve every condition");
            }
        }
    }

    #[test]
    fn ambiguous_rule_selection_is_rejected() {
        let mut r = route();
        let duplicate = r["spec"]["rules"][0].clone();
        r["spec"]["rules"].as_array_mut().unwrap().push(duplicate);
        assert!(delete(&mut r, "/a").is_err());
        assert!(rename(&mut r, "/a", "/c").is_err());
    }
}
