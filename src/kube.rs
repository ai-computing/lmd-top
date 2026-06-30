//! kubectl 셸링 헬퍼 — JSON 으로 받아 serde_json::Value 로 파싱.
//! Phase 1 은 kubectl 셸링으로 시작(빠름). 추후 kube-rs 네이티브 승격 검토.

use anyhow::{anyhow, Result};
use tokio::process::Command;

/// `kubectl <args...> -o json` 실행 → Value. (args 에 -o json 은 호출자가 포함)
pub async fn get_json(args: &[&str]) -> Result<serde_json::Value> {
    let out = Command::new("kubectl").args(args).output().await?;
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
