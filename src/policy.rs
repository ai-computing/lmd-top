//! Policy development and immutable EPP rollout planning. Cluster writes go through
//! the existing admin/confirmation/audit path; all functions here are read-only.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

pub const TYPE: &str = "heterogeneous-capacity";
pub const EXTRACTOR: &str = "heterogeneous-load-extractor";
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Class {
    pub name: String,
    pub selector: BTreeMap<String, String>,
    pub models: Vec<String>,
    pub capacity: f64,
    #[serde(default)]
    pub max_input_tokens: u32,
    pub waiting_metric: String,
    pub running_metric: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parameters {
    pub max_metric_age_seconds: f64,
    pub classes: Vec<Class>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub deployment: String,
    pub container: String,
    pub config_map: String,
    pub config_key: String,
    pub image: String,
    pub profile: String,
    pub metrics_source: String,
    pub weight: f64,
    pub parameters: Parameters,
}
fn nonempty(s: &str) -> bool {
    !s.trim().is_empty()
}
fn resource_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
}
impl Policy {
    pub fn read(path: &str) -> Result<Self> {
        let p: Self = serde_yaml::from_str(
            &std::fs::read_to_string(path).with_context(|| format!("read policy {path}"))?,
        )?;
        p.validate()?;
        Ok(p)
    }
    pub fn validate(&self) -> Result<()> {
        if !resource_name(&self.deployment)
            || !resource_name(&self.config_map)
            || !resource_name(&self.container)
        {
            bail!("deployment, container and configMap must be resource names");
        }
        if [
            &self.config_key,
            &self.image,
            &self.profile,
            &self.metrics_source,
        ]
        .iter()
        .any(|s| !nonempty(s))
        {
            bail!("configKey, image, profile and metricsSource are required");
        }
        if !self.weight.is_finite()
            || self.weight <= 0.0
            || !self.parameters.max_metric_age_seconds.is_finite()
            || self.parameters.max_metric_age_seconds <= 0.0
        {
            bail!("weight and maxMetricAgeSeconds must be finite and positive");
        }
        if self.parameters.classes.is_empty() {
            bail!("classes must not be empty");
        }
        let mut names = HashSet::new();
        for c in &self.parameters.classes {
            if !nonempty(&c.name)
                || !names.insert(&c.name)
                || !c.capacity.is_finite()
                || c.capacity <= 0.0
            {
                bail!("class names must be unique; capacity must be measured positive requests/s");
            }
            if c.selector.is_empty()
                || c.selector.iter().any(|(k, v)| !nonempty(k) || !nonempty(v))
                || c.models.is_empty()
                || c.models.iter().any(|m| !nonempty(m))
                || !nonempty(&c.waiting_metric)
                || !nonempty(&c.running_metric)
            {
                bail!(
                    "class {} needs selector, exact models and waiting/running metric names",
                    c.name
                );
            }
        }
        Ok(())
    }
    /// Merge into one explicit profile, retaining all unrelated plugins and config.
    pub fn render(&self, base: &str) -> Result<String> {
        self.validate()?;
        let mut doc: Value = serde_yaml::from_str(base)?;
        if doc["kind"] != "EndpointPickerConfig" {
            bail!("base must be an EndpointPickerConfig, not a Kubernetes manifest");
        }
        let plugins = doc
            .get_mut("plugins")
            .and_then(Value::as_array_mut)
            .context("base plugins must be a list")?;
        let plugin_name = |v: &Value| {
            v.get("name")
                .or_else(|| v.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let mut seen = HashSet::new();
        for p in plugins.iter() {
            if !seen.insert(plugin_name(p)) {
                bail!("duplicate plugin name in base");
            }
        }
        // Reserved plugin names may be updated only when the type is also ours.
        for ty in [TYPE, EXTRACTOR] {
            if let Some(p) = plugins.iter_mut().find(|p| plugin_name(p) == ty) {
                if p["type"] != ty {
                    bail!("reserved plugin name {ty} is already in use");
                }
                p["parameters"] = serde_json::to_value(&self.parameters)?;
            } else {
                plugins.push(json!({"type": ty, "parameters": self.parameters}));
            }
        }
        if !plugins
            .iter()
            .any(|p| plugin_name(p) == self.metrics_source && p["type"] == "metrics-data-source")
        {
            bail!("metricsSource must name an existing metrics-data-source plugin");
        }
        let profiles = doc
            .get_mut("schedulingProfiles")
            .and_then(Value::as_array_mut)
            .context("explicit schedulingProfiles required (export the active config first)")?;
        if profiles
            .iter()
            .filter(|p| p["name"] == self.profile)
            .count()
            != 1
        {
            bail!("profile must identify exactly one existing profile");
        }
        let profile = profiles
            .iter_mut()
            .find(|p| p["name"] == self.profile)
            .unwrap();
        let refs = profile
            .get_mut("plugins")
            .and_then(Value::as_array_mut)
            .context("profile plugins must be a list")?;
        refs.retain(|r| r["pluginRef"] != TYPE);
        refs.push(json!({"pluginRef": TYPE, "weight": self.weight}));
        if doc.get("dataLayer").is_none() {
            let core_present = doc["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| plugin_name(p) == "core-metrics-extractor");
            if !core_present {
                doc["plugins"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"type":"core-metrics-extractor"}));
            }
            doc["dataLayer"] = json!({"sources":[{"pluginRef": self.metrics_source,"extractors":[{"pluginRef":"core-metrics-extractor"}]}]});
        }
        let sources = doc["dataLayer"]
            .get_mut("sources")
            .and_then(Value::as_array_mut)
            .context("explicit dataLayer.sources must be a list")?;
        if sources
            .iter()
            .filter(|s| s["pluginRef"] == self.metrics_source)
            .count()
            != 1
        {
            bail!("metricsSource must occur exactly once in dataLayer.sources");
        }
        let source = sources
            .iter_mut()
            .find(|s| s["pluginRef"] == self.metrics_source)
            .unwrap();
        let extractors = source
            .get_mut("extractors")
            .and_then(Value::as_array_mut)
            .context("source extractors must be a list")?;
        if !extractors.iter().any(|r| r["pluginRef"] == EXTRACTOR) {
            extractors.push(json!({"pluginRef":EXTRACTOR}));
        }
        Ok(serde_yaml::to_string(&doc)?)
    }
    pub async fn plan(&self, ns: &str) -> Result<String> {
        self.validate()?;
        let deployment = crate::kube::get_json(&[
            "get",
            "deployment",
            &self.deployment,
            "-n",
            ns,
            "-o",
            "json",
        ])
        .await?;
        let cm =
            crate::kube::get_json(&["get", "configmap", &self.config_map, "-n", ns, "-o", "json"])
                .await?;
        self.plan_objects(ns, deployment, cm)
    }
    pub fn plan_objects(&self, ns: &str, mut deployment: Value, cm: Value) -> Result<String> {
        self.validate()?;
        if self.image.contains("TODO-")
            || self.image.ends_with(":latest")
            || !(self.image.contains('@')
                || self.image.rsplit('/').next().unwrap_or("").contains(':'))
        {
            bail!("set an explicit tested plugin image tag/digest (no TODO or latest)");
        }
        if deployment["kind"] != "Deployment"
            || deployment["metadata"]["name"] != self.deployment
            || deployment["metadata"]["namespace"] != ns
            || cm["kind"] != "ConfigMap"
            || cm["metadata"]["name"] != self.config_map
            || cm["metadata"]["namespace"] != ns
        {
            bail!("resource identity/namespace does not match policy target");
        }
        if deployment["metadata"]["resourceVersion"].as_str().is_none() {
            bail!("live deployment resourceVersion required");
        }
        let base = cm["data"][&self.config_key]
            .as_str()
            .context("configKey absent in ConfigMap data")?;
        let rendered = self.render(base)?;
        let containers = deployment["spec"]["template"]["spec"]["containers"]
            .as_array()
            .context("no containers")?;
        let ci = containers
            .iter()
            .position(|c| c["name"] == self.container)
            .context("container not found")?;
        let container = &containers[ci];
        let args: Vec<&str> = container["args"]
            .as_array()
            .context("EPP must have explicit --config-file args")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if args.iter().any(|a| a.starts_with("--config-text")) {
            bail!("inline --config-text is not supported for policy rollout");
        }
        let paths: Vec<&str> = args
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                a.strip_prefix("--config-file=").or_else(|| {
                    if *a == "--config-file" {
                        args.get(i + 1).copied()
                    } else {
                        None
                    }
                })
            })
            .collect();
        if paths.len() != 1 {
            bail!("exactly one --config-file argument required");
        }
        let config_path = paths[0];
        let mounts = container["volumeMounts"]
            .as_array()
            .context("no volume mounts")?;
        let volumes = deployment["spec"]["template"]["spec"]["volumes"]
            .as_array()
            .context("no volumes")?;
        let mut matches = Vec::new();
        for (vi, v) in volumes
            .iter()
            .enumerate()
            .filter(|(_, v)| v["configMap"]["name"] == self.config_map)
        {
            let relative = if let Some(items) = v["configMap"]["items"].as_array() {
                items
                    .iter()
                    .find(|i| i["key"] == self.config_key)
                    .and_then(|i| i["path"].as_str())
            } else {
                Some(self.config_key.as_str())
            };
            let Some(relative) = relative else { continue };
            for m in mounts.iter().filter(|m| m["name"] == v["name"]) {
                if m.get("subPathExpr").is_some() {
                    bail!("subPathExpr mounts are not supported");
                }
                let mount = m["mountPath"].as_str().unwrap_or("");
                let resolved = if let Some(sub) = m["subPath"].as_str() {
                    if sub != relative {
                        continue;
                    };
                    mount.to_string()
                } else {
                    format!("{}/{}", mount.trim_end_matches('/'), relative)
                };
                if resolved == config_path {
                    matches.push(vi);
                }
            }
        }
        if matches.len() != 1 {
            bail!("configMap/key must resolve unambiguously to this container's --config-file");
        }
        let mut data = cm["data"].clone();
        data[&self.config_key] = Value::String(rendered);
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&data)?.hash(&mut hash);
        cm["binaryData"].to_string().hash(&mut hash);
        let new_name = format!(
            "{}-policy-{:016x}",
            self.deployment.chars().take(40).collect::<String>(),
            hash.finish()
        );
        let mut new_cm = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":new_name,"namespace":ns},"immutable":true,"data":data});
        if let Some(binary) = cm.get("binaryData") {
            new_cm["binaryData"] = binary.clone();
        }
        deployment["spec"]["template"]["spec"]["volumes"][matches[0]]["configMap"]["name"] =
            json!(new_name);
        deployment["spec"]["template"]["spec"]["containers"][ci]["image"] = json!(self.image);
        deployment.as_object_mut().unwrap().remove("status");
        let meta = deployment["metadata"].as_object_mut().unwrap();
        for key in ["managedFields", "creationTimestamp", "generation", "uid"] {
            meta.remove(key);
        }
        if let Some(annotations) = meta.get_mut("annotations").and_then(Value::as_object_mut) {
            annotations.remove("kubectl.kubernetes.io/last-applied-configuration");
        }
        Ok(format!("# Policy rollout: {} / {}\n# Original ConfigMap {} retained. Rollback: kubectl -n {} rollout undo deployment/{}\n# Server dry-run checks Kubernetes schema, not plugin behavior. Test the image/config first.\n{}---\n{}",self.deployment,self.profile,self.config_map,ns,self.deployment,serde_yaml::to_string(&new_cm)?,serde_yaml::to_string(&deployment)?))
    }
}

pub fn scaffold(dir: &Path) -> Result<()> {
    // Refuse existing directories, even empty ones: never overwrite a developer workspace.
    std::fs::create_dir(dir).with_context(|| format!("create new workspace {}", dir.display()))?;
    for (name, content) in workspace_files() {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(())
}
fn workspace_files() -> Vec<(&'static str, &'static str)> {
    macro_rules! file {
        ($p:literal) => {
            (
                $p,
                include_str!(concat!("../contrib/epp-score-observer/", $p)),
            )
        };
    }
    vec![
        file!("go.mod"),
        file!("go.sum"),
        file!("main.go"),
        file!("Dockerfile"),
        file!("Makefile"),
        file!("pkg/heterogeneous/policy.go"),
        file!("pkg/heterogeneous/plugin.go"),
        file!("pkg/heterogeneous/extractor.go"),
        file!("pkg/heterogeneous/policy_test.go"),
        file!("pkg/heterogeneous/config_test.go"),
        file!("pkg/scoreobserver/picker.go"),
        file!("pkg/scoreobserver/picker_test.go"),
        file!("cmd/replay/main.go"),
        file!("examples/parameters.json"),
        file!("examples/requests.json"),
        file!("examples/base-epp.yaml"),
        file!("examples/rendered-epp.yaml"),
        file!("policy.yaml"),
        file!("DEVELOPMENT.md"),
        ("LICENSE", include_str!("../LICENSE")),
    ]
}

pub async fn cli(cfg: &crate::config::Config, args: &[String]) -> Result<()> {
    let get = |key| crate::arg_value(args, key);
    if args.iter().any(|a| a == "--apply") {
        bail!("policy CLI produces reviewable plans; use the TUI admin confirmation to apply");
    }
    let op = get("--policy").unwrap_or_default();
    let workspace = get("--workspace").unwrap_or_else(|| "policy-workspace".into());
    let path = get("--file").unwrap_or_else(|| "policy.yaml".into());
    match op.as_str() {
        "init" => {
            scaffold(Path::new(&workspace))?;
            println!("Created {workspace}. Edit policy.yaml; run --policy test/replay/build --workspace {workspace}.");
        }
        "validate" => {
            Policy::read(&path)?;
            println!("policy schema valid (image/plugin presence and inference are not verified)");
        }
        "render" => {
            let base =
                get("--base").context("render requires --base <EndpointPickerConfig.yaml>")?;
            println!(
                "{}",
                Policy::read(&path)?.render(&std::fs::read_to_string(base)?)?
            );
        }
        "plan" => {
            println!("{}", Policy::read(&path)?.plan(&cfg.ns).await?);
        }
        "test" | "replay" | "build" => {
            let mut cmd = tokio::process::Command::new("go");
            cmd.current_dir(&workspace).kill_on_drop(true);
            match op.as_str() {
                "test" => {
                    cmd.args(["test", "-mod=readonly", "./..."]);
                }
                "replay" => {
                    cmd.args([
                        "run",
                        "-mod=readonly",
                        "./cmd/replay",
                        "examples/parameters.json",
                        "examples/requests.json",
                    ]);
                }
                _ => {
                    std::fs::create_dir_all(Path::new(&workspace).join("bin"))?;
                    cmd.env("CGO_ENABLED", "0").args([
                        "build",
                        "-mod=readonly",
                        "-trimpath",
                        "-buildvcs=false",
                        "-o",
                        "bin/epp",
                        ".",
                    ]);
                }
            }
            if !cmd
                .status()
                .await
                .context("run Go developer command (install the Go version in go.mod)")?
                .success()
            {
                bail!("policy {op} failed");
            }
        }
        _ => bail!("--policy expects init|validate|render|plan|test|replay|build"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &str = include_str!("../contrib/epp-score-observer/examples/base-epp.yaml");
    fn policy() -> Policy {
        let mut p: Policy =
            serde_yaml::from_str(include_str!("../contrib/epp-score-observer/policy.yaml"))
                .unwrap();
        p.image = "registry.local/epp-policy:v1".into();
        p
    }
    fn objects(p: &Policy) -> (Value, Value) {
        (
            json!({"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":p.deployment,"namespace":"ns","resourceVersion":"42"},"spec":{"replicas":2,"template":{"spec":{
            "containers":[{"name":"sidecar","image":"sidecar:old"},{"name":p.container,"image":"epp:old","args":["--config-file","/config/default-plugins.yaml"],"volumeMounts":[{"name":"cfg","mountPath":"/config"}]}],
            "volumes":[{"name":"cfg","configMap":{"name":p.config_map}}]}}}}),
            json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":p.config_map,"namespace":"ns"},"data":{p.config_key.clone():BASE,"unrelated":"keep"}}),
        )
    }
    #[test]
    fn merge_preserves_other_profiles_and_is_idempotent() {
        let p = policy();
        let mut base: Value = serde_yaml::from_str(BASE).unwrap();
        base["schedulingProfiles"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"other","plugins":[{"pluginRef":"max-score-picker"}]}));
        base["plugins"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"custom-existing","parameters":{"vendor":"unchanged"}}));
        let original = base.clone();
        let rendered = p.render(&serde_yaml::to_string(&base).unwrap()).unwrap();
        let merged: Value = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(
            merged["schedulingProfiles"][1],
            original["schedulingProfiles"][1]
        );
        assert_eq!(merged["plugins"][3], original["plugins"][3]);
        assert_eq!(p.render(&rendered).unwrap(), rendered);
        let mut invalid = policy();
        invalid.profile = "absent".into();
        assert!(invalid.render(BASE).is_err());
    }
    #[test]
    fn rollout_pairs_new_config_and_image_and_retains_revision() {
        let p = policy();
        let (d, c) = objects(&p);
        let originals = (d.clone(), c.clone());
        let text = p.plan_objects("ns", d, c).unwrap();
        let docs: Vec<Value> = serde_yaml::Deserializer::from_str(&text)
            .map(|d| Value::deserialize(d).unwrap())
            .collect();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0]["immutable"], true);
        assert_ne!(docs[0]["metadata"]["name"], p.config_map);
        assert_eq!(docs[0]["data"]["unrelated"], "keep");
        assert_eq!(docs[1]["metadata"]["resourceVersion"], "42");
        assert_eq!(
            docs[1]["spec"]["template"]["spec"]["containers"][0]["image"],
            "sidecar:old"
        );
        assert_eq!(
            docs[1]["spec"]["template"]["spec"]["containers"][1]["image"],
            p.image
        );
        assert_eq!(
            docs[1]["spec"]["template"]["spec"]["volumes"][0]["configMap"]["name"],
            docs[0]["metadata"]["name"]
        );
        assert_eq!(
            text,
            p.plan_objects("ns", originals.0, originals.1).unwrap()
        );
    }
    #[test]
    fn wrong_mount_namespace_and_unpinned_image_are_rejected() {
        let mut p = policy();
        let (d, c) = objects(&p);
        assert!(p.plan_objects("other", d.clone(), c.clone()).is_err());
        let mut bad = d.clone();
        bad["spec"]["template"]["spec"]["containers"][1]["args"] =
            json!(["--config-file=/elsewhere/nope"]);
        assert!(p.plan_objects("ns", bad, c.clone()).is_err());
        p.image = "registry/epp:latest".into();
        assert!(p.plan_objects("ns", d, c).is_err());
    }
    #[test]
    fn subpath_and_renamed_key_resolve_to_the_actual_file() {
        let p = policy();
        let (mut d, c) = objects(&p);
        d["spec"]["template"]["spec"]["containers"][1]["args"] =
            json!(["--config-file=/etc/epp.yaml"]);
        d["spec"]["template"]["spec"]["containers"][1]["volumeMounts"] =
            json!([{"name":"cfg","mountPath":"/etc/epp.yaml","subPath":"renamed.yaml"}]);
        d["spec"]["template"]["spec"]["volumes"][0]["configMap"]["items"] =
            json!([{"key":p.config_key,"path":"renamed.yaml"}]);
        assert!(p.plan_objects("ns", d, c).is_ok());
    }
    #[test]
    fn bad_capacity_and_schema_typos_are_rejected() {
        let mut p = policy();
        p.parameters.classes[0].capacity = f64::NAN;
        assert!(p.validate().is_err());
        let text = include_str!("../contrib/epp-score-observer/policy.yaml")
            .replace("capacity:", "capcity:");
        assert!(serde_yaml::from_str::<Policy>(&text).is_err());
    }
    #[test]
    fn policy_preview_respects_overlay_precedence() {
        let mut a = crate::app::App::new();
        a.open_policy_form();
        assert_eq!(
            crate::ui::Overlay::top(&a),
            Some(crate::ui::Overlay::PolicyForm)
        );
        assert_eq!(a.mode, crate::app::Mode::Observe);
        assert!(!a.preview_apply);
        a.preview = Some(("preview".into(), "config".into()));
        assert_eq!(
            crate::ui::Overlay::top(&a),
            Some(crate::ui::Overlay::Preview)
        );
    }
}
