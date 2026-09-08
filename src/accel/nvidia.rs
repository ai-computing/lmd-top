//! NVIDIA GPUs, via DCGM.
//!
//! GPUs serve HuggingFace weights directly, so there is no ahead-of-time compile step —
//! `compiles_ahead_of_time: false` is what makes a GPU compile unrepresentable rather than
//! merely rejected. DCGM reports framebuffer memory in MiB and utilisation in percent
//! (observed max 96 on this cluster, confirming the 0–100 scale).

use super::*;
use crate::collect::AccelKind;

pub static PACK: Pack = Pack {
    id: "gpu",
    aliases: &["nvidia", "cuda"],
    label: "GPU",
    display: "NVIDIA",
    kind: AccelKind::Gpu,
    engine: "vLLM",
    // NVIDIA publishes Nemotron and NIM-targeted models under nvidia.
    hf_orgs: &["nvidia"],
    accent: 0,
    exporter: "dcgm",
    family: "NVIDIA GPU (DCGM)",
    labels: Labels {
        // UUID, not the `gpu` index: the index is per-node, so on a multi-node cluster two
        // hosts both have gpu="0". Keying the join on it silently merged them and attributed
        // one node's temperature/power/clock to the other — a 67 °C device read as 44 °C
        // (BUG-19). UUID is unique and present on every DCGM series.
        key: "UUID",
        id: "gpu",
        node: "Hostname",
        model: Some("modelName"),
        busy: Some("exported_pod"),
    },
    series: &[
        Series {
            field: Field::Util,
            metric: "DCGM_FI_DEV_GPU_UTIL",
            unit: Unit::Percent,
            agg: Agg::Max,
            missing: "GPU util unavailable",
        },
        Series {
            field: Field::Temp,
            metric: "DCGM_FI_DEV_GPU_TEMP",
            unit: Unit::Celsius,
            agg: Agg::Max,
            missing: "GPU temp unavailable",
        },
        Series {
            field: Field::Power,
            metric: "DCGM_FI_DEV_POWER_USAGE",
            unit: Unit::Watt,
            agg: Agg::Max,
            missing: "GPU power unavailable",
        },
        Series {
            field: Field::MemUsed,
            metric: "DCGM_FI_DEV_FB_USED",
            unit: Unit::Mib,
            agg: Agg::Max,
            missing: "GPU mem used unavailable (unified-mem falls back to host)",
        },
        Series {
            field: Field::MemTotal,
            metric: "DCGM_FI_DEV_FB_TOTAL",
            unit: Unit::Mib,
            agg: Agg::Max,
            missing: "GPU mem total unavailable (unified-mem falls back to host)",
        },
        Series {
            field: Field::MemBandwidth,
            metric: "DCGM_FI_DEV_MEM_COPY_UTIL",
            unit: Unit::Percent,
            agg: Agg::Max,
            missing: "GPU memory-bandwidth column empty",
        },
        Series {
            field: Field::ClockMhz,
            metric: "DCGM_FI_DEV_SM_CLOCK",
            unit: Unit::Count,
            agg: Agg::Max,
            missing: "GPU SM clock column empty",
        },
        Series {
            field: Field::MemTemp,
            metric: "DCGM_FI_DEV_MEMORY_TEMP",
            unit: Unit::Celsius,
            agg: Agg::Max,
            missing: "GPU memory temperature column empty",
        },
        Series {
            field: Field::Energy,
            metric: "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION",
            unit: Unit::Millijoule,
            agg: Agg::Max,
            missing: "session energy (Wh since start) unavailable",
        },
    ],
    caps: Caps {
        compiles_ahead_of_time: false,
        // DCGM has no single liveness series — a scraped device is a present device.
        health: None,
        throttle: false,
        energy: true,
        // Detected per model (GB10/GH200 share the host pool); see `is_unified`.
        unified_memory: false,
        serving_tp_unit: None,
        max_tensor_parallel: None,
    },
    scheduling: Scheduling {
        resource_key: "nvidia.com/gpu",
        product_label: None,
        route_segment: "gpu",
    },
};
