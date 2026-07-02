//! CLI 계약 통합 테스트 — 클러스터 없이 도는 부분(QA-TESTPLAN A 섹션 자동화).
//! 빌드된 바이너리를 실제 실행해 --help/미지원 플래그/잘못된 --mode 의 종료코드·출력을 검증.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_lmd-top")
}

#[test]
fn help_prints_usage_and_exits_zero() {
    for flag in ["--help", "-h"] {
        let out = Command::new(bin()).arg(flag).output().expect("run");
        assert!(out.status.success(), "{flag} should exit 0");
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("USAGE"), "{flag} prints usage");
        assert!(s.contains("--mode") && s.contains("--json"), "{flag} lists options");
    }
}

#[test]
fn unknown_flag_exits_2_with_help() {
    let out = Command::new(bin()).arg("--totally-bogus").output().expect("run");
    assert_eq!(out.status.code(), Some(2), "unknown flag → exit 2");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown argument"), "names the bad flag");
    assert!(err.contains("USAGE"), "shows help on error");
}

#[test]
fn bad_mode_exits_2() {
    let out = Command::new(bin()).args(["--mode", "wizard"]).output().expect("run");
    assert_eq!(out.status.code(), Some(2), "invalid --mode → exit 2");
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid --mode"));
}

#[test]
fn missing_mode_value_exits_2() {
    let out = Command::new(bin()).arg("--mode").output().expect("run");
    assert_eq!(out.status.code(), Some(2), "--mode without value → exit 2");
}
