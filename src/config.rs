//! 런타임 설정 단일 출처 — 흩어져 하드코딩되던 튜닝값을 한 곳에.
//! 우선순위: 환경변수 > `~/.config/lmd-top/lmd-top.yaml` > 기본값.
//! (메트릭 이름은 코드-결합이라 metrics.rs 에 상수로 둠 — 여기엔 순수 튜닝값만.)

#[derive(Clone)]
pub struct Config {
    pub prom: String,          // Prometheus host:port (평문 HTTP)
    pub ns: String,            // 대상 네임스페이스
    pub grafana: String,       // g 키가 여는 Grafana base URL
    pub interval_full: u64,    // full 수집 주기(초)
    pub interval_fast: u64,    // fast tier(가속기/노드) 수집 주기(초)
    pub theme: usize,          // 시작 테마: 0 default·1 high-contrast·2 colorblind·3 soft
}

impl Default for Config {
    fn default() -> Self {
        let y = load_yaml();
        // 헬퍼: env > yaml > default
        let s = |env: &str, key: &str, def: &str| -> String {
            std::env::var(env)
                .ok()
                .or_else(|| y.as_ref().and_then(|v| v.get(key)).and_then(|v| v.as_str().map(String::from)))
                .unwrap_or_else(|| def.to_string())
        };
        let u = |env: &str, key: &str, def: u64| -> u64 {
            std::env::var(env)
                .ok()
                .and_then(|s| s.parse().ok())
                .or_else(|| y.as_ref().and_then(|v| v.get(key)).and_then(|v| v.as_u64()))
                .unwrap_or(def)
        };
        // 테마: 이름(soft/colorblind/high-contrast/default) 또는 번호(0~3) 허용.
        // 기본은 soft(Catppuccin) — 눈편함·미감. ANSI-16 원하면 LMD_THEME=default 또는 실행 중 `t`.
        let theme = match s("LMD_THEME", "theme", "soft").to_lowercase().as_str() {
            "1" | "high-contrast" | "hc" => 1,
            "2" | "colorblind" | "cb" => 2,
            "3" | "soft" | "catppuccin" => 3,
            _ => 0,
        };
        Config {
            prom: s("LMD_PROM", "prometheus", "192.0.2.5:30090"),
            ns: s("LMD_NS", "namespace", "llm-serving"),
            grafana: s("LMD_GRAFANA", "grafana", "http://192.0.2.5:30300"),
            interval_full: u("LMD_INTERVAL", "interval_full", 3).max(1),
            interval_fast: u("LMD_FAST_INTERVAL", "interval_fast", 1).max(1),
            theme,
        }
    }
}

/// `~/.config/lmd-top/lmd-top.yaml` 파싱(없으면 None). columns/tunables 공용 파일.
pub fn load_yaml() -> Option<serde_yaml::Value> {
    let path = std::env::var("HOME").ok()? + "/.config/lmd-top/lmd-top.yaml";
    let txt = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&txt).ok()
}
