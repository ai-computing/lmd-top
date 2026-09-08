//! llm-d gateway routing documents that accompany a serving Deployment.
//!
//! Mirrors the wiring proven in-cluster (`manifests/epp/*`): a ServiceAccount, the two
//! RoleBindings onto the shared `llmd-router-epp-sa` / `-non-sa` Roles, a ClusterRoleBinding
//! for TokenReview delegation, the EPP plugin ConfigMap, the EPP Deployment and Service, the
//! InferencePool selecting the serving pods, and the HTTPRoute at `/<accel>/<model>`.
//!
//! Built as data (see [`crate::manifest`]) — including the EPP config, which is itself a YAML
//! document embedded in a ConfigMap value and used to be written as `\x20`-indented text.

use crate::manifest::{args, s, seq, Doc, Manifest};
use crate::ymap;
use serde_yaml::Value;

/// EPP image — the llm-d router endpoint picker.
const EPP_IMAGE: &str = "ghcr.io/llm-d/llm-d-router-endpoint-picker-dev:main";

/// Scorers and their weights in the default scheduling profile.
const SCORERS: &[(&str, u32)] = &[
    ("queue-scorer", 2),
    ("kv-cache-utilization-scorer", 2),
    ("prefix-cache-scorer", 3),
    ("no-hit-lru-scorer", 2),
];

/// URL path segment naming the accelerator family a route serves — declared in its pack.
fn accel_segment(vendor: &str) -> &'static str {
    crate::accel::by_id(vendor)
        .map(|p| p.scheduling.route_segment)
        .unwrap_or("gpu")
}

/// The EndpointPickerConfig document embedded in the EPP ConfigMap.
///
/// `core-metrics-extractor` needs explicit metric names for Furiosa: its engine exposes
/// `furiosa_llm_*` rather than `vllm:*`, and without this the queue/KV scorers read nothing.
fn epp_config(vendor: &str) -> Value {
    let mut plugins: Vec<Value> = SCORERS
        .iter()
        .map(|(name, _)| ymap! { "type" => s(*name) })
        .collect();
    plugins.push(ymap! {
        "type" => s("metrics-data-source"),
        "parameters" => ymap! {
            "scheme" => s("http"),
            "path" => s("/metrics"),
            "insecureSkipVerify" => Value::Bool(true),
        },
    });
    plugins.push(if vendor == "furiosa" {
        ymap! {
            "type" => s("core-metrics-extractor"),
            "parameters" => ymap! {
                "defaultEngine" => s("vllm"),
                "engineConfigs" => seq(vec![ymap! {
                    "name" => s("vllm"),
                    "queuedRequestsSpec" => s("furiosa_llm_num_requests_waiting"),
                    "runningRequestsSpec" => s("furiosa_llm_num_requests_running"),
                    "kvUsageSpec" => s("furiosa_llm_kv_cache_usage_percent"),
                    "loraSpec" => s(""),
                    "cacheInfoSpec" => s("furiosa_llm_cache_config_info"),
                }]),
            },
        }
    } else {
        // vLLM/RBLN expose `vllm:*`, which the extractor's defaults already read.
        ymap! { "type" => s("core-metrics-extractor") }
    });

    ymap! {
        "apiVersion" => s("inference.networking.x-k8s.io/v1alpha1"),
        "kind" => s("EndpointPickerConfig"),
        "plugins" => seq(plugins),
        "schedulingProfiles" => seq(vec![ymap! {
            "name" => s("default"),
            "plugins" => seq(
                SCORERS
                    .iter()
                    .map(|(name, weight)| ymap! {
                        "pluginRef" => s(*name),
                        "weight" => Value::from(*weight),
                    })
                    .collect(),
            ),
        }]),
    }
}

/// Build the routing documents for serving deployment `name`.
pub fn routing_docs(ns: &str, name: &str, vendor: &str, served: &str) -> Manifest {
    let epp = format!("{}-epp", name);
    let pool = format!("{}-pool", name);
    let slug = served
        .rsplit('/')
        .next()
        .unwrap_or(served)
        .to_lowercase()
        .replace(['.', '_'], "-");
    let path = format!("/{}/{}", accel_segment(vendor), slug);

    let sa_subject = seq(vec![ymap! {
        "kind" => s("ServiceAccount"),
        "name" => s(epp.clone()),
        "namespace" => s(ns),
    }]);
    let role_binding = |bind: &str, role: &str| {
        ymap! {
            "apiVersion" => s("rbac.authorization.k8s.io/v1"),
            "kind" => s("RoleBinding"),
            "metadata" => ymap! { "name" => s(bind), "namespace" => s(ns) },
            "roleRef" => ymap! {
                "apiGroup" => s("rbac.authorization.k8s.io"),
                "kind" => s("Role"),
                "name" => s(role),
            },
            "subjects" => sa_subject.clone(),
        }
    };
    let probe = |extra: Vec<(&str, Value)>| {
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            "grpc".into(),
            ymap! { "port" => Value::from(9003), "service" => s("inference-extension") },
        );
        for (k, v) in extra {
            m.insert(k.into(), v);
        }
        Value::Mapping(m)
    };
    let field_env = |name: &str, path: &str| {
        ymap! {
            "name" => s(name),
            "valueFrom" => ymap! { "fieldRef" => ymap! { "fieldPath" => s(path) } },
        }
    };
    let port = |n: &str, num: i64| ymap! { "name" => s(n), "containerPort" => Value::from(num) };

    let epp_cfg_text = serde_yaml::to_string(&epp_config(vendor))
        .unwrap_or_else(|e| format!("# ERROR: could not serialize EPP config: {}\n", e));

    Manifest::new()
        .note(format!(
            "llm-d routing: gateway {} → InferencePool({}) → EPP → this serving deployment",
            path, pool
        ))
        .note("(the shared Roles llmd-router-epp-sa/-non-sa are assumed to exist already)")
        .push(Doc::new(ymap! {
            "apiVersion" => s("v1"),
            "kind" => s("ServiceAccount"),
            "metadata" => ymap! { "name" => s(epp.clone()), "namespace" => s(ns) },
        }))
        .push(Doc::new(role_binding(
            &format!("{}-sa", epp),
            "llmd-router-epp-sa",
        )))
        .push(Doc::new(role_binding(
            &format!("{}-non-sa", epp),
            "llmd-router-epp-non-sa",
        )))
        .push(
            Doc::new(ymap! {
                "apiVersion" => s("rbac.authorization.k8s.io/v1"),
                "kind" => s("ClusterRoleBinding"),
                "metadata" => ymap! { "name" => s(format!("{}-auth-delegator", epp)) },
                "roleRef" => ymap! {
                    "apiGroup" => s("rbac.authorization.k8s.io"),
                    "kind" => s("ClusterRole"),
                    "name" => s("system:auth-delegator"),
                },
                "subjects" => sa_subject.clone(),
            })
            .note("EPP verifies InferencePool membership via TokenReview, so it needs auth delegation."),
        )
        .push(Doc::new(ymap! {
            "apiVersion" => s("v1"),
            "kind" => s("ConfigMap"),
            "metadata" => ymap! { "name" => s(epp.clone()), "namespace" => s(ns) },
            "data" => ymap! { "default-plugins.yaml" => s(epp_cfg_text) },
        }))
        .push(Doc::new(ymap! {
            "apiVersion" => s("apps/v1"),
            "kind" => s("Deployment"),
            "metadata" => ymap! { "name" => s(epp.clone()), "namespace" => s(ns) },
            "spec" => ymap! {
                "replicas" => Value::from(1),
                "selector" => ymap! { "matchLabels" => ymap! { "app" => s(epp.clone()) } },
                "template" => ymap! {
                    "metadata" => ymap! { "labels" => ymap! { "app" => s(epp.clone()) } },
                    "spec" => ymap! {
                        "serviceAccountName" => s(epp.clone()),
                        "containers" => seq(vec![ymap! {
                            "name" => s("epp"),
                            "image" => s(EPP_IMAGE),
                            "args" => args([
                                "--pool-name".to_string(), pool.clone(),
                                "--pool-namespace".to_string(), ns.to_string(),
                                "--pool-group".to_string(), "inference.networking.k8s.io".to_string(),
                                "--config-file".to_string(), "/config/default-plugins.yaml".to_string(),
                                "--zap-encoder".to_string(), "json".to_string(),
                                "--tracing=false".to_string(),
                            ]),
                            "ports" => seq(vec![
                                port("grpc", 9002),
                                port("grpc-health", 9003),
                                port("metrics", 9090),
                            ]),
                            "livenessProbe" => probe(vec![
                                ("initialDelaySeconds", Value::from(5)),
                                ("periodSeconds", Value::from(10)),
                            ]),
                            "readinessProbe" => probe(vec![("periodSeconds", Value::from(2))]),
                            "env" => seq(vec![
                                field_env("NAMESPACE", "metadata.namespace"),
                                field_env("POD_NAME", "metadata.name"),
                            ]),
                            "volumeMounts" => seq(vec![
                                ymap! { "name" => s("plugins"), "mountPath" => s("/config") },
                            ]),
                        }]),
                        "volumes" => seq(vec![ymap! {
                            "name" => s("plugins"),
                            "configMap" => ymap! { "name" => s(epp.clone()) },
                        }]),
                    },
                },
            },
        }))
        .push(Doc::new(ymap! {
            "apiVersion" => s("v1"),
            "kind" => s("Service"),
            "metadata" => ymap! { "name" => s(epp.clone()), "namespace" => s(ns) },
            "spec" => ymap! {
                "selector" => ymap! { "app" => s(epp.clone()) },
                "ports" => seq(vec![
                    ymap! {
                        "name" => s("grpc-ext-proc"),
                        "port" => Value::from(9002),
                        "targetPort" => Value::from(9002),
                    },
                    ymap! {
                        "name" => s("http-metrics"),
                        "port" => Value::from(9090),
                        "targetPort" => Value::from(9090),
                    },
                ]),
            },
        }))
        .push(Doc::new(ymap! {
            "apiVersion" => s("inference.networking.k8s.io/v1"),
            "kind" => s("InferencePool"),
            "metadata" => ymap! { "name" => s(pool.clone()), "namespace" => s(ns) },
            "spec" => ymap! {
                "selector" => ymap! { "matchLabels" => ymap! { "app" => s(name) } },
                "targetPorts" => seq(vec![ymap! { "number" => Value::from(8000) }]),
                "endpointPickerRef" => ymap! {
                    "group" => s(""),
                    "kind" => s("Service"),
                    "name" => s(epp.clone()),
                    "port" => ymap! { "number" => Value::from(9002) },
                    "failureMode" => s("FailClose"),
                },
            },
        }))
        .push(Doc::new(ymap! {
            "apiVersion" => s("gateway.networking.k8s.io/v1"),
            "kind" => s("HTTPRoute"),
            "metadata" => ymap! { "name" => s(format!("{}-route", name)), "namespace" => s(ns) },
            "spec" => ymap! {
                "parentRefs" => seq(vec![ymap! { "name" => s("llm-d-gateway") }]),
                "rules" => seq(vec![ymap! {
                    "matches" => seq(vec![ymap! {
                        "path" => ymap! { "type" => s("PathPrefix"), "value" => s(path.clone()) },
                    }]),
                    "filters" => seq(vec![ymap! {
                        "type" => s("URLRewrite"),
                        "urlRewrite" => ymap! {
                            "path" => ymap! {
                                "type" => s("ReplacePrefixMatch"),
                                "replacePrefixMatch" => s("/v1"),
                            },
                        },
                    }]),
                    "backendRefs" => seq(vec![ymap! {
                        "group" => s("inference.networking.k8s.io"),
                        "kind" => s("InferencePool"),
                        "name" => s(pool.clone()),
                    }]),
                }]),
            },
        }))
}
