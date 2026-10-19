//! Single source of truth for runtime settings — tunables that used to be
//! hardcoded all over the place live here.
//! Precedence: env var > `~/.config/lmd-top/lmd-top.yaml` > default.
//! (Metric names stay as constants in metrics.rs because they are code-coupled —
//! only pure tunables belong here.)

#[derive(Clone)]
pub struct Config {
    pub prom: String,       // Prometheus host:port (plain HTTP)
    pub ns: String,         // target namespace
    pub grafana: String,    // Grafana base URL opened by the `g` key
    pub interval_full: u64, // full collection interval (seconds)
    pub interval_fast: u64, // fast-tier (accelerator/node) interval (seconds)
    pub theme: usize,       // startup theme: 0 default·1 high-contrast·2 colorblind·3 soft
    pub airm_ns: String,    // namespace of ai-rm ConfigMap `airm-placement` (read-only)
}

impl Default for Config {
    fn default() -> Self {
        let y = load_yaml();
        // helper: env > yaml > default
        let s = |env: &str, key: &str, def: &str| -> String {
            std::env::var(env)
                .ok()
                .or_else(|| {
                    y.as_ref()
                        .and_then(|v| v.get(key))
                        .and_then(|v| v.as_str().map(String::from))
                })
                .unwrap_or_else(|| def.to_string())
        };
        let u = |env: &str, key: &str, def: u64| -> u64 {
            std::env::var(env)
                .ok()
                .and_then(|s| s.parse().ok())
                .or_else(|| y.as_ref().and_then(|v| v.get(key)).and_then(|v| v.as_u64()))
                .unwrap_or(def)
        };
        // Theme: accepts a name (soft/colorblind/high-contrast/default) or a number (0–3).
        // Default is soft (Catppuccin) — easy on the eyes. For ANSI-16 use LMD_THEME=default
        // or press `t` at runtime.
        let theme = match s("LMD_THEME", "theme", "soft").to_lowercase().as_str() {
            "1" | "high-contrast" | "hc" => 1,
            "2" | "colorblind" | "cb" => 2,
            "3" | "soft" | "catppuccin" => 3,
            _ => 0,
        };
        Config {
            prom: s("LMD_PROM", "prometheus", "localhost:9090"),
            ns: s("LMD_NS", "namespace", "llm-serving"),
            grafana: s("LMD_GRAFANA", "grafana", "http://localhost:3000"),
            interval_full: u("LMD_INTERVAL", "interval_full", 3).max(1),
            interval_fast: u("LMD_FAST_INTERVAL", "interval_fast", 1).max(1),
            theme,
            airm_ns: s("LMD_AIRM_NS", "airm_namespace", "llm-d"),
        }
    }
}

/// Parse `~/.config/lmd-top/lmd-top.yaml` (None if absent). Shared file for columns/tunables.
pub fn load_yaml() -> Option<serde_yaml::Value> {
    let path = config_path()?;
    let txt = match std::fs::read_to_string(&path) {
        Ok(txt) => txt,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("lmd-top: cannot read config {}: {e}", path.display());
            return None;
        }
    };
    match serde_yaml::from_str(&txt) {
        Ok(value @ serde_yaml::Value::Mapping(_)) => Some(value),
        Ok(_) => {
            eprintln!(
                "lmd-top: config {} must be a YAML mapping; using defaults/environment",
                path.display()
            );
            None
        }
        Err(_) => {
            // Do not echo parser errors: YAML snippets may contain credentials.
            eprintln!(
                "lmd-top: invalid YAML in {}; using defaults/environment",
                path.display()
            );
            None
        }
    }
}

/// Explicit config selection also makes onboarding possible without touching HOME.
pub fn config_path() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("LMD_CONFIG").filter(|p| !p.is_empty()) {
        return Some(path.into());
    }
    Some(std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config/lmd-top/lmd-top.yaml"))
}
