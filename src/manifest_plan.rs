//! Pure manifest planning and preflight validation, independent of kubectl.
use anyhow::{anyhow, Result};

/// Immutable preparation bound to the namespace and exact reviewed source.
#[derive(Clone)]
pub struct Reviewed {
    namespace: String,
    source: String,
    steps: Vec<(&'static str, String)>,
}
impl Reviewed {
    pub(crate) fn new(namespace: &str, source: &str, steps: Vec<(&'static str, String)>) -> Self {
        Self {
            namespace: namespace.into(),
            source: source.into(),
            steps,
        }
    }
    pub fn steps(&self, namespace: &str, source: &str) -> Result<&[(&'static str, String)]> {
        if namespace != self.namespace || source != self.source {
            return Err(anyhow!(
                "manifest changed after review; prepare and review again"
            ));
        }
        Ok(&self.steps)
    }
    pub fn targets(&self) -> Vec<serde_json::Value> {
        self.steps.iter().filter_map(|(verb, body)| {
            let value: serde_json::Value = serde_yaml::from_str(body).ok()?;
            Some(serde_json::json!({"operation":verb,"api_version":value["apiVersion"],"kind":value["kind"],"namespace":value["metadata"]["namespace"],"default_namespace":self.namespace,"name":value["metadata"]["name"],"generate_name":value["metadata"]["generateName"],"uid":value["metadata"]["uid"],"resource_version":value["metadata"]["resourceVersion"]}))
        }).collect()
    }
}

/// Plan every YAML document before doing I/O. Flatten Lists so embedded Jobs
/// receive the same create-only semantics as standalone Jobs.
pub fn plan(namespace: &str, yaml: &str) -> Result<Vec<(&'static str, String)>> {
    use serde::Deserialize;
    fn append(
        value: serde_yaml::Value,
        depth: usize,
        steps: &mut Vec<(&'static str, String)>,
    ) -> Result<()> {
        if depth > 16 {
            return Err(anyhow!("manifest List nesting exceeds 16 levels"));
        }
        if value.is_null() {
            return Ok(());
        }
        let kind = value
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("manifest resource requires kind"))?;
        let api = value
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("manifest resource requires apiVersion"))?;
        if kind.ends_with("List") {
            let items = value
                .get("items")
                .and_then(|v| v.as_sequence())
                .ok_or_else(|| anyhow!("manifest List requires items array"))?;
            for item in items {
                let mut item = item.clone();
                if kind != "List" {
                    let map = item
                        .as_mapping_mut()
                        .ok_or_else(|| anyhow!("typed List item must be an object"))?;
                    map.entry(serde_yaml::Value::String("kind".into()))
                        .or_insert_with(|| {
                            serde_yaml::Value::String(kind.trim_end_matches("List").into())
                        });
                    map.entry(serde_yaml::Value::String("apiVersion".into()))
                        .or_insert_with(|| serde_yaml::Value::String(api.into()));
                }
                append(item, depth + 1, steps)?;
            }
        } else {
            // Only the Kubernetes batch Job has execution/create-only semantics.
            let verb = if kind == "Job" && api.starts_with("batch/") {
                "create"
            } else {
                "apply"
            };
            steps.push((verb, serde_yaml::to_string(&value)?));
        }
        Ok(())
    }
    let mut steps = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        append(serde_yaml::Value::deserialize(doc)?, 0, &mut steps)?;
    }
    if steps.is_empty() {
        return Err(anyhow!("manifest contains no resources"));
    }
    let mut targets = std::collections::BTreeSet::new();
    for (_, body) in &steps {
        let value: serde_yaml::Value = serde_yaml::from_str(body)?;
        let Some(name) = value["metadata"]["name"]
            .as_str()
            .filter(|name| !name.is_empty())
        else {
            continue; // generateName, if supported, allocates distinct identities server-side.
        };
        let kind = value["kind"].as_str().unwrap();
        let api = value["apiVersion"].as_str().unwrap();
        let group = api.split_once('/').map_or("", |(group, _)| group);
        let ns = value["metadata"]["namespace"]
            .as_str()
            .filter(|ns| !ns.is_empty())
            .unwrap_or(namespace);
        if !targets.insert((
            group.to_string(),
            kind.to_string(),
            ns.to_string(),
            name.to_string(),
        )) {
            return Err(anyhow!("duplicate manifest target {group}/{kind} {ns}/{name}; include each target only once"));
        }
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_targets_are_rejected_across_versions_lists_and_default_namespace() {
        let first = serde_json::json!({"apiVersion":"batch/v1", "kind":"Job", "metadata":{"name":"compile"}});
        let duplicate = serde_json::json!({"apiVersion":"batch/v1beta1", "kind":"Job", "metadata":{"name":"compile", "namespace":"test-ns"}});
        let list = serde_json::json!({"apiVersion":"v1", "kind":"List", "items":[duplicate]});
        let yaml = format!("{first}\n---\n{list}");
        let error = plan("test-ns", &yaml).unwrap_err();
        assert!(error.to_string().contains("duplicate manifest target"));
        assert!(error.to_string().contains("test-ns/compile"));
        // Both real apply and validation stop in local planning, before kubectl.
        for dry in [true, false] {
            assert!(crate::kube::apply_manifest("test-ns", &yaml, dry)
                .unwrap_err()
                .to_string()
                .contains("duplicate manifest target"));
        }
    }

    #[test]
    fn distinct_targets_and_generated_names_do_not_collide() {
        let resources = [
            serde_json::json!({"apiVersion":"batch/v1", "kind":"Job", "metadata":{"name":"same"}}),
            serde_json::json!({"apiVersion":"v1", "kind":"ConfigMap", "metadata":{"name":"same"}}),
            serde_json::json!({"apiVersion":"example.test/v1", "kind":"Job", "metadata":{"name":"same"}}),
            serde_json::json!({"apiVersion":"batch/v1", "kind":"Job", "metadata":{"name":"same", "namespace":"another"}}),
            serde_json::json!({"apiVersion":"batch/v1", "kind":"Job", "metadata":{"generateName":"compile-"}}),
            serde_json::json!({"apiVersion":"batch/v1", "kind":"Job", "metadata":{"generateName":"compile-"}}),
        ];
        let yaml = resources
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert_eq!(plan("test-ns", &yaml).unwrap().len(), resources.len());
    }

    #[test]
    fn manifest_plans_jobs_as_create_and_preserves_document_order() {
        let yaml = "---\r\napiVersion: v1\r\nkind: ConfigMap\r\nmetadata: {name: script}\r\n--- # execution\r\napiVersion: batch/v1\r\nkind: Job\r\nmetadata: {name: build, namespace: other}\r\n";
        let steps = plan("test-ns", yaml).unwrap();
        assert_eq!(
            steps.iter().map(|s| s.0).collect::<Vec<_>>(),
            ["apply", "create"]
        );
        let job: serde_yaml::Value = serde_yaml::from_str(&steps[1].1).unwrap();
        assert_eq!(job["metadata"]["name"].as_str(), Some("build"));
        assert_eq!(job["metadata"]["namespace"].as_str(), Some("other"));
    }

    #[test]
    fn manifest_lists_cannot_bypass_job_create_semantics() {
        let steps = plan("test-ns", "apiVersion: v1\nkind: List\nitems:\n- apiVersion: batch/v1\n  kind: Job\n  metadata: {name: build}\n").unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].0, "create");
        let typed = plan(
            "test-ns",
            "apiVersion: batch/v1\nkind: JobList\nitems:\n- metadata: {name: build}\n",
        )
        .unwrap();
        assert_eq!(typed[0].0, "create");
        let item: serde_yaml::Value = serde_yaml::from_str(&typed[0].1).unwrap();
        assert_eq!(item["kind"].as_str(), Some("Job"));
        assert_eq!(item["apiVersion"].as_str(), Some("batch/v1"));
        for invalid in [
            "",
            "---\n# empty",
            "apiVersion: v1\nkind: List\nitems: {}",
            "kind: Job",
            "apiVersion: batch/v1\nkind: Job\n---\n[broken",
        ] {
            assert!(plan("test-ns", invalid).is_err(), "{invalid}");
        }
    }
}
