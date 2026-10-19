//! LoRA training Job — `--plan train`.
//!
//! The trainer is npu-lora (upstream: ai-computing/npu-train, package `npu-lora`), vendored into
//! `assets/npu-lora/` by its `scripts/vendor-into-lmd-top.sh` and embedded here, so this
//! binary alone can produce a runnable Job: the official furiosa-llm image already carries
//! furiosa-torch, torch and transformers, and the package rides along as a ConfigMap.
//!
//! The objects mirror `npu_lora/k8s.py` one for one — ConfigMap (source), Job, Service and a
//! ServiceMonitor so the cluster Prometheus scrapes `npu_lora_*` from the running trainer.

use crate::manifest::{args, env_val, mount, s, seq, Doc, Manifest};
use crate::quote::valid_model_id;
use crate::ymap;
use serde_yaml::{Mapping, Value};

/// Vendored npu-lora sources, mounted as the `npu_lora` package. Order is the ConfigMap's.
pub const SOURCES: &[(&str, &str)] = &[
    (
        "__init__.py",
        include_str!("../../assets/npu-lora/__init__.py"),
    ),
    (
        "backends.py",
        include_str!("../../assets/npu-lora/backends.py"),
    ),
    ("bench.py", include_str!("../../assets/npu-lora/bench.py")),
    ("cli.py", include_str!("../../assets/npu-lora/cli.py")),
    (
        "compare.py",
        include_str!("../../assets/npu-lora/compare.py"),
    ),
    ("data.py", include_str!("../../assets/npu-lora/data.py")),
    (
        "diagnose.py",
        include_str!("../../assets/npu-lora/diagnose.py"),
    ),
    ("export.py", include_str!("../../assets/npu-lora/export.py")),
    ("graph.py", include_str!("../../assets/npu-lora/graph.py")),
    ("k8s.py", include_str!("../../assets/npu-lora/k8s.py")),
    ("llama.py", include_str!("../../assets/npu-lora/llama.py")),
    (
        "metrics.py",
        include_str!("../../assets/npu-lora/metrics.py"),
    ),
    (
        "trainer.py",
        include_str!("../../assets/npu-lora/trainer.py"),
    ),
];
pub const NPU_LORA_VERSION: &str = include_str!("../../assets/npu-lora/VERSION");

const IMAGE: &str = crate::accel::furiosa::DEFAULT_IMAGE;
const STORE: &str = "/mnt/store";
const STORE_PVC: &str = "model-store";
const METRICS_PORT: u16 = 9400;
const TARGETS: &[&str] = &["q", "k", "v", "o"];

/// Every `--set` key `--plan train` accepts. Anything else is rejected, so a typo cannot
/// silently fall back to a default.
pub const KEYS: &[&str] = &[
    "data",
    "name",
    "steps",
    "seq_len",
    "rank",
    "alpha",
    "targets",
    "lr",
    "accum",
    "node",
    "hf_host_path",
    "cache_host_path",
    "image",
    "merge",
    "full",
];

#[derive(Debug, Clone, PartialEq)]
pub struct TrainForm {
    pub model: String,
    pub data: String,
    pub run: String,
    pub steps: u32,
    pub seq_len: u32,
    pub rank: u32,
    pub alpha: f64,
    pub targets: String,
    pub lr: f64,
    pub accum: u32,
    pub node: Option<String>,
    pub hf_host_path: Option<String>,
    pub cache_host_path: Option<String>,
    pub image: String,
    pub merge: bool,
    /// Full fine-tuning: every decoder-layer weight is trained and the output is a whole HF model
    /// rather than a PEFT adapter. Needs the model to fit one RNGD (≈1-3B).
    pub full: bool,
}

/// Same rule as npu-lora's `_name`: lowercase, runs of anything outside [a-z0-9-] become one
/// `-`, trimmed, at most 52 so `npu-lora-<run>-src` stays a valid DNS-1123 name.
pub fn run_name(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let t: String = out.trim_matches('-').chars().take(52).collect();
    let t = t.trim_end_matches('-').to_string();
    if t.is_empty() {
        "run".into()
    } else {
        t
    }
}

fn abs_path(key: &str, v: &str) -> Result<String, String> {
    if v.starts_with('/') && !v.contains("..") && !v.chars().any(char::is_whitespace) {
        Ok(v.to_string())
    } else {
        Err(format!(
            "{key} must be an absolute path without spaces or '..' (got {v:?})"
        ))
    }
}

fn num<T: std::str::FromStr>(key: &str, v: &str) -> Result<T, String> {
    v.parse().map_err(|_| format!("{key}: not a number: {v:?}"))
}

impl TrainForm {
    pub fn from_overrides(model: &str, overrides: &[(String, String)]) -> Result<Self, String> {
        if !model.contains('/') || !valid_model_id(model) {
            return Err(format!("not a Hugging Face model id (org/name): {model:?}"));
        }
        for (k, _) in overrides {
            if !KEYS.contains(&k.as_str()) {
                return Err(format!(
                    "unknown --set {k} for train; expected one of {}",
                    KEYS.join(", ")
                ));
            }
        }
        let get = |k: &str| {
            overrides
                .iter()
                .rev()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.as_str())
        };
        let data = abs_path(
            "data",
            get("data").ok_or("--plan train needs --set data=/mnt/store/datasets/<file>.jsonl")?,
        )?;
        let short = model.rsplit('/').next().unwrap_or(model);
        let run = run_name(get("name").unwrap_or(&format!("lora-{short}")));
        let targets = get("targets").unwrap_or("q,v").to_string();
        if targets.split(',').any(|t| !TARGETS.contains(&t)) || targets.is_empty() {
            return Err(format!("targets: comma list of q,k,v,o (got {targets:?})"));
        }
        let f = TrainForm {
            model: model.to_string(),
            data,
            run,
            steps: num("steps", get("steps").unwrap_or("100"))?,
            seq_len: num("seq_len", get("seq_len").unwrap_or("512"))?,
            rank: num("rank", get("rank").unwrap_or("16"))?,
            alpha: num("alpha", get("alpha").unwrap_or("32"))?,
            targets,
            lr: num("lr", get("lr").unwrap_or("2e-4"))?,
            accum: num("accum", get("accum").unwrap_or("1"))?,
            node: get("node").map(str::to_string),
            hf_host_path: get("hf_host_path")
                .map(|v| abs_path("hf_host_path", v))
                .transpose()?,
            cache_host_path: get("cache_host_path")
                .map(|v| abs_path("cache_host_path", v))
                .transpose()?,
            image: get("image").unwrap_or(IMAGE).to_string(),
            merge: matches!(get("merge"), Some("1" | "true" | "yes")),
            full: matches!(get("full"), Some("1" | "true" | "yes")),
        };
        if f.steps == 0 || f.rank == 0 || f.accum == 0 || !(64..=8192).contains(&f.seq_len) {
            return Err("steps, rank, accum must be > 0 and seq_len within 64..=8192".into());
        }
        if !(f.lr > 0.0 && f.lr < 1.0) || f.alpha <= 0.0 {
            return Err("lr must be in (0, 1) and alpha > 0".into());
        }
        Ok(f)
    }

    pub fn app(&self) -> String {
        format!("npu-lora-{}", self.run)
    }

    pub fn out_dir(&self) -> String {
        format!(
            "{}/adapters/{}/{}",
            STORE,
            self.model.replace('/', "--"),
            self.run
        )
    }

    fn command(&self) -> Vec<String> {
        let out = self.out_dir();
        let mut c: Vec<String> = [
            "python3",
            "-m",
            "npu_lora.cli",
            "train",
            "--model",
            &self.model,
            "--data",
            &self.data,
            "--out",
            &out,
            "--backend",
            "furiosa:auto",
        ]
        .iter()
        .map(|x| x.to_string())
        .collect();
        for (k, v) in [
            ("--steps", self.steps.to_string()),
            ("--seq-len", self.seq_len.to_string()),
            ("--rank", self.rank.to_string()),
            ("--alpha", self.alpha.to_string()),
            ("--targets", self.targets.clone()),
            ("--lr", self.lr.to_string()),
            ("--accum", self.accum.to_string()),
            ("--run-name", self.run.clone()),
            ("--metrics-port", METRICS_PORT.to_string()),
            ("--log-file", format!("{out}/train.jsonl")),
            ("--cache-dir", "/cache/furiosa".to_string()),
        ] {
            c.push(k.into());
            c.push(v);
        }
        if self.full {
            c.push("--full".into());
        }
        if self.merge && !self.full {
            c.push("--merge-out".into());
            c.push(format!("{out}/merged"));
        }
        c
    }
}

fn labels(f: &TrainForm) -> Value {
    ymap! {
        "app" => s(f.app()),
        "app.kubernetes.io/part-of" => s("npu-lora"),
        "npu-lora/run" => s(f.run.clone()),
    }
}

/// Build the manifest. Returns (title, yaml).
pub fn build(f: &TrainForm, ns: &str) -> (String, String) {
    let app = f.app();
    let out = f.out_dir();

    let mut src = Mapping::new();
    for (name, body) in SOURCES {
        src.insert(s(*name), s(*body));
    }
    let cm = ymap! {
        "apiVersion" => s("v1"), "kind" => s("ConfigMap"),
        "metadata" => ymap! { "name" => s(format!("{app}-src")), "namespace" => s(ns), "labels" => labels(f) },
        "data" => Value::Mapping(src),
    };

    let mut env = vec![
        env_val("PYTHONPATH", "/opt/npu-lora"),
        env_val("NPU_LORA_RNGD_FILE", "/run/npu-lora/rngd"),
        env_val("PYTHONUNBUFFERED", "1"),
        env_val("HF_HUB_OFFLINE", "1"),
    ];
    let mut volumes = vec![
        ymap! { "name" => s("rngd"), "emptyDir" => Value::Mapping(Mapping::new()) },
        ymap! { "name" => s("src"), "configMap" => ymap! { "name" => s(format!("{app}-src")) } },
        ymap! { "name" => s("store"), "persistentVolumeClaim" => ymap! { "claimName" => s(STORE_PVC) } },
    ];
    let mut mounts = vec![
        mount("rngd", "/run/npu-lora", false),
        mount("src", "/opt/npu-lora/npu_lora", true),
        mount("store", STORE, false),
    ];
    match &f.hf_host_path {
        Some(p) => {
            volumes.push(ymap! { "name" => s("hf"),
            "hostPath" => ymap! { "path" => s(p.clone()), "type" => s("Directory") } });
            mounts.push(mount("hf", "/hf", true));
            env.push(env_val("HF_HOME", "/hf"));
        }
        None => env.push(env_val("HF_HOME", format!("{STORE}/hub"))),
    }
    match &f.cache_host_path {
        Some(p) => {
            volumes.push(ymap! { "name" => s("cache"),
            "hostPath" => ymap! { "path" => s(p.clone()), "type" => s("DirectoryOrCreate") } });
            mounts.push(mount("cache", "/cache", false));
        }
        None => {
            volumes.push(ymap! { "name" => s("cache"),
            "persistentVolumeClaim" => ymap! { "claimName" => s(STORE_PVC) } });
            mounts.push(ymap! { "name" => s("cache"), "mountPath" => s("/cache"), "subPath" => s("npu-lora-cache") });
        }
    }
    env.push(env_val("OUT", out.clone()));

    // The official furiosa-llm image serves prebuilt artifacts and lacks the aarch64 cross
    // toolchain the compiler shells out to; without it the first compile fails with
    // "No such file or directory".
    let toolchain = "command -v aarch64-linux-gnu-gcc >/dev/null || (apt-get update -qq && \
                     DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
                     gcc-aarch64-linux-gnu libc6-dev-arm64-cross libc6-dev)";
    let mut command = vec![
        "bash".to_string(),
        "-c".into(),
        format!("{toolchain} && mkdir -p \"$OUT\" && exec \"$@\""),
        "--".into(),
    ];
    command.extend(f.command());

    let container = ymap! {
        "name" => s("train"), "image" => s(f.image.clone()), "imagePullPolicy" => s("IfNotPresent"),
        "command" => args(command),
        "env" => seq(env),
        "ports" => seq(vec![ymap! { "name" => s("metrics"), "containerPort" => Value::from(METRICS_PORT) }]),
        "securityContext" => ymap! { "privileged" => Value::Bool(true) },
        "resources" => ymap! {
            "limits" => ymap! { "furiosa.ai/rngd" => Value::from(1) },
            "requests" => ymap! { "cpu" => s("8"), "memory" => s("64Gi") },
        },
        "volumeMounts" => seq(mounts),
    };
    let mut pod = Mapping::new();
    pod.insert("restartPolicy".into(), s("Never"));
    // furiosa-torch opens a card only from a privileged container, which then sees every RNGD on
    // the node. A non-privileged init container records the card the device plugin assigned, and
    // the trainer maps that name to furiosa-torch's ordinal, so Kubernetes' accounting still holds.
    let detect = "ls /dev/rngd | grep -oE '^npu[0-9]+' | sort -u | head -1 > /run/npu-lora/rngd \
                  && echo assigned $(cat /run/npu-lora/rngd)";
    let init = ymap! {
        "name" => s("assigned-rngd"), "image" => s("busybox:1.36"), "imagePullPolicy" => s("IfNotPresent"),
        "command" => args(["sh", "-c", detect]),
        "resources" => ymap! { "limits" => ymap! { "furiosa.ai/rngd" => Value::from(1) } },
        "volumeMounts" => seq(vec![mount("rngd", "/run/npu-lora", false)]),
    };
    pod.insert("initContainers".into(), seq(vec![init]));
    pod.insert("containers".into(), seq(vec![container]));
    pod.insert("volumes".into(), seq(volumes));
    if let Some(n) = &f.node {
        pod.insert(
            "nodeSelector".into(),
            ymap! { "kubernetes.io/hostname" => s(n.clone()) },
        );
    }
    let job = ymap! {
        "apiVersion" => s("batch/v1"), "kind" => s("Job"),
        "metadata" => ymap! {
            "name" => s(app.clone()), "namespace" => s(ns), "labels" => labels(f),
            "annotations" => ymap! {
                "npu-lora/model" => s(f.model.clone()), "npu-lora/data" => s(f.data.clone()), "npu-lora/out" => s(out.clone()),
            },
        },
        "spec" => ymap! {
            "backoffLimit" => Value::from(0), "ttlSecondsAfterFinished" => Value::from(86400),
            "template" => ymap! { "metadata" => ymap! { "labels" => labels(f) }, "spec" => Value::Mapping(pod) },
        },
    };
    let svc = ymap! {
        "apiVersion" => s("v1"), "kind" => s("Service"),
        "metadata" => ymap! { "name" => s(app.clone()), "namespace" => s(ns), "labels" => labels(f) },
        "spec" => ymap! {
            "selector" => ymap! { "app" => s(app.clone()) },
            "ports" => seq(vec![ymap! { "name" => s("metrics"), "port" => Value::from(METRICS_PORT) }]),
        },
    };
    let mut sm_labels = Mapping::new();
    sm_labels.insert("release".into(), s("kube-prometheus-stack"));
    if let Value::Mapping(l) = labels(f) {
        sm_labels.extend(l);
    }
    let sm = ymap! {
        "apiVersion" => s("monitoring.coreos.com/v1"), "kind" => s("ServiceMonitor"),
        // Same namespace as the Job: lmd-top applies a plan into one namespace, and this
        // cluster's Prometheus selects ServiceMonitors from every namespace.
        "metadata" => ymap! { "name" => s(app.clone()), "namespace" => s(ns), "labels" => Value::Mapping(sm_labels) },
        "spec" => ymap! {
            "namespaceSelector" => ymap! { "matchNames" => seq(vec![s(ns)]) },
            "selector" => ymap! { "matchLabels" => ymap! { "app" => s(app.clone()) } },
            "endpoints" => seq(vec![ymap! { "port" => s("metrics"), "path" => s("/metrics"), "interval" => s("15s") }]),
        },
    };

    let title = format!(
        "{} train {} on RNGD ({} steps)",
        if f.full { "Full" } else { "LoRA" },
        f.model,
        f.steps
    );
    let yaml = Manifest::new()
        .note(format!("npu-lora {} — LoRA fine-tuning on FuriosaAI RNGD", NPU_LORA_VERSION.trim()))
        .note(format!("model {}  data {}  r={} alpha={} targets={}", f.model, f.data, f.rank, f.alpha, f.targets))
        .note(format!("adapter (PEFT) -> {out}; progress: kubectl logs job/{app}, metrics npu_lora_*"))
        .push(Doc::new(cm).note("npu-lora package source, mounted at /opt/npu-lora/npu_lora"))
        .push(Doc::new(job).note("first run compiles the layer and head graphs part by part (~1-2 h for Llama-8B); later runs load them from /cache"))
        .push(Doc::new(svc))
        .push(Doc::new(sm))
        .to_yaml();
    (title, yaml)
}

/// `--plan train` entry.
pub fn plan(
    model: &str,
    vendor: &str,
    overrides: &[(String, String)],
    ns: &str,
) -> Result<(String, String), String> {
    if vendor != "furiosa" {
        return Err(format!(
            "LoRA training runs on furiosa (RNGD) for now; '{vendor}' has no npu-lora backend yet. \
             GPUs can run the same trainer directly: `npu-lora train --backend cuda:0`."
        ));
    }
    let f = TrainForm::from_overrides(model, overrides)?;
    Ok(build(&f, ns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn run_name_matches_npu_lora_rule() {
        assert_eq!(run_name("Demo Run 1"), "demo-run-1");
        assert_eq!(run_name("a__b..c"), "a-b-c");
        assert_eq!(run_name("!!!"), "run");
        assert!(run_name(&"x".repeat(80)).len() <= 52);
    }

    #[test]
    fn plan_builds_four_objects_with_vendored_source() {
        let (_, y) = plan(
            "meta-llama/Llama-3.1-8B-Instruct",
            "furiosa",
            &kv(&[
                ("data", "/mnt/store/datasets/d.jsonl"),
                ("name", "Demo Run"),
                ("steps", "7"),
            ]),
            "llm-serving",
        )
        .unwrap();
        let docs: Vec<Value> = serde_yaml::Deserializer::from_str(&y)
            .map(|d| Value::deserialize(d).unwrap())
            .collect();
        let kinds: Vec<&str> = docs.iter().map(|d| d["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["ConfigMap", "Job", "Service", "ServiceMonitor"]);
        let data = docs[0]["data"].as_mapping().unwrap();
        assert_eq!(data.len(), SOURCES.len());
        assert!(data.contains_key("trainer.py"));
        let cmd: Vec<&str> = docs[1]["spec"]["template"]["spec"]["containers"][0]["command"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(cmd.windows(2).any(|w| w == ["--steps", "7"]));
        assert!(!cmd.contains(&"--full"));
        assert!(cmd.contains(&"/mnt/store/adapters/meta-llama--Llama-3.1-8B-Instruct/demo-run"));
        assert_eq!(
            docs[1]["metadata"]["name"].as_str(),
            Some("npu-lora-demo-run")
        );
    }

    #[test]
    fn every_vendored_module_is_embedded() {
        // vendor-into-lmd-top.sh copies whatever npu_lora has; a module it adds but SOURCES
        // misses would ship a ConfigMap whose package fails to import inside the Job.
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/npu-lora");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".py"))
            .collect();
        on_disk.sort();
        let mut embedded: Vec<String> = SOURCES.iter().map(|(n, _)| n.to_string()).collect();
        embedded.sort();
        assert_eq!(on_disk, embedded);
    }

    #[test]
    fn rejects_bad_input() {
        let m = "meta-llama/Llama-3.1-8B-Instruct";
        assert!(plan(m, "rbln", &kv(&[("data", "/d.jsonl")]), "ns").is_err());
        assert!(plan(m, "furiosa", &[], "ns").unwrap_err().contains("data"));
        assert!(plan(m, "furiosa", &kv(&[("data", "rel.jsonl")]), "ns").is_err());
        assert!(plan(
            m,
            "furiosa",
            &kv(&[("data", "/d.jsonl"), ("stepz", "1")]),
            "ns"
        )
        .unwrap_err()
        .contains("unknown"));
        assert!(plan(
            m,
            "furiosa",
            &kv(&[("data", "/d.jsonl"), ("targets", "q,x")]),
            "ns"
        )
        .is_err());
        assert!(plan("not a model", "furiosa", &kv(&[("data", "/d.jsonl")]), "ns").is_err());
        // a value that would break YAML or a shell if spliced in is just data here
        let (_, y) = plan(m, "furiosa", &kv(&[("data", "/d\";rm.jsonl")]), "ns").unwrap();
        assert!(y.contains("rm.jsonl"));
    }
}
