//! Accelerator packs — one record per accelerator family.
//!
//! A vendor's identity used to be spread across twenty files as string literals and match
//! arms: its resource key in the deploy path, its metric names in two separate lists, its
//! colour in the theme, its aliases in the CLI parser. Adding a third accelerator meant
//! touching all of them, and every two-way `if rbln { … } else { … }` had somewhere for a
//! third vendor to fall wrongly — which is exactly how `--plan compile --vendor gpu` came to
//! emit the RBLN script with the Furiosa image (BUG-07).
//!
//! Everything expressible as data lives in a [`Pack`]. What genuinely needs code — reading a
//! device out of a metric series, predicting invalid parameter combinations, modelling memory —
//! stays in the collectors and the compile module, but keyed off the pack rather than a string.
//!
//! Adding an accelerator is a new file here plus one line in [`PACKS`].

pub mod furiosa;
pub mod nvidia;
pub mod rbln;

use crate::collect::AccelKind;

/// Physical unit a metric series reports in.
///
/// Declared per series because guessing does not work: the old code inferred "a utilisation
/// under 1.0 must be a ratio" and turned a GPU genuinely at 1 % into 100 % (BUG-04). All three
/// accelerators here report percent; the type exists so the next one can say otherwise.
// Ratio and the non-Max aggregations are extension points: no accelerator here reports a
// 0..1 utilisation or needs summing, but declaring the unit is precisely what stops the next
// one from being guessed at (BUG-04). Kept deliberately, not dead by accident.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    /// 0–100.
    Percent,
    /// 0.0–1.0, scaled to percent on read.
    Ratio,
    /// Bytes, converted to GB (10^9).
    Bytes,
    /// Mebibytes, converted to GB.
    Mib,
    Celsius,
    Watt,
    /// Millijoules, cumulative.
    Millijoule,
    /// A bare count (throttle events, health codes).
    Count,
}

impl Unit {
    /// Normalise a raw sample into the unit the UI stores (percent, GB, °C, W, mJ, count).
    pub fn normalise(self, v: f64) -> f64 {
        if v.is_nan() {
            return f64::NAN;
        }
        match self {
            Unit::Percent => v.clamp(0.0, 100.0),
            Unit::Ratio => (v * 100.0).clamp(0.0, 100.0),
            Unit::Bytes => v / 1.0e9,
            Unit::Mib => v / 1024.0,
            Unit::Celsius | Unit::Watt | Unit::Millijoule | Unit::Count => v,
        }
    }
}

/// How several samples for one device collapse into a value.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Agg {
    Max,
    Avg,
    Sum,
}

/// A device-level signal lmd-top reads.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Field {
    Util,
    Temp,
    Power,
    MemUsed,
    MemTotal,
    /// Liveness/health, interpreted per [`Health`].
    Health,
    /// Cumulative throttling events; > 0 means throttled.
    Throttle,
    /// Memory-bandwidth pressure, percent.
    MemBandwidth,
    ClockMhz,
    MemTemp,
    Energy,
}

/// One Prometheus series backing a [`Field`].
pub struct Series {
    pub field: Field,
    pub metric: &'static str,
    pub unit: Unit,
    pub agg: Agg,
    /// Impact statement used by `--doctor` when the metric is absent.
    pub missing: &'static str,
}

/// Which labels identify a device in this vendor's series.
pub struct Labels {
    /// Label joining a device's series together (usually a UUID).
    pub key: &'static str,
    /// Label carrying the human-facing device id (rbln0, npu0, gpu0).
    pub id: &'static str,
    /// Label carrying the node name.
    pub node: &'static str,
    /// Optional label carrying the hardware model name.
    pub model: Option<&'static str>,
    /// Optional label naming the pod currently occupying the device.
    pub busy: Option<&'static str>,
}

/// How a health/liveness series should be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Health {
    /// Non-zero means alive (furiosa_npu_alive).
    NonZeroIsAlive,
    /// Zero means healthy — the value is an error code (RBLN_DEVICE_STATUS:HEALTH).
    ZeroIsHealthy,
}

/// What this accelerator can do and report.
///
/// Absent capabilities are `None`/`false` rather than a zero reading. That distinction matters:
/// writing `throttle: 0.0` for hardware that does not report throttling is indistinguishable
/// from "never throttled", so the UI cannot tell you which it is looking at.
pub struct Caps {
    /// Needs (and supports) an ahead-of-time compile step. GPUs serve HF weights directly, so
    /// a GPU compile is not a rejected request — it is not representable.
    pub compiles_ahead_of_time: bool,
    pub health: Option<Health>,
    pub throttle: bool,
    pub energy: bool,
    /// CPU and accelerator share one memory pool (GB10/GH200): device VRAM is the host pool.
    pub unified_memory: bool,
    /// Maximum devices usable as one tensor-parallel group, when the hardware caps it.
    pub max_tensor_parallel: Option<u32>,
    /// Set when serving tensor-parallel width is counted in sub-device units rather than in
    /// cards — one RNGD exposes 8 PEs, so `--tensor-parallel-size 8` on a single card is
    /// normal and TP is a separate axis from the device request. `None` means TP == devices.
    pub serving_tp_unit: Option<&'static str>,
}

/// Serving/scheduling identity in Kubernetes.
pub struct Scheduling {
    /// Extended resource name requested per device.
    pub resource_key: &'static str,
    /// Node label identifying this product, when placement should prefer it.
    pub product_label: Option<(&'static str, &'static str)>,
    /// URL path segment for generated routes (`/atom/…`, `/rngd/…`, `/gpu/…`).
    pub route_segment: &'static str,
}

/// One accelerator family.
pub struct Pack {
    /// Canonical id used in forms, manifest paths and `--vendor`.
    pub id: &'static str,
    /// Accepted spellings on the command line.
    pub aliases: &'static [&'static str],
    /// Short display label (table badges, node summaries).
    pub label: &'static str,
    /// Longer human name, for prose and menus ("Furiosa" vs the terse "RNGD").
    pub display: &'static str,
    pub kind: AccelKind,
    /// Serving engine name shown in the Models view.
    pub engine: &'static str,
    /// Prometheus job-name fragment, for `--doctor`'s exporter survey.
    pub exporter: &'static str,
    /// Index into the theme's vendor-accent ramp. Identity, not a colour: the theme decides
    /// what each slot looks like, so a new accelerator picks the next index and every theme
    /// keeps working (the ramp wraps).
    pub accent: usize,
    pub labels: Labels,
    pub series: &'static [Series],
    pub caps: Caps,
    pub scheduling: Scheduling,
    /// Doctor groups its coverage table by this name.
    pub family: &'static str,
}

impl Pack {
    /// The series backing `field`, if this accelerator reports it.
    pub fn series_for(&self, field: Field) -> Option<&'static Series> {
        self.series.iter().find(|s| s.field == field)
    }
}

/// Every known accelerator. Adding one means adding its module and a line here.
pub static PACKS: &[&Pack] = &[&rbln::PACK, &furiosa::PACK, &nvidia::PACK];

/// Look up by canonical id or alias — the single place `--vendor` spellings are resolved.
pub fn by_id(name: &str) -> Option<&'static Pack> {
    let lower = name.to_lowercase();
    PACKS
        .iter()
        .copied()
        .find(|p| p.id == lower || p.aliases.contains(&lower.as_str()))
}

/// Look up by the accelerator kind carried on collected devices.
pub fn by_kind(kind: AccelKind) -> &'static Pack {
    PACKS
        .iter()
        .copied()
        .find(|p| p.kind == kind)
        .expect("every AccelKind has a pack")
}

/// Packs that support an ahead-of-time compile (the ones a compile form can target).
pub fn compilable() -> impl Iterator<Item = &'static Pack> {
    PACKS
        .iter()
        .copied()
        .filter(|p| p.caps.compiles_ahead_of_time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_aliases_are_unique_and_resolvable() {
        let mut seen = std::collections::BTreeSet::new();
        for p in PACKS {
            assert!(seen.insert(p.id), "duplicate pack id {}", p.id);
            for a in p.aliases {
                assert!(seen.insert(a), "alias {} collides", a);
                assert_eq!(by_id(a).map(|q| q.id), Some(p.id), "alias {} resolves", a);
            }
            assert_eq!(by_id(p.id).map(|q| q.id), Some(p.id));
            // Case-insensitive, as typed on the command line.
            assert_eq!(by_id(&p.id.to_uppercase()).map(|q| q.id), Some(p.id));
        }
        assert!(by_id("definitely-not-a-vendor").is_none());
    }

    #[test]
    fn every_kind_has_exactly_one_pack() {
        for kind in [AccelKind::Gpu, AccelKind::Rbln, AccelKind::Rngd] {
            let matching: Vec<&str> = PACKS
                .iter()
                .filter(|p| p.kind == kind)
                .map(|p| p.id)
                .collect();
            assert_eq!(matching.len(), 1, "{:?} maps to {:?}", kind, matching);
        }
    }

    /// BUG-04 as a property of the type: a declared percent stays put, and only a declared
    /// ratio is scaled. No sample value can change which happens.
    #[test]
    fn units_do_not_guess() {
        assert_eq!(Unit::Percent.normalise(1.0), 1.0);
        assert_eq!(Unit::Percent.normalise(0.5), 0.5);
        assert_eq!(Unit::Percent.normalise(96.0), 96.0);
        assert_eq!(Unit::Percent.normalise(140.0), 100.0);
        assert_eq!(Unit::Ratio.normalise(0.5), 50.0);
        assert_eq!(Unit::Ratio.normalise(1.0), 100.0);
        assert_eq!(Unit::Bytes.normalise(2.0e9), 2.0);
        assert_eq!(Unit::Mib.normalise(2048.0), 2.0);
        assert!(Unit::Percent.normalise(f64::NAN).is_nan());
    }

    /// Utilisation must be declared for every accelerator — it drives alert thresholds,
    /// colours, sparklines and the placement solver's idea of "free".
    #[test]
    fn every_pack_reports_utilisation_in_percent() {
        for p in PACKS {
            let s = p
                .series_for(Field::Util)
                .unwrap_or_else(|| panic!("{} declares no Util series", p.id));
            assert_eq!(
                s.unit,
                Unit::Percent,
                "{} util is {:?}; if a vendor really reports a ratio, say so here rather than \
                 inferring it from the value",
                p.id,
                s.unit
            );
        }
    }

    /// A capability and the series backing it must agree, or the UI shows a zero it cannot explain.
    #[test]
    fn caps_match_declared_series() {
        for p in PACKS {
            assert_eq!(
                p.caps.health.is_some(),
                p.series_for(Field::Health).is_some(),
                "{}: health capability and series disagree",
                p.id
            );
            assert_eq!(
                p.caps.throttle,
                p.series_for(Field::Throttle).is_some(),
                "{}: throttle capability and series disagree",
                p.id
            );
            assert_eq!(
                p.caps.energy,
                p.series_for(Field::Energy).is_some(),
                "{}: energy capability and series disagree",
                p.id
            );
        }
    }

    #[test]
    fn only_npus_compile_ahead_of_time() {
        let ids: Vec<&str> = compilable().map(|p| p.id).collect();
        assert_eq!(ids, vec!["rbln", "furiosa"]);
        assert!(
            !by_id("gpu").unwrap().caps.compiles_ahead_of_time,
            "a GPU compile must not be representable — BUG-07 was it being merely rejected"
        );
    }

    /// BUG-19: the join key must identify a *device*, not a per-node ordinal. DCGM's `gpu`
    /// label is an index, so two hosts both have gpu="0"; keying on it merged them and one
    /// node's temperature/power was reported for the other (a 67 °C device read as 44 °C).
    #[test]
    fn join_keys_are_globally_unique_identifiers() {
        for p in PACKS {
            assert_ne!(
                p.labels.key, p.labels.id,
                "{}: the join key must be a unique device identifier, not the display index — \
                 a per-node ordinal collapses devices across nodes",
                p.id
            );
            assert!(
                !p.labels.key.is_empty() && p.labels.key != p.labels.node,
                "{}: join key {:?} cannot identify a device",
                p.id,
                p.labels.key
            );
        }
    }

    #[test]
    fn scheduling_keys_are_distinct() {
        let mut keys = std::collections::BTreeSet::new();
        for p in PACKS {
            assert!(
                keys.insert(p.scheduling.resource_key),
                "{} reuses resource key {}",
                p.id,
                p.scheduling.resource_key
            );
        }
    }
}
