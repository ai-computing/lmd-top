//! Pure EPP configuration interpretation, independent of Kubernetes collection.
//! Known plugin types are classified conservatively; unresolved roles stay explicit.

#[derive(Clone, PartialEq)]
pub struct EppCfg {
    pub profiles: Vec<String>,
    pub entries: Vec<EppProfile>,
}

impl std::ops::Deref for EppCfg {
    type Target = EppProfile;
    fn deref(&self) -> &Self::Target {
        &self.entries[0]
    }
}

#[derive(Clone, PartialEq)]
pub struct EppProfile {
    pub other_plugins: Vec<String>,
    pub profile: String,
    pub scorers: Vec<(String, f64)>,
    pub picker: String,
    pub picker_refs: Vec<String>,
}

/// Relative nonnegative coefficient share, not endpoint-selection probability.
/// Scaling first avoids overflow when several finite coefficients are very large.
pub fn weight_shares(weights: &[f64]) -> Option<Vec<f64>> {
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight < 0.0)
    {
        return None;
    }
    let maximum = weights.iter().copied().fold(0.0, f64::max);
    if maximum == 0.0 {
        return None;
    }
    let scaled: Vec<_> = weights.iter().map(|weight| weight / maximum).collect();
    let total: f64 = scaled.iter().sum();
    Some(
        scaled
            .into_iter()
            .map(|weight| weight / total * 100.0)
            .collect(),
    )
}

pub fn parse(text: &str) -> Option<EppCfg> {
    let v: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    let profiles = v.get("schedulingProfiles")?.as_sequence()?;
    let prof = profiles.first()?;
    prof.as_mapping()?;

    let profile_names = profiles
        .iter()
        .map(|p| p["name"].as_str().unwrap_or("unnamed").to_string())
        .collect();
    let mut profile_ids = std::collections::BTreeSet::new();
    for candidate in profiles {
        candidate.as_mapping()?;
        let id = candidate
            .get("name")
            .map_or(Some("default"), |name| name.as_str())?;
        if id.is_empty() || !profile_ids.insert(id) {
            return None;
        }
    }
    let definitions = match v.get("plugins") {
        Some(plugins) => Some(plugins.as_sequence()?),
        None => None,
    };
    if let Some(definitions) = definitions {
        let mut names = std::collections::BTreeSet::new();
        for definition in definitions {
            let kind = definition["type"]
                .as_str()
                .filter(|kind| !kind.is_empty())?;
            let name = definition
                .get("name")
                .map_or(Some(kind), |name| name.as_str())?;
            if name.is_empty() || !names.insert(name) {
                return None;
            }
        }
    }
    let mut entries = Vec::new();
    for prof in profiles {
        let profile = prof["name"].as_str().unwrap_or("default").to_string();
        let mut scorers = Vec::new();
        let mut pickers = Vec::new();
        let mut other_plugins = Vec::new();
        if let Some(plugins) = prof.get("plugins") {
            let plugins = plugins.as_sequence()?;
            let mut references = std::collections::BTreeSet::new();
            for p in plugins {
                let name = p["pluginRef"]
                    .as_str()
                    .filter(|name| !name.is_empty())?
                    .to_string();
                if !references.insert(name.clone()) {
                    return None;
                }
                let w = match p.get("weight") {
                    Some(value) => value.as_f64()?,
                    None => 1.0,
                };
                if !w.is_finite() {
                    return None;
                }
                let plugin_type = match definitions {
                    Some(defs) => defs
                        .iter()
                        .find(|d| {
                            d["name"].as_str().or_else(|| d["type"].as_str()) == Some(name.as_str())
                        })
                        .and_then(|d| d["type"].as_str())
                        .unwrap_or("unresolved"),
                    None => name.as_str(),
                };
                match plugin_type {
                    "max-score-picker" => pickers.push(name),
                    "queue-scorer"
                    | "kv-cache-scorer"
                    | "kv-cache-utilization-scorer"
                    | "prefix-cache-scorer"
                    | "no-hit-lru-scorer"
                    | "load-aware-scorer"
                    | "active-request-scorer"
                    | "session-affinity-scorer"
                    | "latency-prediction-scorer"
                    | "heterogeneous-capacity" => scorers.push((name, w)),
                    _ => other_plugins.push(format!("{name} ({plugin_type}; role unverified)")),
                }
            }
        }
        entries.push(EppProfile {
            other_plugins,
            profile,
            scorers,
            picker_refs: pickers.clone(),
            picker: if pickers.is_empty() {
                "unverified (inspect plugin configuration)".into()
            } else {
                pickers.join(", ")
            },
        });
    }
    Some(EppCfg {
        profiles: profile_names,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coefficient_shares_do_not_overflow_or_invent_zero_denominators() {
        assert_eq!(weight_shares(&[f64::MAX, f64::MAX]), Some(vec![50.0, 50.0]));
        assert_eq!(weight_shares(&[0.0, 2.0]), Some(vec![0.0, 100.0]));
        for weights in [
            vec![],
            vec![0.0, 0.0],
            vec![-1.0, 2.0],
            vec![f64::NAN],
            vec![f64::INFINITY],
        ] {
            assert!(weight_shares(&weights).is_none());
        }
    }

    #[test]
    fn profile_selection_changes_rows_and_resets_local_weights() {
        let mut app = crate::app::App::new();
        app.view = crate::app::View::Epp;
        app.snap.epp = parse("schedulingProfiles:\n- name: first\n  plugins: [{pluginRef: queue-scorer, weight: 2}]\n- name: second\n  plugins: [{pluginRef: max-score-picker}]\n");
        assert_eq!(app.selected_epp().unwrap().profile, "first");
        app.epp_weights.insert("queue-scorer".into(), 9.0);
        app.cycle_epp_profile();
        assert_eq!(app.selected_epp().unwrap().profile, "second");
        assert!(app.selected_epp().unwrap().scorers.is_empty());
        assert!(app.epp_weights.is_empty());
        assert!(app.order().is_empty());
        app.cycle_epp_profile();
        assert_eq!(app.selected_epp().unwrap().profile, "first");
        assert_eq!(app.order().len(), 1);
        assert!(
            parse("schedulingProfiles: [{name: first}, {name: second, plugins: invalid}]")
                .is_none()
        );
    }

    #[test]
    fn epp_rejects_ambiguous_profile_and_plugin_identities() {
        for yaml in [
            "plugins: invalid\nschedulingProfiles: [{name: default}]",
            "plugins: [{name: same, type: queue-scorer}, {name: same, type: max-score-picker}]\nschedulingProfiles: [{name: default}]",
            "plugins: [{name: missing-type}]\nschedulingProfiles: [{name: default}]",
            "schedulingProfiles: [{name: same}, {name: same}]",
            "schedulingProfiles: [{name: default}, 42]",
            "schedulingProfiles: [{plugins: [{pluginRef: queue-scorer}, {pluginRef: queue-scorer}]}]",
        ] { assert!(parse(yaml).is_none(), "{yaml}"); }
    }

    #[test]
    fn epp_resolves_aliases_without_counting_picker_as_scorer() {
        let cfg = parse("plugins:\n- {name: pick, type: max-score-picker}\n- {name: queue, type: queue-scorer}\nschedulingProfiles:\n- name: first\n  plugins: [{pluginRef: pick}, {pluginRef: queue, weight: 2}, {pluginRef: unknown}]\n- {name: second, plugins: []}\n").unwrap();
        assert_eq!(cfg.scorers, vec![("queue".into(), 2.0)]);
        assert_eq!(cfg.picker, "pick");
        assert_eq!(cfg.profiles, vec!["first", "second"]);
        assert_eq!(cfg.other_plugins.len(), 1);
    }

    #[test]
    fn parse_epp_scorers() {
        let yaml = "schedulingProfiles:\n  - name: default\n    plugins:\n      - pluginRef: kv-cache-scorer\n        weight: 2\n      - pluginRef: queue-scorer\n        weight: 1\n";
        let cfg = parse(yaml).expect("parses");
        assert_eq!(cfg.profile, "default");
        assert_eq!(cfg.scorers.len(), 2);
        assert_eq!(cfg.scorers[0], ("kv-cache-scorer".to_string(), 2.0));
        for invalid in [
            "42",
            "{}",
            "schedulingProfiles: []",
            "schedulingProfiles: [42]",
            "schedulingProfiles: [{plugins: [{pluginRef: test, weight: .nan}]}]",
            "schedulingProfiles: [{plugins: [{pluginRef: test, weight: invalid}]}]",
        ] {
            assert!(parse(invalid).is_none(), "{invalid}");
        }
        assert!(cfg.picker.contains("unverified"));
    }
}
