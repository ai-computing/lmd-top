//! Golden manifest snapshots — the safety net for the manifest refactor.
//!
//! `--plan compile|deploy` output *is* this tool's product: operators review it and
//! `kubectl apply` it. Restructuring how it is built (text formatting → serialization) must
//! not change what it says, so every (model × vendor × op) combination is pinned to a file
//! under `tests/golden/`.
//!
//! Regenerate after an intentional change:  `UPDATE_GOLDEN=1 cargo test golden`
//! then read the diff — an unreviewed regeneration defeats the purpose.

use crate::app::App;
use crate::collect::{Accel, AccelKind, NodeInfo, Snapshot};

/// Deterministic cluster fixture. Manifests depend on discovered nodes/devices (placement,
/// node selectors, compile-host detection), so the snapshot has to be fixed for the goldens
/// to be stable — a live cluster would make them flap.
fn fixture() -> Snapshot {
    let node = |name: &str, npu: &str| NodeInfo {
        name: name.into(),
        load1: 1.0,
        mem_used_gb: 32.0,
        mem_total_gb: 512.0,
        cpu_pct: 10.0,
        disk_used_gb: 100.0,
        disk_total_gb: 1000.0,
        ready: true,
        cordoned: false,
        pressure: false,
        version: "v1.31.14".into(),
        npu: npu.into(),
    };
    let dev = |kind: AccelKind, id: &str, node: &str| Accel {
        kind,
        model: String::new(),
        id: id.into(),
        node: node.into(),
        util: 0.0,
        mem_used_gb: 0.0,
        mem_total_gb: 16.0,
        temp: 40.0,
        power: 20.0,
        busy_model: String::new(),
        alive: true,
        throttle: 0.0,
        unified_mem: false,
        mem_bw: f64::NAN,
        clock_mhz: f64::NAN,
        mem_temp: f64::NAN,
        energy_mj: f64::NAN,
    };
    Snapshot {
        ts: 1_700_000_000,
        nodes: vec![
            node("npu-1", "RBLN drv3.0.0 · RNGD drv2026.3.0"),
            node("gpu-1", ""),
        ],
        accel: vec![
            dev(AccelKind::Rbln, "rbln0", "npu-1"),
            dev(AccelKind::Rbln, "rbln1", "npu-1"),
            dev(AccelKind::Rbln, "rbln2", "npu-1"),
            dev(AccelKind::Rbln, "rbln3", "npu-1"),
            dev(AccelKind::Rngd, "npu0", "npu-1"),
            dev(AccelKind::Rngd, "npu1", "npu-1"),
            dev(AccelKind::Gpu, "gpu0", "gpu-1"),
        ],
        pvcs: vec!["model-store".into()],
        ..Default::default()
    }
}

fn app() -> App {
    let mut a = App::new();
    a.ns = "llm-serving".into();
    // LMD_COMPILE_ENV is a passthrough into the Job's env, so a value in the developer's
    // shell — or set by a test running in parallel — would change the generated manifest.
    // Pinned here for the same reason the images are.
    std::env::remove_var("LMD_COMPILE_ENV");
    // Pin the image inputs — otherwise LMD_*_IMAGE in the environment would change the goldens.
    a.img_rbln = None;
    a.img_furiosa = Some("furiosaai/furiosa-llm:latest".into());
    a.img_serving = None;
    a.apply(fixture());
    a
}

/// (name, op, vendor, model) — the combinations worth pinning.
const CASES: &[(&str, &str, &str, &str)] = &[
    ("compile-rbln-qwen", "compile", "rbln", "Qwen/Qwen2.5-0.5B-Instruct"),
    ("compile-rbln-llama", "compile", "rbln", "meta-llama/Llama-3.1-8B-Instruct"),
    ("compile-furiosa-qwen", "compile", "furiosa", "furiosa-ai/Qwen3-4B-FP8"),
    ("compile-furiosa-llama", "compile", "furiosa", "meta-llama/Llama-3.1-8B-Instruct"),
    ("deploy-rbln-qwen", "deploy", "rbln", "Qwen/Qwen2.5-0.5B-Instruct"),
    ("deploy-rbln-llama", "deploy", "rbln", "meta-llama/Llama-3.1-8B-Instruct"),
    ("deploy-furiosa-qwen", "deploy", "furiosa", "furiosa-ai/Qwen3-4B-FP8"),
    ("deploy-gpu-qwen", "deploy", "gpu", "Qwen/Qwen2.5-0.5B-Instruct"),
    ("deploy-gpu-llama", "deploy", "gpu", "meta-llama/Llama-3.1-8B-Instruct"),
    // Store maintenance: the `model` column carries the store path instead of an HF id.
    (
        "store-delete",
        "store-delete",
        "",
        "compiled/Qwen--Qwen2.5-0.5B-Instruct/rbln/RBLN-CA22-tp4-s8192",
    ),
    (
        "store-move",
        "store-move",
        "",
        "compiled/Qwen--Qwen2.5-0.5B-Instruct/rbln/RBLN-CA22-tp4-s8192",
    ),
    // Provenance probe: `model` carries the node-local artifact path.
    (
        "probe-provenance",
        "probe",
        "",
        "/home/gspark/rbln-gemma4-26b-a4b-tp4-s8192",
    ),
];

fn generate(op: &str, vendor: &'static str, model: &str) -> Result<(String, String), String> {
    let _g = crate::audit::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut a = app();
    match op {
        "compile" => a.plan_compile_for_model(model, vendor, &[]),
        "deploy" => a.plan_deploy_for_model(model, vendor, &[]),
        // `rm -rf` and `mv` in a Job: worth pinning byte-for-byte, and worth having the API
        // server confirm it would accept (scripts/validate-golden.sh).
        "store-delete" => Ok((
            format!("delete {}", model),
            crate::store::delete_manifest("llm-serving", "model-store", model, "12G").to_yaml(),
        )),
        "store-move" => Ok((
            format!("move {}", model),
            crate::store::move_manifest(
                "llm-serving",
                "model-store",
                model,
                "compiled/archive/Qwen--Qwen2.5-0.5B-Instruct/rbln/RBLN-CA22-tp4-s8192",
            )
            .to_yaml(),
        )),
        "probe" => Ok((
            format!("provenance {}", model),
            crate::probe::provenance_manifest("llm-serving", "gemma4-rbln", "etri-001", model)
                .0
                .to_yaml(),
        )),
        _ => unreachable!(),
    }
}

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{}.yaml", name))
}

#[test]
fn manifests_match_golden_snapshots() {
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let mut drifted: Vec<String> = Vec::new();
    for (name, op, vendor, model) in CASES {
        let (title, yaml) = generate(op, vendor, model)
            .unwrap_or_else(|e| panic!("{}: generation failed: {}", name, e));
        let body = format!("# title: {}\n{}", title, yaml);
        let path = golden_path(name);
        if update {
            std::fs::write(&path, &body).expect("write golden");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(want) if want == body => {}
            Ok(want) => {
                let at = want
                    .lines()
                    .zip(body.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or_else(|| want.lines().count().min(body.lines().count()));
                drifted.push(format!(
                    "{}: first difference at line {}\n  golden: {:?}\n  actual: {:?}",
                    name,
                    at + 1,
                    want.lines().nth(at).unwrap_or("<eof>"),
                    body.lines().nth(at).unwrap_or("<eof>"),
                ));
            }
            Err(_) => drifted.push(format!("{}: no golden file yet ({})", name, path.display())),
        }
    }
    assert!(
        drifted.is_empty(),
        "generated manifests drifted from tests/golden/ ({} case(s)).\n{}\n\n\
         If the change is intended: UPDATE_GOLDEN=1 cargo test golden — then review the diff.",
        drifted.len(),
        drifted.join("\n")
    );
}

/// The goldens pin bytes; these assertions pin *meaning*, so a reformat is allowed to change
/// the files but a semantic change (wrong image, resource key, model, replica count) is not.
#[test]
fn manifests_carry_the_right_semantics() {
    for (name, op, vendor, model) in CASES {
        let (_, yaml) = generate(op, vendor, model).unwrap_or_else(|e| panic!("{}: {}", name, e));
        let docs: Vec<serde_yaml::Value> = serde_yaml::Deserializer::from_str(&yaml)
            .map(|d| {
                <serde_yaml::Value as serde::Deserialize>::deserialize(d)
                    .unwrap_or_else(|e| panic!("{}: invalid YAML: {}", name, e))
            })
            .filter(|v: &serde_yaml::Value| !v.is_null())
            .collect();
        assert!(!docs.is_empty(), "{}: no documents", name);

        // Every document is a namespaced k8s object in the target namespace.
        for d in &docs {
            assert!(d.get("apiVersion").is_some(), "{}: doc without apiVersion", name);
            assert!(d.get("kind").is_some(), "{}: doc without kind", name);
            if let Some(ns) = d["metadata"]["namespace"].as_str() {
                assert_eq!(ns, "llm-serving", "{}: wrong namespace", name);
            }
        }

        // The manifest is about the requested model (BUG-17 guard, as data not substring luck).
        let repo_dir = model.replace('/', "--");
        let slug = model.replace(['/', '.'], "-").to_lowercase();
        if *op == "probe" {
            let kinds: Vec<&str> = docs.iter().filter_map(|d| d["kind"].as_str()).collect();
            assert_eq!(kinds, vec!["Job"], "{}: probe is a single Job", name);
            let pod = &docs[0]["spec"]["template"]["spec"];
            assert!(
                pod["nodeSelector"]["kubernetes.io/hostname"].as_str().is_some(),
                "{}: a hostPath probe must be pinned to its node",
                name
            );
            assert_eq!(
                pod["containers"][0]["volumeMounts"][0]["readOnly"].as_bool(),
                Some(true),
                "{}: a probe must mount read-only",
                name
            );
            assert_eq!(
                pod["volumes"][0]["hostPath"]["path"].as_str(),
                Some(*model),
                "{}: must mount the artifact directory itself",
                name
            );
            continue;
        }
        if op.starts_with("store-") {
            // Store maintenance: one Job, the path under the store mount, and — the point of
            // the whole module — an argv command with no shell to reinterpret the path.
            let kinds: Vec<&str> = docs.iter().filter_map(|d| d["kind"].as_str()).collect();
            assert_eq!(kinds, vec!["Job"], "{}: store op is a single Job", name);
            let pod = &docs[0]["spec"]["template"]["spec"];
            assert_eq!(
                pod["volumes"][0]["persistentVolumeClaim"]["claimName"].as_str(),
                Some("model-store"),
                "{}: must mount the store PVC",
                name
            );
            let argv = |v: &serde_yaml::Value| -> Vec<String> {
                v.as_sequence()
                    .unwrap_or_else(|| panic!("{}: command must be an argv sequence", name))
                    .iter()
                    .map(|x| x.as_str().unwrap_or_default().to_string())
                    .collect()
            };
            let cmd = argv(&pod["containers"][0]["command"]);
            for shell in ["sh", "bash", "-c"] {
                assert!(
                    !cmd.iter().any(|a| a == shell),
                    "{}: {:?} would put the path through a shell",
                    name,
                    cmd
                );
            }
            assert!(
                cmd.iter().any(|a| a == &format!("/mnt/store/{}", model)),
                "{}: the store path must appear as its own argv element, got {:?}",
                name,
                cmd
            );
            match *op {
                "store-delete" => assert_eq!(cmd[0], "rm", "{}: {:?}", name, cmd),
                "store-move" => {
                    assert_eq!(cmd[0], "mv", "{}: {:?}", name, cmd);
                    let init = argv(&pod["initContainers"][0]["command"]);
                    assert_eq!(init[0], "mkdir", "{}: parent must be created first", name);
                }
                _ => unreachable!(),
            }
            continue;
        }
        if *op == "compile" {
            assert!(
                yaml.contains(&repo_dir),
                "{}: compile manifest should reference {}",
                name,
                repo_dir
            );
            let kinds: Vec<&str> = docs.iter().filter_map(|d| d["kind"].as_str()).collect();
            assert!(kinds.contains(&"Job"), "{}: compile needs a Job, got {:?}", name, kinds);
        } else {
            assert!(
                yaml.contains(&format!("serve-{}", slug)),
                "{}: deploy manifest should target serve-{}",
                name,
                slug
            );
            let kinds: Vec<&str> = docs.iter().filter_map(|d| d["kind"].as_str()).collect();
            for want in ["Deployment", "Service", "InferencePool", "HTTPRoute"] {
                assert!(kinds.contains(&want), "{}: deploy missing {}", name, want);
            }
        }

        // Vendor identity: the scheduler-visible resource key must match the target.
        let want_key = match *vendor {
            "rbln" => "rebellions.ai/ATOM",
            "furiosa" => "furiosa.ai/rngd",
            _ => "nvidia.com/gpu",
        };
        if *op == "deploy" {
            assert!(
                yaml.contains(want_key),
                "{}: deploy should request {}",
                name,
                want_key
            );
            for other in ["rebellions.ai/ATOM", "furiosa.ai/rngd", "nvidia.com/gpu"] {
                if other != want_key {
                    assert!(
                        !yaml.contains(other),
                        "{}: deploy for {} must not mention {}",
                        name,
                        vendor,
                        other
                    );
                }
            }
        }
    }
}
