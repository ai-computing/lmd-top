//! 변경 작업 감사 로그 — lmd-top 이 클러스터에 적용한 모든 mutation 을 파일에 남긴다.
//! "누가(mode)·언제(ts)·무엇을(action)·어디에(target)·결과(ok/FAIL·이유)" 한 줄.
//! 관측 도구가 실제 변경 작업을 수행하므로, 사후 추적/책임 소재를 위해 필수.
//!
//! 경로: $LMD_AUDIT > ~/.config/lmd-top/audit.log. 실패해도 앱 흐름은 막지 않는다(best-effort).
//! 순수 Rust만(외부 시간 크레이트 없음) — epoch 초를 자체 civil-date 계산으로 사람이 읽는 UTC 로.

use crate::app::Mode;
use std::io::Write;

/// 감사 로그 파일 경로. env override → XDG 관례.
fn log_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("LMD_AUDIT") {
        return Some(std::path::PathBuf::from(p));
    }
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".config/lmd-top/audit.log"))
}

/// epoch 초 → "YYYY-MM-DDTHH:MM:SSZ" (UTC). Howard Hinnant civil-from-days 알고리즘.
fn iso_utc(secs: u64) -> String {
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

/// TSV 필드 안전화 — 개행/탭을 공백으로, 앞 한 줄만(멀티라인 kubectl 에러 대비).
fn sanitize(s: &str) -> String {
    s.lines().next().unwrap_or("").replace('\t', " ").trim().to_string()
}

/// 변경 작업 한 건을 기록. result: Ok(요약) | Err(이유). best-effort(실패해도 조용히 무시).
pub fn record(mode: Mode, action: &str, target: &str, result: Result<&str, &str>) {
    let Some(path) = log_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let ts = iso_utc(crate::collect::now_secs());
    let (status, detail) = match result {
        Ok(msg) => ("ok", sanitize(msg)),
        Err(e) => ("FAIL", sanitize(e)),
    };
    // TSV: ts \t mode \t action \t target \t status \t detail
    let line = format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        ts,
        mode.name(),
        action,
        sanitize(target),
        status,
        detail
    );
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// `--audit` — 감사 로그 파일 경로와 내용을 표준출력으로. 없으면 안내만.
pub fn print_log() {
    let Some(path) = log_path() else {
        eprintln!("lmd-top: HOME 미설정 — 감사 로그 경로를 정할 수 없음");
        return;
    };
    match std::fs::read_to_string(&path) {
        Ok(body) if !body.trim().is_empty() => {
            println!("# audit log: {}", path.display());
            println!("# ts\tmode\taction\ttarget\tstatus\tdetail");
            print!("{}", body);
        }
        _ => println!("# audit log 비어있음(변경 작업 없음): {}", path.display()),
    }
}

/// 테스트 전용 락 — `LMD_AUDIT` 환경변수는 프로세스 전역이라, 이를 만지는 테스트끼리
/// 병렬 실행하면 서로의 값을 덮어써 레이스가 난다. 관련 테스트는 이 락으로 직렬화한다.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_utc_known_epochs() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        // 2021-01-01T00:00:00Z = 1609459200
        assert_eq!(iso_utc(1_609_459_200), "2021-01-01T00:00:00Z");
        // 2000-02-29 (윤년) 12:34:56 = 951827696
        assert_eq!(iso_utc(951_827_696), "2000-02-29T12:34:56Z");
    }

    #[test]
    fn sanitize_strips_newlines() {
        assert_eq!(sanitize("line1\nline2"), "line1");
        assert_eq!(sanitize("  a\tb  "), "a b");
    }

    #[test]
    fn record_writes_tsv_line() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join("lmd-audit-test.log");
        let _ = std::fs::remove_file(&path);
        std::env::set_var("LMD_AUDIT", &path);
        record(Mode::Admin, "scale→2", "ds4", Ok("scaled"));
        record(Mode::Admin, "delete-pod", "p1", Err("boom\nsecond line"));
        std::env::remove_var("LMD_AUDIT");
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        let f0: Vec<&str> = lines[0].split('\t').collect();
        assert_eq!(f0.len(), 6);
        assert_eq!(&f0[1..], ["admin", "scale→2", "ds4", "ok", "scaled"]);
        let f1: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(&f1[3..], ["p1", "FAIL", "boom"]); // 멀티라인 이유는 첫 줄만
        let _ = std::fs::remove_file(&path);
    }
}
