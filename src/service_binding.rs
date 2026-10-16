//! Declared Service selector relationships. Names alone are not ownership evidence.
use serde_json::Value;
use std::collections::BTreeMap;

/// Route targets come from actual namespace Services, including selectorless or
/// temporarily endpoint-free Services. Deployment names are not Service names.
pub fn names_in_namespace(services: &Value, namespace: &str) -> Vec<String> {
    services["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|service| service["metadata"]["namespace"].as_str() == Some(namespace))
        .filter_map(|service| {
            service["metadata"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
        })
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn declared_bindings(services: &Value, deployments: &Value) -> BTreeMap<String, Vec<String>> {
    let mut bindings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Some(services) = services["items"].as_array() else {
        return bindings;
    };
    let Some(deployments) = deployments["items"].as_array() else {
        return bindings;
    };
    for service in services {
        let Some(name) = service["metadata"]["name"]
            .as_str()
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(namespace) = service["metadata"]["namespace"]
            .as_str()
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(selector) = service["spec"]["selector"]
            .as_object()
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let matching: Vec<&Value> = deployments
            .iter()
            .filter(|deployment| {
                Some(namespace) == deployment["metadata"]["namespace"].as_str()
                    && selector.iter().all(|(key, value)| {
                        value.as_str().is_some()
                            && deployment["spec"]["template"]["metadata"]["labels"][key] == *value
                    })
            })
            .collect();
        // A selector shared by deployments cannot attribute a Service aggregate to one.
        if let [deployment] = matching.as_slice() {
            if let Some(deployment) = deployment["metadata"]["name"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                bindings
                    .entry(deployment.into())
                    .or_default()
                    .push(name.into());
            }
        }
    }
    for services in bindings.values_mut() {
        services.sort();
        services.dedup();
    }
    bindings
}

pub fn metric_service<T>(
    deployment: &str,
    bindings: &BTreeMap<String, Vec<String>>,
    observed: &BTreeMap<String, T>,
) -> Option<String> {
    let mut candidates = bindings
        .get(deployment)?
        .iter()
        .filter(|service| observed.contains_key(*service));
    let first = candidates.next()?;
    candidates.next().is_none().then(|| first.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn deployment(name: &str, app: &str) -> Value {
        json!({"metadata":{"name":name,"namespace":"ns"},"spec":{"template":{"metadata":{"labels":{"app":app}}}}})
    }
    fn service(name: &str, app: &str) -> Value {
        json!({"metadata":{"name":name,"namespace":"ns"},"spec":{"selector":{"app":app}}})
    }
    #[test]
    fn selectors_connect_unrelated_names_but_not_similar_names() {
        let bindings = declared_bindings(
            &json!({"items":[service("totally-different-service","a"),service("model-fp8","b")]}),
            &json!({"items":[deployment("model","a"),deployment("model-fp8","b")]}),
        );
        let observed = BTreeMap::from([
            ("totally-different-service".into(), ()),
            ("model-fp8".into(), ()),
        ]);
        assert_eq!(
            metric_service("model", &bindings, &observed).as_deref(),
            Some("totally-different-service")
        );
        assert_eq!(metric_service("serve-model", &bindings, &observed), None);
    }
    #[test]
    fn shared_selectors_and_multiple_observed_services_are_ambiguous() {
        let deployments = json!({"items":[deployment("one","a"), deployment("two","a")]});
        assert!(
            declared_bindings(&json!({"items":[service("shared","a")]}), &deployments).is_empty()
        );
        let bindings = declared_bindings(
            &json!({"items":[service("a","a"), service("b","a")]}),
            &json!({"items":[deployment("one","a")]}),
        );
        assert_eq!(
            metric_service(
                "one",
                &bindings,
                &BTreeMap::from([("a".into(), ()), ("b".into(), ())])
            ),
            None
        );
        assert_eq!(
            metric_service("one", &bindings, &BTreeMap::from([("b".into(), ())])).as_deref(),
            Some("b")
        );
    }
    #[test]
    fn selectorless_or_other_namespace_service_never_binds() {
        let mut other = service("other", "a");
        other["metadata"]["namespace"] = json!("other");
        let selectorless =
            json!({"metadata":{"name":"external","namespace":"ns"},"spec":{"selector":{}}});
        assert!(declared_bindings(
            &json!({"items":[other,selectorless]}),
            &json!({"items":[deployment("one","a")]})
        )
        .is_empty());
    }
}

fn items(value: &Value) -> &[Value] {
    value["items"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn controller(object: &Value, kind: &str) -> Option<(String, String)> {
    let owners = object["metadata"]["ownerReferences"].as_array()?;
    let mut controllers = owners.iter().filter(|r| r["controller"] == true);
    let owner = controllers.next()?;
    if controllers.next().is_some() || owner["kind"] != kind {
        return None;
    }
    Some((
        owner["name"].as_str()?.to_string(),
        owner["uid"].as_str().filter(|s| !s.is_empty())?.to_string(),
    ))
}

fn owned_by(pod: &Value, deployment: &Value, replicasets: &[Value]) -> bool {
    let Some((name, uid)) = controller(pod, "ReplicaSet") else {
        return false;
    };
    let Some(rs) = replicasets.iter().find(|rs| {
        rs["metadata"]["name"] == name
            && rs["metadata"]["uid"] == uid
            && rs["metadata"]["namespace"] == pod["metadata"]["namespace"]
    }) else {
        return false;
    };
    let Some((owner, uid)) = controller(rs, "Deployment") else {
        return false;
    };
    owner == deployment["metadata"]["name"] && uid == deployment["metadata"]["uid"]
}

/// Current Deployment membership from controller UID chains, without name prefixes.
pub fn deployment_pods(
    deployments: &Value,
    pods: &Value,
    replicasets: &Value,
) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::new();
    for deployment in items(deployments) {
        let Some(name) = deployment["metadata"]["name"]
            .as_str()
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let mut names: Vec<String> = items(pods)
            .iter()
            .filter(|pod| {
                pod["metadata"]["namespace"] == deployment["metadata"]["namespace"]
                    && owned_by(pod, deployment, items(replicasets))
            })
            .filter_map(|pod| {
                pod["metadata"]["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .collect();
        names.sort();
        names.dedup();
        result.insert(name.to_owned(), names);
    }
    result
}

/// Pool membership requires the supported selector and a complete current Pod
/// owner chain. Keep multiple verified Deployments, but never infer empty pools.
pub fn pool_bindings(
    pools: &Value,
    deployments: &Value,
    pods: &Value,
    replicasets: &Value,
) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::new();
    for pool in items(pools) {
        let Some(name) = pool["metadata"]["name"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(ns) = pool["metadata"]["namespace"]
            .as_str()
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let selector = &pool["spec"]["selector"];
        let Some(labels) = selector["matchLabels"]
            .as_object()
            .filter(|labels| !labels.is_empty() && labels.values().all(Value::is_string))
        else {
            continue;
        };
        if selector
            .as_object()
            .is_none_or(|object| object.keys().any(|key| key != "matchLabels"))
        {
            continue;
        }
        let selected: Vec<_> = items(pods)
            .iter()
            .filter(|pod| {
                pod["metadata"]["namespace"] == ns
                    && labels
                        .iter()
                        .all(|(key, value)| pod["metadata"]["labels"][key] == *value)
            })
            .collect();
        if selected.is_empty() {
            continue;
        }
        let mut owners = std::collections::BTreeSet::new();
        let complete = selected.iter().all(|pod| {
            if pod["metadata"]["uid"].as_str().is_none_or(str::is_empty) {
                return false;
            }
            let candidates: Vec<_> = items(deployments)
                .iter()
                .filter(|deployment| {
                    deployment["metadata"]["namespace"] == ns
                        && owned_by(pod, deployment, items(replicasets))
                })
                .collect();
            if candidates.len() != 1 {
                return false;
            }
            let Some(name) = candidates[0]["metadata"]["name"]
                .as_str()
                .filter(|s| !s.is_empty())
            else {
                return false;
            };
            owners.insert(name.to_string());
            true
        });
        if complete {
            result.insert(name.into(), owners.into_iter().collect());
        }
    }
    result
}

/// Require current endpoint UID evidence in addition to declared selectors.
pub fn verified_bindings(
    services: &Value,
    deployments: &Value,
    pods: &Value,
    replicasets: &Value,
    slices: &Value,
) -> BTreeMap<String, Vec<String>> {
    let mut bindings = declared_bindings(services, deployments);
    for (deployment_name, candidates) in &mut bindings {
        let Some(deployment) = items(deployments)
            .iter()
            .find(|d| d["metadata"]["name"] == *deployment_name)
        else {
            candidates.clear();
            continue;
        };
        candidates.retain(|service_name| {
            let Some(service) = items(services).iter().find(|s| {
                s["metadata"]["name"] == *service_name
                    && s["metadata"]["namespace"] == deployment["metadata"]["namespace"]
            }) else {
                return false;
            };
            let namespace = &service["metadata"]["namespace"];
            let selector = service["spec"]["selector"].as_object().unwrap();
            // Also reject selector-matching Pods outside the endpoint list if a different
            // controller owns them; they may be entering readiness during collection.
            if items(pods)
                .iter()
                .filter(|p| {
                    p["metadata"]["namespace"] == *namespace
                        && selector
                            .iter()
                            .all(|(k, v)| p["metadata"]["labels"][k] == *v)
                })
                .any(|pod| !owned_by(pod, deployment, items(replicasets)))
            {
                return false;
            }
            let mut count = 0;
            for slice in items(slices).iter().filter(|slice| {
                slice["metadata"]["namespace"] == *namespace
                    && slice["metadata"]["labels"]["kubernetes.io/service-name"] == *service_name
            }) {
                let Some((owner, uid)) = controller(slice, "Service") else {
                    return false;
                };
                if owner != *service_name || service["metadata"]["uid"] != uid {
                    return false;
                }
                let Some(endpoints) = slice["endpoints"].as_array() else {
                    return false;
                };
                for endpoint in endpoints {
                    count += 1;
                    let target = &endpoint["targetRef"];
                    if target["kind"] != "Pod" || target["namespace"] != *namespace {
                        return false;
                    }
                    let Some(uid) = target["uid"].as_str().filter(|s| !s.is_empty()) else {
                        return false;
                    };
                    let Some(pod) = items(pods).iter().find(|p| {
                        p["metadata"]["namespace"] == *namespace
                            && p["metadata"]["name"] == target["name"]
                            && p["metadata"]["uid"] == uid
                    }) else {
                        return false;
                    };
                    if !selector
                        .iter()
                        .all(|(key, value)| pod["metadata"]["labels"][key] == *value)
                        || !owned_by(pod, deployment, items(replicasets))
                    {
                        return false;
                    }
                }
            }
            count > 0
        });
    }
    bindings.retain(|_, candidates| !candidates.is_empty());
    bindings
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> [Value; 5] {
        [
            json!({"items":[{"metadata":{"name":"service","namespace":"ns","uid":"service-uid"},"spec":{"selector":{"app":"model"}}}]}),
            json!({"items":[{"metadata":{"name":"deployment","namespace":"ns","uid":"deployment-uid"},"spec":{"template":{"metadata":{"labels":{"app":"model"}}}}}]}),
            json!({"items":[{"metadata":{"name":"pod","namespace":"ns","uid":"pod-uid","labels":{"app":"model"},"ownerReferences":[{"controller":true,"kind":"ReplicaSet","name":"rs","uid":"rs-uid"}]}}]}),
            json!({"items":[{"metadata":{"name":"rs","namespace":"ns","uid":"rs-uid","ownerReferences":[{"controller":true,"kind":"Deployment","name":"deployment","uid":"deployment-uid"}]}}]}),
            json!({"items":[{"metadata":{"name":"slice","namespace":"ns","labels":{"kubernetes.io/service-name":"service"},"ownerReferences":[{"controller":true,"kind":"Service","name":"service","uid":"service-uid"}]},"endpoints":[{"targetRef":{"kind":"Pod","name":"pod","namespace":"ns","uid":"pod-uid"}}]}]}),
        ]
    }
    fn bindings(v: &[Value; 5]) -> BTreeMap<String, Vec<String>> {
        verified_bindings(&v[0], &v[1], &v[2], &v[3], &v[4])
    }
    #[test]
    fn deployment_membership_uses_uids_and_namespace_not_name_prefixes() {
        let objects = fixture();
        let resolve = |v: &[Value; 5]| deployment_pods(&v[1], &v[2], &v[3]);
        assert_eq!(resolve(&objects)["deployment"], ["pod"]);
        for (index, path) in [
            (2, "/items/0/metadata/ownerReferences/0/uid"),
            (3, "/items/0/metadata/ownerReferences/0/uid"),
            (2, "/items/0/metadata/namespace"),
        ] {
            let mut broken = objects.clone();
            broken[2]["items"][0]["metadata"]["name"] = json!("deployment-lookalike");
            *broken[index].pointer_mut(path).unwrap() = json!("different");
            assert!(resolve(&broken)["deployment"].is_empty());
        }
    }

    #[test]
    fn pools_require_complete_selector_and_owner_evidence() {
        let mut objects = fixture();
        let mut pools = json!({"items":[{"metadata":{"name":"public-pool","namespace":"ns"},"spec":{"selector":{"matchLabels":{"app":"model"}}}}]});
        let resolve = |pools: &Value, objects: &[Value; 5]| {
            pool_bindings(pools, &objects[1], &objects[2], &objects[3])
        };
        assert_eq!(resolve(&pools, &objects)["public-pool"], ["deployment"]);
        for (index, path) in [
            (2, "/items/0/metadata/ownerReferences/0/uid"),
            (3, "/items/0/metadata/ownerReferences/0/uid"),
            (1, "/items/0/metadata/namespace"),
        ] {
            let mut broken = objects.clone();
            *broken[index].pointer_mut(path).unwrap() = json!("changed");
            assert!(resolve(&pools, &broken).is_empty());
        }
        let mut empty = objects.clone();
        empty[2] = json!({"items":[]});
        assert!(resolve(&pools, &empty).is_empty());
        pools["items"][0]["spec"]["selector"]["matchExpressions"] = json!([]);
        assert!(resolve(&pools, &objects).is_empty());
        pools["items"][0]["spec"]["selector"]
            .as_object_mut()
            .unwrap()
            .remove("matchExpressions");
        let mut mixed = objects.clone();
        for index in [1, 2, 3] {
            let mut second = objects[index]["items"][0].clone();
            for field in ["name", "uid"] {
                second["metadata"][field] =
                    json!(format!("{}-2", second["metadata"][field].as_str().unwrap()));
                if index != 1 {
                    second["metadata"]["ownerReferences"][0][field] = json!(format!(
                        "{}-2",
                        second["metadata"]["ownerReferences"][0][field]
                            .as_str()
                            .unwrap()
                    ));
                }
            }
            mixed[index]["items"].as_array_mut().unwrap().push(second);
        }
        assert_eq!(
            resolve(&pools, &mixed)["public-pool"],
            ["deployment", "deployment-2"]
        );
        let mut foreign = objects[2]["items"][0].clone();
        foreign["metadata"]["name"] = json!("foreign");
        foreign["metadata"]["ownerReferences"][0]["kind"] = json!("StatefulSet");
        objects[2]["items"].as_array_mut().unwrap().push(foreign);
        assert!(resolve(&pools, &objects).is_empty());
    }

    #[test]
    fn uid_chain_and_current_endpoint_membership_are_required() {
        assert_eq!(bindings(&fixture())["deployment"], ["service"]);
        for (index, pointer, replacement) in [
            (
                4,
                "/items/0/endpoints/0/targetRef/uid",
                json!("old-pod-uid"),
            ),
            (
                2,
                "/items/0/metadata/ownerReferences/0/kind",
                json!("StatefulSet"),
            ),
            (
                2,
                "/items/0/metadata/ownerReferences/0/uid",
                json!("old-rs-uid"),
            ),
            (
                3,
                "/items/0/metadata/ownerReferences/0/uid",
                json!("old-deployment-uid"),
            ),
            (
                4,
                "/items/0/metadata/ownerReferences/0/uid",
                json!("old-service-uid"),
            ),
            (4, "/items/0/endpoints", json!([])),
            (2, "/items/0/metadata/labels/app", json!("different")),
        ] {
            let mut v = fixture();
            *v[index].pointer_mut(pointer).unwrap() = replacement;
            assert!(bindings(&v).is_empty(), "{pointer}");
        }
    }
    #[test]
    fn foreign_controller_with_same_labels_blocks_binding_before_readiness() {
        let mut v = fixture();
        let mut pod = v[2]["items"][0].clone();
        pod["metadata"]["name"] = json!("foreign");
        pod["metadata"]["uid"] = json!("foreign-uid");
        pod["metadata"]["ownerReferences"][0]["kind"] = json!("Job");
        v[2]["items"].as_array_mut().unwrap().push(pod);
        assert!(bindings(&v).is_empty());
    }
}

#[cfg(test)]
mod namespace_service_tests {
    use super::*;
    #[test]
    fn route_candidates_use_actual_namespace_objects_without_requiring_endpoints() {
        let list = serde_json::json!({"items":[
            {"metadata":{"name":"public-service","namespace":"ns"}},
            {"metadata":{"name":"public-service","namespace":"ns"}},
            {"metadata":{"name":"other-service","namespace":"elsewhere"}},
            {"metadata":{"name":"unknown-namespace"}},
            {"metadata":{"name":"","namespace":"ns"}},
            {"metadata":{"name":"idle-service","namespace":"ns"},"spec":{"selector":{"app":"scaled-to-zero"}}}
        ]});
        assert_eq!(
            names_in_namespace(&list, "ns"),
            ["idle-service", "public-service"]
        );
        assert!(names_in_namespace(&serde_json::json!({"items":[]}), "ns").is_empty());
    }
}
