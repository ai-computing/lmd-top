//! Rebellions RBLN (ATOM / CA22).
//!
//! Telemetry comes from Prometheus recording rules (`RBLN_DEVICE_STATUS:*`) published by the
//! rbln metrics exporter, all in percent / bytes / °C / W.

use super::*;
use crate::collect::AccelKind;

pub static PACK: Pack = Pack {
    id: "rbln",
    aliases: &["atom", "rebellions"],
    label: "RBLN",
    display: "Rebellions",
    kind: AccelKind::Rbln,
    engine: "vLLM-RBLN",
    accent: 1,
    exporter: "rbln",
    family: "Rebellions RBLN",
    labels: Labels {
        key: "uuid",
        id: "name",
        node: "node",
        model: None,
        // The exporter relabels the serving pod onto the device series.
        busy: Some("exported_pod"),
    },
    series: &[
        Series {
            field: Field::Util,
            metric: "RBLN_DEVICE_STATUS:UTILIZATION",
            // rbln-stat reports util as a percentage.
            unit: Unit::Percent,
            agg: Agg::Max,
            missing: "RBLN util unavailable",
        },
        Series {
            field: Field::Temp,
            metric: "RBLN_DEVICE_STATUS:TEMPERATURE",
            unit: Unit::Celsius,
            agg: Agg::Max,
            missing: "RBLN temp unavailable",
        },
        Series {
            field: Field::Power,
            metric: "RBLN_DEVICE_STATUS:CARD_POWER",
            unit: Unit::Watt,
            agg: Agg::Max,
            missing: "RBLN power unavailable",
        },
        Series {
            field: Field::MemUsed,
            metric: "RBLN_DEVICE_STATUS:DRAM_USED",
            unit: Unit::Bytes,
            agg: Agg::Max,
            missing: "RBLN mem used unavailable",
        },
        Series {
            field: Field::MemTotal,
            metric: "RBLN_DEVICE_STATUS:DRAM_TOTAL",
            unit: Unit::Bytes,
            agg: Agg::Max,
            missing: "RBLN mem total unavailable",
        },
        Series {
            field: Field::Health,
            metric: "RBLN_DEVICE_STATUS:HEALTH",
            unit: Unit::Count,
            agg: Agg::Max,
            missing: "RBLN health unavailable",
        },
    ],
    caps: Caps {
        compiles_ahead_of_time: true,
        // HEALTH carries an error code: 0 is the healthy value, not a missing reading.
        health: Some(Health::ZeroIsHealthy),
        throttle: false,
        energy: false,
        unified_memory: false,
        // CA22 exposes four chips as one tensor-parallel group.
        serving_tp_unit: None,
        max_tensor_parallel: Some(4),
    },
    scheduling: Scheduling {
        resource_key: "rebellions.ai/ATOM",
        product_label: Some(("rebellions.ai/npu.product", "RBLN-CA22")),
        route_segment: "atom",
    },
};
