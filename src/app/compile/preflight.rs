//! Compile pre-flight checks and hardware/runtime compatibility validation.

use crate::collect::{NodeInfo, StoredModel};
use crate::ops::CompileForm;

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
    let tp = getn("tp").unwrap_or(1);
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
pub fn compile_already_stored(
    stored: &[StoredModel],
    form: &CompileForm,
) -> Option<String> {
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
    let mid = form.model_id.to_lowercase();

    // Warn if already compiled in store (not a blocker; recompile overwrites)
    if let Some(path) = compile_already_stored(stored, form) {
        out.push((
            true,
            format!(
                "⚠ 이미 컴파일됨 — {} (재컴파일 시 덮어씀; 불필요하면 취소하고 Deploy)",
                path
            ),
        ));
    }

    if form.vendor == "furiosa" {
        // fxb build compiles checkpoints from the furiosa-ai org with quantization
        let quant = ["fp8", "nvfp4", "-w8", "-w4", "awq", "gptq", "int4", "int8"]
            .iter()
            .any(|q| mid.contains(q));
        let org = mid.starts_with("furiosa-ai/");
        out.push((
            org && quant,
            if org && quant {
                "registry: furiosa-ai 양자화 체크포인트 — fxb 등록 대상".into()
            } else {
                format!(
                    "registry: fxb 는 furiosa-ai 양자화 모델만 빌드(예: furiosa-ai/Qwen3-4B-FP8) — '{}' 미등록 가능성",
                    form.model_id
                )
            },
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
            "compile-only: rbln_create_runtimes=False — 디바이스 점유 없이 컴파일(서빙 중에도 OK)".into(),
        ));
        let compat_ok = crate::compat::compilable_vendors(&form.model_id).contains(&"rbln");
        out.push((
            compat_ok,
            format!(
                "registry: RBLN 지원 계열 {}",
                if compat_ok {
                    "확인됨(npu-compat)"
                } else {
                    "미확인"
                }
            ),
        ));
        out.push((
            true,
            "compile node: AOT — 가속기 불필요, 아무 노드(CPU 포함)에서 실행. 이미지에 toolchain 내장".into(),
        ));
    } else {
        // RBLN execution environment check: image or host rebel-compiler stack
        let host_node = nodes
            .iter()
            .find(|n| n.npu.to_uppercase().contains("RBLN"))
            .map(|n| n.name.clone());
        match (img_rbln.is_some(), host_node) {
            (true, _) => out.push((
                true,
                "compile node: LMD_COMPILE_IMAGE_RBLN 이미지로 실행(아무 노드)".into(),
            )),
            (false, Some(n)) => out.push((
                true,
                format!(
                    "compile node: {} 의 rebel-compiler 호스트 스택 사용(hostPath) — 레지스트리 이미지 불필요",
                    n
                ),
            )),
            (false, None) => out.push((
                false,
                "compile node: rebel-compiler 이미지도, 그게 깔린 노드도 없음 — LMD_COMPILE_IMAGE_RBLN 지정 또는 RBLN 노드 필요".into(),
            )),
        }
    }
    out
}
