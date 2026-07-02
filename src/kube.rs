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

// ── HTTPRoute 편집(라우트 관리) — get→JSON 수정→server-side apply ──
fn route_load(ns: &str, name: &str) -> Result<serde_json::Value> {
    let out = std::process::Command::new("kubectl")
        .args(["get", "httproute", name, "-n", ns, "-o", "json", "--request-timeout=8s"])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

fn route_save(ns: &str, mut v: serde_json::Value) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;
    // SSA 를 위해 서버 관리 필드 제거(resourceVersion/managedFields/status 등).
    if let Some(m) = v.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        for k in ["managedFields", "resourceVersion", "uid", "creationTimestamp", "generation"] {
            m.remove(k);
        }
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("status");
    }
    let body = serde_json::to_string(&v)?;
    let mut cmd = std::process::Command::new("kubectl");
    cmd.args(["apply", "--server-side", "--force-conflicts", "-n", ns, "-f", "-", "--request-timeout=8s"]);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?.write_all(body.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 라우트 경로 변경(rename) — HTTPRoute 내 matches[].path.value == old 를 new 로.
pub fn route_set_path(ns: &str, route: &str, old: &str, new: &str) -> Result<String> {
    let mut v = route_load(ns, route)?;
    let mut found = false;
    if let Some(rules) = v["spec"]["rules"].as_array_mut() {
        for rule in rules.iter_mut() {
            if let Some(ms) = rule["matches"].as_array_mut() {
                for m in ms.iter_mut() {
                    if m["path"]["value"].as_str() == Some(old) {
                        m["path"]["value"] = serde_json::Value::String(new.to_string());
                        found = true;
                    }
                }
            }
        }
    }
    if !found {
        return Err(anyhow!("path {} not found in httproute {}", old, route));
    }
    route_save(ns, v)
}

/// 라우트 규칙 삭제(delete) — 해당 path 를 가진 rule 제거.
pub fn route_delete_rule(ns: &str, route: &str, path: &str) -> Result<String> {
    let mut v = route_load(ns, route)?;
    if let Some(rules) = v["spec"]["rules"].as_array_mut() {
        let before = rules.len();
        rules.retain(|rule| {
            rule["matches"]
                .as_array()
                .map(|ms| !ms.iter().any(|m| m["path"]["value"].as_str() == Some(path)))
                .unwrap_or(true)
        });
        if rules.len() == before {
            return Err(anyhow!("path {} not found in httproute {}", path, route));
        }
    }
    route_save(ns, v)
}

/// 라우트 백엔드 변경(retarget) — 해당 path 의 backendRefs 를 새 backend/kind 로.
pub fn route_retarget(ns: &str, route: &str, path: &str, backend: &str, kind: &str) -> Result<String> {
    let mut v = route_load(ns, route)?;
    let group = if kind == "InferencePool" { "inference.networking.k8s.io" } else { "" };
    let mut found = false;
    if let Some(rules) = v["spec"]["rules"].as_array_mut() {
        for rule in rules.iter_mut() {
            let hit = rule["matches"]
                .as_array()
                .map(|ms| ms.iter().any(|m| m["path"]["value"].as_str() == Some(path)))
                .unwrap_or(false);
            if hit {
                rule["backendRefs"] = serde_json::json!([{ "group": group, "kind": kind, "name": backend }]);
                found = true;
            }
        }
    }
    if !found {
        return Err(anyhow!("path {} not found in httproute {}", path, route));
    }
    route_save(ns, v)
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
