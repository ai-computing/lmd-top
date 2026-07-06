//! NPU compile-support detection — matches HF/GPU models against family keywords in the vendors' official
//! support list (src/npu-compat.json, embedded in the binary) to see if they compile for RBLN / Furiosa.
//! To update the list, edit only npu-compat.json (no code changes needed).

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
    DB.get_or_init(|| {
        serde_json::from_str::<Db>(RAW)
            .map(|d| d.families)
            .unwrap_or_default()
    })
}

/// Supported family matching the model id/name (the first match). Case-insensitive.
pub fn family_of(model_id: &str) -> Option<&'static Family> {
    let lc = model_id.to_lowercase();
    db().iter()
        .find(|f| f.matches.iter().any(|m| lc.contains(m.as_str())))
}

/// Vendors that can compile this model ("rbln" / "furiosa"). Empty vector if none.
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
        assert_eq!(
            family_of("meta-llama/Llama-3.1-8B-Instruct").map(|f| f.name.as_str()),
            Some("Llama")
        );
        assert!(compilable_vendors("meta-llama/Llama-3.1-8B").contains(&"rbln"));
        assert!(compilable_vendors("meta-llama/Llama-3.1-8B").contains(&"furiosa"));
        // Gemma → RBLN only.
        let g = compilable_vendors("google/gemma-2-9b");
        assert!(g.contains(&"rbln") && !g.contains(&"furiosa"));
        // Qwen3.
        assert!(compilable_vendors("Qwen/Qwen3-4B").contains(&"furiosa"));
        // Unsupported model → empty list.
        assert!(compilable_vendors("some-org/totally-unknown-arch").is_empty());
    }
}
