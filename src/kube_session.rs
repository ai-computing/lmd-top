//! Freeze the selected kubeconfig for one process so later context switches cannot retarget it.
use crate::process::BlockingCommand;
use anyhow::{anyhow, bail, Result};
use std::{fs::OpenOptions, io::Write, path::PathBuf, sync::OnceLock};

static SESSION: OnceLock<Result<PinnedConfig, String>> = OnceLock::new();

struct PinnedConfig {
    path: PathBuf,
    context: String,
}
impl PinnedConfig {
    fn from_json(bytes: &[u8]) -> Result<Self> {
        let config: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| anyhow!("invalid kubeconfig JSON"))?;
        let context = config["current-context"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("kubeconfig has no current context"))?
            .to_string();
        let selected = config["contexts"]
            .as_array()
            .and_then(|items| items.iter().find(|c| c["name"].as_str() == Some(&context)))
            .ok_or_else(|| anyhow!("selected context is absent"))?;
        let cluster = selected["context"]["cluster"]
            .as_str()
            .ok_or_else(|| anyhow!("selected context has no cluster"))?;
        if !config["clusters"].as_array().is_some_and(|items| {
            items.iter().any(|c| {
                c["name"].as_str() == Some(cluster)
                    && c["cluster"]["server"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
            })
        }) {
            bail!("selected cluster has no server");
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("lmd-top-kube-{}-{nonce}.json", std::process::id()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        let pin = Self { path, context };
        file.write_all(bytes)?;
        Ok(pin)
    }
}
impl Drop for PinnedConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn initialize() -> Result<PinnedConfig> {
    // Only this bootstrap command reads the user's current kubeconfig. Never print raw output.
    let output = BlockingCommand::new("kubectl")
        .args([
            "config",
            "view",
            "--minify",
            "--flatten",
            "--raw",
            "-o",
            "json",
        ])
        .output()?;
    if !output.status.success() {
        bail!(
            "cannot snapshot current kubeconfig; check kubectl configuration and restart lmd-top"
        );
    }
    PinnedConfig::from_json(&output.stdout)
}
fn pinned() -> Result<&'static PinnedConfig> {
    SESSION
        .get_or_init(|| initialize().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow!("Kubernetes session unavailable: {e}"))
}
pub fn context() -> Option<&'static str> {
    SESSION
        .get()
        .and_then(|s| s.as_ref().ok())
        .map(|s| s.context.as_str())
}
pub fn command() -> Result<tokio::process::Command> {
    let pin = pinned()?;
    let mut cmd = tokio::process::Command::new("kubectl");
    cmd.arg("--kubeconfig").arg(&pin.path);
    Ok(cmd)
}
pub fn blocking_command() -> Result<BlockingCommand> {
    let pin = pinned()?;
    let mut cmd = BlockingCommand::new("kubectl");
    cmd.arg("--kubeconfig").arg(&pin.path);
    Ok(cmd)
}
/// Main owns this guard; OnceLock itself is never dropped. No file is created for offline commands.
pub struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(Ok(pin)) = SESSION.get() {
            let _ = std::fs::remove_file(&pin.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_is_private_validated_and_removed() {
        let bytes = br#"{"current-context":"a","contexts":[{"name":"a","context":{"cluster":"a"}}],"clusters":[{"name":"a","cluster":{"server":"https://a"}}],"users":[{"name":"a","user":{"token":"private-test-token"}}]}"#;
        let pin = PinnedConfig::from_json(bytes).unwrap();
        assert_eq!(pin.context, "a");
        assert_eq!(std::fs::read(&pin.path).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&pin.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let path = pin.path.clone();
        drop(pin);
        assert!(!path.exists());
        for invalid in [
            b"{}".as_slice(),
            b"not json",
            br#"{"current-context":"missing"}"#,
        ] {
            assert!(PinnedConfig::from_json(invalid).is_err());
        }
    }
}
