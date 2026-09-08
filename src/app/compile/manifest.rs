//! Compile Job manifest — a `batch/v1` Job plus the ConfigMap carrying its recipe.
//!
//! Built as data via [`crate::manifest`] and serialized, not formatted as text: the model id
//! and every form value are user- (and, via the Zoo view, network-) supplied, and text
//! assembly made escaping the author's problem at each interpolation (BUG-06).
//! The recipes themselves live in `assets/recipes/` and take all input through the
//! environment, so no value is ever spliced into a script.

use crate::app::compile_job_name;
use crate::collect::NodeInfo;
use crate::manifest::{args, env_secret, env_val, mount, s, seq, Doc, Manifest};
use crate::ops::CompileForm;
use crate::quote::valid_model_id;
use crate::ymap;

/// Compile recipes, embedded from `assets/recipes/`. Real files so they can be linted, diffed
/// and run by hand; the Job mounts them from a ConfigMap.
const RECIPE_RBLN: &str = include_str!("../../../assets/recipes/rbln-compile.py");
const RECIPE_FURIOSA: &str = include_str!("../../../assets/recipes/furiosa-compile.sh");

/// Shared model store — compile output and the HF cache both live here.
const STORE_MOUNT: &str = "/mnt/store";
const STORE_PVC: &str = "model-store";

/// The outcome of building a compile manifest from a user-configured form.
pub enum CompileManifestOutcome {
    /// Manifest generated successfully and ready for preview / kubectl apply.
    Ready { title: String, yaml: String },
    /// Blocked due to known parameter invalidity (e.g., flash_attn kvpart mismatch).
    Blocked { issue: String },
    /// Blocked because model_id is not a resolvable HuggingFace repo (org/name) or local path.
    /// The caller holds the form, so only the offending id travels with the outcome.
    UnresolvedSource { model_id: String },
    /// Attempted compile for a non-NPU vendor.
    InvalidVendor { vendor: String },
}

/// How a vendor's compile container is shaped — the parts that genuinely differ.
struct ContainerPlan {
    /// Recipe file contents and the ConfigMap key it is mounted as.
    recipe: (&'static str, &'static str),
    /// `command` for the container.
    command: Vec<String>,
    /// Environment beyond the shared set.
    extra_env: Vec<serde_yaml::Value>,
    /// Volumes and mounts beyond store + script + work.
    extra_volumes: Vec<serde_yaml::Value>,
    extra_mounts: Vec<serde_yaml::Value>,
    /// One-line explanation shown above the Job.
    note: &'static str,
}

/// Construct the compile Job manifest (recipe ConfigMap + Job).
pub fn build_compile_manifest(
    form: &CompileForm,
    ns: &str,
    img_rbln: Option<&str>,
    img_furiosa: Option<&str>,
    nodes: &[NodeInfo],
) -> CompileManifestOutcome {
    let model_id = &form.model_id;
    let vendor = form.vendor;

    if !matches!(vendor, "rbln" | "furiosa") {
        return CompileManifestOutcome::InvalidVendor {
            vendor: vendor.to_string(),
        };
    }

    if !model_id.contains('/') || !valid_model_id(model_id) {
        return CompileManifestOutcome::UnresolvedSource {
            model_id: model_id.clone(),
        };
    }

    if vendor == "rbln" {
        if let Some(issue) = super::preflight::rbln_param_issue(form) {
            return CompileManifestOutcome::Blocked { issue };
        }
    }

    let target = form.target();
    let repo_dir = model_id.replace('/', "--");
    let name = compile_job_name(&repo_dir, &target);
    let outdir = format!("{}/compiled/{}/{}/{}", STORE_MOUNT, repo_dir, vendor, target);

    // Destination node: explicit picker selection takes precedence over any auto-selection.
    let node_pick = if form.dest.is_empty() {
        "any"
    } else {
        &form.dest
    };
    let node_host = node_pick.split('(').next().unwrap_or("any").trim();

    // RBLN host-stack fallback: with no registry image configured, the compile runs against the
    // target node's own rebel-compiler install via hostPath, so it must land on an RBLN node.
    let rbln_host_stack = vendor == "rbln" && img_rbln.is_none();
    let auto_rbln_node = nodes
        .iter()
        .find(|n| n.npu.to_uppercase().contains("RBLN"))
        .map(|n| n.name.clone());

    let image = if vendor == "rbln" {
        if rbln_host_stack {
            "ubuntu:22.04".to_string()
        } else {
            img_rbln.unwrap_or_default().to_string()
        }
    } else {
        img_furiosa
            .map(str::to_string)
            .unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into())
    };

    // nodeSelector is omitted entirely when unconstrained — the device resource request is what
    // schedules an unpinned compile.
    let node_selector = if node_host != "any" && !node_host.is_empty() {
        Some(ymap! { "kubernetes.io/hostname" => s(node_host) })
    } else if rbln_host_stack {
        Some(match &auto_rbln_node {
            Some(n) => ymap! { "kubernetes.io/hostname" => s(n.clone()) },
            None => ymap! { "rebellions.ai/npu.product" => s("RBLN-CA22") },
        })
    } else {
        None
    };

    let plan = if vendor == "furiosa" {
        furiosa_plan(form, &repo_dir)
    } else {
        rbln_plan(rbln_host_stack, form)
    };

    // Shared environment: where the weights come from and where the artifact goes.
    let mut env = vec![
        env_secret("HF_TOKEN", "hf-token", "HF_TOKEN", true),
        env_val("MODEL_STORE", STORE_MOUNT),
        env_val("MODEL_ID", model_id.clone()),
        env_val("OUTPUT", outdir.clone()),
    ];
    env.extend(plan.extra_env.clone());

    let (recipe_body, recipe_key) = plan.recipe;
    let cm_name = format!("{}-script", name);

    let mut volumes = vec![
        ymap! {
            "name" => s("store"),
            "persistentVolumeClaim" => ymap! { "claimName" => s(STORE_PVC) },
        },
        ymap! {
            "name" => s("script"),
            "configMap" => ymap! { "name" => s(cm_name.clone()) },
        },
        ymap! { "name" => s("work"), "emptyDir" => ymap! {} },
    ];
    volumes.extend(plan.extra_volumes.clone());

    let mut mounts = vec![
        mount("store", STORE_MOUNT, false),
        mount("script", "/scripts", true),
        mount("work", "/work", false),
    ];
    mounts.extend(plan.extra_mounts.clone());

    let mut pod_spec = serde_yaml::Mapping::new();
    pod_spec.insert("restartPolicy".into(), s("Never"));
    if let Some(sel) = node_selector {
        pod_spec.insert("nodeSelector".into(), sel);
    }
    pod_spec.insert("volumes".into(), seq(volumes));
    pod_spec.insert(
        "containers".into(),
        seq(vec![ymap! {
            "name" => s("compile"),
            "image" => s(image),
            "command" => args(plan.command.clone()),
            "env" => seq(env),
            "resources" => ymap! {
                "requests" => ymap! { "cpu" => s("8"), "memory" => s("16Gi") },
            },
            "volumeMounts" => seq(mounts),
        }]),
    );

    let job = ymap! {
        "apiVersion" => s("batch/v1"),
        "kind" => s("Job"),
        "metadata" => ymap! { "name" => s(name.clone()), "namespace" => s(ns) },
        "spec" => ymap! {
            "backoffLimit" => serde_yaml::Value::from(0),
            "ttlSecondsAfterFinished" => serde_yaml::Value::from(3600),
            "template" => ymap! { "spec" => serde_yaml::Value::Mapping(pod_spec) },
        },
    };

    let config_map = ymap! {
        "apiVersion" => s("v1"),
        "kind" => s("ConfigMap"),
        "metadata" => ymap! { "name" => s(cm_name), "namespace" => s(ns) },
        "data" => ymap! { recipe_key => s(recipe_body) },
    };

    let opts_summary: String = form
        .fields
        .iter()
        .map(|f| format!("{}={}", f.key, f.value))
        .collect::<Vec<_>>()
        .join("  ");

    let yaml = Manifest::new()
        .note("Compile Job preview. Review, then apply with `kubectl apply -f -`.")
        .note(format!(
            "Model {} -> {} compile -> shared store compiled/{}/{}/{}",
            model_id, vendor, repo_dir, vendor, target
        ))
        .note(format!("Compile-time fixed options: {}", opts_summary))
        .push(
            Doc::new(config_map)
                .note(format!("Compile recipe ({}), mounted at /scripts.", recipe_key)),
        )
        .push(Doc::new(job).note(plan.note))
        .to_yaml();

    CompileManifestOutcome::Ready {
        title: format!("compile {} → {}", form.model, target),
        yaml,
    }
}

/// Furiosa: `fxb build` from the vendor image, driven entirely by environment variables.
fn furiosa_plan(form: &CompileForm, repo_dir: &str) -> ContainerPlan {
    let or = |key: &str, def: &str| {
        let v = form.get(key);
        if v.is_empty() {
            def.to_string()
        } else {
            v
        }
    };
    ContainerPlan {
        recipe: (RECIPE_FURIOSA, "compile.sh"),
        command: vec!["sh".into(), "/scripts/compile.sh".into()],
        extra_env: vec![
            // The downloader cannot write to the SMB-backed store (os error 95), so the HF cache
            // is local scratch and only the finished artifact is copied back.
            env_val("HF_HOME", "/work/hub"),
            env_val(
                "PREFETCHED_DIR",
                format!("{}/hub/hub/models--{}", STORE_MOUNT, repo_dir),
            ),
            env_val("TP", or("tp", "1")),
            env_val("PP", or("pp", "1")),
            env_val("MAX_LEN", or("max-len", "8192")),
        ],
        extra_volumes: Vec::new(),
        extra_mounts: Vec::new(),
        note: "Furiosa: fxb build for furiosa-ai quantized checkpoints. Installs the aarch64 \
               cross-compiler, builds in local scratch, then copies to the model store.",
    }
}

/// RBLN: optimum-rbln recipe. Without a registry image, borrow the node's own compiler stack.
fn rbln_plan(host_stack: bool, form: &CompileForm) -> ContainerPlan {
    let params = rbln_param_env(form);
    if !host_stack {
        return ContainerPlan {
            recipe: (RECIPE_RBLN, "compile.py"),
            command: vec!["python3".into(), "/scripts/compile.py".into()],
            extra_env: [vec![env_val("HF_HOME", format!("{}/hub", STORE_MOUNT))], params]
                .concat(),
            extra_volumes: Vec::new(),
            extra_mounts: Vec::new(),
            note: "RBLN: runs the optimum-rbln recipe inside LMD_COMPILE_IMAGE_RBLN \
                   (rbln_create_runtimes=False, so serving may hold the chips).",
        };
    }
    // hostPath fallback: a bare ubuntu image plus the node's python/rebel-compiler install.
    // tzdata is needed because pandas→pytz reads /usr/share/zoneinfo, which minimal images lack.
    let host = [
        ("hp-local", "/home/gspark/.local/lib/python3.10/site-packages", "/home/gspark/.local/lib/python3.10/site-packages"),
        ("hp-sys", "/usr/local/lib/python3.10/dist-packages", "/host-sys"),
        ("hp-lib", "/usr/lib", "/host-lib"),
        ("hp-bin", "/usr/bin", "/host-bin"),
    ];
    ContainerPlan {
        recipe: (RECIPE_RBLN, "compile.py"),
        command: vec![
            "bash".into(),
            "-c".into(),
            "set -e; export DEBIAN_FRONTEND=noninteractive; apt-get update -qq >/dev/null 2>&1; \
             apt-get install -y -qq --no-install-recommends python3.10 libnuma1 libgomp1 \
             ca-certificates tzdata >/dev/null 2>&1; \
             ln -sf /usr/bin/python3.10 /usr/local/bin/python3; python3 /scripts/compile.py"
                .into(),
        ],
        extra_env: [
            vec![
                env_val("HF_HOME", format!("{}/hub", STORE_MOUNT)),
                env_val(
                    "PYTHONPATH",
                    "/home/gspark/.local/lib/python3.10/site-packages:/host-sys:/host-lib/python3/dist-packages",
                ),
                env_val("LD_LIBRARY_PATH", "/host-lib:/host-lib/x86_64-linux-gnu"),
                env_val(
                    "PATH",
                    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/host-bin",
                ),
            ],
            params,
        ]
        .concat(),
        extra_volumes: host
            .iter()
            .map(|(name, path, _)| {
                ymap! {
                    "name" => s(*name),
                    "hostPath" => ymap! { "path" => s(*path), "type" => s("Directory") },
                }
            })
            .collect(),
        extra_mounts: host
            .iter()
            .map(|(name, _, at)| mount(name, at, false))
            .collect(),
        note: "RBLN: no registry image configured, so this borrows the target node's host \
               rebel-compiler stack via hostPath (rbln_create_runtimes=False).",
    }
}

/// Map compile form fields onto the RBLN_* names the optimum-rbln recipe reads.
///
/// Only RBLN needs this: the Furiosa recipe takes its parameters as TP/PP/MAX_LEN, set in its
/// own plan. (The old generic arms — TENSOR_PARALLEL_SIZE and friends — were unreachable, since
/// only the RBLN branch ever consumed this list.)
fn rbln_param_env(form: &CompileForm) -> Vec<serde_yaml::Value> {
    form.fields
        .iter()
        .filter(|f| !f.value.is_empty() && f.value != "none")
        .filter_map(|f| {
            let key = match f.key.as_str() {
                "tp" => "RBLN_TENSOR_PARALLEL_SIZE",
                "max-len" => "RBLN_MAX_SEQ_LEN",
                "batch" => "RBLN_BATCH_SIZE",
                "attn" => "RBLN_ATTN_IMPL",
                "kvpart" => "RBLN_KVCACHE_PARTITION_LEN",
                "npu" => "RBLN_NPU",
                "quant" => "RBLN_QUANTIZATION",
                _ => return None,
            };
            Some(env_val(key, f.value.clone()))
        })
        .collect()
}
