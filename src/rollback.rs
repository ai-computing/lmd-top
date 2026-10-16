//! Select rollback history by controller UID and explicit revision, never pod/name prefixes.
use crate::resource_guard::{required, Identity};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
const REVISION: &str = "deployment.kubernetes.io/revision";

pub struct Plan {
    pub patch: Value,
    pub revision: u64,
    pub replica_set: String,
}
fn revision(value: &Value) -> Result<u64> {
    value["metadata"]["annotations"][REVISION]
        .as_str()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow!("rollout revision unavailable or invalid"))
}
fn template(value: &Value) -> Result<Value> {
    let mut template = value["spec"]["template"].clone();
    if template["spec"]["containers"]
        .as_array()
        .is_none_or(|c| c.is_empty())
    {
        bail!("rollback template has no containers");
    }
    if let Some(labels) = template["metadata"]["labels"].as_object_mut() {
        labels.remove("pod-template-hash");
    }
    Ok(template)
}
pub fn plan(expected: Option<&Identity>, deployment: &Value, replica_sets: &Value) -> Result<Plan> {
    let expected = required(expected)?;
    expected.verify(deployment)?;
    if deployment["spec"]["paused"].as_bool() == Some(true) {
        bail!("deployment is paused; resume and review before rollback");
    }
    let current = revision(deployment)?;
    let mut candidates = Vec::new();
    for rs in replica_sets["items"]
        .as_array()
        .ok_or_else(|| anyhow!("ReplicaSet list unavailable"))?
    {
        let owned = rs["metadata"]["ownerReferences"]
            .as_array()
            .is_some_and(|refs| {
                refs.iter().any(|r| {
                    r["kind"] == "Deployment"
                        && r["uid"].as_str() == Some(&expected.uid)
                        && r["controller"].as_bool() == Some(true)
                })
            });
        if !owned {
            continue;
        }
        let number = revision(rs)?;
        if number < current {
            candidates.push((number, rs));
        }
    }
    candidates.sort_by_key(|(number, _)| *number);
    let (revision, rs) = candidates.last().ok_or_else(|| {
        anyhow!("no previous ReplicaSet owned by this deployment (history may have been pruned)")
    })?;
    if candidates.iter().filter(|(n, _)| n == revision).count() != 1 {
        bail!("ambiguous ReplicaSets for rollback revision {revision}");
    }
    let previous = template(rs)?;
    if previous == template(deployment)? {
        bail!("previous revision already has the current pod template; rollback would be a no-op");
    }
    let replica_set = rs["metadata"]["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("ReplicaSet name missing"))?
        .to_string();
    Ok(Plan {
        patch: expected.patch("/spec/template", previous),
        revision: *revision,
        replica_set,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn deployment() -> Value {
        json!({"metadata":{"uid":"original","resourceVersion":"12","annotations":{REVISION:"3"}},"spec":{"replicas":2,"template":{"metadata":{"labels":{"app":"m"}},"spec":{"containers":[{"name":"server","image":"new"}]}}}})
    }
    fn rs(name: &str, uid: &str, rev: &str) -> Value {
        json!({"metadata":{"name":name,"annotations":{REVISION:rev},"ownerReferences":[{"kind":"Deployment","uid":uid,"controller":true}]},"spec":{"template":{"metadata":{"labels":{"app":"m","pod-template-hash":"generated"},"annotations":{"preserve":"yes"}},"spec":{"containers":[{"name":"server","image":"old"}]}}}})
    }
    #[test]
    fn rollback_uses_previous_owned_revision_and_preserves_template() {
        let d = deployment();
        let id = Identity::read(&d).unwrap();
        let p=plan(Some(&id), &d, &json!({"items":[rs("foreign","other","2"),rs("latest","original","3"),rs("old","original","1"),rs("previous","original","2")]})).unwrap();
        assert_eq!(p.revision, 2);
        assert_eq!(p.replica_set, "previous");
        assert_eq!(p.patch[0]["value"], "original");
        assert_eq!(p.patch[1]["value"], "12");
        assert_eq!(p.patch[2]["path"], "/spec/template");
        assert!(p.patch[2]["value"]["metadata"]["labels"]
            .get("pod-template-hash")
            .is_none());
        assert_eq!(
            p.patch[2]["value"]["metadata"]["annotations"]["preserve"],
            "yes"
        );
    }
    #[test]
    fn invalid_revision_and_noop_are_not_reported_as_success() {
        let d = deployment();
        let id = Identity::read(&d).unwrap();
        assert!(plan(
            Some(&id),
            &d,
            &json!({"items":[rs("bad","original","invalid")]})
        )
        .is_err());
        let mut previous = rs("same", "original", "2");
        previous["spec"]["template"] = d["spec"]["template"].clone();
        previous["spec"]["template"]["metadata"]["labels"]["pod-template-hash"] =
            json!("generated");
        assert!(plan(Some(&id), &d, &json!({"items":[previous]})).is_err());
    }

    #[test]
    fn ambiguous_missing_history_paused_or_changed_deployment_are_rejected() {
        let mut d = deployment();
        let id = Identity::read(&d).unwrap();
        assert!(plan(Some(&id), &d, &json!({"items":[rs("other","foreign","2")]})).is_err());
        assert!(plan(
            Some(&id),
            &d,
            &json!({"items":[rs("one","original","2"),rs("two","original","2")]})
        )
        .is_err());
        let history = json!({"items":[rs("previous","original","2")]});
        d["spec"]["paused"] = json!(true);
        assert!(plan(Some(&id), &d, &history).is_err());
        d["spec"]["paused"] = json!(false);
        d["metadata"]["uid"] = json!("replacement");
        assert!(plan(Some(&id), &d, &history).is_err());
        assert!(plan(None, &deployment(), &history).is_err());
    }
}
