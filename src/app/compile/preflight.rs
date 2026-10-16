//! Compile pre-flight checks and hardware/runtime compatibility validation.

use crate::collect::{NodeInfo, StoredModel};
use crate::ops::CompileForm;

/// Validate numeric input without silently replacing malformed values with defaults.
/// Numeric choices are suggestions, not an exhaustive list of supported sizes.
pub fn compile_param_issue(form: &CompileForm) -> Option<String> {
    let profile = super::fields::compile_profile(form.vendor)?;
    for field in profile.fields.iter().filter(|field| field.numeric) {
        let value = form.get(field.key);
        if field.key == "chunk" && value == "none" {
            continue;
        }
        if !matches!(value.parse::<u32>(), Ok(n) if n > 0) {
            return Some(format!(
                "{} must be a positive integer; got {:?}",
                field.label, value
            ));
        }
    }
    for field in profile.fields.iter().filter(|field| !field.numeric) {
        let value = form.get(field.key);
        if !field.choices.contains(&value.as_str()) {
            return Some(format!(
                "{} must be one of {}; got {:?}",
                field.label,
                field.choices.join("/"),
                value
            ));
        }
    }
    if form.vendor == "furiosa" {
        // Legacy fields do not have equivalents in the FXB recipe. Never publish
        // a build pretending that a changed value was honored.
        for (key, unchanged) in [
            ("batch", "1"),
            ("chunk", "none"),
            ("block", "16"),
            ("quant", "none"),
        ] {
            if form.get(key) != unchanged {
                return Some(format!("Furiosa FXB recipe does not support overriding {key}; restore {unchanged}. Bucket and quantization settings are resolved by the installed SDK/model preset."));
            }
        }
    }
    if form.vendor == "rbln" {
        rbln_param_issue(form)
    } else {
        None
    }
}

/// Validate RBLN (optimum-rbln) parameter combinations before launching a Job.
///
/// Returns an actionable error message with suggested remediation if invalid, or None if valid.
/// Key constraints:
/// - TP must be 1, 2, or 4 for RBLN-CA22.
/// - When `attn-impl == flash_attn`:
///   - `kvcache-partition` must be a power of two.
///   - Flash attention requires at least 2 partitions: `max-seq-len` must be strictly
///     greater than `kvcache-partition` and an exact multiple of it.
pub fn rbln_param_issue(form: &CompileForm) -> Option<String> {
    let getn = |k: &str| form.get(k).parse::<i64>().ok();
    let tp = getn("tp").unwrap_or(0);
    if !matches!(tp, 1 | 2 | 4) {
        return Some(format!("tensor-parallel {} — RBLN-CA22 supports 1/2/4", tp));
    }
    if form.get("attn") == "flash_attn" {
        let max_len = getn("max-len").unwrap_or(0);
        let kvpart = getn("kvpart").unwrap_or(0);
        if kvpart <= 0 || (kvpart & (kvpart - 1)) != 0 {
            return Some(format!(
                "kvcache-partition {} must be a power of two for flash_attn",
                kvpart
            ));
        }
        if max_len <= kvpart || max_len % kvpart != 0 {
            let fix = [16384i64, 8192, 4096, 2048]
                .into_iter()
                .find(|&k| k < max_len && max_len % k == 0);
            return Some(match fix {
                Some(k) => format!(
                    "flash_attn needs max-seq-len ({}) to be a multiple of a *smaller* kvcache-partition (≥2 partitions); {} is invalid → set kvcache-partition to {}",
                    max_len, kvpart, k
                ),
                None => format!(
                    "flash_attn needs max-seq-len (≥4096) divisible by kvcache-partition; {} is too small → switch attn-impl to eager",
                    max_len
                ),
            });
        }
    }
    None
}

/// Check if a compiled artifact matching this form's target already exists in the model store.
pub fn compile_already_stored(stored: &[StoredModel], form: &CompileForm) -> Option<String> {
    let repo_dir = form.model_id.replace('/', "--");
    let expected = format!("compiled/{}/{}/{}", repo_dir, form.vendor, form.target());
    stored
        .iter()
        .map(|s| s.path.trim_end_matches('/'))
        .find(|p| *p == expected)
        .map(|p| p.to_string())
}

/// Run a comprehensive pre-flight checklist before submitting the compile job.
///
/// Returns a list of `(is_satisfied, message)`. If any check fails, the compile job
/// is likely to encounter errors.
pub fn compile_preflight(
    stored: &[StoredModel],
    nodes: &[NodeInfo],
    img_rbln: Option<&str>,
    form: &CompileForm,
) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    if let Some(issue) = compile_param_issue(form) {
        out.push((false, issue));
    }

    // Publication refuses to overwrite an existing build; retain it or choose a new configuration.
    if let Some(path) = compile_already_stored(stored, form) {
        out.push((
            true,
            format!(
                "⚠ 이미 컴파일됨 — {} (기존 빌드는 덮어쓰지 않음; Deploy 또는 명시적으로 이동/삭제 후 재컴파일)",
                path
            ),
        ));
    }

    // Surface the toolchain that produced the builds of this family that already exist.
    //
    // This is the check that would have shortened the RBLN investigation by hours: the only
    // artifact that worked on this hardware recorded optimum-rbln 0.10.2, while every rebuild
    // used a newer one, and nothing in the tool ever said so. It is information, not a
    // verdict — we cannot know what version the Job will resolve until it runs — so it always
    // passes, and only appears when a build of the family actually carries provenance.
    let known: Vec<&str> = {
        let mut v: Vec<&str> = stored
            .iter()
            .filter(|s| s.family == crate::collect::model_family(&form.model_id, &form.model_id))
            .filter(|s| s.format == form.vendor)
            .map(|s| s.built_with.trim())
            .filter(|b| !b.is_empty() && *b != "-")
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    if !known.is_empty() {
        out.push((
            true,
            format!(
                "toolchain: 이 계열의 기존 빌드는 {} 로 만들어졌음 — 재현하려면 같은 버전을 쓰라 \
                 (LMD_RBLN_TOOLCHAIN)",
                known.join(", ")
            ),
        ));
    }

    if form.vendor == "furiosa" {
        // FXB accepts supported architectures from any HF organization. A repository
        // name is not evidence of compiler compatibility or checkpoint quantization.
        out.push((
            true,
            format!("⚠ FXB compatibility unverified for {} — the installed SDK must resolve the architecture and quantization; repository organization is not a compatibility check", form.model_id),
        ));
        out.push((
            true,
            "toolchain: aarch64 크로스컴파일러 매니페스트가 자동 설치".into(),
        ));
        out.push((
            true,
            "build I/O: 로컬 emptyDir 빌드→스토어 복사(SMB os error 95 회피)".into(),
        ));
        out.push((
            true,
            "compile node: AOT — 가속기 불필요, 아무 노드(CPU 포함)에서 실행. 이미지에 toolchain 내장".into(),
        ));
    } else {
        out.push((
            true,
            "compile-only: rbln_create_runtimes=False — 디바이스 점유 없이 컴파일(서빙 중에도 OK)"
                .into(),
        ));
        let compat_ok = crate::compat::compilable_vendors(&form.model_id).contains(&"rbln");
        out.push((
            compat_ok,
            format!(
                "registry: RBLN catalog family {} (installed SDK compatibility unverified)",
                if compat_ok {
                    "확인됨(npu-compat)"
                } else {
                    "미확인"
                }
            ),
        ));
        // RBLN execution environment check: image or host rebel-compiler stack
        let host_node = nodes
            .iter()
            .find(|n| n.npu.to_uppercase().contains("RBLN"))
            .map(|n| n.name.clone());
        match (img_rbln.is_some(), host_node) {
            (true, _) => out.push((
                true,
                "compile image: LMD_COMPILE_IMAGE_RBLN configured (pull, toolchain and node compatibility unverified)".into(),
            )),
            // Works, but inherits whatever the node's Python environment happens to be: an
            // unrelated pip install on that host silently changes the compile toolchain, which
            // is how every RBLN compile here came to fail against transformers 5. Flag it as a
            // warning rather than a clean pass so the fragility is visible before the build.
            (false, Some(n)) => out.push((
                true,
                format!(
                    "⚠ compile node: {} 의 호스트 rebel-compiler 스택(hostPath) 사용 — 그 노드의 \
                     python 환경을 그대로 물려받는다(무관한 pip install 이 컴파일을 깨뜨림). \
                     고정하려면 LMD_COMPILE_IMAGE_RBLN 에 핀된 이미지를 지정",
                    n
                ),
            )),
            (false, None) => out.push((
                false,
                "compile node: configured compiler image and RBLN node label not observed — configure LMD_COMPILE_IMAGE_RBLN or inspect node/toolchain availability".into(),
            )),
        }
    }
    out
}
