//! `lmd-top --doctor` — full Prometheus metric survey + gap analysis.
//! (1) detected exporters (jobs) (2) presence/absence of metrics lmd-top reads + impact when absent
//! (3) unused accelerator metrics (= new signal candidates). Diagnoses "why is this view empty" in one shot.

use crate::config::Config;
use crate::metrics::{ACCEL_PREFIXES, DEPS};
use crate::prom;
use std::collections::BTreeSet;

pub async fn run(cfg: &Config) {
    println!("lmd-top doctor · prometheus {} · ns {}\n", cfg.prom, cfg.ns);

    let names = match prom::label_values(&cfg.prom, "__name__").await {
        Ok(n) => n,
        Err(e) => {
            println!("✗ cannot reach Prometheus at {} — {}", cfg.prom, e);
            println!("  check LMD_PROM (host:port, plain HTTP) and network reachability.");
            return;
        }
    };
    let present: BTreeSet<&str> = names.iter().map(|s| s.as_str()).collect();

    // exporters (job label)
    match prom::label_values(&cfg.prom, "job").await {
        Ok(jobs) => {
            let accel_jobs: Vec<&String> = jobs
                .iter()
                .filter(|j| {
                    let l = j.to_lowercase();
                    l.contains("dcgm")
                        || l.contains("furiosa")
                        || l.contains("rbln")
                        || l.contains("node")
                        || l.contains("gpu")
                })
                .collect();
            println!(
                "exporters (accelerator/host jobs): {}",
                if accel_jobs.is_empty() {
                    "(none detected)".to_string()
                } else {
                    accel_jobs
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("  ")
                }
            );
        }
        Err(_) => println!("exporters: (job label unavailable)"),
    }
    println!("total metric names in Prometheus: {}\n", names.len());

    // coverage: present/absent per family + impact when absent
    println!("metric coverage (what lmd-top reads):");
    let mut fam = "";
    let (mut have, mut miss) = (0usize, 0usize);
    let mut affected: Vec<&str> = Vec::new();
    for (family, metric, impact) in DEPS {
        if *family != fam {
            println!("  {}", family);
            fam = family;
        }
        if present.contains(metric) {
            have += 1;
            println!("    ✓ {}", metric);
        } else {
            miss += 1;
            affected.push(impact);
            println!("    ✗ {:<48} → {}", metric, impact);
        }
    }
    println!(
        "\nsummary: {}/{} expected metrics present, {} missing.",
        have,
        have + miss,
        miss
    );

    // unused accelerator metrics (= new signal candidates) — those not in DEPS but with an accelerator family prefix
    let known: BTreeSet<&str> = DEPS.iter().map(|(_, m, _)| *m).collect();
    let mut candidates: Vec<&str> = names
        .iter()
        .map(|s| s.as_str())
        .filter(|n| ACCEL_PREFIXES.iter().any(|p| n.starts_with(p)) && !known.contains(n))
        .collect();
    candidates.sort_unstable();
    if candidates.is_empty() {
        println!("\nunused accelerator metrics: (none)");
    } else {
        println!(
            "\nunused accelerator metrics present ({} — candidate new signals to wire):",
            candidates.len()
        );
        for n in &candidates {
            println!("    · {}", n);
        }
    }
}
