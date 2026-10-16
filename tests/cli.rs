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
        assert!(
            s.contains("--mode") && s.contains("--json"),
            "{flag} lists options"
        );
    }
}

#[test]
fn unknown_flag_exits_2_with_help() {
    let out = Command::new(bin())
        .arg("--totally-bogus")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2), "unknown flag → exit 2");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown argument"), "names the bad flag");
    assert!(err.contains("USAGE"), "shows help on error");
}

#[test]
fn bad_mode_exits_2() {
    let out = Command::new(bin())
        .args(["--mode", "wizard"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2), "invalid --mode → exit 2");
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid --mode"));
}

#[test]
fn missing_mode_value_exits_2() {
    let out = Command::new(bin()).arg("--mode").output().expect("run");
    assert_eq!(out.status.code(), Some(2), "--mode without value → exit 2");
}

#[test]
fn version_is_available_without_cluster() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        concat!("lmd-top ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn init_saves_valid_settings_and_preserves_existing_file() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = std::env::temp_dir().join(format!("lmd-init-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.yaml");
    let mut child = Command::new(bin())
        .arg("--init")
        .env("LMD_CONFIG", &path)
        .env("LMD_NS", "llm-serving")
        .env("LMD_PROM", "localhost:9090")
        .env("LMD_GRAFANA", "http://localhost:3000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"team-a\nprom.internal:9090\n\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let original = std::fs::read_to_string(&path).unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&original).unwrap();
    assert_eq!(value["namespace"].as_str(), Some("team-a"));
    assert_eq!(value["prometheus"].as_str(), Some("prom.internal:9090"));
    let out = Command::new(bin())
        .arg("--init")
        .env("LMD_CONFIG", &path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    let cancelled = dir.join("cancelled.yaml");
    let out = Command::new(bin())
        .arg("--init")
        .env("LMD_CONFIG", &cancelled)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!cancelled.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn setup_check_reports_missing_cluster_with_failure_exit() {
    let out = Command::new(bin())
        .arg("--check-setup")
        .env("PATH", "") // No kubectl: exercise the offline failure path without cluster access.
        .env("LMD_CONFIG", "/dev/null")
        .env("LMD_NS", "llm-serving")
        .env("LMD_PROM", "127.0.0.1:1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("? [Cluster] kubectl access"));
    assert!(stdout.contains("0 required checks missing; 1 checks unknown"));
}

#[test]
fn policy_workspace_renders_offline_and_refuses_overwrite_or_apply() {
    let dir = std::env::temp_dir().join(format!("lmd-policy-test-{}", std::process::id()));
    let init = || {
        Command::new(bin())
            .args(["--policy", "init", "--workspace"])
            .arg(&dir)
            .env("PATH", "")
            .output()
            .unwrap()
    };
    let out = init();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join("pkg/heterogeneous/config_test.go").is_file());
    let original = std::fs::read(dir.join("policy.yaml")).unwrap();
    assert!(!init().status.success());
    assert_eq!(std::fs::read(dir.join("policy.yaml")).unwrap(), original);
    let out = Command::new(bin())
        .args(["--policy", "render", "--file"])
        .arg(dir.join("policy.yaml"))
        .arg("--base")
        .arg(dir.join("examples/base-epp.yaml"))
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rendered: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
    let expected: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(dir.join("examples/rendered-epp.yaml")).unwrap())
            .unwrap();
    assert_eq!(rendered, expected);
    let out = Command::new(bin())
        .args(["--policy", "plan", "--apply"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("confirmation"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn plan_apply_requires_admin_before_cluster_collection() {
    for mode in ["observe", "debug"] {
        let output = Command::new(bin())
            .args([
                "--plan",
                "deploy",
                "--model",
                "org/model",
                "--vendor",
                "gpu",
                "--apply",
                "--mode",
                mode,
            ])
            .env("PATH", "/nonexistent")
            .env("LMD_PROM", "127.0.0.1:1")
            .output()
            .expect("run absolute binary");
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("requires --mode admin"), "{stderr}");
        assert!(
            !stderr.contains("kubectl"),
            "must fail before collection: {stderr}"
        );
    }
}

#[test]
#[cfg(unix)]
fn kubectl_session_ignores_later_current_context_changes_and_cleans_snapshot() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("lmd-context-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join("kubectl");
    let trace = dir.join("trace.jsonl");
    // The fake changes the source context immediately after config view. Every subsequent
    // invocation must use the private snapshot rather than discovering the new context.
    std::fs::write(&executable, r#"#!/usr/bin/python3
import json, os, sys
from pathlib import Path
args = sys.argv[1:]
root = Path(os.environ['TEST_KUBE_ROOT'])
if args[:2] == ['config', 'view']:
    config = {'current-context':'cluster-a','contexts':[{'name':'cluster-a','context':{'cluster':'a'}}], 'clusters':[{'name':'a','cluster':{'server':'https://cluster-a.invalid'}}]}
    (root/'current').write_text('cluster-b')
    print(json.dumps(config))
else:
    assert args[0] == '--kubeconfig', args
    path = Path(args[1])
    config = json.loads(path.read_text())
    assert config['current-context'] == 'cluster-a', config
    assert (root/'current').read_text() == 'cluster-b'
    assert path.stat().st_mode & 0o777 == 0o600
    with (root/'trace.jsonl').open('a') as f:
        f.write(json.dumps({'snapshot':str(path), 'args':args[2:]})+'\n')
    print(json.dumps({'items':[]}))
"#).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(bin())
        .arg("--json")
        .env("PATH", &dir)
        .env("TEST_KUBE_ROOT", &dir)
        .env("LMD_CONFIG", "/dev/null")
        .env("LMD_PROM", "127.0.0.1:1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(state["kubernetes_context"], "cluster-a");
    let trace = std::fs::read_to_string(trace).unwrap();
    let calls: Vec<serde_json::Value> = trace
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(calls.len() > 2, "exercise repeated collection calls");
    for call in calls {
        assert!(
            !std::path::Path::new(call["snapshot"].as_str().unwrap()).exists(),
            "snapshot removed on normal exit"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_set_exits_before_any_cluster_work() {
    for value in ["batch", "=2", "batch=", " =2"] {
        let out = Command::new(bin())
            .args(["--plan", "compile", "--set", value])
            .env("PATH", "/nonexistent")
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(2), "{value:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("expected nonempty key=value"));
    }
}
