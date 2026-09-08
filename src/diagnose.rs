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

    // ── Vendor compiler internals: generic, but say which stage and what to try ──
    if lc.contains("error occurred while compiling the model") {
        return Failure::new(
            "rbln-codegen",
            "rebel-compiler failed during code generation",
            "often model-version support: try a lower max-len, attn=eager, or a newer \
             optimum-rbln image",
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
        assert!(f.line().contains("max-len"), "should say what to change: {}", f.line());
        // Not the useless generic we used to show.
        assert!(!f.line().contains("see logs"));
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
