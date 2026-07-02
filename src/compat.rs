//! NPU 컴파일 지원 모델 판별 — 벤더 공식 지원 목록(src/npu-compat.json, 바이너리에 임베드)에서
//! HF/GPU 모델이 RBLN·Furiosa 로 컴파일 가능한지 계열 키워드로 매칭.
//! 목록 갱신은 npu-compat.json 만 고치면 됨(코드 무관).

use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize, Clone)]
pub struct Family {
    pub name: String,
    #[serde(default)]
    pub arch: String,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    #[serde(default)]
    pub rbln: bool,
    #[serde(default)]
    pub furiosa: bool,
    #[serde(default)]
    pub note: String,
}

#[derive(Deserialize)]
struct Db {
    families: Vec<Family>,
}

const RAW: &str = include_str!("npu-compat.json");

fn db() -> &'static Vec<Family> {
    static DB: OnceLock<Vec<Family>> = OnceLock::new();
    DB.get_or_init(|| serde_json::from_str::<Db>(RAW).map(|d| d.families).unwrap_or_default())
}

/// 모델 id/이름에 매칭되는 지원 계열(가장 먼저 매칭되는 것). 대소문자 무시.
pub fn family_of(model_id: &str) -> Option<&'static Family> {
    let lc = model_id.to_lowercase();
    db().iter().find(|f| f.matches.iter().any(|m| lc.contains(m.as_str())))
}

/// 이 모델을 컴파일할 수 있는 벤더 목록("rbln"/"furiosa"). 없으면 빈 벡터.
pub fn compilable_vendors(model_id: &str) -> Vec<&'static str> {
    let mut v = Vec::new();
    if let Some(f) = family_of(model_id) {
        if f.rbln {
            v.push("rbln");
        }
        if f.furiosa {
            v.push("furiosa");
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_parses_nonempty() {
        assert!(!db().is_empty(), "npu-compat.json should parse");
    }

    #[test]
    fn matches_known_families() {
        assert_eq!(family_of("meta-llama/Llama-3.1-8B-Instruct").map(|f| f.name.as_str()), Some("Llama"));
        assert!(compilable_vendors("meta-llama/Llama-3.1-8B").contains(&"rbln"));
        assert!(compilable_vendors("meta-llama/Llama-3.1-8B").contains(&"furiosa"));
        // Gemma → RBLN only.
        let g = compilable_vendors("google/gemma-2-9b");
        assert!(g.contains(&"rbln") && !g.contains(&"furiosa"));
        // Qwen3.
        assert!(compilable_vendors("Qwen/Qwen3-4B").contains(&"furiosa"));
        // 미지원 모델 → 빈 목록.
        assert!(compilable_vendors("some-org/totally-unknown-arch").is_empty());
    }
}
