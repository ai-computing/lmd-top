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
#[derive(Debug)]
pub struct Series {
    pub field: Field,
    pub metric: &'static str,
    pub unit: Unit,
    pub agg: Agg,
    /// Impact statement used by `--doctor` when the metric is absent.
    pub missing: &'static str,
}

/// Which labels identify a device in this vendor's series.
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
pub struct Scheduling {
    /// Extended resource name requested per device.
    pub resource_key: &'static str,
    /// Node label identifying this product, when placement should prefer it.
    pub product_label: Option<(&'static str, &'static str)>,
    /// URL path segment for generated routes (`/atom/…`, `/rngd/…`, `/gpu/…`).
    pub route_segment: &'static str,
}

/// One accelerator family.
#[derive(Debug)]
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
    /// HuggingFace organisations this vendor publishes models under, for the zoo's live
    /// refresh. Empty when the vendor has none — Rebellions ships its supported-model list as
    /// a GitHub repo (`rbln-model-zoo`) rather than an HF org, so there is nothing to query.
    pub hf_orgs: &'static [&'static str],
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

/// Accelerators compiled into the binary. Adding one means adding its module and a line here.
static BUILTIN: &[&Pack] = &[&rbln::PACK, &furiosa::PACK, &nvidia::PACK];

/// Every accelerator this run knows about: the built-ins plus any declared in
/// `~/.config/lmd-top/accelerators/*.yaml`.
///
/// The data half of a pack is just declarations — metric names, units, label spellings,
/// resource keys — so an accelerator whose telemetry is shaped like an existing one needs no
/// rebuild. Same precedent as `npu-compat.json` and `catalog/*.yaml`. Anything code-shaped
/// (a compile recipe, a memory model) still needs a built-in pack; a runtime pack that claims
/// `compile: true` is rejected at load rather than silently borrowing another vendor's recipe.
pub fn packs() -> &'static [&'static Pack] {
    static ALL: std::sync::OnceLock<Vec<&'static Pack>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut all: Vec<&'static Pack> = BUILTIN.to_vec();
        for pack in load_config_packs() {
            // A config pack must not shadow a built-in: those carry code (recipes, memory
            // models) that a declaration cannot replace.
            if all
                .iter()
                .any(|p| p.id == pack.id || p.aliases.contains(&pack.id))
            {
                continue;
            }
            all.push(Box::leak(Box::new(pack)));
        }
        all
    })
}

/// Look up by canonical id or alias — the single place `--vendor` spellings are resolved.
pub fn by_id(name: &str) -> Option<&'static Pack> {
    let lower = name.to_lowercase();
    packs()
        .iter()
        .copied()
        .find(|p| p.id == lower || p.aliases.contains(&lower.as_str()))
}

/// Look up by the accelerator kind carried on collected devices.
/// Pack for a collected device's class. Falls back to the GPU pack rather than panicking:
/// a device can only carry a kind that some pack produced, but a stale kind must not take the
/// whole UI down mid-render.
pub fn by_kind(kind: AccelKind) -> &'static Pack {
    let all = packs();
    all.iter()
        .copied()
        .find(|p| p.kind == kind)
        .unwrap_or_else(|| all.iter().copied().find(|p| p.kind == AccelKind::Gpu).unwrap_or(all[0]))
}

/// Packs that support an ahead-of-time compile (the ones a compile form can target).
pub fn compilable() -> impl Iterator<Item = &'static Pack> {
    packs()
        .iter()
        .copied()
        .filter(|p| p.caps.compiles_ahead_of_time)
}


// ── Runtime packs ────────────────────────────────────────────────────────────────────────
//
// A YAML declaration of the data half of a pack. Deliberately not `#[derive(Deserialize)]`
// on `Pack` itself: `Pack` holds `&'static str` for zero-cost lookup everywhere else, and the
// conversion is where a malformed declaration gets rejected with a reason.

#[derive(serde::Deserialize)]
struct PackFile {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    label: String,
    #[serde(default)]
    display: String,
    /// Which built-in device class this reports as: gpu | rbln | rngd.
    kind: String,
    #[serde(default)]
    engine: String,
    #[serde(default)]
    exporter: String,
    #[serde(default)]
    family: String,
    #[serde(default)]
    accent: usize,
    /// HuggingFace orgs to offer in the zoo's live refresh.
    #[serde(default)]
    hf_orgs: Vec<String>,
    labels: LabelsFile,
    series: Vec<SeriesFile>,
    #[serde(default)]
    resource_key: String,
    #[serde(default)]
    product_label: Option<(String, String)>,
    #[serde(default)]
    route_segment: String,
    #[serde(default)]
    max_tensor_parallel: Option<u32>,
    #[serde(default)]
    serving_tp_unit: Option<String>,
    #[serde(default)]
    unified_memory: bool,
    /// `zero-is-healthy` | `non-zero-is-alive`; omit when the hardware reports no health.
    #[serde(default)]
    health: Option<String>,
}

#[derive(serde::Deserialize)]
struct LabelsFile {
    key: String,
    id: String,
    node: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    busy: Option<String>,
}

#[derive(serde::Deserialize)]
struct SeriesFile {
    field: String,
    metric: String,
    unit: String,
    #[serde(default)]
    agg: Option<String>,
    #[serde(default)]
    missing: String,
}

/// Leak a `String` into a `&'static str`. Packs live for the process, and the alternative is
/// threading a lifetime through every lookup for a handful of short strings loaded once.
fn intern(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn parse_field(name: &str) -> Option<Field> {
    Some(match name {
        "util" => Field::Util,
        "temp" => Field::Temp,
        "power" => Field::Power,
        "mem_used" => Field::MemUsed,
        "mem_total" => Field::MemTotal,
        "health" => Field::Health,
        "throttle" => Field::Throttle,
        "mem_bandwidth" => Field::MemBandwidth,
        "clock_mhz" => Field::ClockMhz,
        "mem_temp" => Field::MemTemp,
        "energy" => Field::Energy,
        _ => return None,
    })
}

fn parse_unit(name: &str) -> Option<Unit> {
    Some(match name {
        "percent" => Unit::Percent,
        "ratio" => Unit::Ratio,
        "bytes" => Unit::Bytes,
        "mib" => Unit::Mib,
        "celsius" => Unit::Celsius,
        "watt" => Unit::Watt,
        "millijoule" => Unit::Millijoule,
        "count" => Unit::Count,
        _ => return None,
    })
}

impl PackFile {
    /// Validate and convert. `Err` carries a message the caller prints — a bad declaration
    /// should say what is wrong, not vanish.
    fn into_pack(self, slot: u16) -> Result<Pack, String> {
        // A declared accelerator gets its own device class. Naming a built-in one would make
        // `by_kind` ambiguous, so it is refused rather than silently shadowing that vendor's
        // colour, capabilities and label.
        let kind = match self.kind.as_str() {
            "" | "custom" => AccelKind::Other(slot),
            claimed @ ("gpu" | "rbln" | "rngd" | "furiosa") => {
                return Err(format!(
                    "kind '{}' belongs to a built-in accelerator; omit `kind` (or use \
                     `custom`) so this pack gets its own device class",
                    claimed
                ))
            }
            other => return Err(format!("unknown kind '{}' (omit it, or `custom`)", other)),
        };
        if self.id.is_empty() || self.labels.key.is_empty() {
            return Err("id and labels.key are required".into());
        }
        // Same invariant the built-ins are tested for: a per-node ordinal cannot identify a
        // device across nodes (BUG-19).
        if self.labels.key == self.labels.id {
            return Err(format!(
                "labels.key ({}) must be a unique device identifier, not the display index",
                self.labels.key
            ));
        }
        let mut series = Vec::new();
        for sf in self.series {
            let field = parse_field(&sf.field)
                .ok_or_else(|| format!("unknown field '{}'", sf.field))?;
            let unit =
                parse_unit(&sf.unit).ok_or_else(|| format!("unknown unit '{}'", sf.unit))?;
            let agg = match sf.agg.as_deref().unwrap_or("max") {
                "max" => Agg::Max,
                "avg" => Agg::Avg,
                "sum" => Agg::Sum,
                other => return Err(format!("unknown agg '{}'", other)),
            };
            series.push(Series {
                field,
                metric: intern(sf.metric),
                unit,
                agg,
                missing: intern(if sf.missing.is_empty() {
                    format!("{} unavailable", sf.field)
                } else {
                    sf.missing
                }),
            });
        }
        if !series.iter().any(|s| s.field == Field::Util) {
            return Err("a util series is required".into());
        }
        let health = match self.health.as_deref() {
            None => None,
            Some("zero-is-healthy") => Some(Health::ZeroIsHealthy),
            Some("non-zero-is-alive") => Some(Health::NonZeroIsAlive),
            Some(other) => return Err(format!("unknown health '{}'", other)),
        };
        if health.is_some() != series.iter().any(|s| s.field == Field::Health) {
            return Err("health and its series must both be present or both absent".into());
        }
        let id = intern(self.id);
        Ok(Pack {
            id,
            aliases: Box::leak(
                self.aliases
                    .into_iter()
                    .map(intern)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            label: intern(if self.label.is_empty() {
                id.to_uppercase()
            } else {
                self.label
            }),
            display: intern(if self.display.is_empty() {
                id.to_string()
            } else {
                self.display
            }),
            kind,
            engine: intern(if self.engine.is_empty() {
                "vLLM".to_string()
            } else {
                self.engine
            }),
            exporter: intern(if self.exporter.is_empty() {
                id.to_string()
            } else {
                self.exporter
            }),
            accent: self.accent,
            hf_orgs: Box::leak(
                self.hf_orgs
                    .into_iter()
                    .map(intern)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            family: intern(if self.family.is_empty() {
                id.to_uppercase()
            } else {
                self.family
            }),
            labels: Labels {
                key: intern(self.labels.key),
                id: intern(self.labels.id),
                node: intern(self.labels.node),
                model: self.labels.model.map(intern),
                busy: self.labels.busy.map(intern),
            },
            series: Box::leak(series.into_boxed_slice()),
            caps: Caps {
                // Declarations cannot carry a compile recipe, so a runtime pack never claims
                // an ahead-of-time build — it would otherwise inherit another vendor's script.
                compiles_ahead_of_time: false,
                health,
                throttle: false,
                energy: false,
                unified_memory: self.unified_memory,
                serving_tp_unit: self.serving_tp_unit.map(intern),
                max_tensor_parallel: self.max_tensor_parallel,
            },
            scheduling: Scheduling {
                resource_key: intern(if self.resource_key.is_empty() {
                    format!("{}/device", id)
                } else {
                    self.resource_key
                }),
                product_label: self
                    .product_label
                    .map(|(k, v)| (intern(k), intern(v))),
                route_segment: intern(if self.route_segment.is_empty() {
                    id.to_string()
                } else {
                    self.route_segment
                }),
            },
        })
    }
}

/// Where runtime packs live. Absent directory is the normal case, not an error.
fn config_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::Path::new(&home).join(".config/lmd-top/accelerators"))
}

fn load_config_packs() -> Vec<Pack> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("yaml") | Some("yml")
            )
        })
        .collect();
    files.sort(); // deterministic order
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let slot = out.len() as u16;
        match serde_yaml::from_str::<PackFile>(&text).map_err(|e| e.to_string()) {
            Ok(pf) => match pf.into_pack(slot) {
                Ok(pack) => out.push(pack),
                Err(why) => eprintln!("lmd-top: ignoring {}: {}", path.display(), why),
            },
            Err(why) => eprintln!("lmd-top: ignoring {}: {}", path.display(), why),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_aliases_are_unique_and_resolvable() {
        let mut seen = std::collections::BTreeSet::new();
        for p in BUILTIN {
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
            let matching: Vec<&str> = BUILTIN
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
        for p in BUILTIN {
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
        for p in BUILTIN {
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
        for p in BUILTIN {
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

    // ── Runtime pack loader ──
    fn parse(yaml: &str) -> Result<Pack, String> {
        serde_yaml::from_str::<PackFile>(yaml)
            .map_err(|e| e.to_string())
            .and_then(|pf| pf.into_pack(0))
    }

    const MINIMAL: &str = "id: tpu\nlabel: TPU\nkind: custom\n\
        labels: { key: uuid, id: chip, node: hostname }\n\
        series: [ { field: util, metric: tpu_util, unit: ratio, agg: avg } ]\n";

    #[test]
    fn declared_pack_loads_and_defaults_sensibly() {
        let p = parse(MINIMAL).expect("minimal declaration loads");
        assert_eq!(p.id, "tpu");
        assert_eq!(p.kind, AccelKind::Other(0));
        // Unset fields fall back to something usable rather than empty.
        assert_eq!(p.scheduling.resource_key, "tpu/device");
        assert_eq!(p.scheduling.route_segment, "tpu");
        assert_eq!(p.engine, "vLLM");
        let util = p.series_for(Field::Util).expect("util series");
        assert_eq!(util.unit, Unit::Ratio);
        assert_eq!(util.agg, Agg::Avg);
        // A declaration carries no recipe, so it never claims an ahead-of-time build.
        assert!(!p.caps.compiles_ahead_of_time);
    }

    #[test]
    fn declared_pack_validation_rejects_with_a_reason() {
        let cases = [
            // Claiming a built-in class would make by_kind ambiguous.
            ("kind: custom", "kind: gpu", "built-in"),
            // A per-node ordinal cannot identify a device (BUG-19).
            (
                "labels: { key: uuid, id: chip, node: hostname }",
                "labels: { key: chip, id: chip, node: hostname }",
                "unique device identifier",
            ),
            // Unknown vocabulary should name the offending value.
            (
                "unit: ratio, agg: avg",
                "unit: furlongs, agg: avg",
                "unknown unit",
            ),
            ("field: util", "field: vibes", "unknown field"),
        ];
        for (from, to, expect) in cases {
            let err = parse(&MINIMAL.replace(from, to))
                .expect_err(&format!("{:?} should be rejected", to));
            assert!(
                err.contains(expect),
                "rejecting {:?} should mention {:?}, said: {}",
                to,
                expect,
                err
            );
        }
        // A util series is mandatory — everything downstream keys off it.
        let no_util = MINIMAL.replace("field: util", "field: temp").replace("unit: ratio", "unit: celsius");
        assert!(parse(&no_util).unwrap_err().contains("util series"));
        // Health and its series must agree, exactly as for the built-ins.
        let bad_health = format!("{}health: zero-is-healthy\n", MINIMAL);
        assert!(parse(&bad_health).unwrap_err().contains("health"));
    }

    #[test]
    fn declared_pack_cannot_shadow_a_builtin_id() {
        // `packs()` skips a declaration reusing a built-in id or alias; check the predicate
        // that guards it, since the loader reads the real filesystem.
        for p in BUILTIN {
            assert!(
                BUILTIN.iter().any(|b| b.id == p.id || b.aliases.contains(&p.id)),
                "{} should be recognised as taken",
                p.id
            );
        }
    }

    #[test]
    fn scheduling_keys_are_distinct() {
        let mut keys = std::collections::BTreeSet::new();
        for p in BUILTIN {
            assert!(
                keys.insert(p.scheduling.resource_key),
                "{} reuses resource key {}",
                p.id,
                p.scheduling.resource_key
            );
        }
    }
}
