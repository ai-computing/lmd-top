//! kubectl 셸링 헬퍼 — JSON 으로 받아 serde_json::Value 로 파싱.
//! Phase 1 은 kubectl 셸링으로 시작(빠름). 추후 kube-rs 네이티브 승격 검토.

use anyhow::{anyhow, Result};
use tokio::process::Command;

/// `kubectl <args...> -o json` 실행 → Value. (args 에 -o json 은 호출자가 포함)
pub async fn get_json(args: &[&str]) -> Result<serde_json::Value> {
    let out = Command::new("kubectl").args(args).arg("--request-timeout=15s").output().await?;
    if !out.status.success() {
        return Err(anyhow!(
            "kubectl {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    Ok(v)
}

/// `.data["<key>"]` 텍스트 추출 (ConfigMap 등).
pub fn cm_data<'a>(cm: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    cm["data"][key].as_str()
}

/// pod 로그 tail (동기, UI 스레드에서 호출). --all-containers, 최근 tail 줄.
pub fn logs(ns: &str, pod: &str, tail: u32) -> Result<Vec<String>> {
    let out = std::process::Command::new("kubectl")
        .args([
            "logs",
            pod,
            "-n",
            ns,
            "--all-containers=true",
            "--prefix=false",
            &format!("--tail={}", tail),
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(|l| l.to_string()).collect())
}

/// 변경 액션: deploy scale. (M5) — 동기 std 사용(블로킹 UI 스레드에서 호출).
pub fn scale_deploy(ns: &str, name: &str, replicas: i64) -> Result<()> {
    let out = std::process::Command::new("kubectl")
        .args([
            "scale",
            "deployment",
            name,
            "-n",
            ns,
            &format!("--replicas={}", replicas),
            "--request-timeout=8s",
        ])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "scale failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// 매니페스트를 stdin 으로 `kubectl apply -f -` 적용. dry_run=true 면 서버 dry-run(무변경 검증).
/// 성공 시 kubectl 출력(생성/변경 요약)을 반환.
pub fn apply_manifest(ns: &str, yaml: &str, dry_run: bool) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut cmd = std::process::Command::new("kubectl");
    cmd.args(["apply", "-n", ns, "-f", "-", "--request-timeout=8s"]);
    if dry_run {
        cmd.arg("--dry-run=server");
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?.write_all(yaml.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 파드 삭제(`kubectl delete pod <name> -n ns`) — 재스케줄 유도. admin 액션.
pub fn delete_pod(ns: &str, name: &str) -> Result<()> {
    let out = std::process::Command::new("kubectl").args(["delete", "pod", name, "-n", ns, "--wait=false", "--request-timeout=8s"]).output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// 리소스 live YAML 조회(`kubectl get <kind> <name> [-n ns] -o yaml`) — 읽기전용 preview 용.
pub fn resource_yaml(kind: &str, ns: Option<&str>, name: &str) -> Result<String> {
    let mut args: Vec<String> = vec!["get".into(), kind.into(), name.into(), "-o".into(), "yaml".into()];
    if let Some(n) = ns {
        args.push("-n".into());
        args.push(n.into());
    }
    args.push("--request-timeout=8s".into());
    let out = std::process::Command::new("kubectl").args(&args).output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 노드 cordon/uncordon — 스케줄 차단/해제. admin 액션(네임스페이스 무관).
pub fn cordon(node: &str, on: bool) -> Result<()> {
    let verb = if on { "cordon" } else { "uncordon" };
    let out = std::process::Command::new("kubectl").args([verb, node, "--request-timeout=8s"]).output()?;
    if !out.status.success() {
        return Err(anyhow!("{} failed: {}", verb, String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// 롤아웃 재시작(`kubectl rollout restart deploy/<name>`) — 롤링 재기동. admin 액션.
pub fn rollout_restart(ns: &str, name: &str) -> Result<()> {
    let out = std::process::Command::new("kubectl")
        .args(["rollout", "restart", "deployment", name, "-n", ns, "--request-timeout=8s"])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "rollout restart failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}
