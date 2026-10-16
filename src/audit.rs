//! Mutation audit log — lmd-top records every mutation it applies to the cluster to a file.
//! One line per: "who (mode) · when (ts) · what (action) · where (target) · result (ok/FAIL · reason)".
//! Since an observability tool performs real mutations, this is essential for after-the-fact tracing/accountability.
//!
//! Path: $LMD_AUDIT > ~/.config/lmd-top/audit.log. Write failures are reported separately from operation results.
//! Pure Rust only (no external time crate) — converts epoch seconds to human-readable UTC via its own civil-date computation.

use crate::app::Mode;
use std::io::Write;

/// Audit log file path. env override → XDG convention.
fn log_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("LMD_AUDIT") {
        return Some(std::path::PathBuf::from(p));
    }
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".config/lmd-top/audit.log"))
}

/// epoch seconds → "YYYY-MM-DDTHH:MM:SSZ" (UTC). Howard Hinnant civil-from-days algorithm.
pub(crate) fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // days since 1970-01-01 → (y, m, d)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}

/// Make a TSV field safe — newlines/tabs to spaces, first line only (for multiline kubectl errors).
fn sanitize(s: &str) -> String {
    s.lines()
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Clone, Default)]
pub struct Scope {
    pub context: String,
    pub namespace: String,
    pub uid: String,
    pub resource_version: String,
    pub elapsed_ms: u128,
    pub manifest_targets: Vec<serde_json::Value>,
}

/// Preserve the original six TSV columns; append execution scope for newer records.
/// Failures are returned so the caller can report an incomplete audit without rerunning the mutation.
pub fn record(
    mode: Mode,
    action: &str,
    target: &str,
    result: Result<&str, &str>,
    scope: &Scope,
) -> std::io::Result<()> {
    let path = log_path().ok_or_else(|| {
        std::io::Error::other("cannot locate audit log: HOME/LMD_AUDIT unavailable")
    })?;
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let (status, detail) = match result {
        Ok(msg) => ("ok", msg),
        Err(e) => ("FAIL", e),
    };
    let line = format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        iso_utc(crate::collect::now_secs()),
        mode.name(),
        sanitize(action),
        sanitize(target),
        status,
        sanitize(detail),
        sanitize(&scope.context),
        sanitize(&scope.namespace),
        sanitize(&scope.uid),
        sanitize(&scope.resource_version),
        scope.elapsed_ms,
        serde_json::to_string(&scope.manifest_targets).unwrap_or_else(|_| "[]".into())
    );
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(line.as_bytes())
}

/// `--audit` — print the audit log file path and contents to stdout. If absent, just a notice.
pub fn print_log() {
    let Some(path) = log_path() else {
        eprintln!("lmd-top: HOME not set — cannot determine audit log path");
        return;
    };
    match std::fs::read_to_string(&path) {
        Ok(body) if !body.trim().is_empty() => {
            println!("# audit log: {}", path.display());
            println!("# ts\tmode\taction\ttarget\tstatus\tdetail\tcontext\tnamespace\tuid\tresourceVersion\telapsed_ms\tmanifest_targets_json");
            println!("# Older records may omit trailing scope columns.");
            print!("{}", body);
        }
        _ => println!("# audit log empty (no mutations): {}", path.display()),
    }
}

/// Test-only lock — the `LMD_AUDIT` env var is process-global, so tests that touch it
/// race by overwriting each other's value when run in parallel. Related tests serialize on this lock.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) struct TestSink {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}
#[cfg(test)]
impl TestSink {
    pub fn new() -> Self {
        let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("LMD_AUDIT");
        std::env::set_var("LMD_AUDIT", "/dev/null");
        Self {
            _lock: lock,
            previous,
        }
    }
}
#[cfg(test)]
impl Drop for TestSink {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("LMD_AUDIT", previous);
        } else {
            std::env::remove_var("LMD_AUDIT");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_utc_known_epochs() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        // 2021-01-01T00:00:00Z = 1609459200
        assert_eq!(iso_utc(1_609_459_200), "2021-01-01T00:00:00Z");
        // 2000-02-29 (leap year) 12:34:56 = 951827696
        assert_eq!(iso_utc(951_827_696), "2000-02-29T12:34:56Z");
    }

    #[test]
    fn sanitize_strips_newlines() {
        assert_eq!(sanitize("line1\nline2"), "line1");
        assert_eq!(sanitize("  a\tb  "), "a b");
    }

    #[test]
    fn audit_write_failure_is_returned_and_control_characters_are_removed() {
        let _sink = TestSink::new();
        std::env::set_var("LMD_AUDIT", std::env::temp_dir()); // a directory, not an appendable file
        assert!(record(
            Mode::Admin,
            "apply",
            "target",
            Ok("done"),
            &Scope::default()
        )
        .is_err());
        assert!(!sanitize("action\u{1b}[31m\ttext")
            .chars()
            .any(char::is_control));
    }

    #[test]
    fn record_writes_tsv_line() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join("lmd-audit-test.log");
        let _ = std::fs::remove_file(&path);
        std::env::set_var("LMD_AUDIT", &path);
        record(
            Mode::Admin,
            "scale→2",
            "ds4",
            Ok("scaled"),
            &Scope::default(),
        )
        .unwrap();
        record(
            Mode::Admin,
            "delete-pod",
            "p1",
            Err("boom\nsecond line"),
            &Scope::default(),
        )
        .unwrap();
        let reviewed = crate::manifest_plan::Reviewed::new("ns", "original source", vec![("replace", r#"{"apiVersion":"v1","kind":"Secret","metadata":{"name":"token","uid":"original-uid","resourceVersion":"42"},"data":{"token":"DO-NOT-LOG"}}"#.into())]);
        record(
            Mode::Admin,
            "apply",
            "reviewed",
            Err("Conflict"),
            &Scope {
                manifest_targets: reviewed.targets(),
                ..Default::default()
            },
        )
        .unwrap();
        std::env::remove_var("LMD_AUDIT");
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(!body.contains("DO-NOT-LOG"));
        let fields: Vec<&str> = lines[2].split('\t').collect();
        let targets: serde_json::Value = serde_json::from_str(fields[11]).unwrap();
        assert_eq!(targets[0]["uid"], "original-uid");
        assert_eq!(targets[0]["resource_version"], "42");
        assert_eq!(targets[0]["default_namespace"], "ns");
        assert!(targets[0]["namespace"].is_null());
        let f0: Vec<&str> = lines[0].split('\t').collect();
        assert_eq!(f0.len(), 12);
        assert_eq!(f0[11], "[]");
        assert_eq!(&f0[1..6], ["admin", "scale→2", "ds4", "ok", "scaled"]);
        let f1: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(&f1[3..6], ["p1", "FAIL", "boom"]); // multiline reason keeps only the first line
        let _ = std::fs::remove_file(&path);
    }
}
