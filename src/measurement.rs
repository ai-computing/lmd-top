//! Shared presentation of measured values; unknown is distinct from zero.
/// Shared utilization threshold for the terminal and machine-readable busy count.
pub(crate) const IDLE_UTIL: f64 = 10.0;

pub(crate) fn busy_count(values: impl IntoIterator<Item = f64>) -> Option<usize> {
    let mut count = 0;
    let mut observed = false;
    for value in values {
        if !value.is_finite() {
            return None;
        }
        observed = true;
        count += usize::from(value > IDLE_UTIL);
    }
    observed.then_some(count)
}

/// Numeric ordering with unavailable values last in either direction.
pub(crate) fn compare(a: Option<f64>, b: Option<f64>, descending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.filter(|v| v.is_finite()), b.filter(|v| v.is_finite())) {
        (Some(a), Some(b)) => {
            let order = a.partial_cmp(&b).unwrap();
            if descending {
                order.reverse()
            } else {
                order
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// A measured value: preserve numeric formatting, but never print NaN/inf as telemetry.
pub(crate) struct Metric(pub f64);
impl std::fmt::Display for Metric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_finite() {
            std::fmt::Display::fmt(&self.0, f)
        } else {
            // String precision truncates text; ignore numeric precision for the placeholder.
            let width = f.width().unwrap_or(1);
            write!(f, "{:>width$}", "–", width = width)
        }
    }
}

pub(crate) fn percent(used: f64, total: f64) -> f64 {
    if used.is_finite() && total.is_finite() && total > 0.0 {
        used / total * 100.0
    } else {
        f64::NAN
    }
}

/// A total is known only if every member is measured. Empty is not a measured zero.
pub(crate) fn complete_sum(values: impl IntoIterator<Item = Option<f64>>) -> f64 {
    let mut total = 0.0;
    let mut count = 0;
    for value in values {
        match value {
            Some(value) if value.is_finite() => {
                total += value;
                count += 1;
            }
            _ => return f64::NAN,
        }
    }
    if count > 0 {
        total
    } else {
        f64::NAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metric_order_is_transitive_and_missing_is_always_last() {
        let values = [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(-0.0),
            Some(0.0),
            Some(3.0),
            Some(9.0),
        ];
        for desc in [false, true] {
            for a in values {
                for b in values {
                    for c in values {
                        let ab = compare(a, b, desc);
                        let bc = compare(b, c, desc);
                        assert_eq!(ab, compare(b, a, desc).reverse());
                        if ab.is_le() && bc.is_le() {
                            assert!(compare(a, c, desc).is_le());
                        }
                    }
                }
            }
            assert!(compare(Some(0.0), None, desc).is_lt());
            assert!(compare(Some(f64::NAN), Some(9.0), desc).is_gt());
        }
    }

    #[test]
    fn busy_requires_complete_observations_and_uses_one_threshold() {
        assert_eq!(busy_count([]), None);
        assert_eq!(busy_count([0.0, 7.0, 10.0]), Some(0));
        assert_eq!(busy_count([10.1, 80.0]), Some(2));
        assert_eq!(busy_count([80.0, f64::NAN]), None);
        assert_eq!(busy_count([f64::INFINITY]), None);
    }

    #[test]
    fn incomplete_aggregate_is_not_a_measured_total() {
        assert!(complete_sum([]).is_nan());
        assert!(complete_sum([Some(4.0), None]).is_nan());
        assert!(complete_sum([Some(f64::INFINITY)]).is_nan());
        assert_eq!(complete_sum([Some(0.0), Some(4.0)]), 4.0);
    }
}
