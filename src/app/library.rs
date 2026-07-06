//! Library/catalog tree — family›version grouping, catalog↔store unification,
//! placement resolution, and synthetic artifacts. Many helpers here are shared
//! by the compile/deploy/action submodules, so they are `pub(super)`.
//! Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    /// family›version 계층 그룹핑 — 키 목록을 (family, version) 로 받아, family/version
    /// 첫 등장 순서를 보존하며 같은 그룹을 인접시킨 원본 인덱스 순서를 돌려준다(트리 표시·내비 공용).
    pub(super) fn grouped_indices(keys: &[(String, String)]) -> Vec<usize> {
        let mut fam_order: Vec<&str> = Vec::new();
        for (f, _) in keys {
            if !fam_order.contains(&f.as_str()) {
                fam_order.push(f);
            }
        }
        let mut out = Vec::with_capacity(keys.len());
        for f in &fam_order {
            let mut ver_order: Vec<&str> = Vec::new();
            for (ff, v) in keys {
                if ff == f && !ver_order.contains(&v.as_str()) {
                    ver_order.push(v);
                }
            }
            for v in &ver_order {
                for (i, (ff, vv)) in keys.iter().enumerate() {
                    if ff == f && vv == v {
                        out.push(i);
                    }
                }
            }
        }
        out
    }

    /// 아티팩트의 version(중간 티어) 라벨 — HF repo/소스(양자화·revision 을 가르는 자리).
    pub fn artifact_version(a: &crate::collect::ModelArtifact) -> String {
        if a.source.is_empty() {
            a.model.clone()
        } else {
            a.source.clone()
        }
    }
    /// 카탈로그 모델의 family 키 — NPU 지원목록 계열명, 없으면 id.
    pub fn catalog_family(m: &crate::catalog::CatModel) -> String {
        crate::compat::family_of(&m.id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| m.id.clone())
    }

    /// Serving 트리(배포 아티팩트)의 그룹 순서 원본 인덱스.
    pub fn serving_order(&self) -> Vec<usize> {
        let keys: Vec<(String, String)> = self
            .snap
            .artifacts
            .iter()
            .map(|a| (a.family.clone(), Self::artifact_version(a)))
            .collect();
        Self::grouped_indices(&keys)
    }
    /// family 그룹 키(소문자) — 카탈로그·스토어를 한 트리로 묶을 때 공용.
    pub(super) fn lib_family(&self, it: LibItem) -> String {
        match it {
            LibItem::Catalog(i) => Self::catalog_family(&self.catalog[i]).to_lowercase(),
            LibItem::Stored(i) => self.snap.stored[i].family.to_lowercase(),
        }
    }
    /// family 첫 등장 순서를 보존하며 같은 family 를 인접시킨다(존 내부 그룹핑).
    pub(super) fn group_by_family(&self, items: &[LibItem]) -> Vec<LibItem> {
        let mut fam_order: Vec<String> = Vec::new();
        for &it in items {
            let f = self.lib_family(it);
            if !fam_order.contains(&f) {
                fam_order.push(f);
            }
        }
        let mut out = Vec::with_capacity(items.len());
        for f in &fam_order {
            for &it in items {
                if &self.lib_family(it) == f {
                    out.push(it);
                }
            }
        }
        out
    }
    /// Library 패널0 통합 배포 트리 — 카탈로그(조직 제공)를 **상단**에, 그 아래 스토어 컴파일본.
    /// 각 존은 family 로 묶는다. order()·렌더 공용. 카탈로그가 "무엇을 배포할 수 있나"의 1차 관문.
    pub fn library_items(&self) -> Vec<LibItem> {
        let cat: Vec<LibItem> = (0..self.catalog.len()).map(LibItem::Catalog).collect();
        let sto: Vec<LibItem> = (0..self.snap.stored.len()).map(LibItem::Stored).collect();
        let mut out = self.group_by_family(&cat);
        out.extend(self.group_by_family(&sto));
        out
    }
    /// Library 패널0에서 선택된 항목(카탈로그 모델 또는 스토어 빌드).
    pub fn selected_lib_item(&self) -> Option<LibItem> {
        if self.view == View::Library && self.panel_focus == 0 {
            self.sel_orig()
                .and_then(|i| self.library_items().get(i).copied())
        } else {
            None
        }
    }
    /// Library 패널0에서 선택된 스토어 빌드(있으면).
    pub fn selected_stored(&self) -> Option<&crate::collect::StoredModel> {
        match self.selected_lib_item() {
            Some(LibItem::Stored(i)) => self.snap.stored.get(i),
            _ => None,
        }
    }

    /// 아티팩트의 모델 식별자 — HF id(source) 우선, 없으면 family.
    pub(super) fn artifact_model_id(a: &crate::collect::ModelArtifact) -> String {
        if a.source.contains('/') && !a.source.starts_with('/') {
            a.source.clone()
        } else {
            a.family.clone()
        }
    }
    pub(super) fn opt_or<'a>(a: &'a crate::collect::ModelArtifact, k: &str, def: &'a str) -> String {
        a.opts
            .iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| def.to_string())
    }

    pub fn selected_catalog_model(&self) -> Option<&crate::catalog::CatModel> {
        match self.selected_lib_item() {
            Some(LibItem::Catalog(i)) => self.catalog.get(i),
            _ => None,
        }
    }

    pub(super) fn preferred_catalog_placement<'a>(
        &self,
        m: &'a crate::catalog::CatModel,
    ) -> Option<&'a crate::catalog::CatPlacement> {
        m.placements.iter().max_by_key(|p| {
            let ready = match crate::catalog::solve(p, &self.snap.inventory).0 {
                crate::catalog::Ready::Ready => 3,
                crate::catalog::Ready::NeedsArtifact => 2,
                crate::catalog::Ready::NoCapacity => 1,
            };
            (ready, (!p.requires_artifact) as i32)
        })
    }

    pub(super) fn placement_vendor(p: &crate::catalog::CatPlacement) -> &'static str {
        let sig = format!("{} {} {} {}", p.engine, p.accel, p.resource, p.uri).to_lowercase();
        if sig.contains("rbln") || sig.contains("rebellions") || sig.contains("atom") {
            "rbln"
        } else if sig.contains("furiosa") || sig.contains("rngd") {
            "furiosa"
        } else {
            "gpu"
        }
    }

    pub(super) fn placement_engine(p: &crate::catalog::CatPlacement) -> &'static str {
        match Self::placement_vendor(p) {
            "rbln" => "vLLM-RBLN",
            "furiosa" => "Furiosa-LLM",
            _ => "vLLM",
        }
    }

    pub(super) fn placement_model_id(
        m: &crate::catalog::CatModel,
        p: &crate::catalog::CatPlacement,
    ) -> String {
        let uri = p.uri.trim();
        if let Some(hf) = uri.strip_prefix("hf://") {
            hf.trim_start_matches('/').to_string()
        } else if uri.contains('/') && !uri.starts_with("pvc://") {
            uri.to_string()
        } else {
            m.id.clone()
        }
    }

    pub(super) fn placement_mount(m: &crate::catalog::CatModel, p: &crate::catalog::CatPlacement) -> String {
        let uri = p.uri.trim();
        if let Some(path) = uri.strip_prefix("pvc://") {
            format!("/mnt/store/{}", path.trim_start_matches('/'))
        } else if let Some(hf) = uri.strip_prefix("hf://") {
            hf.trim_start_matches('/').to_string()
        } else if uri.is_empty() {
            let repo_dir = Self::placement_model_id(m, p).replace('/', "--");
            format!("/mnt/store/compiled/{}", repo_dir)
        } else {
            uri.to_string()
        }
    }

    pub(super) fn catalog_artifact(
        m: &crate::catalog::CatModel,
        p: &crate::catalog::CatPlacement,
    ) -> crate::collect::ModelArtifact {
        let model_id = Self::placement_model_id(m, p);
        crate::collect::ModelArtifact {
            model: if m.display.is_empty() {
                m.id.clone()
            } else {
                m.display.clone()
            },
            family: m.id.clone(),
            engine: Self::placement_engine(p).to_string(),
            node: String::new(),
            image: String::new(),
            source: model_id,
            mount: Self::placement_mount(m, p),
            opts: vec![("tp".into(), p.count.max(1).to_string())],
        }
    }

    pub(super) fn selected_catalog_artifact(&self) -> Option<crate::collect::ModelArtifact> {
        let m = self.selected_catalog_model()?;
        let p = self.preferred_catalog_placement(m)?;
        Some(Self::catalog_artifact(m, p))
    }
}
