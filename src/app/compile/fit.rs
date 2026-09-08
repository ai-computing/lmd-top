//! NPU memory and resource fit estimation for compile configurations.

use crate::collect::{Accel, AccelKind, NodeInfo};
use crate::ops::{CompileForm, FitEstimate, FitVerdict};

/// Parse the estimated parameter count (in billions) from a model name or identifier.
///
/// Looks for patterns like "8B", "1.5b", "0.5B", "32b" where 'b' is directly after
/// the number and not part of an abbreviation like "fp8".
pub fn est_params_b(name: &str) -> Option<f64> {
    let lower = name.to_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'b') {
                let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphabetic();
                if before_ok {
                    if let Ok(v) = lower[start..i].parse::<f64>() {
                        if (0.1..=2000.0).contains(&v) {
                            return Some(v);
                        }
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Calculate estimated memory fit and provide optimization tips for a compile form.
pub fn estimate_compile_fit(
    accel: &[Accel],
    nodes: &[NodeInfo],
    form: &CompileForm,
) -> FitEstimate {
    let rbln = form.vendor == "rbln";
    let params_b = est_params_b(&form.model_id).or_else(|| est_params_b(&form.model));
    let tp = form.get("tp").parse::<f64>().unwrap_or(1.0).max(1.0);
    let pp = form.get("pp").parse::<f64>().unwrap_or(1.0).max(1.0);
    let seq = form
        .get("max-len")
        .parse::<f64>()
        .unwrap_or(8192.0)
        .max(1.0);
    let batch = form.get("batch").parse::<f64>().unwrap_or(1.0).max(1.0);

    // Bytes per parameter based on quantization and model name tags
    let q = form.get("quant").to_lowercase();
    let name_l = form.model_id.to_lowercase();
    let dtype_bytes = if rbln {
        if q.contains("w4") {
            0.5
        } else if q.contains("w8") {
            1.0
        } else {
            2.0
        }
    } else if name_l.contains("fp8") || name_l.contains("-w8") {
        1.0
    } else if name_l.contains("int4") || name_l.contains("awq") || name_l.contains("gptq") {
        0.5
    } else {
        2.0
    };

    // Available memory per chip: average from discovered matching devices, or vendor default
    let want_kind = if rbln {
        AccelKind::Rbln
    } else {
        AccelKind::Rngd
    };
    let mems: Vec<f64> = accel
        .iter()
        .filter(|a| a.kind == want_kind && a.mem_total_gb > 0.0)
        .map(|a| a.mem_total_gb)
        .collect();
    let avail_gb = if mems.is_empty() {
        if rbln {
            15.7
        } else {
            48.0
        }
    } else {
        mems.iter().sum::<f64>() / mems.len() as f64
    };

    // Parallelism chip scaling: RBLN = TP, Furiosa ≈ ceil(TP/8) * PP
    let chips = if rbln {
        tp
    } else {
        (tp / 8.0).ceil().max(1.0) * pp
    };

    let weight_gb = params_b.map(|p| p * dtype_bytes).unwrap_or(0.0);
    // KV cache estimate: scaled from Llama-8B bf16 ≈ 0.25 MB/token
    let kv_per_tok_mb = 0.25 * (params_b.unwrap_or(8.0) / 8.0);
    let kv_gb = batch * seq * kv_per_tok_mb / 1024.0;
    let overhead_gb = 2.0;
    let per_chip_gb = (weight_gb + kv_gb) / chips + overhead_gb;
    let ratio = per_chip_gb / avail_gb;

    let verdict = if params_b.is_none() {
        FitVerdict::Unknown
    } else if ratio > 1.0 {
        FitVerdict::Oom
    } else if ratio > 0.85 {
        FitVerdict::Tight
    } else {
        FitVerdict::Fits
    };

    let mut tips: Vec<String> = Vec::new();
    let max_chips = if rbln { 4.0 } else { 8.0 };
    if matches!(verdict, FitVerdict::Oom | FitVerdict::Tight) {
        if tp < max_chips {
            tips.push(format!(
                "TP↑ {}→{} (칩 추가로 칩당 부담↓)",
                tp as i64,
                (tp * 2.0).min(max_chips) as i64
            ));
        }
        if seq > 2048.0 {
            tips.push(format!(
                "max-seq-len↓ {}→{} (KV {:.1}GiB↓)",
                seq as i64,
                (seq / 2.0) as i64,
                kv_gb / 2.0
            ));
        }
        if batch > 1.0 {
            tips.push(format!(
                "batch↓ {}→{} (KV 절반)",
                batch as i64,
                (batch / 2.0) as i64
            ));
        }
        if dtype_bytes >= 2.0 {
            tips.push(if rbln {
                "양자화 w4a16/w8a8 로 가중치↓".into()
            } else {
                "FP8 체크포인트 사용 시 가중치 절반".into()
            });
        }
    } else if matches!(verdict, FitVerdict::Fits) && ratio < 0.4 {
        if batch < 8.0 {
            tips.push(format!(
                "여유 있음 — batch↑ {}→{} 로 처리량 확보 여지",
                batch as i64,
                (batch * 2.0) as i64
            ));
        }
        if tp > 1.0 && rbln {
            tips.push(format!(
                "TP↓ {}→{} 로 칩 절약(가능 시)",
                tp as i64,
                (tp / 2.0) as i64
            ));
        }
    }

    // Verify requested devices against physical chip requirement
    if let Ok(dev) = form.get("devices").parse::<f64>() {
        if dev > 0.0 && dev < chips {
            tips.push(format!(
                "⚠ devices {} < 필요 {} 칩 — 이 TP/PP 로는 컴파일 불가",
                dev as i64, chips as i64
            ));
        }
    }

    // RBLN kvcache_partition_len must be power of two
    if rbln {
        if let Ok(k) = form.get("kvpart").parse::<u64>() {
            if k == 0 || (k & (k - 1)) != 0 {
                tips.push(format!("⚠ kvcache-partition {} 는 2의 거듭제곱이 아님", k));
            }
        }
    }

    // Verify driver on explicitly picked target node
    let node_host = if form.dest.is_empty() { "any" } else { &form.dest };
    let node_host = node_host.split('(').next().unwrap_or("any").trim();
    if node_host != "any" && !node_host.is_empty() {
        if let Some(nd) = nodes.iter().find(|n| n.name == node_host) {
            let want = if rbln { "RBLN" } else { "RNGD" };
            if !nd.npu.to_uppercase().contains(want) {
                tips.push(format!(
                    "⚠ 노드 {} 에 {} 드라이버 없음(accel: {}) — 컴파일 실패",
                    node_host,
                    want,
                    if nd.npu.is_empty() { "none" } else { &nd.npu }
                ));
            }
        }
    }

    FitEstimate {
        params_b,
        weight_gb,
        kv_gb,
        overhead_gb,
        chips,
        per_chip_gb,
        avail_gb,
        verdict,
        tips,
    }
}
