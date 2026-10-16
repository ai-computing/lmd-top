//! Evidence-based SLO evaluation. Missing measurements never count as success.
use crate::{
    collect::PerfRow,
    ops::{Objective, PerfAdvice},
};
pub fn evaluate(row: &PerfRow, objective: Option<&Objective>, fresh: bool) -> PerfAdvice {
    let mut result = PerfAdvice {
        has_obj: objective.is_some_and(|o| !o.is_empty()),
        checks: vec![],
        unknown: vec![],
        tips: vec![],
    };
    let Some(objective) = objective else {
        return result;
    };
    let traffic = row.req.is_finite() && row.req > 0.0;
    for (name, target, measured, minimum) in [
        ("TTFT", objective.ttft_ms, row.ttft_p95 * 1000.0, false),
        ("TPOT", objective.tpot_ms, row.tpot_p95 * 1000.0, false),
        ("E2E", objective.e2e_ms, row.e2e_p95 * 1000.0, false),
        ("tok/s", objective.min_tps, row.tps, true),
    ] {
        let Some(target) = target else {
            continue;
        };
        let reason = if !target.is_finite() || target <= 0.0 {
            Some("invalid target")
        } else if !fresh {
            Some("performance collection stale or failed")
        } else if !traffic {
            Some("positive request traffic not observed")
        } else if !measured.is_finite() || measured < 0.0 {
            Some("measurement unavailable or invalid")
        } else {
            None
        };
        if let Some(reason) = reason {
            result.unknown.push((name, reason.into()));
        } else {
            result.checks.push((
                name,
                if minimum {
                    measured >= target
                } else {
                    measured <= target
                },
            ));
        }
    }
    if result.checks.iter().any(|(_, met)| !met) {
        if row.preempt.is_finite() && row.preempt > 0.0 {
            result.tips.push(format!("preemption observed ({:.2}/s): inspect KV capacity and scheduler events before changing concurrency",row.preempt));
        }
        let stages = [
            ("queue", row.queue_p95),
            ("prefill", row.prefill_p95),
            ("decode", row.decode_p95),
        ];
        if stages.iter().all(|(_, v)| v.is_finite() && *v >= 0.0) {
            if let Some((stage, value)) = stages
                .iter()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, v)| *v > 0.0)
            {
                result.tips.push(format!("{stage} has the largest observed stage p95 ({:.0}ms); independent percentiles are not additive and do not prove a bottleneck",value*1000.0));
            }
        } else {
            result.tips.push("stage latency coverage incomplete: cannot attribute the delay to queue, prefill or decode".into());
        }
        if result
            .checks
            .iter()
            .any(|(name, met)| *name == "tok/s" && !met)
        {
            result.tips.push("tok/s below target under observed traffic: compare request concurrency and output lengths; low throughput alone does not establish spare capacity or compute saturation".into());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> PerfRow {
        PerfRow {
            model: "model".into(),
            req: 1.0,
            tps: 100.0,
            ttft_p95: 0.1,
            tpot_p95: f64::NAN,
            e2e_p95: 1.0,
            in_tok_p95: f64::NAN,
            out_tok_p95: f64::NAN,
            queue_p95: f64::NAN,
            prefill_p95: f64::NAN,
            decode_p95: f64::NAN,
            preempt: f64::NAN,
        }
    }
    #[test]
    fn partial_or_missing_measurements_do_not_pass_all_targets() {
        let target = Objective {
            ttft_ms: Some(200.0),
            tpot_ms: Some(50.0),
            ..Default::default()
        };
        let result = evaluate(&row(), Some(&target), true);
        assert_eq!(result.checks, vec![("TTFT", true)]);
        assert_eq!(result.unknown.len(), 1);
        assert!(!result.all_met());
        assert!(!evaluate(&row(), Some(&Objective::default()), true).all_met());
        assert!(!evaluate(&row(), Some(&target), false).all_met());
    }
    #[test]
    fn idle_invalid_and_stale_samples_are_unknown_not_violations_or_success() {
        let target = Objective {
            min_tps: Some(200.0),
            ..Default::default()
        };
        for req in [0.0, f64::NAN, f64::INFINITY] {
            let r = PerfRow { req, ..row() };
            let result = evaluate(&r, Some(&target), true);
            assert!(result.checks.is_empty());
            assert_eq!(result.unknown.len(), 1);
            assert!(result.tips.is_empty());
        }
        let r = PerfRow {
            tps: f64::INFINITY,
            ..row()
        };
        assert_eq!(evaluate(&r, Some(&target), true).unknown.len(), 1);
    }
    #[test]
    fn violation_advice_does_not_invent_accelerator_headroom() {
        let result = evaluate(
            &row(),
            Some(&Objective {
                min_tps: Some(200.0),
                ..Default::default()
            }),
            true,
        );
        assert_eq!(result.checks, vec![("tok/s", false)]);
        assert!(result
            .tips
            .iter()
            .any(|s| s.contains("coverage incomplete")));
        assert!(!result.tips.iter().any(|s| s.contains("replicas↑")));
    }
}
