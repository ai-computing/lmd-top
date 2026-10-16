//! Bind asynchronous preparation to the exact confirmation being displayed.
use super::{Mode, Pending};
use crate::manifest_plan::Reviewed;

type Key = (String, String, Mode, String);
#[derive(Default)]
pub struct State {
    generation: u64,
    key: Option<Key>,
    started: bool,
    pub error: Option<String>,
}
impl State {
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.key = None;
        self.started = false;
        self.error = None;
    }
    pub fn sync(&mut self, pending: Option<&Pending>, namespace: &str) {
        let key = match pending {
            Some(Pending::Apply {
                title,
                yaml,
                required,
                reviewed: None,
            }) => Some((title.clone(), yaml.clone(), *required, namespace.into())),
            _ => None,
        };
        if self.key != key {
            self.reset();
            self.key = key;
        }
    }
    pub fn request(
        &mut self,
        pending: Option<&Pending>,
        namespace: &str,
        mode: Mode,
    ) -> Option<(u64, String)> {
        self.sync(pending, namespace);
        if self.started || pending?.authorize(mode).is_err() {
            return None;
        }
        let (_, yaml, _, _) = self.key.as_ref()?;
        self.started = true;
        Some((self.generation, yaml.clone()))
    }
    pub fn finish(
        &mut self,
        pending: &mut Option<Pending>,
        namespace: &str,
        generation: u64,
        result: Result<Reviewed, String>,
    ) -> Option<Result<(), String>> {
        self.sync(pending.as_ref(), namespace);
        if self.key.is_none() || generation != self.generation {
            return None;
        }
        let Some(Pending::Apply { reviewed, yaml, .. }) = pending else {
            return None;
        };
        match result.and_then(|plan| {
            plan.steps(namespace, yaml)
                .map(|_| ())
                .map_err(|e| e.to_string())
                .map(|_| plan)
        }) {
            Ok(plan) => {
                *reviewed = Some(plan);
                Some(Ok(()))
            }
            Err(error) => {
                self.error = Some(error.clone());
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending(yaml: &str) -> Option<Pending> {
        Some(Pending::Apply {
            title: "test".into(),
            yaml: yaml.into(),
            required: Mode::Admin,
            reviewed: None,
        })
    }
    fn plan(yaml: &str) -> Reviewed {
        Reviewed::new("ns", yaml, vec![])
    }
    #[test]
    fn close_reopen_and_edit_discard_old_preparation() {
        let mut state = State::default();
        let mut confirm = pending("original");
        let (old, _) = state.request(confirm.as_ref(), "ns", Mode::Admin).unwrap();
        assert!(state.request(confirm.as_ref(), "ns", Mode::Admin).is_none());
        state.sync(None, "ns");
        let (new, _) = state.request(confirm.as_ref(), "ns", Mode::Admin).unwrap();
        assert!(state
            .finish(&mut confirm, "ns", old, Ok(plan("original")))
            .is_none());
        confirm = pending("edited");
        assert!(state
            .finish(&mut confirm, "ns", new, Ok(plan("original")))
            .is_none());
        let (edited, _) = state.request(confirm.as_ref(), "ns", Mode::Admin).unwrap();
        assert!(state
            .finish(&mut confirm, "ns", edited, Ok(plan("edited")))
            .unwrap()
            .is_ok());
        assert!(matches!(
            confirm,
            Some(Pending::Apply {
                reviewed: Some(_),
                ..
            })
        ));
    }
    #[test]
    fn errors_require_retry_and_wrong_source_cannot_become_reviewed() {
        let mut state = State::default();
        let mut confirm = pending("source");
        assert!(state
            .request(confirm.as_ref(), "ns", Mode::Observe)
            .is_none());
        let (id, _) = state.request(confirm.as_ref(), "ns", Mode::Admin).unwrap();
        assert!(state
            .finish(&mut confirm, "ns", id, Ok(plan("wrong")))
            .unwrap()
            .is_err());
        assert!(state.error.is_some());
        assert!(state.request(confirm.as_ref(), "ns", Mode::Admin).is_none());
        state.reset();
        let (retry, _) = state.request(confirm.as_ref(), "ns", Mode::Admin).unwrap();
        assert!(state
            .finish(&mut confirm, "other-ns", retry, Ok(plan("source")))
            .is_none());
    }
    #[test]
    fn unprepared_and_edited_plans_fail_before_execution() {
        let _audit = crate::audit::TestSink::new();
        let result = crate::mutation::run_mutation(pending("source").unwrap(), "ns", Mode::Admin);
        assert!(result.result.err().unwrap().contains("not prepared"));
        for (namespace, source) in [("other", "source"), ("ns", "edited")] {
            assert!(crate::kube::apply_reviewed_manifest(
                namespace,
                source,
                &plan("source"),
                false
            )
            .unwrap_err()
            .to_string()
            .contains("changed after review"));
        }
    }
}
