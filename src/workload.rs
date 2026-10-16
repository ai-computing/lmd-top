//! Deployment classification from container runtime evidence, never deployment names.
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Model,
    Prefill,
    Decode,
    Gateway,
    Epp,
    Unknown,
}
impl Role {
    pub fn is_model(self) -> bool {
        matches!(self, Self::Model | Self::Prefill | Self::Decode)
    }
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "model" | "solo" => Self::Model,
            "prefill" => Self::Prefill,
            "decode" => Self::Decode,
            "gateway" => Self::Gateway,
            "epp" => Self::Epp,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Identity {
    pub name: String,
    pub role: Role,
    pub engine: String,
    pub container: String,
    pub evidence: String,
}

/// Heuristic evidence, not proof that a server is responding or a model has loaded.
pub fn container(c: &Value) -> (Role, &'static str, &'static str) {
    let command = ["command", "args"]
        .iter()
        .flat_map(|key| c[*key].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let image = c["image"].as_str().unwrap_or("").to_lowercase();
    let words: Vec<_> = command
        .split_whitespace()
        .map(|s| s.trim_matches(['\'', '"', ';', '\\']))
        .collect();
    let role = words
        .windows(2)
        .find(|w| w[0] == "--role")
        .and_then(|w| Role::parse(w[1]))
        .or_else(|| {
            words
                .iter()
                .find_map(|w| w.strip_prefix("--role=").and_then(Role::parse))
        });
    if command.contains("rngd_pd.serving.cluster") {
        return (
            role.unwrap_or(Role::Unknown),
            "PD worker",
            "rngd_pd runtime and explicit role",
        );
    }
    if image.contains("endpoint-picker") || image.contains("llm-d-inference-scheduler") {
        return (Role::Epp, "EPP", "endpoint picker image");
    }
    if command.contains("ds4-server") {
        return (Role::Model, "DS4", "ds4-server command");
    }
    if command.contains("llama-server") {
        return (Role::Model, "llama.cpp", "llama-server command");
    }
    let runtime = if command.contains("sglang.launch_server") || image.contains("sglang") {
        Some("SGLang")
    } else if command.contains("vllm.entrypoints")
        || words.windows(2).any(|w| w == ["vllm", "serve"])
        || image.contains("vllm-openai")
    {
        Some(if command.contains("rbln") || image.contains("rbln") {
            "vLLM-RBLN"
        } else {
            "vLLM"
        })
    } else if command.contains("furiosa-llm") || image.contains("furiosa-llm") {
        Some("Furiosa-LLM")
    } else if image.contains("ollama/ollama") || words.windows(2).any(|w| w == ["ollama", "serve"])
    {
        Some("Ollama")
    } else if image.contains("text-generation-inference")
        || command.contains("text-generation-launcher")
    {
        Some("TGI")
    } else {
        None
    };
    if let Some(engine) = runtime {
        return (
            Role::Model,
            engine,
            "recognized inference runtime command/image",
        );
    }
    if image.contains("envoy")
        || image.contains("llm-d-routing-sidecar")
        || words.iter().any(|w| w.ends_with("/proxy.py"))
    {
        return (Role::Gateway, "proxy", "proxy runtime command/image");
    }
    (
        Role::Unknown,
        "unknown",
        "no recognized inference runtime; explicit role annotation available",
    )
}

pub fn classify(d: &Value) -> Identity {
    let containers = d["spec"]["template"]["spec"]["containers"].as_array();
    let selected = containers
        .into_iter()
        .flatten()
        .rev()
        .map(|c| (c, container(c)))
        .max_by_key(|(_, (role, _, _))| {
            if role.is_model() {
                2
            } else if *role != Role::Unknown {
                1
            } else {
                0
            }
        });
    let (c, (mut role, engine, mut evidence)) =
        selected.unwrap_or((&Value::Null, (Role::Unknown, "unknown", "no containers")));
    // Explicit opt-in supports custom servers without guessing from their names or model mounts.
    for metadata in [&d["metadata"], &d["spec"]["template"]["metadata"]] {
        if let Some(explicit) = metadata["annotations"]["lmd-top.io/workload-role"]
            .as_str()
            .and_then(Role::parse)
        {
            role = explicit;
            evidence = "explicit lmd-top.io/workload-role annotation";
            break;
        }
    }
    Identity {
        name: d["metadata"]["name"].as_str().unwrap_or("").into(),
        role,
        engine: engine.into(),
        container: c["name"].as_str().unwrap_or("").into(),
        evidence: evidence.into(),
    }
}

pub fn report(workloads: &[Identity]) -> String {
    let mut lines =
        vec!["Workload classification (runtime evidence, not a model health check)".into()];
    for w in workloads {
        lines.push(format!(
            "\n{} · {:?} · {} · container {}\n  {}",
            w.name, w.role, w.engine, w.container, w.evidence
        ));
    }
    if workloads.is_empty() {
        lines.push("No deployment observations yet.".into());
    }
    lines.push("\nCustom runtimes: annotate the Deployment with lmd-top.io/workload-role=model, prefill, or decode.\nNon-model roles: gateway, epp, unknown. Unknown workloads remain listed here and in JSON.\nReady replicas indicate Kubernetes readiness, not proof of loaded model or successful inference.".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn deployment(name: &str, containers: Value) -> Value {
        json!({"metadata":{"name":name},"spec":{"template":{"spec":{"containers":containers}}}})
    }
    #[test]
    fn proxy_names_and_model_environment_do_not_prove_inference() {
        let d = deployment(
            "vllm-model-server",
            json!([{"name":"server","image":"python:3.11", "command":["python3","/app/proxy.py"],"env":[{"name":"MODEL","value":"org/model"}]}]),
        );
        assert_eq!(classify(&d).role, Role::Gateway);
        assert_eq!(
            classify(&deployment(
                "epp-model",
                json!([{"image":"vllm/vllm-openai:tag"}])
            ))
            .role,
            Role::Model
        );
    }
    #[test]
    fn sidecar_does_not_hide_model_and_image_entrypoint_is_recognized() {
        let d = deployment(
            "any-name",
            json!([{"name":"proxy","image":"envoyproxy/envoy"},{"name":"server","image":"vllm/vllm-openai:tag","args":["--model=/models/m"]}]),
        );
        let identity = classify(&d);
        assert_eq!(identity.engine, "vLLM");
        assert_eq!(identity.container, "server");
    }
    #[test]
    fn pd_gateway_and_workers_are_distinct() {
        for (role, expected) in [
            ("gateway", Role::Gateway),
            ("prefill", Role::Prefill),
            ("decode", Role::Decode),
        ] {
            let d = deployment(
                "arbitrary",
                json!([{"command":["bash","-c",format!("exec python -m rngd_pd.serving.cluster --role {role}")]}]),
            );
            assert_eq!(classify(&d).role, expected);
        }
    }
    #[test]
    fn unknown_stays_visible_and_custom_runtime_can_opt_in() {
        let mut d = deployment(
            "my-server",
            json!([{"image":"custom/image", "args":["--model","org/model"]}]),
        );
        assert_eq!(classify(&d).role, Role::Unknown);
        d["metadata"]["annotations"] = json!({"lmd-top.io/workload-role":"model"});
        assert_eq!(classify(&d).role, Role::Model);
    }
}
