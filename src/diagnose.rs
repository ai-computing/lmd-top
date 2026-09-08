//! Turn a failed Job's log into something actionable.
//!
//! A failed compile used to surface as "failed — see logs", and `ttlSecondsAfterFinished`
//! deletes the Job an hour later — so the reason was gone before anyone read it, and the next
//! attempt repeated the same options. This classifies the failure from the log tail and says
//! what to change.
//!
//! Patterns are ordered most-specific first: a memory error inside a compiler traceback should
//! be reported as a memory error, not as the traceback's generic final line.

/// A classified failure: what happened, and what to do about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// One-line cause, short enough for a table cell.
    pub summary: String,
    /// What to change, when the cause implies an action. Empty when it does not.
    pub hint: String,
    /// Stable identifier for grouping repeats in history (`oom`, `rbln-codegen`, …).
    pub kind: &'static str,
}

impl Failure {
    fn new(kind: &'static str, summary: impl Into<String>, hint: impl Into<String>) -> Self {
        Failure {
            kind,
            summary: summary.into(),
            hint: hint.into(),
        }
    }

    /// Single-line rendering for a status column.
    pub fn line(&self) -> String {
        if self.hint.is_empty() {
            self.summary.clone()
        } else {
            format!("{} — {}", self.summary, self.hint)
        }
    }
}

/// Lines that look like a diagnosis rather than progress or an echoed configuration.
///
/// Needed because recipes print their settings: the RBLN recipe echoes
/// `RBLN_CONFIG {... 'rbln_kvcache_partition_len': 4096}`, and matching that token anywhere in
/// the log reported "invalid kvcache-partition" for a build whose partition length was fine.
fn error_context(log: &str) -> String {
    log.lines()
        .filter(|l| {
            let lc = l.to_lowercase();
            (lc.contains("error")
                || lc.contains("invalid")
                || lc.contains("must ")
                || lc.contains("cannot")
                || lc.contains("failed")
                || lc.contains("unsupported")
                || lc.contains("not supported")
                || lc.contains("traceback")
                || lc.contains("raise "))
                // An echoed configuration is not a diagnosis.
                && !lc.contains("_config {")
                && !lc.contains("config:")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

/// Classify a compile/prefetch failure from its log and container exit code.
pub fn classify(log: &str, exit_code: Option<i32>) -> Failure {
    let lc = log.to_lowercase();
    // Parameter and support diagnoses are matched only against error-bearing lines; an option
    // name appearing in an echoed config says nothing about why the build failed.
    let err = error_context(log);

    // ── A rejected vendor flag: nothing to do with the model or its options ──
    //
    // The SDK validates its RBLN_* flags and aborts before compiling. Classifying this as a
    // compile failure would be wrong twice over: it hides the real message, and it teaches the
    // advisor that the *options* failed when they were never tried.
    if let Some(line) = log
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("[flag]"))
    {
        let detail = line.trim_start_matches("[flag]").trim();
        return Failure::new("vendor-flag", clip(detail), "fix or unset that flag in LMD_COMPILE_ENV");
    }

    // ── Environment and inputs: fix these before touching compile options ──
    if exit_code == Some(137) || lc.contains("oomkilled") {
        return Failure::new(
            "oom",
            "container OOM-killed (exit 137)",
            "raise the Job memory request, or lower tp / max-len",
        );
    }
    if lc.contains("no space left on device") {
        return Failure::new(
            "disk-full",
            "no space left on device",
            "free space on the model store PVC",
        );
    }
    if lc.contains("os error 95") || lc.contains("operation not supported") {
        return Failure::new(
            "store-io",
            "store rejected the I/O (os error 95)",
            "the shared store is SMB — build in local scratch and copy the artifact back",
        );
    }
    if lc.contains("401 client error") || lc.contains("gated repo") || lc.contains("is not authorized")
    {
        return Failure::new(
            "hf-auth",
            "HuggingFace rejected the download (401 / gated)",
            "set the hf-token secret, and accept the model licence on HF",
        );
    }
    if lc.contains("404 client error")
        || lc.contains("repositorynotfounderror")
        || lc.contains("is not a valid model identifier")
    {
        return Failure::new(
            "hf-missing",
            "model not found on HuggingFace (404)",
            "check the source id — it must be org/name",
        );
    }
    if lc.contains("connection error") || lc.contains("failed to resolve") || lc.contains("timed out")
    {
        return Failure::new(
            "network",
            "network failure while fetching weights",
            "check egress to huggingface.co, or prefetch to the store first",
        );
    }

    // ── Accelerator memory: the compiler's own capacity errors ──
    if err.contains("out of memory")
        || err.contains("not enough memory")
        || err.contains("insufficient memory")
        || err.contains("exceeds the available")
    {
        return Failure::new(
            "device-oom",
            "model does not fit the requested devices",
            "raise tp, lower max-len, or quantise (w8a8/w4a16)",
        );
    }

    // ── RBLN parameter combinations the compiler validates late ──
    if err.contains("kvcache_partition_len") || err.contains("kvcache partition") {
        return Failure::new(
            "rbln-kvpart",
            "invalid kvcache-partition / max-seq-len combination",
            "max-len must be a multiple of kvpart (or use attn=eager)",
        );
    }
    if err.contains("flash_attn") && (err.contains("not supported") || err.contains("invalid")) {
        return Failure::new(
            "rbln-attn",
            "flash_attn rejected for this model",
            "try attn=eager",
        );
    }

    // ── Model support ──
    if err.contains("unsupported model")
        || err.contains("is not supported")
        || err.contains("no support for")
        || err.contains("keyerror") && err.contains("architectures")
    {
        return Failure::new(
            "unsupported-model",
            "this architecture is not supported by the installed toolchain",
            "check the vendor's support list, or update the compiler image",
        );
    }

    // The recipe prints whatever detail the vendor compiler's exception carried, since the
    // traceback alone says only "Error occurred while compiling the model".
    if let Some(detail) = log
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("LMD_COMPILE_CAUSE "))
        .or_else(|| {
            log.lines()
                .rev()
                .find_map(|l| l.trim().strip_prefix("LMD_COMPILE_ERROR "))
        })
    {
        let detail = detail.trim();
        // Only worth surfacing when it says more than the generic wrapper already did.
        if !detail.is_empty() && !detail.to_lowercase().contains("error occurred while compiling")
        {
            return Failure::new("vendor-compiler", clip(detail), "");
        }
    }

    // ── Vendor compiler internals: generic, but say which stage and what to try ──
    if lc.contains("error occurred while compiling the model") {
        return Failure::new(
            "rbln-codegen",
            "rebel-compiler failed during code generation (detail suppressed by the SDK)",
            // The hint here used to suggest a lower max-len, attn=eager or a newer image.
            // All three were tested and none helped — see docs/RBLN-COMPILE-INCIDENT.md.
            // Saying "try these" would send the next person round the same 90 minutes.
            "not option-related in the case investigated here — check \
             docs/RBLN-COMPILE-INCIDENT.md before re-running with different parameters",
        );
    }
    if lc.contains("fxb") && lc.contains("error") {
        return Failure::new(
            "furiosa-build",
            "fxb build failed",
            "furiosa-llm builds furiosa-ai quantised checkpoints — check the source is one",
        );
    }

    // ── Fall back to the exception line, which beats "see logs" ──
    if let Some(line) = last_exception_line(log) {
        return Failure::new("error", line, "");
    }
    if let Some(line) = last_meaningful_line(log) {
        return Failure::new("error", line, "");
    }
    match exit_code {
        Some(c) => Failure::new("error", format!("exited with code {}", c), ""),
        None => Failure::new("error", "failed with no output", ""),
    }
}

/// The final `SomeError: message` line of a Python traceback, trimmed for a table cell.
fn last_exception_line(log: &str) -> Option<String> {
    log.lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .find(|l| {
            // "RuntimeError: ...", "ValueError: ..." — an error type followed by a message.
            l.split_once(": ").is_some_and(|(head, rest)| {
                !rest.is_empty()
                    && head.ends_with("Error")
                    && head.chars().next().is_some_and(char::is_uppercase)
                    && !head.contains(' ')
            })
        })
        .map(clip)
}

/// Last line that carries information — skips progress bars and blank padding.
fn last_meaningful_line(log: &str) -> Option<String> {
    log.lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.len() > 3)
        .find(|l| {
            // Progress renderers repaint one line with bars and carriage returns.
            !l.contains('█') && !l.contains('\r') && !l.starts_with("File \"")
        })
        .map(clip)
}

fn clip(l: &str) -> String {
    const MAX: usize = 120;
    if l.chars().count() <= MAX {
        return l.to_string();
    }
    let mut out: String = l.chars().take(MAX - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure actually observed on this cluster: an RBLN compile of Qwen3-4B that dies in
    /// codegen with a generic RuntimeError after the progress bars finish.
    const RBLN_CODEGEN: &str = "\
2026-09-08 06:58:02,837 INFO [rebel-compiler] -- Number of devices: 4
Computation graph optimization ████████████████████████████████████████ 100%  00:11
Traceback (most recent call last):
  File \"/scripts/compile.py\", line 15, in <module>
    m = M.from_pretrained(os.environ[\"MODEL_ID\"], export=True, **cfg)
  File \"<frozen core.compilation._impl>\", line 974, in compile
RuntimeError: Error occurred while compiling the model
";

    #[test]
    fn observed_rbln_codegen_failure_is_actionable() {
        let f = classify(RBLN_CODEGEN, Some(1));
        assert_eq!(f.kind, "rbln-codegen");
        assert!(f.line().contains("code generation"), "{}", f.line());
        // Not the useless generic we used to show.
        assert!(!f.line().contains("see logs"));
        // And no longer a parameter suggestion: lowering max-len, switching to eager and
        // attaching a device were all tested against this failure and none helped, so
        // recommending them would cost the next person the same 90 minutes.
        assert!(
            !f.hint.contains("lower max-len") && !f.hint.contains("attn=eager"),
            "hint should not recommend the disproven remedies: {}",
            f.hint
        );
    }

    /// The real failure log echoes the compile settings before failing:
    ///   RBLN_CONFIG {... 'rbln_kvcache_partition_len': 4096}
    /// Matching that token anywhere in the log blamed the partition length for a build whose
    /// 8192/4096 was perfectly valid, and told the operator to change the wrong thing.
    #[test]
    fn an_echoed_config_is_not_a_diagnosis() {
        let log = format!(
            "RBLN_CONFIG {{'rbln_npu': 'RBLN-CA22', 'rbln_max_seq_len': 8192, \
             'rbln_attn_impl': 'flash_attn', 'rbln_kvcache_partition_len': 4096}}\n{}",
            RBLN_CODEGEN
        );
        let f = classify(&log, Some(1));
        assert_eq!(
            f.kind, "rbln-codegen",
            "the echoed config must not be read as a kvpart error: {:?}",
            f
        );
        // A real kvpart error still classifies as one.
        let real = format!("{}\nValueError: rbln_kvcache_partition_len must divide max_seq_len\n", log);
        assert_eq!(classify(&real, Some(1)).kind, "rbln-kvpart");
    }

    #[test]
    fn specific_causes_beat_the_traceback_line() {
        // A memory error inside a traceback must classify as memory, not as the final line.
        let log = format!("{}\nRuntimeError: Out of memory on device 0\n", RBLN_CODEGEN);
        assert_eq!(classify(&log, Some(1)).kind, "device-oom");
        // OOMKilled is about the container, and outranks anything in the log.
        assert_eq!(classify(RBLN_CODEGEN, Some(137)).kind, "oom");
    }

    #[test]
    fn environment_failures_are_named() {
        let cases = [
            ("401 Client Error: Unauthorized for url: https://huggingface.co/...", "hf-auth"),
            ("404 Client Error. Repository Not Found for url", "hf-missing"),
            ("OSError: [Errno 95] Operation not supported", "store-io"),
            ("OSError: [Errno 28] No space left on device", "disk-full"),
            ("ValueError: rbln_kvcache_partition_len must divide max_seq_len", "rbln-kvpart"),
            ("RuntimeError: model exceeds the available device memory", "device-oom"),
            ("ValueError: architecture Qwen3ForCausalLM is not supported", "unsupported-model"),
        ];
        for (log, want) in cases {
            let f = classify(log, Some(1));
            assert_eq!(f.kind, want, "classifying {:?} gave {:?}", log, f);
            assert!(!f.summary.is_empty());
        }
    }

    /// When the recipe manages to extract the vendor compiler's own detail, that outranks the
    /// generic wrapper — the whole point is to stop reporting "an error occurred".
    #[test]
    fn vendor_detail_outranks_the_generic_wrapper() {
        let log = format!(
            "{}\nLMD_COMPILE_ERROR RuntimeError (\'Error occurred while compiling the model\',)\n\
             LMD_COMPILE_CAUSE CompileError (\'unsupported op aten::foo at layer 3\',)\n",
            RBLN_CODEGEN
        );
        let f = classify(&log, Some(1));
        assert_eq!(f.kind, "vendor-compiler");
        assert!(f.summary.contains("unsupported op"), "{}", f.summary);

        // A cause line that only restates the wrapper is not an improvement, so the
        // stage-level classification stands.
        let vague = format!(
            "{}\nLMD_COMPILE_ERROR RuntimeError (\'Error occurred while compiling the model\',)\n",
            RBLN_CODEGEN
        );
        assert_eq!(classify(&vague, Some(1)).kind, "rbln-codegen");
    }

    #[test]
    fn falls_back_to_the_exception_line_not_a_progress_bar() {
        let log = "downloading ████████ 40%\nValueError: something specific went wrong\n";
        let f = classify(log, Some(1));
        assert_eq!(f.summary, "ValueError: something specific went wrong");
        // Progress bars must never be reported as the cause.
        let bars = "step ████████████ 100%\nstep ████████████ 100%\n";
        assert_eq!(classify(bars, Some(2)).summary, "exited with code 2");
        assert_eq!(classify("", None).summary, "failed with no output");
    }

    #[test]
    fn long_lines_are_clipped_for_a_table_cell() {
        let long = format!("ValueError: {}", "x".repeat(400));
        let f = classify(&long, Some(1));
        assert!(f.summary.chars().count() <= 120, "len {}", f.summary.chars().count());
        assert!(f.summary.ends_with('…'));
    }
}

/// Current advice for a classified failure kind, independent of what was concluded when the
/// record was written.
///
/// History is append-only, so a record keeps the text produced at the time. When the advice
/// changes — as it did for `rbln-codegen`, whose remedies were disproven by experiment — the
/// stored text becomes stale while the kind stays valid. Rendering from the kind means
/// `--history` shows what is currently known rather than a snapshot of an earlier belief.
pub fn advice_for_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "oom" => "raise the Job memory request, or lower tp / max-len",
        "disk-full" => "free space on the model store PVC",
        "store-io" => "build in local scratch and copy the artifact back",
        "hf-auth" => "set the hf-token secret, and accept the model licence on HF",
        "hf-missing" => "check the source id — it must be org/name",
        "network" => "check egress to huggingface.co, or prefetch to the store first",
        "device-oom" => "raise tp, lower max-len, or quantise (w8a8/w4a16)",
        "rbln-kvpart" => "max-len must be a multiple of kvpart (or use attn=eager)",
        "rbln-attn" => "try attn=eager",
        "unsupported-model" => "check the vendor's support list, or update the compiler image",
        "vendor-flag" => "fix or unset that flag in LMD_COMPILE_ENV",
        "rbln-codegen" => {
            "not option-related in the case investigated here — see docs/RBLN-COMPILE-INCIDENT.md"
        }
        "furiosa-build" => {
            "furiosa-llm builds furiosa-ai quantised checkpoints — check the source is one"
        }
        _ => return None,
    })
}

// ── Toolchain ───────────────────────────────────────────────────────────────────────────────

/// Parse the `LMD_TOOLCHAIN pkg=ver …` line the recipes print.
///
/// Recorded for successes as well as failures: a version skew is only visible as the
/// difference between the two.
pub fn toolchain(log: &str) -> std::collections::BTreeMap<String, String> {
    let mut out: std::collections::BTreeMap<String, String> = log
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("LMD_TOOLCHAIN "))
        .map(|rest| {
            rest.split_whitespace()
                .filter_map(|kv| kv.split_once('='))
                .filter(|(_, v)| *v != "absent")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    // A disagreement the recipe measured against the vendor's own declared pins. Stored under
    // a reserved key so it cannot collide with a package name.
    if let Some(detail) = log
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("LMD_DEPS_MISMATCH "))
    {
        out.insert("_mismatch".into(), detail.trim().to_string());
    }
    out
}

/// A dependency set that disagrees with what the vendor package itself declares.
///
/// Deliberately *not* an inferred compatibility matrix. An earlier version of this guessed
/// that optimum-rbln 0.11 could not work with transformers 5 — it looked obvious, transformers
/// 5 being a major release — and told operators to downgrade. Reading the package metadata on
/// the host showed the opposite: optimum-rbln 0.11.0.post1 *pins* `transformers==5.8.1` and
/// `torch==2.11.0+cpu`. The installed set was exactly what the vendor asked for, and the
/// advice would have broken a working environment.
///
/// So the only mismatch reported here is one the recipe measured: it compares installed
/// versions against the vendor package's own declared requirements and prints
/// `LMD_DEPS_MISMATCH …`. No version knowledge lives in lmd-top.
pub fn toolchain_skew(
    versions: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    versions.get("_mismatch").map(|detail| {
        format!(
            "installed packages disagree with what the vendor SDK declares: {} — reinstall the \
             vendor SDK's pinned dependencies on the compile host, or set a pinned compile image",
            detail
        )
    })
}

#[cfg(test)]
mod toolchain_tests {
    use super::*;

    /// Advice is rendered from the kind, so an old record shows current understanding.
    /// Every kind classify() can produce must have an entry, or history loses its hint.
    #[test]
    fn every_classified_kind_has_current_advice() {
        let samples = [
            ("oom", ("", Some(137))),
            ("disk-full", ("OSError: No space left on device", Some(1))),
            ("store-io", ("OSError: [Errno 95] Operation not supported", Some(1))),
            ("hf-auth", ("401 Client Error: Unauthorized", Some(1))),
            ("hf-missing", ("404 Client Error. Repository Not Found", Some(1))),
            ("network", ("Connection error while fetching", Some(1))),
            ("device-oom", ("RuntimeError: out of memory on device", Some(1))),
            ("rbln-kvpart", ("ValueError: kvcache_partition_len invalid", Some(1))),
            ("unsupported-model", ("ValueError: architecture X is not supported", Some(1))),
            ("rbln-codegen", ("Error occurred while compiling the model", Some(1))),
        ];
        for (want_kind, (log, exit)) in samples {
            let f = classify(log, exit);
            assert_eq!(f.kind, want_kind, "classifying {:?}", log);
            assert!(
                advice_for_kind(f.kind).is_some(),
                "kind {:?} has no current advice, so history would print none",
                f.kind
            );
        }
        // The rbln-codegen advice must no longer name the disproven remedies.
        let a = advice_for_kind("rbln-codegen").unwrap();
        assert!(!a.contains("lower max-len") && !a.contains("attn=eager"), "{}", a);
        assert!(a.contains("RBLN-COMPILE-INCIDENT"), "{}", a);
        // A kind with genuinely no action gets no invented one.
        assert_eq!(advice_for_kind("error"), None);
    }

    /// A flag the SDK rejects aborts the run before any compiling happens. Observed:
    ///   [flag] invalid value for RBLN_COMPILER_LOG_LEVEL: expected int, got "debug"
    ///   [flag] environment variable RBLN_DEBUG_LEVEL is dev-only and cannot be used ...
    /// Reporting this as a compile failure would hide the message and, worse, teach the
    /// advisor that the options failed when they were never tried.
    #[test]
    fn a_rejected_vendor_flag_is_not_a_compile_failure() {
        for line in [
            "[flag] invalid value for RBLN_COMPILER_LOG_LEVEL: expected int, got \"debug\" (raw: \"debug\")",
            "[flag] environment variable RBLN_DEBUG_LEVEL is dev-only and cannot be used in a deploy build (raw: \"1\")",
        ] {
            let f = classify(&format!("LMD_TOOLCHAIN x=1\n{}\n", line), Some(1));
            assert_eq!(f.kind, "vendor-flag", "classifying {:?}", line);
            assert!(f.summary.contains("RBLN_"), "names the flag: {}", f.summary);
            assert!(f.hint.contains("LMD_COMPILE_ENV"), "says where to fix it: {}", f.hint);
        }
        // And it outranks the generic compile wrapper, which appears in the same log once the
        // run aborts.
        let both = concat!(
            "[flag] invalid value for RBLN_COMPILER_LOG_LEVEL: expected int\n",
            "Traceback (most recent call last):\n",
            "RuntimeError: Error occurred while compiling the model\n"
        );
        assert_eq!(classify(both, Some(1)).kind, "vendor-flag");
    }

    #[test]
    fn parses_the_reported_line() {
        let log = "starting\nLMD_TOOLCHAIN optimum-rbln=0.11.0.post1 rebel-compiler=0.11.0                    transformers=5.8.1 torch=2.11.0+cpu\nRBLN_CONFIG {...}\n";
        let t = toolchain(log);
        assert_eq!(t.get("optimum-rbln").map(String::as_str), Some("0.11.0.post1"));
        assert_eq!(t.get("transformers").map(String::as_str), Some("5.8.1"));
        assert_eq!(t.get("torch").map(String::as_str), Some("2.11.0+cpu"));
        // A package the recipe could not find is omitted rather than recorded as "absent".
        assert_eq!(toolchain("LMD_TOOLCHAIN furiosa-llm=absent torch=2.4.0\n").len(), 1);
        assert!(toolchain("no such line here").is_empty());
        // A measured mismatch travels alongside the versions.
        let with_bad = toolchain(
            "LMD_TOOLCHAIN transformers=4.40.0\nLMD_DEPS_MISMATCH transformers 4.40.0 != required 5.8.1\n",
        );
        assert_eq!(
            with_bad.get("_mismatch").map(String::as_str),
            Some("transformers 4.40.0 != required 5.8.1")
        );
    }

    /// Only a mismatch the recipe *measured* is reported. This cluster is the cautionary
    /// case: optimum-rbln 0.11.0.post1 pins transformers==5.8.1 and torch==2.11.0+cpu, so the
    /// installed set was exactly correct — an inferred "transformers 5 is too new" rule would
    /// have told the operator to break a healthy environment.
    #[test]
    fn reports_only_a_measured_mismatch() {
        let vendor_consistent: std::collections::BTreeMap<String, String> = [
            ("optimum-rbln", "0.11.0.post1"),
            ("transformers", "5.8.1"),
            ("torch", "2.11.0+cpu"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(
            toolchain_skew(&vendor_consistent),
            None,
            "the vendor's own pins must never be reported as a skew"
        );

        // When the recipe measures a real disagreement, it travels under `_mismatch`.
        let mut measured = vendor_consistent.clone();
        measured.insert("_mismatch".into(), "transformers 4.40.0 != required 5.8.1".into());
        let why = toolchain_skew(&measured).expect("measured mismatch is reported");
        assert!(why.contains("transformers 4.40.0"), "{}", why);
        assert!(why.contains("vendor SDK declares"), "{}", why);

        assert_eq!(toolchain_skew(&Default::default()), None);
    }
}
