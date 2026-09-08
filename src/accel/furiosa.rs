//! Furiosa RNGD.
//!
//! Telemetry comes from furiosa-metrics-exporter (`furiosa_npu_*`). Core utilisation is
//! furiosa-smi's `pe_usage_percentage`, i.e. already a percentage; it is reported per PE core,
//! so it averages across cores to give a per-device figure.

use super::*;
use crate::collect::AccelKind;

pub static PACK: Pack = Pack {
    id: "furiosa",
    aliases: &["rngd"],
    label: "RNGD",
    display: "Furiosa",
    kind: AccelKind::Rngd,
    engine: "Furiosa-LLM",
    // Furiosa publishes pre-quantised checkpoints under furiosa-ai.
    hf_orgs: &["furiosa-ai"],
    accent: 2,
    exporter: "furiosa",
    family: "Furiosa RNGD",
    labels: Labels {
        key: "uuid",
        id: "device",
        node: "hostname",
        model: None,
        busy: None,
    },
    series: &[
        Series {
            field: Field::Util,
            metric: "furiosa_npu_core_utilization",
            unit: Unit::Percent,
            // Per-PE series: average over cores for the device figure.
            agg: Agg::Avg,
            missing: "RNGD util unavailable",
        },
        Series {
            field: Field::Temp,
            metric: "furiosa_npu_hw_temperature",
            unit: Unit::Celsius,
            agg: Agg::Max,
            missing: "RNGD temp unavailable",
        },
        Series {
            field: Field::Power,
            metric: "furiosa_npu_hw_power",
            unit: Unit::Watt,
            agg: Agg::Max,
            missing: "RNGD power unavailable",
        },
        Series {
            field: Field::MemUsed,
            metric: "furiosa_npu_dram_usage",
            unit: Unit::Bytes,
            agg: Agg::Max,
            missing: "RNGD mem used unavailable",
        },
        Series {
            field: Field::MemTotal,
            metric: "furiosa_npu_dram_total",
            unit: Unit::Bytes,
            agg: Agg::Max,
            missing: "RNGD mem total unavailable",
        },
        Series {
            field: Field::Health,
            metric: "furiosa_npu_alive",
            unit: Unit::Count,
            agg: Agg::Max,
            missing: "RNGD liveness unavailable",
        },
        Series {
            field: Field::Throttle,
            metric: "furiosa_npu_throttling_events_count",
            unit: Unit::Count,
            agg: Agg::Max,
            missing: "RNGD throttle detection unavailable",
        },
    ],
    caps: Caps {
        compiles_ahead_of_time: true,
        health: Some(Health::NonZeroIsAlive),
        throttle: true,
        energy: false,
        unified_memory: false,
        // One RNGD exposes 8 PEs (full) or 4 (half); serving TP counts PEs, not cards.
        serving_tp_unit: Some("PE"),
        max_tensor_parallel: Some(8),
    },
    scheduling: Scheduling {
        resource_key: "furiosa.ai/rngd",
        product_label: Some(("furiosa.ai/npu.product", "rngd")),
        route_segment: "rngd",
    },
};
