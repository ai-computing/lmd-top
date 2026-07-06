//! Deploy▸Zoo — 벤더(Furiosa/Rebellions) 모델 zoo. 공개 HF 모델을 브라우징하고
//! ⏎ 로 Prefetch(가중치 사전 다운로드)·Compile→벤더. 컴파일본이 스토어에 생기면
//! Deploy▸Library 에서 배포한다(배포는 컴파일 산출물 기준이므로 흐름을 분리).
//! 컴파일 가능 벤더는 [[npu-compat]](src/npu-compat.json) 패밀리 지원에서 자동 판정.

use super::*;

impl App {
    /// 현재 Zoo 뷰(상단 패널)에서 선택된 모델(정렬·필터 반영). 하단 Activity 패널이면 None.
    pub(super) fn selected_zoo(&self) -> Option<&crate::catalog::ZooModel> {
        if self.view != View::Zoo || self.panel_focus != 0 {
            return None;
        }
        self.sel_orig().and_then(|i| self.zoo.get(i))
    }

    /// 이 zoo 모델을 컴파일할 수 있는 벤더들(compat 기반). 없으면 GPU 직서빙만 가능.
    pub fn zoo_vendors(source: &str) -> Vec<&'static str> {
        crate::compat::compilable_vendors(source)
    }

    /// 스토어에 이미 이 모델의 컴파일본이 있는가(repo 매칭). Deploy 안내용.
    pub fn zoo_in_store(&self, source: &str) -> bool {
        let repo = source.to_lowercase();
        let short = repo.rsplit('/').next().unwrap_or(&repo);
        self.snap
            .stored
            .iter()
            .any(|s| s.repo.to_lowercase() == repo || s.repo.to_lowercase().contains(short))
    }

    /// 모델의 상황별 상태(Zoo STATUS 열) — 스토어/컴파일중/프리페치중/게이트/미확보.
    /// 반환 `(label, sev)`: sev 0=built, 1=진행중, 2=게이트(토큰필요), 3=available.
    pub fn zoo_state(&self, z: &crate::catalog::ZooModel) -> (&'static str, u8) {
        if self.zoo_in_store(&z.source) {
            return ("● built", 0);
        }
        let repo = z.source.to_lowercase().replace('/', "--");
        let active = |prefetch: bool| {
            self.snap.compiles.iter().any(|c| {
                (c.vendor == "prefetch") == prefetch
                    && (c.status == "Running" || c.status == "Pending")
                    && c.name.to_lowercase().contains(&repo)
            })
        };
        if active(false) {
            return ("◐ compiling", 1);
        }
        if active(true) {
            return ("⇊ prefetch", 1);
        }
        if z.note.to_lowercase().contains("gated") {
            return ("⇩ gated", 2);
        }
        ("○ available", 3)
    }

    /// 선택 zoo 모델 → 컴파일 흐름이 쓰는 합성 아티팩트(첫 컴파일 벤더 기준).
    pub(super) fn selected_zoo_artifact(&self) -> Option<crate::collect::ModelArtifact> {
        let z = self.selected_zoo()?;
        let vendor = Self::zoo_vendors(&z.source).first().copied().unwrap_or("gpu");
        Some(Self::synthetic_artifact_for(&z.source, vendor, String::new(), &[]))
    }

    /// Prefetch — HF 가중치를 공유 스토어 캐시(/mnt/store/hub)로 사전 다운로드하는 Job.
    /// 컴파일 전에 받아두면 컴파일이 오프라인·빠르게 진행되고, 노드 간 공유된다.
    pub fn prefetch_selected_zoo(&mut self) {
        let Some(z) = self.selected_zoo() else {
            return;
        };
        let source = z.source.clone();
        let repo_dir = source.replace('/', "--");
        let name = job_name("prefetch", &repo_dir, "");
        let ns = &self.ns;
        let yaml = format!(
            "# Prefetch Job — download HF weights for {source} into the shared store cache.\n\
             # Reused by compile (HF_HOME=/mnt/store/hub). Review, then apply.\n\
             apiVersion: batch/v1\n\
             kind: Job\n\
             metadata: {{ name: {name}, namespace: {ns} }}\n\
             spec:\n\
             \x20 backoffLimit: 0\n\
             \x20 ttlSecondsAfterFinished: 3600\n\
             \x20 template:\n\
             \x20   spec:\n\
             \x20     restartPolicy: Never\n\
             \x20     volumes:\n\
             \x20       - {{ name: store, persistentVolumeClaim: {{ claimName: model-store }} }}\n\
             \x20     containers:\n\
             \x20       - name: prefetch\n\
             \x20         image: python:3.10-slim\n\
             \x20         resources: {{ requests: {{ cpu: \"2\", memory: \"4Gi\" }} }}\n\
             \x20         env:\n\
             \x20           - {{ name: HF_HOME, value: /mnt/store/hub }}\n\
             \x20           - {{ name: HF_HUB_ENABLE_HF_TRANSFER, value: \"0\" }}\n\
             \x20           - {{ name: HF_TOKEN, valueFrom: {{ secretKeyRef: {{ name: hf-token, key: HF_TOKEN, optional: true }} }} }}\n\
             \x20         command: [\"bash\", \"-c\"]\n\
             \x20         args:\n\
             \x20           - |-\n\
             \x20             set -e\n\
             \x20             pip install -q --no-cache-dir huggingface_hub\n\
             \x20             python -c \"from huggingface_hub import snapshot_download; snapshot_download(repo_id='{source}')\"\n\
             \x20             echo PREFETCH_DONE {source}\n\
             \x20         volumeMounts:\n\
             \x20           - {{ name: store, mountPath: /mnt/store }}\n",
            source = source,
            name = name,
            ns = ns
        );
        self.confirm = Some(Pending::Apply {
            title: format!("prefetch {} → store", source),
            yaml,
        });
        self.confirm_yes = false;
    }
}

#[cfg(test)]
mod zoo_tests {
    use super::*;

    #[test]
    fn bundled_zoo_loads_and_has_valid_hf_ids() {
        let a = App::new();
        assert!(!a.zoo.is_empty(), "bundled zoo.yaml should load");
        // 모든 source 는 org/name 형태(HF repo id) 여야 compile/prefetch 가 동작.
        for z in &a.zoo {
            assert!(z.source.contains('/'), "zoo source must be an HF repo id: {}", z.source);
        }
    }

    #[test]
    fn zoo_vendors_derived_from_compat() {
        // Llama 는 RBLN·Furiosa 둘 다 컴파일 가능.
        let v = App::zoo_vendors("meta-llama/Llama-3.1-8B-Instruct");
        assert!(v.contains(&"rbln") || v.contains(&"furiosa"), "Llama should be compilable");
    }

    #[test]
    fn prefetch_builds_snapshot_download_job() {
        let mut a = App::new();
        a.view = View::Zoo;
        a.selected = 0;
        a.prefetch_selected_zoo();
        match a.confirm {
            Some(Pending::Apply { ref yaml, .. }) => {
                assert!(yaml.contains("kind: Job"));
                assert!(yaml.contains("snapshot_download"));
                assert!(yaml.contains("/mnt/store/hub"), "cache into shared store");
                assert!(yaml.contains("claimName: model-store"));
            }
            _ => panic!("prefetch should stage a Pending::Apply Job"),
        }
    }

    #[test]
    fn merge_zoo_dedups_by_source() {
        use crate::catalog::{merge_zoo, ZooModel};
        let z = |s: &str| ZooModel {
            display: s.into(),
            source: s.into(),
            role: "chat".into(),
            vendor: "furiosa".into(),
            note: String::new(),
        };
        let base = vec![z("furiosa-ai/A"), z("furiosa-ai/B")];
        let live = vec![z("furiosa-ai/B"), z("furiosa-ai/C")]; // B dup, C new
        let merged = merge_zoo(base, live);
        let ids: Vec<_> = merged.iter().map(|m| m.source.clone()).collect();
        assert_eq!(ids, vec!["furiosa-ai/A", "furiosa-ai/B", "furiosa-ai/C"]);
    }

    #[test]
    fn zoo_compile_resolves_source_to_valid_hf_id() {
        // Zoo 선택 → 컴파일 폼이 유효한 HF MODEL_ID 를 만든다(bare name 404 회귀 방지).
        let mut a = App::new();
        a.view = View::Zoo;
        a.selected = 0;
        let art = a.selected_zoo_artifact().expect("zoo artifact");
        assert!(App::artifact_model_id(&art).contains('/'), "compile model_id must be an HF id");
    }
}
