//! Confirmation lifecycle: waiting must preserve the reviewed operation.
use super::{App, Pending};

impl App {
    /// Take an approved operation only when it can enter the execution slot.
    /// Cancellation stays available while another operation is in progress.
    pub fn resolve_confirmation(&mut self, execute: bool) -> Option<Pending> {
        self.confirm_yes = false;
        if !execute {
            self.confirm = None;
            self.manifest_review_details = false;
            self.manifest_review.reset();
            self.notify("cancelled".into());
            return None;
        }
        if self.inflight.is_some() {
            self.notify("operation in progress — confirmation preserved; review and retry after it completes".into());
            return None;
        }
        if matches!(self.confirm, Some(Pending::Apply { reviewed: None, .. })) {
            self.notify(
                "manifest targets not prepared — wait, or v to retry after an error".into(),
            );
            return None;
        }
        let pending = self.confirm.take()?;
        self.manifest_review_details = false;
        self.inflight = Some(pending.prompt());
        Some(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn busy_slot_preserves_reviewed_identity_until_explicit_retry() {
        let mut app = App::new();
        let expected = crate::resource_guard::Identity {
            uid: "reviewed".into(),
            version: "42".into(),
        };
        app.confirm = Some(Pending::Scale {
            expected: Some(expected.clone()),
            name: "model".into(),
            target: 2,
        });
        app.inflight = Some("previous operation".into());
        app.confirm_yes = true;
        assert!(app.resolve_confirmation(true).is_none());
        assert_eq!(
            app.confirm.as_ref().unwrap().reviewed_identity(),
            Some(&expected)
        );
        assert_eq!(app.inflight.as_deref(), Some("previous operation"));
        assert!(!app.confirm_yes);
        app.inflight = None;
        let operation = app.resolve_confirmation(true).unwrap();
        assert_eq!(operation.reviewed_identity(), Some(&expected));
        assert!(app.confirm.is_none() && app.inflight.is_some());
    }
    #[test]
    fn unprepared_plan_cannot_start_but_can_be_cancelled_while_busy() {
        let mut app = App::new();
        app.confirm = Some(Pending::Apply {
            title: "plan".into(),
            yaml: "source".into(),
            required: super::super::Mode::Admin,
            reviewed: None,
        });
        assert!(app.resolve_confirmation(true).is_none());
        assert!(app.confirm.is_some() && app.inflight.is_none());
        app.inflight = Some("previous operation".into());
        app.manifest_review_details = true;
        assert!(app.resolve_confirmation(false).is_none());
        assert!(app.confirm.is_none() && !app.manifest_review_details);
        assert_eq!(app.inflight.as_deref(), Some("previous operation"));
    }
}
