//! NPU compile flow — form construction, fit estimation, preflight checks, and
//! headless compile/deploy planning.
//!
//! Submodules:
//! - [`fields`]: Compilation field definitions, defaults, and option schemas.
//! - [`fit`]: Model parameter estimation and memory fit heuristics.
//! - [`preflight`]: Pre-compile validation, hardware compatibility, and store collision checks.
//! - [`manifest`]: Kubernetes Job and ConfigMap manifest generation.
//! - [`plan`]: Headless and CLI planning utilities.

pub mod fields;
pub mod fit;
pub mod manifest;
pub mod plan;
pub mod preflight;

pub use fields::*;
pub use manifest::*;

use super::*;
use crate::ops::{CompileField, CompileForm, FitEstimate};

impl App {
    /// `[c] compile` — Open NPU compile form for selected artifact.
    /// If engine is NPU, targets that vendor; if GPU/HF but in npu-compat list,
    /// allows compiling for supported NPU vendors (GPU→NPU migration path).
    pub fn compile_preview(&mut self) {
        let owned;
        let a = if let Some(a) = self.selected_artifact() {
            a
        } else if let Some(cat) = self.selected_catalog_artifact() {
            owned = cat;
            &owned
        } else if let Some(z) = self.selected_zoo_artifact() {
            owned = z;
            &owned
        } else {
            return;
        };
        let model_id = Self::artifact_model_id(a);
        let vendor: Option<&'static str> = if a.engine.contains("RBLN") {
            Some("rbln")
        } else if a.engine.contains("Furiosa") {
            Some("furiosa")
        } else {
            crate::compat::compilable_vendors(&model_id)
                .first()
                .copied()
        };
        match vendor {
            Some(v) => {
                let form = self.build_compile_form(a, v);
                self.compile_form = Some(form);
            }
            None => {
                self.preview = Some((
                    format!("compile · {}", a.model),
                    format!(
                        "# {}\n# This model family is not in the NPU compile support list (RBLN/Furiosa).\n# Supported families: Llama, Qwen2/3, Gemma, Mistral, EXAONE, Phi, OPT, GPT2, SOLAR, DeepSeek, T5, ...\n# Source list: src/npu-compat.json (based on vendor documentation)\n",
                        model_id
                    ),
                ));
                self.preview_scroll = 0;
                self.preview_apply = false;
            }
        }
    }

    /// Open compile form explicitly targeting a vendor (used by 'Compile → RBLN/Furiosa' action menu).
    pub fn compile_form_for(&mut self, vendor: &'static str) {
        let owned;
        let a = if let Some(a) = self.selected_artifact() {
            a
        } else if let Some(cat) = self.selected_catalog_artifact() {
            owned = cat;
            &owned
        } else if let Some(z) = self.selected_zoo_artifact() {
            owned = z;
            &owned
        } else {
            return;
        };
        let form = self.build_compile_form(a, vendor);
        self.compile_form = Some(form);
    }

    /// Build a compile form for the given artifact and target vendor.
    pub(super) fn build_compile_form(
        &self,
        a: &crate::collect::ModelArtifact,
        vendor: &'static str,
    ) -> CompileForm {
        let rbln = vendor == "rbln";
        let model_id = Self::artifact_model_id(a);
        let mut form_fields = vendor_compile_fields(vendor, a);

        let want_kind = if rbln {
            crate::collect::AccelKind::Rbln
        } else {
            crate::collect::AccelKind::Rngd
        };

        // Determine maximum accelerator count per node across cluster for devices choices
        let max_dev = self
            .snap
            .accel
            .iter()
            .filter(|ac| ac.kind == want_kind && !ac.node.is_empty())
            .fold(std::collections::HashMap::new(), |mut acc, ac| {
                *acc.entry(&ac.node).or_insert(0i64) += 1;
                acc
            })
            .into_values()
            .max()
            .unwrap_or(4)
            .max(1);

        // Required default device count: RBLN=TP, Furiosa=ceil(TP/8)*PP
        let tp_v = form_fields
            .iter()
            .find(|f| f.key == "tp")
            .and_then(|f| f.value.parse::<i64>().ok())
            .unwrap_or(1)
            .max(1);
        let pp_v = form_fields
            .iter()
            .find(|f| f.key == "pp")
            .and_then(|f| f.value.parse::<i64>().ok())
            .unwrap_or(1)
            .max(1);
        let dev_default = if rbln {
            tp_v
        } else {
            ((tp_v as f64 / 8.0).ceil() as i64).max(1) * pp_v
        };
        let dev_choices: Vec<String> = (1..=max_dev.max(dev_default))
            .map(|i| i.to_string())
            .collect();
        form_fields.push(CompileField {
            key: "devices".into(),
            label: "devices".into(),
            value: dev_default.to_string(),
            choices: dev_choices,
            numeric: true,
            help: "Requested accelerator device count (resources.limits). Usually TP for Rebellions or ceil(TP/8)×PP for Furiosa.".into(),
        });

        CompileForm {
            model: a.model.clone(),
            model_id,
            vendor,
            engine: a.engine.clone(),
            fields: form_fields,
            cursor: 0,
            editing: false,
            dest: String::new(),
        }
    }

    /// Submit current compile form, validating constraints and generating the Job preview manifest.
    pub fn compile_form_submit(&mut self) {
        let Some(form) = self.compile_form.take() else {
            return;
        };

        match build_compile_manifest(
            &form,
            &self.ns,
            self.img_rbln.as_deref(),
            self.img_furiosa.as_deref(),
            &self.snap.nodes,
        ) {
            CompileManifestOutcome::Ready { title, yaml } => {
                self.confirm = Some(Pending::Apply { title, yaml });
                self.confirm_yes = false;
            }
            CompileManifestOutcome::Blocked { issue } => {
                self.notify(format!("compile blocked — {}", issue));
                self.compile_form = Some(form);
            }
            CompileManifestOutcome::UnresolvedSource { model_id } => {
                self.preview = Some((
                    format!("compile · {} — source unresolved", form.model),
                    format!(
                        "# Cannot compile: MODEL_ID '{id}' is not a valid Hugging Face repo id (expected org/name)\n\
                         # and is not a local path. RBLN/Furiosa compile downloads the source weights via\n\
                         # from_pretrained(MODEL_ID), so it would 404 on https://huggingface.co/{id}.\n\
                         #\n\
                         # Fix: give this model a canonical HF source. In your catalog (catalog/models.yaml or LMD_CATALOG):\n\
                         #   - id: {id}\n\
                         #     source: <org>/<name>        # e.g. meta-llama/Llama-3.1-8B-Instruct\n\
                         # or add an `hf://<org>/<name>` placement. Alternatively pre-place the weights in the store.\n",
                        id = model_id
                    ),
                ));
                self.preview_scroll = 0;
                self.preview_apply = false;
            }
            CompileManifestOutcome::InvalidVendor { vendor } => {
                self.notify(format!(
                    "compile is NPU-only (rbln/furiosa) — '{}' models are served directly",
                    vendor
                ));
            }
        }
    }

    /// Estimate compile memory fit and optimization advice.
    pub fn compile_fit(&self, form: &CompileForm) -> FitEstimate {
        fit::estimate_compile_fit(&self.snap.accel, &self.snap.nodes, form)
    }

    /// Run preflight checklist before compilation.
    pub fn compile_preflight(&self, form: &CompileForm) -> Vec<(bool, String)> {
        preflight::compile_preflight(
            &self.snap.stored,
            &self.snap.nodes,
            self.img_rbln.as_deref(),
            form,
        )
    }

    /// Create synthetic model artifact for headless planning.
    pub(super) fn synthetic_artifact_for(
        model_id: &str,
        vendor: &'static str,
        mount: String,
        overrides: &[(String, String)],
    ) -> crate::collect::ModelArtifact {
        plan::synthetic_artifact_for(model_id, vendor, mount, overrides)
    }

    /// Headless compilation planning.
    pub fn plan_compile_for_model(
        &mut self,
        model_id: &str,
        vendor: &'static str,
        overrides: &[(String, String)],
    ) -> Result<(String, String), String> {
        plan::plan_compile_for_model(self, model_id, vendor, overrides)
    }

    /// Headless deployment planning.
    pub fn plan_deploy_for_model(
        &mut self,
        model_id: &str,
        vendor: &'static str,
        overrides: &[(String, String)],
    ) -> Result<(String, String), String> {
        plan::plan_deploy_for_model(self, model_id, vendor, overrides)
    }
}

#[cfg(test)]
mod compile_tests {
    use super::*;
    use crate::catalog::{CatModel, CatPlacement};

    fn placement(engine: &str, resource: &str, uri: &str) -> CatPlacement {
        CatPlacement {
            engine: engine.into(),
            accel: String::new(),
            resource: resource.into(),
            count: 4,
            replicas: 1,
            uri: uri.into(),
            requires_artifact: uri.starts_with("pvc://"),
        }
    }
    fn model(id: &str, source: &str, placements: Vec<CatPlacement>) -> CatModel {
        CatModel {
            id: id.into(),
            display: String::new(),
            role: "chat".into(),
            source: source.into(),
            placements,
        }
    }

    #[test]
    fn hf_source_prefers_sibling_hf_placement_over_pvc() {
        let m = model(
            "llama3.1-8b",
            "",
            vec![
                placement(
                    "vllm",
                    "nvidia.com/gpu",
                    "hf://meta-llama/Llama-3.1-8B-Instruct",
                ),
                placement(
                    "vllm-rbln",
                    "rebellions.ai/ATOM",
                    "pvc://rbln-artifacts/llama31-8b-tp4",
                ),
            ],
        );
        assert_eq!(
            App::catalog_hf_source(&m),
            "meta-llama/Llama-3.1-8B-Instruct"
        );
        let rbln_p = &m.placements[1];
        let art = App::catalog_artifact(&m, rbln_p);
        assert_eq!(
            App::artifact_model_id(&art),
            "meta-llama/Llama-3.1-8B-Instruct"
        );
    }

    #[test]
    fn hf_source_explicit_field_wins() {
        let m = model(
            "qwen3-embedding-8b",
            "Qwen/Qwen3-Embedding-8B",
            vec![placement(
                "furiosa",
                "furiosa.ai/rngd",
                "pvc://furiosa-artifacts/qwen3-embed",
            )],
        );
        assert_eq!(App::catalog_hf_source(&m), "Qwen/Qwen3-Embedding-8B");
    }

    #[test]
    fn hf_source_falls_back_to_id_when_unknown() {
        let m = model(
            "koni-llama3.1-8b",
            "",
            vec![placement(
                "vllm-rbln",
                "rebellions.ai/ATOM",
                "pvc://rbln-artifacts/koni-tp4",
            )],
        );
        assert_eq!(App::catalog_hf_source(&m), "koni-llama3.1-8b");
    }

    #[test]
    fn compile_blocks_invalid_hf_id() {
        let mut a = App::new();
        let r = a.plan_compile_for_model("llama3.1-8b", "rbln", &[]);
        let err = r.expect_err("bare name must not produce a compile manifest");
        assert!(
            err.contains("source unresolved") && err.contains("llama3.1-8b"),
            "error should carry the guidance: {}",
            err
        );
    }

    #[test]
    fn compile_allows_valid_hf_id() {
        let mut a = App::new();
        let r = a.plan_compile_for_model("meta-llama/Llama-3.1-8B-Instruct", "rbln", &[]);
        let (_, yaml) = r.expect("valid HF id should produce a manifest");
        assert!(yaml.contains("meta-llama/Llama-3.1-8B-Instruct"));
        assert!(yaml.contains("/mnt/store/hub"), "HF cache on shared store");
    }

    fn ov(kvs: &[(&str, &str)]) -> Vec<(String, String)> {
        kvs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn rbln_blocks_flash_attn_kvpart_mismatch() {
        let mut a = App::new();
        let r = a.plan_compile_for_model(
            "meta-llama/Llama-3.1-8B-Instruct",
            "rbln",
            &ov(&[
                ("max-len", "8192"),
                ("kvpart", "16384"),
                ("attn", "flash_attn"),
            ]),
        );
        assert!(
            r.is_err(),
            "8192 not a multiple of 16384 must be blocked before the Job"
        );
    }

    #[test]
    fn rbln_allows_valid_flash_attn_combo() {
        let mut a = App::new();
        let r = a.plan_compile_for_model(
            "meta-llama/Llama-3.1-8B-Instruct",
            "rbln",
            &ov(&[
                ("max-len", "16384"),
                ("kvpart", "8192"),
                ("attn", "flash_attn"),
            ]),
        );
        assert!(r.is_ok(), "16384 % 8192 == 0 should compile");
    }

    #[test]
    fn rbln_eager_skips_kvpart_divisibility() {
        let mut a = App::new();
        let r = a.plan_compile_for_model(
            "meta-llama/Llama-3.1-8B-Instruct",
            "rbln",
            &ov(&[("max-len", "2048"), ("attn", "eager")]),
        );
        assert!(r.is_ok(), "eager path doesn't require kvpart divisibility");
    }

    #[test]
    fn default_rbln_form_is_valid() {
        assert!(preflight::rbln_param_issue(&App::new().build_compile_form(
            &App::synthetic_artifact_for(
                "meta-llama/Llama-3.1-8B-Instruct",
                "rbln",
                String::new(),
                &[]
            ),
            "rbln",
        ))
        .is_none());
    }
}
