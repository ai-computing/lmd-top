//! Model catalog — deployable models × accelerator placement candidates (read-only).
//! The default catalog is embedded in the binary (catalog/models.yaml). Override with LMD_CATALOG.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CatModel {
    pub id: String,
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub role: String,
    /// Canonical Hugging Face weights id (org/name) for **compilation** — the source to build
    /// RBLN/Furiosa artifacts from. Falls back to the first `hf://` placement, then `id`.
    /// Set this for models whose only placement is a precompiled `pvc://` artifact.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub placements: Vec<CatPlacement>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatPlacement {
    pub engine: String,
    pub accel: String,
    pub resource: String,
    pub count: i64,
    #[serde(default = "one")]
    pub replicas: i64,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub requires_artifact: bool,
}
fn one() -> i64 {
    1
}

#[derive(Debug, Deserialize)]
struct Root {
    #[serde(default)]
    models: Vec<CatModel>,
}

const DEFAULT: &str = include_str!("../catalog/models.yaml");

pub fn load() -> Vec<CatModel> {
    let txt = std::env::var("LMD_CATALOG")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DEFAULT.to_string());
    serde_yaml::from_str::<Root>(&txt)
        .map(|r| r.models)
        .unwrap_or_default()
}

/// Vendor model-zoo entry — a Hugging Face model that can be prefetched/compiled/served
/// on Furiosa(RNGD) / Rebellions(RBLN). Compilable vendors are derived from `source`
/// via `crate::compat`, so this only needs the canonical HF id + display.
#[derive(Debug, Clone, Deserialize)]
pub struct ZooModel {
    pub display: String,
    pub source: String, // HF repo id (org/name)
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub vendor: String, // 출처 벤더 힌트(furiosa=사전양자화 / rbln=지원패밀리). 컴파일 벤더는 compat 로 별도 판정.
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Deserialize)]
struct ZooRoot {
    #[serde(default)]
    models: Vec<ZooModel>,
}

const DEFAULT_ZOO: &str = include_str!("../catalog/zoo.yaml");

/// Load the vendor model zoo (bundled `catalog/zoo.yaml`, override with `LMD_ZOO`).
pub fn load_zoo() -> Vec<ZooModel> {
    let txt = std::env::var("LMD_ZOO")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DEFAULT_ZOO.to_string());
    serde_yaml::from_str::<ZooRoot>(&txt)
        .map(|r| r.models)
        .unwrap_or_default()
}

/// Live-refresh the vendor model zoos from HuggingFace.
///
/// Each accelerator pack declares the HF organisations its vendor publishes under, so a newly
/// registered accelerator's models appear here without touching this function. Rebellions has
/// no HF org — its supported-model list is a GitHub repo folded into `catalog/zoo.yaml` by
/// `scripts/fetch-zoo.sh` — so it contributes nothing to a live refresh, by design.
///
/// Uses `curl` because lmd-top ships no in-binary TLS (same shell-out ethos as kubectl).
/// Independent results per org: preserve successful entries and report failures,
/// so an unreachable vendor never produces a false "up to date" message.
pub struct ZooRefresh {
    pub models: Vec<ZooModel>,
    pub errors: Vec<String>,
}

impl ZooRefresh {
    pub fn message(&self, added: usize) -> String {
        if self.errors.is_empty() {
            format!("zoo refresh completed — +{added} new models")
        } else {
            format!(
                "zoo refresh incomplete — +{added} new; {}",
                self.errors.join("; ")
            )
        }
    }
}

pub async fn fetch_zoo_live() -> ZooRefresh {
    let mut set = tokio::task::JoinSet::new();
    for pack in crate::accel::packs().iter().copied() {
        for org in pack.hf_orgs {
            set.spawn(fetch_hf_org(pack, org));
        }
    }
    let mut result = ZooRefresh {
        models: Vec::new(),
        errors: Vec::new(),
    };
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(models)) => result.models.extend(models),
            Ok(Err(error)) => result.errors.push(error),
            Err(error) => result.errors.push(format!("refresh task failed: {error}")),
        }
    }
    result.models.sort_by_key(|z| z.source.to_lowercase());
    result.errors.sort();
    result
}

async fn fetch_hf_org(
    pack: &'static crate::accel::Pack,
    org: &str,
) -> Result<Vec<ZooModel>, String> {
    let url = format!("https://huggingface.co/api/models?author={}&limit=500", org);
    let mut command = tokio::process::Command::new("curl");
    command.args(["-fsSL", "--max-time", "8", &url]);
    let out = crate::process::run(command, None, tokio::time::Duration::from_secs(10))
        .await
        .map_err(|error| format!("{org}: {error}"))?;
    if !out.status.success() {
        return Err(format!("{org}: curl failed ({})", out.status));
    }
    parse_hf_org(pack, org, &out.stdout)
}

fn parse_hf_org(
    pack: &crate::accel::Pack,
    org: &str,
    bytes: &[u8],
) -> Result<Vec<ZooModel>, String> {
    let arr: Vec<serde_json::Value> = serde_json::from_slice(bytes)
        .map_err(|error| format!("{org}: invalid model list: {error}"))?;
    let mut zoo = Vec::new();
    for m in &arr {
        let id = m["id"].as_str().unwrap_or("");
        // Remote input: these ids end up in generated manifests. Reject anything that is not a
        // plain HF repo id right here, so a hostile or malformed upstream entry can never reach
        // manifest generation (BUG-06).
        if !crate::quote::valid_model_id(id) {
            return Err(format!("{org}: invalid model id in response"));
        }
        let role = match m["pipeline_tag"].as_str().unwrap_or("") {
            "text-generation" => "chat",
            "sentence-similarity" => "embedding",
            "text-classification" => "reranker",
            // Anything else is not something this tool knows how to serve.
            _ => continue,
        };
        let disp = id.rsplit('/').next().unwrap_or(id);
        zoo.push(ZooModel {
            display: format!("{} ({})", disp, pack.display),
            source: id.to_string(),
            role: role.to_string(),
            vendor: pack.id.to_string(),
            note: format!("{} HF org: {}", pack.display, org),
        });
    }
    Ok(zoo)
}

/// Merge live entries into a base list, deduped by `source` (base kept; genuinely new appended).
pub fn merge_zoo(mut base: Vec<ZooModel>, extra: Vec<ZooModel>) -> Vec<ZooModel> {
    let mut have: std::collections::HashSet<String> =
        base.iter().map(|z| z.source.to_lowercase()).collect();
    for z in extra {
        if have.insert(z.source.to_lowercase()) {
            base.push(z);
        }
    }
    base
}

/// 배치 준비 상태.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ready {
    Ready,         // 즉시 배포 가능(용량 충분 + 가중치/아티팩트 준비)
    NeedsArtifact, // 사전 컴파일/다운로드 산출물 필요
    NoCapacity,    // 가속기 여유 부족
}

/// 배치 후보 × 라이브 재고 → 준비상태 + 여유 수량.
/// inventory: (resource, total, used)
pub fn solve(p: &CatPlacement, inventory: &[(String, i64, i64)]) -> (Ready, i64, i64) {
    let (total, used) = inventory
        .iter()
        .find(|(r, _, _)| r == &p.resource)
        .map(|(_, t, u)| (*t, *u))
        .unwrap_or((0, 0));
    let free = (total - used).max(0);
    let need = p.count * p.replicas.max(1);
    let state = if free < need {
        Ready::NoCapacity
    } else if p.requires_artifact {
        Ready::NeedsArtifact
    } else {
        Ready::Ready
    };
    (state, free, need)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(count: i64, replicas: i64, requires_artifact: bool) -> CatPlacement {
        CatPlacement {
            engine: "vllm".into(),
            accel: "gpu".into(),
            resource: "nvidia.com/gpu".into(),
            count,
            replicas,
            uri: String::new(),
            requires_artifact,
        }
    }

    #[test]
    fn solve_readiness_and_capacity() {
        let inv = vec![("nvidia.com/gpu".to_string(), 8, 2)]; // free = 8 - 2 = 6

        // need = count * replicas = 2*2 = 4 ≤ 6, no artifact → Ready
        let (state, free, need) = solve(&placement(2, 2, false), &inv);
        assert_eq!((free, need), (6, 4));
        assert_eq!(state, Ready::Ready);

        // capacity ok but artifact required → NeedsArtifact
        assert_eq!(solve(&placement(2, 2, true), &inv).0, Ready::NeedsArtifact);

        // need (4*2=8) > free (6) → NoCapacity wins over artifact requirement
        assert_eq!(solve(&placement(4, 2, true), &inv).0, Ready::NoCapacity);

        // resource absent from inventory → (total,used)=(0,0) fallback → free 0 → NoCapacity
        let (state2, free2, need2) = solve(&placement(1, 1, false), &[]);
        assert_eq!((free2, need2), (0, 1));
        assert_eq!(state2, Ready::NoCapacity);

        // replicas < 1 is clamped to 1 when computing need
        assert_eq!(solve(&placement(3, 0, false), &inv).2, 3);
    }
}

#[cfg(test)]
mod live_tests {
    /// The refresh is driven by the packs, so a newly registered accelerator's HF org is
    /// queried without editing catalog.rs. Rebellions declares none on purpose — its list is
    /// a GitHub repo, and silently querying a nonexistent org would look like a broken fetch.
    #[test]
    fn live_refresh_covers_declared_orgs() {
        let orgs: Vec<&str> = crate::accel::packs()
            .iter()
            .flat_map(|p| p.hf_orgs.iter().copied())
            .collect();
        assert!(orgs.contains(&"furiosa-ai"), "furiosa org: {:?}", orgs);
        assert!(orgs.contains(&"nvidia"), "nvidia org: {:?}", orgs);
        assert!(
            crate::accel::by_id("rbln").unwrap().hf_orgs.is_empty(),
            "Rebellions has no HF org publishing models — declaring one would query nothing"
        );
    }

    /// Live entries are tagged with the pack that produced them, so the zoo can filter and
    /// colour them and the compile action targets the right accelerator.
    #[test]
    fn merge_keeps_bundled_entries_and_appends_new_ones() {
        let base = vec![super::ZooModel {
            display: "Kept".into(),
            source: "furiosa-ai/Existing".into(),
            role: "chat".into(),
            vendor: "furiosa".into(),
            note: "bundled".into(),
        }];
        let live = vec![
            super::ZooModel {
                display: "Dup".into(),
                source: "FURIOSA-AI/EXISTING".into(), // same id, different case
                role: "chat".into(),
                vendor: "furiosa".into(),
                note: "live".into(),
            },
            super::ZooModel {
                display: "New".into(),
                source: "nvidia/Nemotron-X".into(),
                role: "chat".into(),
                vendor: "gpu".into(),
                note: "live".into(),
            },
        ];
        let merged = super::merge_zoo(base, live);
        assert_eq!(merged.len(), 2, "case-insensitive dedup by source");
        assert_eq!(merged[0].note, "bundled", "bundled entry wins");
        assert_eq!(merged[1].vendor, "gpu");
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    #[test]
    fn malformed_remote_catalog_is_not_a_successful_empty_refresh() {
        let pack = crate::accel::packs()[0];
        for payload in [
            b"{}".as_slice(),
            b"null",
            b"[{}]",
            br#"[{"id":"bad;command","pipeline_tag":"text-generation"}]"#,
        ] {
            assert!(parse_hf_org(pack, "test", payload).is_err());
        }
        assert!(parse_hf_org(pack, "test", b"[]").unwrap().is_empty());
        let failed = ZooRefresh {
            models: Vec::new(),
            errors: vec!["test: unavailable".into()],
        };
        assert!(failed.message(0).contains("incomplete"));
        assert!(failed.message(0).contains("unavailable"));
    }

    #[test]
    fn partial_refresh_preserves_base_and_deduplicates_remote_results() {
        let pack = crate::accel::packs()[0];
        let payload = br#"[{"id":"vendor/new","pipeline_tag":"text-generation"},{"id":"vendor/new","pipeline_tag":"text-generation"},{"id":"vendor/unsupported","pipeline_tag":"image-classification"}]"#;
        let mut refresh = ZooRefresh {
            models: parse_hf_org(pack, "vendor", payload).unwrap(),
            errors: vec!["other: failed".into()],
        };
        let base = parse_hf_org(
            pack,
            "vendor",
            br#"[{"id":"vendor/existing","pipeline_tag":"sentence-similarity"}]"#,
        )
        .unwrap();
        let merged = merge_zoo(base, std::mem::take(&mut refresh.models));
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|m| m.source == "vendor/existing"));
        assert_eq!(
            merged.iter().filter(|m| m.source == "vendor/new").count(),
            1
        );
        assert!(refresh.message(1).contains("incomplete"));
    }
}
