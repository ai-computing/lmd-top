//! Configuration evidence is not proof that requests traversed an EPP.
use crate::{collect::Snapshot, quality::Quality};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct Configuration {
    pub namespace: Option<String>,
    pub rule_index: usize,
    pub match_index: usize,
    pub implicit_path: bool,
    pub match_conditions: serde_json::Value,
    pub backend_refs: Vec<serde_json::Value>,
}

#[derive(Serialize)]
pub struct Evidence {
    pub state: Quality,
    pub inference_pool_backend_observed: Option<bool>,
    pub runtime_verified: bool,
}

impl Evidence {
    pub fn summary(&self) -> &'static str {
        match self.inference_pool_backend_observed {
            Some(true) => "Pool backend configured; runtime unverified",
            Some(false) => "No Pool backend observed; runtime unverified",
            None => "Route configuration unknown; runtime unverified",
        }
    }
}

/// Include the source position and every non-path match condition for inspection.
pub fn match_summary(m: &serde_json::Value, rule: usize, index: usize) -> String {
    let mut parts = vec![
        format!("rule {} / match {}", rule + 1, index + 1),
        m["path"]["type"]
            .as_str()
            .unwrap_or("PathPrefix")
            .to_owned(),
    ];
    if let Some(fields) = m.as_object() {
        for (key, value) in fields {
            if key != "path" {
                parts.push(format!("{key}={value}"));
            }
        }
    }
    parts.join(" · ")
}

fn single_local_backend(route: &serde_json::Value, rule: &serde_json::Value) -> bool {
    let Some(refs) = rule["backendRefs"].as_array() else {
        return false;
    };
    let [reference] = refs.as_slice() else {
        return false;
    };
    let kind = reference["kind"].as_str().unwrap_or("Service");
    let group = reference["group"].as_str().unwrap_or("");
    let supported = matches!(
        (kind, group),
        ("Service", "") | ("InferencePool", "inference.networking.k8s.io")
    );
    let local = reference.get("namespace").is_none()
        || reference["namespace"].as_str().is_some_and(|ns| {
            !ns.is_empty() && Some(ns) == route["metadata"]["namespace"].as_str()
        });
    supported
        && local
        && reference["name"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
}

/// Decode API configuration without coupling route presentation to kubectl I/O.
pub fn rows(list: &serde_json::Value) -> (Vec<crate::collect::Route>, bool) {
    let mut rows = Vec::new();
    let mut pool = false;
    let empty = serde_json::json!({});
    for route in list["items"].as_array().into_iter().flatten() {
        for (ri, rule) in route["spec"]["rules"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            pool |= has_pool_backend(rule);
            let matches: Vec<&serde_json::Value> = match rule["matches"].as_array() {
                Some(matches) if !matches.is_empty() => matches.iter().collect(),
                _ => vec![&empty],
            };
            for (mi, m) in matches.into_iter().enumerate() {
                let implicit = m["path"]["value"].as_str().is_none();
                let mut summary = match_summary(m, ri, mi);
                if implicit {
                    summary.push_str(" · implicit path");
                }
                rows.push(crate::collect::Route {
                    configuration: Some(Configuration {
                        namespace: route["metadata"]["namespace"].as_str().map(str::to_owned),
                        rule_index: ri,
                        match_index: mi,
                        implicit_path: implicit,
                        match_conditions: m.clone(),
                        backend_refs: rule["backendRefs"].as_array().cloned().unwrap_or_default(),
                    }),
                    identity: crate::resource_guard::Identity::read(route),
                    route: route["metadata"]["name"].as_str().unwrap_or("").to_owned(),
                    path: m["path"]["value"].as_str().unwrap_or("/").to_owned(),
                    backend: rule["backendRefs"][0]["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    kind: rule["backendRefs"][0]["kind"]
                        .as_str()
                        .unwrap_or("Service")
                        .to_owned(),
                    match_summary: summary,
                    backend_summary: format!(
                        "backendRefs={}",
                        rule.get("backendRefs").unwrap_or(&serde_json::Value::Null)
                    ),
                    single_local_backend: single_local_backend(route, rule),
                });
            }
        }
    }
    (rows, pool)
}

pub fn has_pool_backend(rule: &serde_json::Value) -> bool {
    rule["backendRefs"].as_array().is_some_and(|refs| {
        refs.iter().any(|backend| {
            backend["kind"].as_str() == Some("InferencePool")
                && backend["group"].as_str() == Some("inference.networking.k8s.io")
                && backend["name"]
                    .as_str()
                    .is_some_and(|name| !name.is_empty())
        })
    })
}

pub fn owners<'a>(snapshot: &'a Snapshot, route: &crate::collect::Route) -> Vec<&'a str> {
    if !route.single_local_backend {
        return Vec::new();
    }
    let required: &[&str] = match route.kind.as_str() {
        "Service" => &[
            "routes",
            "services",
            "workloads",
            "pods",
            "replicasets",
            "endpointslices",
        ],
        "InferencePool" => &["routes", "pools", "workloads", "pods", "replicasets"],
        _ => return Vec::new(),
    };
    let now = crate::collect::now_secs();
    if !required.iter().all(|source| {
        snapshot.sources.get(*source).is_some_and(|status| {
            status.state(now, snapshot.stale_after_secs.max(10)) == Quality::Fresh
        })
    }) {
        return Vec::new();
    }

    match route.kind.as_str() {
        "InferencePool" => snapshot
            .pool_bindings
            .get(&route.backend)
            .map(|names| names.iter().map(String::as_str).collect())
            .unwrap_or_default(),
        "Service" => snapshot
            .service_bindings
            .iter()
            .filter(|(_, services)| services.contains(&route.backend))
            .map(|(name, _)| name.as_str())
            .collect(),
        _ => Vec::new(),
    }
}

pub fn evidence(snapshot: &Snapshot) -> Evidence {
    let state = snapshot
        .sources
        .get("routes")
        .map(|s| {
            s.state(
                crate::collect::now_secs(),
                snapshot.stale_after_secs.max(10),
            )
        })
        .unwrap_or(Quality::Unknown);
    Evidence {
        state,
        inference_pool_backend_observed: matches!(state, Quality::Fresh | Quality::Empty)
            .then_some(snapshot.epp_in_path),
        runtime_verified: false,
    }
}

#[cfg(test)]
pub fn fresh_relationship_sources(snapshot: &mut Snapshot) {
    for source in [
        "routes",
        "services",
        "pools",
        "workloads",
        "pods",
        "replicasets",
        "endpointslices",
    ] {
        snapshot.sources.insert(
            source.into(),
            crate::quality::SourceStatus::observed(crate::collect::now_secs(), 1, &[]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::SourceStatus;

    #[test]
    fn backend_scope_and_all_weighted_references_are_preserved() {
        let mut snap = Snapshot::default();
        fresh_relationship_sources(&mut snap);
        snap.service_bindings
            .insert("model".into(), vec!["backend".into()]);
        for (refs, expected) in [
            (serde_json::json!([{"name":"backend"}]), true),
            (
                serde_json::json!([{"name":"backend","namespace":"ns"}]),
                true,
            ),
            (
                serde_json::json!([{"name":"backend","namespace":"foreign"}]),
                false,
            ),
            (
                serde_json::json!([{"name":"backend","group":"custom.example"}]),
                false,
            ),
            (
                serde_json::json!([{"name":"backend","weight":90},{"name":"canary","weight":10}]),
                false,
            ),
            (serde_json::json!([]), false),
        ] {
            let list = serde_json::json!({"items":[{"metadata":{"name":"route","namespace":"ns"},"spec":{"rules":[{"backendRefs":refs}]}}]});
            let (rows, _) = rows(&list);
            assert_eq!(rows[0].single_local_backend, expected);
            assert_eq!(!owners(&snap, &rows[0]).is_empty(), expected);
            assert!(rows[0].backend_summary.contains(&refs.to_string()));
        }
    }

    #[test]
    fn route_rows_include_implicit_paths_and_keep_explicit_conditions() {
        let list = serde_json::json!({"items":[{"metadata":{"name":"route", "uid":"uid", "resourceVersion":"7"},
        "spec":{"rules":[
            {"backendRefs":[{"name":"service"}]},
            {"matches":[{"method":"POST"}], "backendRefs":[{"name":"pool","kind":"InferencePool","group":"inference.networking.k8s.io"}]},
            {"matches":[{"path":{"value":"/explicit","type":"Exact"}}]},
            {"matches":[]}
        ]}}]});
        let (rows, pool) = rows(&list);
        assert!(pool);
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["/", "/", "/explicit", "/"]
        );
        assert!(rows[0].match_summary.contains("implicit path"));
        assert!(rows[1].match_summary.contains("POST"));
        assert!(rows[2].match_summary.contains("Exact"));
        assert!(!rows[2].match_summary.contains("implicit"));
        assert_eq!(rows[0].identity.as_ref().unwrap().uid, "uid");
    }

    #[test]
    fn relationships_expire_when_any_prerequisite_is_unavailable() {
        let mut snap = Snapshot::default();
        snap.service_bindings
            .insert("model".into(), vec!["backend".into()]);
        snap.pool_bindings
            .insert("backend".into(), vec!["model".into()]);
        let mut route = crate::collect::Route {
            configuration: None,
            identity: None,
            path: "/".into(),
            backend: "backend".into(),
            kind: "Service".into(),
            route: "route".into(),
            match_summary: String::new(),
            backend_summary: String::new(),
            single_local_backend: true,
        };
        for kind in ["Service", "InferencePool"] {
            route.kind = kind.into();
            fresh_relationship_sources(&mut snap);
            assert_eq!(owners(&snap, &route), ["model"]);
            let required = if kind == "Service" {
                vec![
                    "routes",
                    "services",
                    "workloads",
                    "pods",
                    "replicasets",
                    "endpointslices",
                ]
            } else {
                vec!["routes", "pools", "workloads", "pods", "replicasets"]
            };
            for source in required {
                for status in [
                    None,
                    Some(SourceStatus::observed(1, 1, &[])),
                    Some(SourceStatus::observed(crate::collect::now_secs(), 0, &[])),
                    Some(SourceStatus::observed(
                        crate::collect::now_secs(),
                        0,
                        &["Forbidden".into()],
                    )),
                ] {
                    fresh_relationship_sources(&mut snap);
                    snap.sources.remove(source);
                    if let Some(status) = status {
                        snap.sources.insert(source.into(), status);
                    }
                    assert!(owners(&snap, &route).is_empty(), "{kind} {source}");
                }
            }
        }
    }

    #[test]
    fn match_descriptions_preserve_conditions_and_source_positions() {
        let m = serde_json::json!({"path":{"value":"/a","type":"Exact"},
            "method":"POST", "headers":[{"name":"tenant", "value":"two"}],
            "queryParams":[{"name":"version", "value":"2"}]});
        let text = match_summary(&m, 1, 2);
        for expected in [
            "rule 2 / match 3",
            "Exact",
            "POST",
            "tenant",
            "two",
            "queryParams",
            "version",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        assert!(
            match_summary(&serde_json::json!({"path":{"value":"/a"}}), 0, 0).contains("PathPrefix")
        );
    }

    #[test]
    fn recognizes_pool_in_any_backend_position_without_confusing_api_groups() {
        let mut rule = serde_json::json!({"backendRefs": [
            {"name": "service"},
            {"name": "pool", "kind": "InferencePool", "group": "inference.networking.k8s.io"}
        ]});
        assert!(has_pool_backend(&rule));
        rule["backendRefs"][1]["group"] = "unrelated.example".into();
        assert!(!has_pool_backend(&rule));
        rule["backendRefs"][1]["group"] = serde_json::Value::Null;
        assert!(!has_pool_backend(&rule));
        assert!(!has_pool_backend(&serde_json::json!({})));
    }

    #[test]
    fn route_configuration_never_proves_runtime_and_expires_independently() {
        let mut s = Snapshot {
            epp_in_path: true,
            ..Default::default()
        };
        assert!(evidence(&s).inference_pool_backend_observed.is_none());
        let now = crate::collect::now_secs();
        s.sources
            .insert("routes".into(), SourceStatus::observed(now, 1, &[]));
        let e = evidence(&s);
        assert_eq!(e.inference_pool_backend_observed, Some(true));
        assert!(!e.runtime_verified);
        s.sources.get_mut("routes").unwrap().error = Some("Forbidden".into());
        assert!(evidence(&s).inference_pool_backend_observed.is_none());
        s.sources
            .insert("routes".into(), SourceStatus::observed(1, 1, &[]));
        assert!(evidence(&s).inference_pool_backend_observed.is_none());
        s.epp_in_path = false;
        s.sources
            .insert("routes".into(), SourceStatus::observed(now, 0, &[]));
        assert_eq!(evidence(&s).inference_pool_backend_observed, Some(false));
        assert!(!evidence(&s).runtime_verified);
    }
}
