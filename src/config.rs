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
        }
    }
}

/// Parse `~/.config/lmd-top/lmd-top.yaml` (None if absent). Shared file for columns/tunables.
pub fn load_yaml() -> Option<serde_yaml::Value> {
    let path = std::env::var("HOME").ok()? + "/.config/lmd-top/lmd-top.yaml";
    let txt = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&txt).ok()
}
