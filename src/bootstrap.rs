//! Local onboarding: no Kubernetes writes, and never overwrite an existing config.
use crate::config::{config_path, Config};
use anyhow::{bail, Context, Result};
use std::io::{BufRead, Write};

fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    default: &str,
) -> Result<String> {
    write!(output, "{label} [{default}]: ")?;
    output.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        bail!("setup cancelled: input ended; run lmd-top --init in a terminal");
    }
    let line = line.trim();
    Ok(if line.is_empty() {
        default.into()
    } else {
        line.into()
    })
}

fn settings(input: &mut impl BufRead, output: &mut impl Write, cfg: &Config) -> Result<String> {
    let ns = prompt(input, output, "Serving namespace", &cfg.ns)?;
    if ns.len() > 63
        || ns.is_empty()
        || !ns
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || ns.starts_with('-')
        || ns.ends_with('-')
    {
        bail!("invalid namespace: use lowercase letters, digits and internal hyphens (max 63)");
    }
    let prom = prompt(
        input,
        output,
        "Prometheus HTTP host:port (e.g. localhost:9090)",
        &cfg.prom,
    )?;
    // The collector currently speaks plain HTTP to an authority, not arbitrary URLs.
    let (host, port) = prom
        .rsplit_once(':')
        .context("Prometheus must be host:port")?;
    if host.is_empty()
        || prom.contains(['/', '?', '#', '@'])
        || prom.chars().any(char::is_whitespace)
        || port.parse::<u16>().ok().filter(|p| *p > 0).is_none()
    {
        bail!("Prometheus must be plain HTTP host:port; for TLS/auth use a local port-forward");
    }
    let grafana = prompt(
        input,
        output,
        "Grafana URL (optional integration)",
        &cfg.grafana,
    )?;
    if !grafana.starts_with("http://") && !grafana.starts_with("https://") {
        bail!("Grafana URL must start with http:// or https://");
    }
    Ok(serde_yaml::to_string(&serde_json::json!({
        "namespace": ns, "prometheus": prom, "grafana": grafana,
        "interval_full": cfg.interval_full, "interval_fast": cfg.interval_fast,
        "theme": (["default", "high-contrast", "colorblind", "soft"][cfg.theme]),
    }))?)
}

pub fn run(cfg: &Config) -> Result<()> {
    let path = config_path().context("cannot locate config: set LMD_CONFIG or HOME")?;
    if path.exists() {
        bail!(
            "config already exists: {} — edit it, or select a new file with LMD_CONFIG",
            path.display()
        );
    }
    println!("lmd-top setup — save local settings; uses kubectl's current context.");
    println!("Environment overrides (LMD_NS/LMD_PROM) still take precedence. No cluster resources are changed.");
    let text = settings(
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        cfg,
    )?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("create {} (existing files are preserved)", path.display()))?;
    file.write_all(text.as_bytes())?;
    println!("Saved {}", path.display());
    println!("Next: lmd-top --check-setup, then lmd-top (section 6: Setup).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg() -> Config {
        Config {
            ns: "llm-serving".into(),
            prom: "localhost:9090".into(),
            grafana: "http://localhost:3000".into(),
            interval_full: 3,
            interval_fast: 1,
            theme: 3,
            airm_ns: "llm-d".into(),
        }
    }
    #[test]
    fn defaults_roundtrip_as_yaml() {
        let text = settings(&mut &b"\n\n\n"[..], &mut Vec::new(), &cfg()).unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert_eq!(value["namespace"].as_str(), Some("llm-serving"));
        assert_eq!(value["prometheus"].as_str(), Some("localhost:9090"));
    }
    #[test]
    fn invalid_input_and_eof_do_not_produce_config() {
        for input in [
            "",
            "INVALID\n",
            "ok\nhttps://prom:9090\n",
            "ok\nlocalhost:0\n",
            "ok\nlocalhost:9090\nfile:///tmp/x\n",
        ] {
            assert!(
                settings(&mut input.as_bytes(), &mut Vec::new(), &cfg()).is_err(),
                "{input:?}"
            );
        }
    }
}
