//! NPU compile form field specifications and default option schemas.

use crate::ops::CompileField;

/// Declarative specification for a single compilation option field.
#[derive(Clone, Copy, Debug)]
pub struct CompileFieldDef {
    pub key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    pub choices: &'static [&'static str],
    pub numeric: bool,
    pub help: &'static str,
}

impl CompileFieldDef {
    /// Numeric parameter field (e.g., tp, max-len, batch size).
    pub const fn num(
        key: &'static str,
        label: &'static str,
        default: &'static str,
        choices: &'static [&'static str],
        help: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            default,
            choices,
            numeric: true,
            help,
        }
    }

    /// Discrete option / string choice field (e.g., attn implementation, quantization).
    pub const fn opt(
        key: &'static str,
        label: &'static str,
        default: &'static str,
        choices: &'static [&'static str],
        help: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            default,
            choices,
            numeric: false,
            help,
        }
    }
}

/// Compilation option definitions for Rebellions (RBLN) CA22 NPUs.
pub const RBLN_COMPILE_FIELDS: &[CompileFieldDef] = &[
    CompileFieldDef::num(
        "tp",
        "tensor-parallel",
        "4",
        &["1", "2", "4"],
        "Tensor parallel size = number of RBLN chips (rbln_tensor_parallel_size). CA22 max is 4.",
    ),
    CompileFieldDef::num(
        "max-len",
        "max-seq-len",
        "8192",
        &["2048", "4096", "8192", "16384", "32768"],
        "Compile-time maximum context length (rbln_max_seq_len). Larger values use more memory and compile time.",
    ),
    CompileFieldDef::num(
        "batch",
        "batch-size",
        "1",
        &["1", "2", "4", "8", "16"],
        "Static batch size (rbln_batch_size). RBLN fixes this at compile time.",
    ),
    CompileFieldDef::opt(
        "attn",
        "attn-impl",
        "flash_attn",
        &["flash_attn", "eager"],
        "Attention implementation: flash_attn for SRAM optimized path, eager for PagedAttention.",
    ),
    CompileFieldDef::num(
        "kvpart",
        "kvcache-partition",
        "4096",
        &["2048", "4096", "8192", "16384"],
        "flash_attn only: KV tokens per SRAM partition. Must divide max-seq-len AND be strictly smaller (≥2 partitions) — e.g. max-len 8192 → kvpart 4096 (2 partitions), NOT 8192.",
    ),
    CompileFieldDef::opt(
        "quant",
        "quantization",
        "none",
        &["none", "w8a8", "w4a16"],
        "Weight/activation quantization format (RBLNQuantizationConfig); model support varies.",
    ),
    CompileFieldDef::opt(
        "npu",
        "npu-chip",
        "RBLN-CA22",
        &["RBLN-CA22"],
        "Target RBLN chip (rbln_npu), detected from the cluster.",
    ),
];

/// Compilation option definitions for Furiosa RNGD NPUs.
pub const FURIOSA_COMPILE_FIELDS: &[CompileFieldDef] = &[
    CompileFieldDef::num(
        "tp",
        "tensor-parallel",
        "8",
        &["4", "8"],
        "Tensor parallel size in PE units. RNGD full=8, half=4.",
    ),
    CompileFieldDef::num(
        "pp",
        "pipeline-parallel",
        "1",
        &["1", "2"],
        "Number of pipeline-parallel stages (ParallelConfig).",
    ),
    CompileFieldDef::num(
        "max-len",
        "max-seq-len",
        "8192",
        &["2048", "4096", "8192", "16384"],
        "max_seq_len_to_capture; longer buckets are excluded.",
    ),
    CompileFieldDef::num(
        "batch",
        "batch-size",
        "1",
        &["1", "2", "4", "8"],
        "Prefill/decode bucket batch size (BucketConfig).",
    ),
    CompileFieldDef::num(
        "chunk",
        "prefill-chunk",
        "none",
        &["none", "512", "1024", "2048"],
        "Chunked prefill chunk size (prefill_chunk_size).",
    ),
    CompileFieldDef::num(
        "block",
        "kv-block-size",
        "16",
        &["16", "32"],
        "Tokens per PagedAttention block (paged_attention_block_size).",
    ),
    CompileFieldDef::opt(
        "quant",
        "activation-dq",
        "none",
        &["none", "on"],
        "use_activation_dq: dynamic activation quantization to reduce memory and improve throughput.",
    ),
];

/// Construct standard compile fields for the given vendor and model artifact.
pub fn vendor_compile_fields(
    vendor: &str,
    a: &crate::collect::ModelArtifact,
) -> Vec<CompileField> {
    let defs = compile_profile(vendor)
        .map(|p| p.fields)
        .unwrap_or(FURIOSA_COMPILE_FIELDS);
    defs.iter()
        .map(|d| CompileField {
            key: d.key.into(),
            label: d.label.into(),
            value: crate::app::App::opt_or(a, d.key, d.default),
            choices: d.choices.iter().map(|s| s.to_string()).collect(),
            numeric: d.numeric,
            help: d.help.into(),
        })
        .collect()
}

/// Apply user overrides to a list of compile fields.
pub fn apply_field_overrides(fields: &mut [CompileField], overrides: &[(String, String)]) {
    for (key, val) in overrides {
        if key == "mount" {
            continue;
        }
        if let Some(f) = fields.iter_mut().find(|f| f.key == *key || f.label == *key) {
            f.value = val.clone();
        }
    }
}

/// Extract an override value for a specific key.
pub fn override_value(overrides: &[(String, String)], key: &str) -> Option<String> {
    overrides
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

/// Per-accelerator compile profile: the option schema, and how a build is named in the store.
///
/// Looked up by accelerator id and *total* — an accelerator that declares
/// `compiles_ahead_of_time` but has no profile here is reported as unsupported rather than
/// silently handed another vendor's schema and target prefix. That silent fallback is what
/// made `--vendor gpu` emit a Furiosa-named RBLN job (BUG-07).
pub struct CompileProfile {
    pub fields: &'static [CompileFieldDef],
    /// Store path prefix, e.g. `rbln-ca22` or `rngd`.
    pub target_prefix: &'static str,
    /// Whether the target name carries a pipeline-parallel component.
    pub target_has_pp: bool,
    /// Form field naming the chip variant, folded into the target prefix when present.
    pub chip_field: Option<&'static str>,
}

pub fn compile_profile(vendor: &str) -> Option<&'static CompileProfile> {
    match vendor {
        "rbln" => Some(&RBLN_PROFILE),
        "furiosa" => Some(&FURIOSA_PROFILE),
        _ => None,
    }
}

static RBLN_PROFILE: CompileProfile = CompileProfile {
    fields: RBLN_COMPILE_FIELDS,
    target_prefix: "rbln",
    target_has_pp: false,
    chip_field: Some("npu"),
};

static FURIOSA_PROFILE: CompileProfile = CompileProfile {
    fields: FURIOSA_COMPILE_FIELDS,
    target_prefix: "rngd",
    target_has_pp: true,
    chip_field: None,
};
