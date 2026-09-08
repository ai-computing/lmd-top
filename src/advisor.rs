//! Compile-option recommendations, learned from what has actually worked here.
//!
//! Choosing tp / max-len / batch / attn for a new build is guesswork otherwise: the vendor
//! docs give ranges, the failure modes only appear 20 minutes into a Job, and the cluster
//! forgets (`ttlSecondsAfterFinished`). This reads [`crate::history`] — every compile that
//! finished, and every serving deployment observed under load — and answers two questions:
//!
//!   - which option set to start from, and why
//!   - which option sets to avoid, and what happened
//!
//! Evidence is ranked: the same model on the same accelerator beats the same family, which
//! beats the same accelerator generally. Serving throughput breaks ties among builds that all
//! compiled, because "it compiled" is a lower bar than "it served well".

use crate::history::{family_key, Outcome, Record};
use std::collections::BTreeMap;

/// How closely a historical record applies to the model being planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relevance {
    /// Same accelerator, unrelated model.
    Vendor,
    /// Same accelerator, same model family (precision/hardware tags ignored).
    Family,
    /// Same accelerator, same model.
    Exact,
}

impl Relevance {
    fn label(self) -> &'static str {
        match self {
            Relevance::Exact => "this model",
            Relevance::Family => "same family",
            Relevance::Vendor => "this accelerator",
        }
    }
}

/// One recommended (or discouraged) option set.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub options: BTreeMap<String, String>,
    /// Why — evidence in words (including how relevant it is), for display next to the form.
    pub reason: String,
}

impl Suggestion {
    /// `tp=4 max-len=8192` — the option set as a single line.
    pub fn options_line(&self) -> String {
        self.options
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Everything the advisor concluded for one (model, accelerator) pair.
#[derive(Debug, Clone, Default)]
pub struct Advice {
    /// Best starting point, when history offers one.
    pub best: Option<Suggestion>,
    /// Option sets known to fail, most relevant first.
    pub avoid: Vec<Suggestion>,
    /// When nothing is known to work but something is known to fail: the next thing to try,
    /// derived by applying the failure's remedy to the options that failed. This is the part
    /// that makes progress possible — "avoid X" alone leaves you where you started.
    pub next: Option<Suggestion>,
    /// How many compile records informed this.
    pub compiles_seen: usize,
    /// How many serving observations informed this.
    pub serves_seen: usize,
}

impl Advice {
    /// One-line summary for the compile form. Empty when there is nothing to say — silence is
    /// better than a confident-looking recommendation drawn from no data.
    pub fn line(&self) -> String {
        match (&self.best, &self.next, self.avoid.first()) {
            (Some(b), _, _) => format!("↺ history: try {} — {}", b.options_line(), b.reason),
            (None, Some(n), _) => format!("↺ history: try {} — {}", n.options_line(), n.reason),
            (None, None, Some(a)) => format!("↺ history: avoid {} — {}", a.options_line(), a.reason),
            (None, None, None) => String::new(),
        }
    }
}

fn relevance(rec: &Record, model: &str, vendor: &str) -> Option<Relevance> {
    if rec.vendor != vendor {
        return None;
    }
    if rec.model.eq_ignore_ascii_case(model) {
        Some(Relevance::Exact)
    } else if rec.family() == family_key(model) {
        Some(Relevance::Family)
    } else {
        Some(Relevance::Vendor)
    }
}

/// Options that identify a build, ignoring bookkeeping keys that do not affect the artifact.
fn build_options(rec: &Record) -> BTreeMap<String, String> {
    rec.options
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "devices" | "node" | "dest"))
        .filter(|(_, v)| !v.is_empty() && v.as_str() != "none")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Recommend compile options for `model` on `vendor`, from `history`.
pub fn advise(history: &[Record], model: &str, vendor: &str) -> Advice {
    let mut advice = Advice::default();

    // Serving throughput per option set — "it compiled" is a weaker signal than "it served".
    let mut served: BTreeMap<String, f64> = BTreeMap::new();
    for rec in history.iter().filter(|r| r.kind == "serve") {
        if relevance(rec, model, vendor).is_none() {
            continue;
        }
        if let Some(tps) = rec.tps.filter(|t| *t > 0.0) {
            let key = options_key(&build_options(rec));
            let slot = served.entry(key).or_insert(0.0);
            *slot = slot.max(tps);
        }
    }
    advice.serves_seen = served.len();

    // Candidate compiles, most relevant first; a later failure of the same options wins over an
    // earlier success, because something about this cluster changed.
    let mut ok: Vec<(Relevance, &Record)> = Vec::new();
    let mut bad: Vec<(Relevance, &Record)> = Vec::new();
    for rec in history.iter().filter(|r| r.kind == "compile") {
        let Some(rel) = relevance(rec, model, vendor) else {
            continue;
        };
        advice.compiles_seen += 1;
        match rec.outcome {
            Outcome::Ok => ok.push((rel, rec)),
            Outcome::Fail => bad.push((rel, rec)),
        }
    }

    // Failures: most relevant first, then most recent. Deduplicated by option set.
    bad.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.ts.cmp(&a.1.ts)));
    let mut seen_bad = std::collections::BTreeSet::new();
    for (rel, rec) in &bad {
        let opts = build_options(rec);
        if !seen_bad.insert(options_key(&opts)) {
            continue;
        }
        advice.avoid.push(Suggestion {
            options: opts,
            reason: format!(
                "failed on {} ({})",
                rel.label(),
                if rec.detail.is_empty() {
                    rec.failure_kind.clone()
                } else {
                    rec.detail.clone()
                }
            ),
        });
    }

    // Best: highest relevance, then observed throughput, then most recent, and never an option
    // set that later failed at the same or higher relevance.
    let failed_keys: std::collections::BTreeMap<String, Relevance> = bad
        .iter()
        .map(|(rel, rec)| (options_key(&build_options(rec)), *rel))
        .collect();
    ok.sort_by(|a, b| {
        let (ka, kb) = (options_key(&build_options(a.1)), options_key(&build_options(b.1)));
        b.0.cmp(&a.0)
            .then_with(|| {
                served
                    .get(&kb)
                    .unwrap_or(&0.0)
                    .total_cmp(served.get(&ka).unwrap_or(&0.0))
            })
            .then(b.1.ts.cmp(&a.1.ts))
    });
    for (rel, rec) in &ok {
        let opts = build_options(rec);
        // Nothing to recommend from a record that carries no options (an older build whose
        // parameters were interpolated into its command line rather than passed as env).
        if opts.is_empty() {
            continue;
        }
        let key = options_key(&opts);
        // Superseded by a failure of the same options at least as relevant → not a suggestion.
        if failed_keys.get(&key).is_some_and(|f| *f >= *rel) {
            continue;
        }
        let mut why = format!("compiled on {}", rel.label());
        if let Some(d) = rec.duration_secs {
            why.push_str(&format!(" in {}m", (d / 60).max(1)));
        }
        if let Some(tps) = served.get(&key) {
            why.push_str(&format!(", served {:.0} tok/s", tps));
        }
        advice.best = Some(Suggestion {
            options: opts,
            reason: why,
        });
        break;
    }
    // Nothing known to work, but something known to fail: propose the next experiment by
    // applying that failure's remedy. Derived, not observed — the reason says so.
    if advice.best.is_none() {
        if let Some((rel, rec)) = bad.first() {
            if let Some((opts, change)) = next_experiment(&build_options(rec), &rec.failure_kind) {
                // Only worth proposing if we have not already seen it fail.
                if !failed_keys.contains_key(&options_key(&opts)) {
                    advice.next = Some(Suggestion {
                        options: opts,
                        reason: format!(
                            "untried — {} failed on {} ({}), so {}",
                            build_options(rec)
                                .get(change.0)
                                .map(|v| format!("{}={}", change.0, v))
                                .unwrap_or_else(|| change.0.to_string()),
                            rel.label(),
                            if rec.failure_kind.is_empty() {
                                "failed".to_string()
                            } else {
                                rec.failure_kind.clone()
                            },
                            change.1
                        ),
                    });
                }
            }
        }
    }

    advice
}

/// Given options that failed and why, produce the next combination to try.
///
/// Each remedy changes exactly one parameter, so a failure is narrowed rather than replaced by
/// a different unknown. Returns the new options plus (the key changed, how it changed) for the
/// explanation. `None` when the cause is environmental — no option set fixes a missing token.
fn next_experiment(
    failed: &BTreeMap<String, String>,
    failure_kind: &str,
) -> Option<(BTreeMap<String, String>, (&'static str, String))> {
    let mut opts = failed.clone();
    let halve = |v: Option<&String>| -> Option<i64> {
        let n = v?.parse::<i64>().ok()?;
        (n >= 2048).then_some(n / 2)
    };
    match failure_kind {
        // Codegen and device-memory failures respond to a smaller compile-time context first:
        // it is the cheapest parameter to change and the fastest to rebuild.
        "rbln-codegen" | "furiosa-build" | "device-oom" => {
            if let Some(half) = halve(opts.get("max-len")) {
                // flash_attn partitions must keep dividing the new length.
                if let Some(kv) = opts.get("kvpart").and_then(|v| v.parse::<i64>().ok()) {
                    if half % kv != 0 || half <= kv {
                        opts.insert("attn".into(), "eager".into());
                        opts.remove("kvpart");
                        opts.insert("max-len".into(), half.to_string());
                        return Some((
                            opts,
                            ("max-len", format!("halve it to {} and drop flash_attn", half)),
                        ));
                    }
                }
                opts.insert("max-len".into(), half.to_string());
                return Some((opts, ("max-len", format!("halve it to {}", half))));
            }
            // No room left in max-len: fall back to the simpler attention path.
            if opts.get("attn").map(String::as_str) == Some("flash_attn") {
                opts.insert("attn".into(), "eager".into());
                opts.remove("kvpart");
                return Some((opts, ("attn", "switch to eager".to_string())));
            }
            None
        }
        // The parameter combination itself is invalid — take the constraint out of play.
        "rbln-kvpart" | "rbln-attn" => {
            opts.insert("attn".into(), "eager".into());
            opts.remove("kvpart");
            Some((opts, ("attn", "switch to eager".to_string())))
        }
        // Container memory is a Job resource, not a compile option, but a narrower build needs
        // less of it.
        "oom" => {
            let half = halve(opts.get("max-len"))?;
            opts.insert("max-len".into(), half.to_string());
            Some((opts, ("max-len", format!("halve it to {}", half))))
        }
        // hf-auth, hf-missing, network, disk-full, store-io, unsupported-model: no option set
        // fixes these, and pretending otherwise wastes another build.
        _ => None,
    }
}

/// Canonical string for an option set, so two records with the same options compare equal.
fn options_key(opts: &BTreeMap<String, String>) -> String {
    opts.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(
        id: &str,
        model: &str,
        vendor: &str,
        outcome: Outcome,
        opts: &[(&str, &str)],
        ts: u64,
    ) -> Record {
        Record {
            ts,
            kind: "compile".into(),
            id: id.into(),
            model: model.into(),
            vendor: vendor.into(),
            options: opts
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            outcome,
            duration_secs: Some(600),
            failure_kind: if outcome == Outcome::Fail {
                "rbln-codegen".into()
            } else {
                String::new()
            },
            detail: if outcome == Outcome::Fail {
                "rebel-compiler failed during code generation".into()
            } else {
                String::new()
            },
            tps: None,
            ttft_p95: None,
        }
    }

    fn serve(model: &str, vendor: &str, opts: &[(&str, &str)], tps: f64) -> Record {
        Record {
            kind: "serve".into(),
            tps: Some(tps),
            ..compile("serve-x", model, vendor, Outcome::Ok, opts, 1)
        }
    }

    #[test]
    fn no_history_says_nothing() {
        let a = advise(&[], "Qwen/Qwen3-4B", "rbln");
        assert!(a.best.is_none() && a.avoid.is_empty());
        assert_eq!(a.line(), "", "silence beats a recommendation from no data");
    }

    #[test]
    fn recommends_what_compiled_and_warns_about_what_failed() {
        let h = vec![
            compile("j1", "Qwen/Qwen3-4B", "rbln", Outcome::Fail, &[("tp", "4"), ("max-len", "8192")], 100),
            compile("j2", "Qwen/Qwen3-4B", "rbln", Outcome::Ok, &[("tp", "4"), ("max-len", "4096")], 200),
        ];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        let best = a.best.clone().expect("a suggestion");
        assert_eq!(best.options_line(), "max-len=4096 tp=4");
        assert!(best.reason.contains("this model"), "{}", best.reason);
        assert_eq!(a.avoid.len(), 1);
        assert!(a.avoid[0].reason.contains("code generation"), "{}", a.avoid[0].reason);
        assert!(a.line().contains("try max-len=4096"), "{}", a.line());
    }

    /// The exact model outranks its family, which outranks unrelated models on the same chip.
    #[test]
    fn closer_evidence_wins() {
        let h = vec![
            compile("j1", "meta-llama/Llama-3.1-8B", "rbln", Outcome::Ok, &[("tp", "2")], 300),
            compile("j2", "Qwen/Qwen3-4B-FP8", "rbln", Outcome::Ok, &[("tp", "3")], 200),
            compile("j3", "Qwen/Qwen3-4B", "rbln", Outcome::Ok, &[("tp", "4")], 100),
        ];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        let best = a.best.clone().expect("a suggestion");
        assert_eq!(best.options_line(), "tp=4", "exact model beats family and vendor");
        assert!(best.reason.contains("this model"), "{}", best.reason);

        // Without the exact record, the family record wins over the unrelated one.
        let a2 = advise(&h[..2], "Qwen/Qwen3-4B", "rbln");
        assert_eq!(a2.best.unwrap().options_line(), "tp=3");
    }

    /// Compiling is a lower bar than serving well: throughput breaks ties at equal relevance.
    #[test]
    fn serving_throughput_breaks_ties() {
        let h = vec![
            compile("j1", "Qwen/Qwen3-4B", "rbln", Outcome::Ok, &[("tp", "2")], 100),
            compile("j2", "Qwen/Qwen3-4B", "rbln", Outcome::Ok, &[("tp", "4")], 100),
            serve("Qwen/Qwen3-4B", "rbln", &[("tp", "2")], 40.0),
            serve("Qwen/Qwen3-4B", "rbln", &[("tp", "4")], 180.0),
        ];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        let best = a.best.clone().expect("a suggestion");
        assert_eq!(best.options_line(), "tp=4", "the faster-serving build wins");
        assert!(best.reason.contains("180 tok/s"), "{}", best.reason);
        assert_eq!(a.serves_seen, 2);
    }

    /// A build that used to work and now fails must stop being recommended — something about
    /// the cluster changed, and a stale success is worse than no suggestion.
    #[test]
    fn a_later_failure_supersedes_an_earlier_success() {
        let h = vec![
            compile("j1", "Qwen/Qwen3-4B", "rbln", Outcome::Ok, &[("tp", "4")], 100),
            compile("j2", "Qwen/Qwen3-4B", "rbln", Outcome::Fail, &[("tp", "4")], 200),
        ];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        assert!(a.best.is_none(), "tp=4 both succeeded and failed — do not recommend it");
        assert_eq!(a.avoid.len(), 1);
        assert!(a.line().contains("avoid"), "{}", a.line());
    }

    /// A record with no recorded options cannot be a suggestion — "try (nothing)" is worse
    /// than saying nothing. Older builds interpolated their parameters into the command line,
    /// so history genuinely contains such records.
    #[test]
    fn a_record_without_options_is_not_a_suggestion() {
        let h = vec![compile("j1", "Qwen/Qwen3-4B", "rbln", Outcome::Ok, &[], 100)];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        assert!(a.best.is_none(), "no options recorded — nothing to recommend");
        assert_eq!(a.line(), "");
        // But it still counts as evidence that the model compiles at all.
        assert_eq!(a.compiles_seen, 1);
    }

    /// The point of the feature: after a failure, say what to try next — not just what to
    /// avoid. This is the real Qwen3-4B/RBLN case from this cluster.
    #[test]
    fn proposes_the_next_experiment_after_a_failure() {
        let h = vec![compile(
            "j1",
            "Qwen/Qwen3-4B",
            "rbln",
            Outcome::Fail,
            &[("tp", "4"), ("max-len", "8192"), ("kvpart", "4096"), ("attn", "flash_attn")],
            100,
        )];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        assert!(a.best.is_none(), "nothing has worked yet");
        let next = a.next.clone().expect("a next experiment");
        // max-len halves; 4096 would no longer have ≥2 flash_attn partitions, so attention
        // drops to eager and kvpart goes away rather than staying invalid.
        assert_eq!(next.options.get("max-len").map(String::as_str), Some("4096"));
        assert_eq!(next.options.get("attn").map(String::as_str), Some("eager"));
        assert!(!next.options.contains_key("kvpart"));
        assert_eq!(next.options.get("tp").map(String::as_str), Some("4"), "tp is unchanged");
        assert!(next.reason.contains("untried"), "{}", next.reason);
        assert!(a.line().contains("try "), "{}", a.line());
    }

    /// An invalid parameter combination is removed from play rather than shrunk.
    #[test]
    fn kvpart_failures_switch_attention_not_length() {
        let h = vec![compile(
            "j1", "m/x", "rbln", Outcome::Fail,
            &[("tp", "4"), ("max-len", "8192"), ("kvpart", "16384"), ("attn", "flash_attn")], 100,
        )];
        let mut h = h;
        h[0].failure_kind = "rbln-kvpart".into();
        let next = advise(&h, "m/x", "rbln").next.expect("a next experiment");
        assert_eq!(next.options.get("attn").map(String::as_str), Some("eager"));
        assert_eq!(next.options.get("max-len").map(String::as_str), Some("8192"), "length kept");
        assert!(!next.options.contains_key("kvpart"));
    }

    /// No option set fixes a missing token or a full disk — proposing one wastes a build.
    #[test]
    fn environmental_failures_get_no_experiment() {
        for kind in ["hf-auth", "hf-missing", "network", "disk-full", "store-io", "unsupported-model"] {
            let mut h = vec![compile(
                "j1", "m/x", "rbln", Outcome::Fail, &[("tp", "4"), ("max-len", "8192")], 100,
            )];
            h[0].failure_kind = kind.to_string();
            let a = advise(&h, "m/x", "rbln");
            assert!(a.next.is_none(), "{} should not propose an option change", kind);
            // But it must still be reported, so the operator knows what to fix.
            assert_eq!(a.avoid.len(), 1);
            assert!(a.line().contains("avoid"), "{}: {}", kind, a.line());
        }
    }

    /// Never propose something already known to fail.
    #[test]
    fn does_not_propose_an_already_failed_combination() {
        let mut h = vec![
            compile("j1", "m/x", "rbln", Outcome::Fail, &[("max-len", "8192"), ("tp", "4")], 100),
            compile("j2", "m/x", "rbln", Outcome::Fail, &[("max-len", "4096"), ("tp", "4")], 200),
        ];
        for r in &mut h {
            r.failure_kind = "rbln-codegen".into();
        }
        let a = advise(&h, "m/x", "rbln");
        // Halving 8192 gives 4096, which also failed → propose nothing rather than a repeat.
        if let Some(n) = &a.next {
            assert_ne!(n.options.get("max-len").map(String::as_str), Some("4096"));
        }
        assert_eq!(a.avoid.len(), 2);
    }

    #[test]
    fn other_accelerators_are_not_evidence() {
        let h = vec![compile(
            "j1", "Qwen/Qwen3-4B", "furiosa", Outcome::Ok, &[("tp", "8")], 100,
        )];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        assert!(a.best.is_none(), "a Furiosa build says nothing about RBLN");
        assert_eq!(a.compiles_seen, 0);
    }

    /// Bookkeeping keys must not split otherwise-identical option sets.
    #[test]
    fn placement_keys_do_not_fragment_the_evidence() {
        let h = vec![
            compile("j1", "Qwen/Qwen3-4B", "rbln", Outcome::Ok, &[("tp", "4"), ("node", "npu-1")], 100),
            compile("j2", "Qwen/Qwen3-4B", "rbln", Outcome::Fail, &[("tp", "4"), ("node", "npu-2")], 200),
        ];
        let a = advise(&h, "Qwen/Qwen3-4B", "rbln");
        assert!(a.best.is_none(), "same options, different node — still superseded");
        assert_eq!(a.avoid.len(), 1, "and recorded once, not per node");
        assert_eq!(a.avoid[0].options_line(), "tp=4");
    }
}
