//! Operation-level mode policy. UI gating improves discoverability; this gate protects execution.
use crate::app::{Mode, Pending};
impl Pending {
    pub fn required_mode(&self) -> Mode {
        match self {
            Self::DeletePod { .. } | Self::DeleteJob { .. } | Self::RouteDelete { .. } => {
                Mode::Danger
            }
            Self::Apply { required, .. } => (*required).max(Mode::Admin),
            Self::Scale { .. }
            | Self::Restart { .. }
            | Self::Rollback { .. }
            | Self::Drain { .. }
            | Self::Stop { .. }
            | Self::StoreRefresh
            | Self::Provenance { .. }
            | Self::ApplyUrl { .. }
            | Self::SetSecret { .. }
            | Self::Cordon { .. }
            | Self::RouteRename { .. }
            | Self::RouteRetarget { .. } => Mode::Admin,
        }
    }
    pub fn authorize(&self, mode: Mode) -> Result<(), String> {
        let required = self.required_mode();
        if mode < required {
            return Err(format!(
                "operation requires --mode {}+ (current: {})",
                required.name(),
                mode.name()
            ));
        }
        if let Self::Scale { target, .. } = self {
            if *target < 0 {
                return Err("replicas must be non-negative".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operations() -> Vec<Pending> {
        vec![
            Pending::Scale {
                expected: None,
                name: "model".into(),
                target: 1,
            },
            Pending::Restart {
                expected: None,
                name: "model".into(),
            },
            Pending::Rollback {
                expected: None,
                name: "model".into(),
            },
            Pending::Drain { pod: "pod".into() },
            Pending::Stop {
                expected: None,
                name: "model".into(),
            },
            Pending::StoreRefresh,
            Pending::Provenance {
                deployment: "model".into(),
                node: "node".into(),
                path: "/models".into(),
            },
            Pending::Apply {
                reviewed: None,
                title: "plan".into(),
                yaml: "invalid if executed".into(),
                required: Mode::Admin,
            },
            Pending::ApplyUrl {
                title: "setup".into(),
                url: "invalid if executed".into(),
            },
            Pending::SetSecret {
                name: "secret".into(),
                key: "token".into(),
                value: "DO-NOT-LOG".into(),
                purpose: "HF".into(),
            },
            Pending::Cordon {
                expected: None,
                node: "node".into(),
                on: true,
            },
            Pending::DeletePod {
                name: "pod".into(),
                expected: None,
            },
            Pending::DeleteJob {
                name: "job".into(),
                expected: None,
            },
            Pending::RouteRename {
                expected: None,
                route: "route".into(),
                old: "/a".into(),
                new: "/b".into(),
            },
            Pending::RouteRetarget {
                expected: None,
                route: "route".into(),
                path: "/a".into(),
                backend: "backend".into(),
                kind: "Service".into(),
            },
            Pending::RouteDelete {
                expected: None,
                route: "route".into(),
                path: "/a".into(),
            },
        ]
    }
    #[test]
    fn worker_rejects_every_mutation_in_read_only_modes() {
        let _audit = crate::audit::TestSink::new();
        for mode in [Mode::Observe, Mode::Debug] {
            for operation in operations() {
                let outcome =
                    crate::mutation::run_mutation(operation, "not-a-real-namespace", mode);
                assert_eq!(outcome.audit_action, "denied");
                assert!(!outcome.audit_target.contains("DO-NOT-LOG"));
                let err = outcome.result.err().expect("denied before execution");
                assert!(err.contains("requires --mode"));
                assert!(!err.contains("DO-NOT-LOG"));
            }
        }
    }
    #[test]
    fn destructive_plan_cannot_lose_its_permission_requirement() {
        let _audit = crate::audit::TestSink::new();
        let plan = Pending::Apply {
            reviewed: None,
            title: "delete build".into(),
            yaml: "kind: Job".into(),
            required: Mode::Danger,
        };
        assert!(plan.authorize(Mode::Admin).is_err());
        assert!(plan.authorize(Mode::Danger).is_ok());
        let outcome = crate::mutation::run_mutation(plan, "not-a-real-namespace", Mode::Admin);
        assert_eq!(outcome.audit_action, "denied");
        let malformed = Pending::Apply {
            reviewed: None,
            title: "plan".into(),
            yaml: String::new(),
            required: Mode::Observe,
        };
        assert_eq!(malformed.required_mode(), Mode::Admin);
    }
    #[test]
    fn destructive_actions_require_danger_and_invalid_scale_is_rejected() {
        let _audit = crate::audit::TestSink::new();
        for operation in operations() {
            if matches!(
                operation,
                Pending::DeletePod { .. } | Pending::DeleteJob { .. } | Pending::RouteDelete { .. }
            ) {
                assert!(operation.authorize(Mode::Admin).is_err());
            }
            assert!(operation.authorize(Mode::Danger).is_ok());
        }
        let outcome = crate::mutation::run_mutation(
            Pending::Scale {
                expected: None,
                name: "model".into(),
                target: -1,
            },
            "not-a-real-namespace",
            Mode::Admin,
        );
        assert!(outcome.result.err().unwrap().contains("non-negative"));
    }
}
