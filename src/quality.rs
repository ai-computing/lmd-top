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
            error: (!errors.is_empty()).then(|| summarize(errors)),
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

/// Join collector errors, collapsing repeats of one cause.
///
/// When Prometheus is down every query fails with the same message, and listing all of them
/// (one source had ~100) buried the only fact that mattered. Errors are grouped by the text
/// after a `prom[query]: ` prefix; a repeated cause is shown once with its count and the
/// first query it hit, so the root cause leads and nothing is silently dropped.
pub fn summarize(errors: &[String]) -> String {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for error in errors {
        let (query, cause) = match error
            .strip_prefix("prom[")
            .and_then(|r| r.split_once("]: "))
        {
            Some((query, cause)) => (query, cause),
            None => ("", error.as_str()),
        };
        match groups.iter_mut().find(|(c, _)| *c == cause) {
            Some((_, queries)) => queries.push(query),
            None => groups.push((cause, vec![query])),
        }
    }
    groups
        .iter()
        .map(|(cause, queries)| match queries.as_slice() {
            [""] => cause.to_string(),
            [query] => format!("prom[{query}]: {cause}"),
            ["", ..] => format!("{cause} ×{}", queries.len()),
            [first, ..] => format!("{cause} ×{} queries (first: {first})", queries.len()),
            [] => unreachable!("a group always holds the error that created it"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_cause_is_reported_once_with_its_count() {
        let down = |q: &str| format!("prom[{q}]: prometheus unreachable (probe failed this cycle)");
        let errors = vec![
            down("node_load1"),
            down("node_memory_MemTotal_bytes"),
            "prom[node_cpu_seconds_total]: timeout".to_string(),
            down("node_filesystem_size_bytes"),
            "inventory unavailable: forbidden".to_string(),
        ];
        assert_eq!(
            summarize(&errors),
            "prometheus unreachable (probe failed this cycle) ×3 queries (first: node_load1); \
             prom[node_cpu_seconds_total]: timeout; inventory unavailable: forbidden"
        );
        assert_eq!(
            summarize(&errors[2..3]),
            "prom[node_cpu_seconds_total]: timeout"
        );
    }
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
