//! Persistent compile/serving history.
//!
//! Compile Jobs carry `ttlSecondsAfterFinished: 3600`, so an hour after a build the cluster has
//! no record of which options were tried or why they failed — and the next attempt repeats
//! them. This keeps an append-only log next to the audit log, which is what makes
//! [`crate::advisor`] able to recommend anything at all.
//!
//! Format is JSON Lines: appended under a lock, tolerant of a truncated final line, and
//! readable with `tail`/`jq`. One record per terminal Job outcome, deduplicated by job name so
//! a completed Job observed on ten collect ticks is recorded once.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;

/// How a build or a serving observation turned out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Fail,
}

/// One historical event: a compile that finished, or a serving deployment observed under load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Unix seconds.
    pub ts: u64,
    /// `compile` | `serve`.
    pub kind: String,
    /// Job or deployment name — the dedup key.
    pub id: String,
    /// HuggingFace source id when known, else the display model name.
    pub model: String,
    /// Accelerator pack id.
    pub vendor: String,
    /// Compile/serving options (tp, max-len, batch, attn, kvpart, quant…).
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    pub outcome: Outcome,
    /// Wall-clock seconds the build took, when known.
    #[serde(default)]
    pub duration_secs: Option<u64>,
    /// Classified failure kind (see [`crate::diagnose`]), empty on success.
    #[serde(default)]
    pub failure_kind: String,
    /// Human-readable cause and hint, empty on success.
    #[serde(default)]
    pub detail: String,
    /// Observed serving throughput (tok/s), for `serve` records.
    #[serde(default)]
    pub tps: Option<f64>,
    /// Observed serving TTFT p95 (seconds), for `serve` records.
    #[serde(default)]
    pub ttft_p95: Option<f64>,
}

impl Record {
    /// Normalised model family key — history for `Qwen/Qwen3-4B` should inform `Qwen3-4B-FP8`.
    pub fn family(&self) -> String {
        family_key(&self.model)
    }
}

/// Group models that share a compile profile: lowercase, drop the org, drop precision and
/// hardware tags. `Qwen/Qwen3-4B-FP8` and `qwen3-4b` land on the same key.
pub fn family_key(model: &str) -> String {
    let name = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    let drop = [
        "-fp8", "-fp16", "-bf16", "-awq", "-gptq", "-int8", "-int4", "-w8a8", "-w4a16",
        "-instruct", "-chat", "-rbln", "-rngd", "-npu",
    ];
    let mut out = name;
    let mut changed = true;
    while changed {
        changed = false;
        for tag in drop {
            if let Some(stripped) = out.strip_suffix(tag) {
                out = stripped.to_string();
                changed = true;
            }
        }
    }
    out
}

/// Log path — `LMD_HISTORY` overrides, else next to the audit log.
pub fn path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("LMD_HISTORY") {
        return std::path::PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join(".config/lmd-top/history.jsonl")
}

/// Append a record. Failure to write is not worth interrupting a collect tick for, so it is
/// silent — history is an optimisation, not a correctness requirement.
pub fn append(rec: &Record) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(line) = serde_json::to_string(rec) else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{}", line);
    }
}

/// Load every record. A malformed or half-written line is skipped rather than failing the load:
/// the file is appended to from a live process, so a torn final line is normal.
pub fn load() -> Vec<Record> {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Record>(l).ok())
        .collect()
}

/// Ids already recorded, so a terminal Job seen on every tick is written once.
pub fn recorded_ids() -> std::collections::BTreeSet<String> {
    load().into_iter().map(|r| r.id).collect()
}

/// Print the history as a table (`--history`).
pub fn print_log() {
    let recs = load();
    if recs.is_empty() {
        println!("no history yet ({})", path().display());
        println!("compile and serving outcomes are recorded as jobs finish.");
        return;
    }
    println!(
        "{:<20} {:<8} {:<26} {:<9} {:<9} {}",
        "WHEN", "KIND", "MODEL", "VENDOR", "OUTCOME", "OPTIONS / CAUSE"
    );
    for r in &recs {
        let opts = r
            .options
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(" ");
        let tail = if r.outcome == Outcome::Ok {
            match r.duration_secs {
                Some(d) => format!("{}  ({}m{}s)", opts, d / 60, d % 60),
                None => opts,
            }
        } else {
            format!("{}  ✗ {}", opts, r.detail)
        };
        println!(
            "{:<20} {:<8} {:<26} {:<9} {:<9} {}",
            crate::audit::iso_utc(r.ts),
            r.kind,
            crate::ui::truncw(&r.model, 26),
            r.vendor,
            match r.outcome {
                Outcome::Ok => "ok",
                Outcome::Fail => "FAIL",
            },
            tail
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, model: &str, outcome: Outcome) -> Record {
        Record {
            ts: 1_700_000_000,
            kind: "compile".into(),
            id: id.into(),
            model: model.into(),
            vendor: "rbln".into(),
            options: [("tp".to_string(), "4".to_string())].into_iter().collect(),
            outcome,
            duration_secs: Some(90),
            failure_kind: String::new(),
            detail: String::new(),
            tps: None,
            ttft_p95: None,
        }
    }

    /// Isolate the log per test — it is a process-global path.
    fn with_temp_log<T>(name: &str, f: impl FnOnce() -> T) -> T {
        let _g = crate::audit::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let p = std::env::temp_dir().join(format!("lmd-hist-{}.jsonl", name));
        let _ = std::fs::remove_file(&p);
        std::env::set_var("LMD_HISTORY", &p);
        let out = f();
        std::env::remove_var("LMD_HISTORY");
        let _ = std::fs::remove_file(&p);
        out
    }

    #[test]
    fn appends_and_reloads() {
        with_temp_log("roundtrip", || {
            assert!(load().is_empty(), "starts empty");
            append(&rec("job-a", "Qwen/Qwen3-4B", Outcome::Ok));
            append(&rec("job-b", "Qwen/Qwen3-4B", Outcome::Fail));
            let all = load();
            assert_eq!(all.len(), 2);
            assert_eq!(all[0].id, "job-a");
            assert_eq!(all[1].outcome, Outcome::Fail);
            assert_eq!(all[0].options.get("tp").map(String::as_str), Some("4"));
            assert_eq!(recorded_ids().len(), 2);
        });
    }

    /// The file is appended to by a live process, so a torn last line must not lose the rest.
    #[test]
    fn survives_a_truncated_line() {
        with_temp_log("torn", || {
            append(&rec("job-a", "Qwen/Qwen3-4B", Outcome::Ok));
            let p = path();
            let mut text = std::fs::read_to_string(&p).unwrap();
            text.push_str("{\"ts\":170000,\"kind\":\"comp");
            std::fs::write(&p, text).unwrap();
            let all = load();
            assert_eq!(all.len(), 1, "good records survive a torn tail");
            assert_eq!(all[0].id, "job-a");
        });
    }

    #[test]
    fn family_key_groups_precision_and_hardware_variants() {
        for (a, b) in [
            ("Qwen/Qwen3-4B-FP8", "qwen3-4b"),
            ("furiosa-ai/Qwen3-4B-FP8", "Qwen/Qwen3-4B"),
            ("meta-llama/Llama-3.1-8B-Instruct", "llama-3.1-8b"),
            ("koni-llama3.1-8b-rbln", "koni-llama3.1-8b"),
        ] {
            assert_eq!(
                family_key(a),
                family_key(b),
                "{:?} and {:?} should share a family key",
                a,
                b
            );
        }
        // Genuinely different models must not collide.
        assert_ne!(family_key("Qwen/Qwen3-4B"), family_key("Qwen/Qwen3-8B"));
        assert_ne!(family_key("Qwen/Qwen3-4B"), family_key("Qwen/Qwen2-4B"));
    }
}
