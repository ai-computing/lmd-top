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
