//! Optimistic preconditions captured from the object the operator reviewed.
use anyhow::{bail, Result};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub uid: String,
    pub version: String,
}
impl Identity {
    pub fn read(object: &Value) -> Option<Self> {
        let uid = object["metadata"]["uid"]
            .as_str()
            .filter(|s| !s.is_empty())?;
        let version = object["metadata"]["resourceVersion"]
            .as_str()
            .filter(|s| !s.is_empty())?;
        Some(Self {
            uid: uid.into(),
            version: version.into(),
        })
    }
    pub fn verify(&self, object: &Value) -> Result<()> {
        if Self::read(object).as_ref() != Some(self) {
            bail!("resource changed since confirmation was prepared; refresh and review again");
        }
        Ok(())
    }
    pub fn patch(&self, path: &str, value: Value) -> Value {
        json!([
            {"op":"test","path":"/metadata/uid","value":self.uid},
            {"op":"test","path":"/metadata/resourceVersion","value":self.version},
            {"op":"add","path":path,"value":value}
        ])
    }
}
pub fn required(expected: Option<&Identity>) -> Result<&Identity> {
    expected
        .ok_or_else(|| anyhow::anyhow!("resource identity unavailable; refresh and review again"))
}
pub fn cordon(expected: Option<&Identity>, on: bool) -> Result<Value> {
    Ok(required(expected)?.patch("/spec/unschedulable", json!(on)))
}
pub fn scale(expected: Option<&Identity>, replicas: i64) -> Result<Value> {
    if replicas < 0 {
        bail!("replicas must be non-negative");
    }
    Ok(required(expected)?.patch("/spec/replicas", json!(replicas)))
}
pub fn restart(expected: Option<&Identity>, current: &Value, timestamp: &str) -> Result<Value> {
    let expected = required(expected)?;
    expected.verify(current)?;
    let mut metadata = current["spec"]["template"]["metadata"].clone();
    if metadata.is_null() {
        metadata = json!({});
    }
    if !metadata.is_object() {
        bail!("invalid pod template metadata");
    }
    if metadata["annotations"].is_null() {
        metadata["annotations"] = json!({});
    }
    if !metadata["annotations"].is_object() {
        bail!("invalid pod template annotations");
    }
    metadata["annotations"]["kubectl.kubernetes.io/restartedAt"] = json!(timestamp);
    Ok(expected.patch("/spec/template/metadata", metadata))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cordon_and_uncordon_require_reviewed_identity() {
        assert!(cordon(None, true).is_err());
        let id = Identity {
            uid: "reviewed-node".into(),
            version: "42".into(),
        };
        for on in [true, false] {
            let patch = cordon(Some(&id), on).unwrap();
            assert_eq!(
                patch,
                json!([
                    {"op":"test", "path":"/metadata/uid", "value":"reviewed-node"},
                    {"op":"test", "path":"/metadata/resourceVersion", "value":"42"},
                    {"op":"add", "path":"/spec/unschedulable", "value":on}
                ])
            );
        }
        assert!(crate::kube::cordon("must-not-contact-cluster", true, None).is_err());
    }
    #[test]
    fn scale_tests_uid_and_version_before_change() {
        let id = Identity {
            uid: "original".into(),
            version: "12".into(),
        };
        let patch = scale(Some(&id), 0).unwrap();
        assert_eq!(
            patch[0],
            json!({"op":"test","path":"/metadata/uid","value":"original"})
        );
        assert_eq!(
            patch[1],
            json!({"op":"test","path":"/metadata/resourceVersion","value":"12"})
        );
        assert_eq!(patch[2]["value"], 0);
        assert!(scale(None, 1).is_err());
        assert!(scale(Some(&id), -1).is_err());
    }
    #[test]
    fn restart_preserves_template_fields_and_refuses_recreated_or_changed_resource() {
        let current = json!({"metadata":{"uid":"original","resourceVersion":"12"},"spec":{"template":{"metadata":{"labels":{"app":"server"},"annotations":{"keep":"value"}}}}});
        let id = Identity::read(&current).unwrap();
        let patch = restart(Some(&id), &current, "2026-09-28T00:00:00Z").unwrap();
        assert_eq!(patch[2]["value"]["labels"]["app"], "server");
        assert_eq!(patch[2]["value"]["annotations"]["keep"], "value");
        for (field, value) in [("uid", "replacement"), ("resourceVersion", "13")] {
            let mut changed = current.clone();
            changed["metadata"][field] = json!(value);
            assert!(restart(Some(&id), &changed, "now").is_err());
        }
    }
}

/// Fixed API paths and DeleteOptions, never a name-only delete fallback.
pub fn delete_request(
    ns: &str,
    kind: &str,
    name: &str,
    expected: Option<&Identity>,
) -> Result<(String, Value)> {
    let expected = required(expected)?;
    let safe = |s: &str| {
        !s.is_empty()
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
    };
    if !safe(ns) || !safe(name) {
        bail!("invalid deletion target");
    }
    let prefix = match kind {
        "pods" => "/api/v1",
        "jobs" => "/apis/batch/v1",
        _ => bail!("unsupported deletion resource"),
    };
    Ok((
        format!("{prefix}/namespaces/{ns}/{kind}/{name}"),
        json!({
            "apiVersion": "v1", "kind": "DeleteOptions",
            "preconditions": { "uid": expected.uid, "resourceVersion": expected.version },
            "propagationPolicy": "Background"
        }),
    ))
}

#[cfg(test)]
mod deletion_tests {
    use super::*;
    #[test]
    fn deletion_rejects_unreviewed_objects_and_unsafe_paths_before_io() {
        assert!(crate::kube::delete_pod("ns", "pod", None).is_err());
        assert!(crate::kube::delete_job("ns", "job", None).is_err());
        let identity = Identity {
            uid: "reviewed".into(),
            version: "42".into(),
        };
        for name in [
            "",
            "..",
            "pod/other",
            "pod?dryRun=All",
            "pod%2fother",
            "-pod",
        ] {
            assert!(delete_request("ns", "pods", name, Some(&identity)).is_err());
        }
        assert!(delete_request("ns/other", "pods", "pod", Some(&identity)).is_err());
        assert!(delete_request("ns", "deployments", "pod", Some(&identity)).is_err());
    }
}
