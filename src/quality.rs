//! Per-source collection quality. A successful empty result is different from failure,
//! and a fast-tier refresh must never make workload data look newly collected.
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SourceStatus {
    pub attempted_at: u64,
    pub last_success_at: Option<u64>,
    pub samples: usize,
    pub error: Option<String>,
    pub unsupported: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Unknown,
    Fresh,
    Empty,
    Unsupported,
    Stale,
    Error,
}
impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Fresh => "fresh",
            Self::Empty => "empty",
            Self::Unsupported => "unsupported",
            Self::Stale => "stale",
            Self::Error => "error",
        }
    }
}
pub type Sources = BTreeMap<String, SourceStatus>;

impl SourceStatus {
    pub fn observed(at: u64, samples: usize, errors: &[String]) -> Self {
        Self {
            attempted_at: at,
            last_success_at: errors.is_empty().then_some(at),
            samples,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
            unsupported: false,
        }
    }
    /// Incorporate a later attempt without refreshing the timestamp of retained data.
    pub fn after(mut self, previous: Option<&Self>) -> Self {
        if let Some(previous) = previous {
            if previous.attempted_at > self.attempted_at {
                return previous.clone();
            }
            if self.error.is_some() {
                self.last_success_at = previous.last_success_at;
                if previous.last_success_at.is_some() {
                    self.samples = previous.samples;
                }
            }
        }
        self
    }
    pub fn state(&self, now: u64, max_age: u64) -> Quality {
        if self.unsupported {
            return Quality::Unsupported;
        }
        match self.last_success_at {
            Some(at) if at > now => Quality::Unknown,
            Some(at) if self.error.is_some() || now.saturating_sub(at) > max_age => Quality::Stale,
            Some(_) if self.samples == 0 => Quality::Empty,
            Some(_) => Quality::Fresh,
            None if self.error.is_some() => Quality::Error,
            None => Quality::Unknown,
        }
    }
    pub fn newer_sample_than(&self, previous: Option<&Self>) -> bool {
        self.error.is_none()
            && self.last_success_at.is_some()
            && self.last_success_at != previous.and_then(|p| p.last_success_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outage_keeps_original_age_recovery_and_empty_are_distinct() {
        let good = SourceStatus::observed(100, 3, &[]);
        let failed =
            SourceStatus::observed(105, 0, &["permission denied".into()]).after(Some(&good));
        assert_eq!(failed.last_success_at, Some(100));
        assert_eq!(failed.samples, 3);
        assert_eq!(failed.state(105, 10), Quality::Stale);
        assert!(!failed.newer_sample_than(Some(&good)));
        let empty = SourceStatus::observed(106, 0, &[]).after(Some(&failed));
        assert_eq!(empty.state(106, 10), Quality::Empty);
        assert_eq!(empty.state(117, 10), Quality::Stale);
        assert!(empty.newer_sample_than(Some(&failed)));
        assert_eq!(good.clone().after(Some(&empty)), empty);
    }
    #[test]
    fn future_measurement_time_is_unknown_until_the_clock_catches_up() {
        let source = SourceStatus::observed(200, 1, &[]);
        assert_eq!(source.state(100, 30), Quality::Unknown);
        assert_eq!(source.state(200, 30), Quality::Fresh);
        assert_eq!(source.state(231, 30), Quality::Stale);
    }
    #[test]
    fn initial_error_and_unobserved_are_not_empty_successes() {
        assert_eq!(SourceStatus::default().state(1, 10), Quality::Unknown);
        assert_eq!(
            SourceStatus::observed(1, 0, &["timeout".into()]).state(1, 10),
            Quality::Error
        );
    }
}
