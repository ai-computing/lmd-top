//! The serving Deployment manifest.
//!
//! Vendor differences here are real: NPU engines do not accept vLLM's generic `--model` form,
//! Furiosa needs `--fxb` alongside the HF id so config/tokenizer still resolve, and RBLN has
//! two shapes depending on whether a runtime image is configured. Built as data via
//! [`crate::manifest`]; the host-stack serving script lives in `assets/recipes/`.

use crate::manifest::{args, env_secret, env_val, mount, s, seq, Doc, Manifest};
use crate::ops::DeployForm;
use crate::ymap;
use serde_yaml::Value;

/// Host-stack serving script, embedded from `assets/recipes/`.
const RECIPE_RBLN_SERVE: &str = include_str!("../../../assets/recipes/rbln-serve.sh");

const STORE_MOUNT: &str = "/mnt/store";
const STORE_PVC: &str = "model-store";

/// Images available to the serving container, resolved from LMD_* env by `App`.
pub struct Images<'a> {
    pub furiosa: Option<&'a str>,
    pub serving: Option<&'a str>,
}

/// Inputs the caller has already normalised out of the form.
pub struct ServePlan<'a> {
    pub name: &'a str,
    pub ns: &'a str,
    pub replicas: String,
    pub devices: String,
    pub serve_tp: String,
    pub port: String,
    pub served: String,
    pub res_key: &'static str,
    pub product_label: Option<(&'static str, &'static str)>,
    pub place_host: String,
    pub place_label: String,
}

/// The vendor-specific half of the pod: image, container fields, volumes, and an extra
/// document (the recipe ConfigMap) when the vendor needs one.
struct Vendor {
    image: String,
    note: &'static str,
    container: Vec<(&'static str, Value)>,
    volumes: Vec<Value>,
    extra_doc: Option<Value>,
}

/// Render the serving Deployment (plus any recipe ConfigMap it needs).
pub fn serving_manifest(form: &DeployForm, plan: &ServePlan, images: &Images) -> Manifest {
    let v = match form.vendor {
        "furiosa" => furiosa(form, plan, images),
        "rbln" => rbln(form, plan, images),
        _ => gpu(form, plan, images),
    };

    let mut container = serde_yaml::Mapping::new();
    container.insert("name".into(), s("server"));
    container.insert("image".into(), s(v.image.clone()));
    for (k, val) in v.container {
        container.insert(k.into(), val);
    }

    let mut pod = serde_yaml::Mapping::new();
    // Placement: spread across hosts, pin to one host, or fall back to the accelerator's
    // node label. Unconstrained deployments carry no selector — the device resource schedules them.
    if plan.place_host == "spread" {
        pod.insert(
            "topologySpreadConstraints".into(),
            seq(vec![ymap! {
                "maxSkew" => Value::from(1),
                "topologyKey" => s("kubernetes.io/hostname"),
                "whenUnsatisfiable" => s("DoNotSchedule"),
                "labelSelector" => ymap! {
                    "matchLabels" => ymap! { "app" => s(plan.name) },
                },
            }]),
        );
    } else if !plan.place_host.is_empty() && plan.place_host != "any" {
        pod.insert(
            "nodeSelector".into(),
            ymap! { "kubernetes.io/hostname" => s(plan.place_host.clone()) },
        );
    } else if let Some((key, value)) = plan.product_label {
        pod.insert("nodeSelector".into(), ymap! { key => s(value) });
    }
    pod.insert("volumes".into(), seq(v.volumes));
    pod.insert("containers".into(), seq(vec![Value::Mapping(container)]));

    let deployment = ymap! {
        "apiVersion" => s("apps/v1"),
        "kind" => s("Deployment"),
        "metadata" => ymap! { "name" => s(plan.name), "namespace" => s(plan.ns) },
        "spec" => ymap! {
            "replicas" => plan.replicas.parse::<u32>().map(Value::from).unwrap_or(Value::from(1)),
            "selector" => ymap! { "matchLabels" => ymap! { "app" => s(plan.name) } },
            "template" => ymap! {
                "metadata" => ymap! {
                    "labels" => ymap! {
                        "app" => s(plan.name),
                        "llm-d.ai/inferenceServing" => s("true"),
                        "llm-d.ai/model" => s(plan.name),
                    },
                },
                "spec" => Value::Mapping(pod),
            },
        },
    };

    let mut m = Manifest::new()
        .note("Deployment manifest preview. Review, then apply with `kubectl apply -f -`.")
        .note(format!(
            "Serving model {}. Engine: {}.",
            form.model_id, form.engine
        ))
        .note(format!(
            "Placement: {}. Total device demand = {} x {}.",
            plan.place_label, plan.replicas, plan.devices
        ))
        .note("If the image contains a TODO- placeholder, set LMD_SERVING_IMAGE before applying.");
    if let Some(doc) = v.extra_doc {
        m = m.push(Doc::new(doc).note("Serving recipe, mounted at /scripts."));
    }
    m.push(Doc::new(deployment).note(v.note))
}

/// Device request/limit pair — serving reserves accelerators, so both are set.
fn resources(res_key: &str, devices: &str, cpu: &str, memory: &str) -> Value {
    ymap! {
        "limits" => ymap! { res_key => s(devices) },
        "requests" => ymap! {
            "cpu" => s(cpu),
            "memory" => s(memory),
            res_key => s(devices),
        },
    }
}

fn container_port(port: &str) -> Value {
    seq(vec![ymap! {
        "containerPort" => port.parse::<u32>().map(Value::from).unwrap_or_else(|_| s(port)),
    }])
}

fn store_volume() -> Value {
    ymap! {
        "name" => s("store"),
        "persistentVolumeClaim" => ymap! { "claimName" => s(STORE_PVC) },
    }
}

/// Furiosa: `furiosa-llm serve`. A store-backed artifact is passed as `--fxb` *in addition to*
/// the HF id, so config and tokenizer still come from HF. Serving TP counts PEs, while the
/// resource request counts RNGD cards — they are deliberately different numbers.
fn furiosa(form: &DeployForm, plan: &ServePlan, images: &Images) -> Vendor {
    let store_backed = form.mount.starts_with("/mnt/store/");
    let mut serve_args: Vec<String> = vec![
        "serve".into(),
        form.model_id.clone(),
        "--served-model-name".into(),
        form.model_id.clone(),
        "--port".into(),
        plan.port.clone(),
        "--tensor-parallel-size".into(),
        plan.serve_tp.clone(),
    ];
    if store_backed {
        serve_args.push("--fxb".into());
        serve_args.push(format!("{}/model.fxb", form.mount.trim_end_matches('/')));
    }
    let mut mounts = vec![mount("cache", "/model-cache", false)];
    let mut volumes = vec![ymap! { "name" => s("cache"), "emptyDir" => ymap! {} }];
    if store_backed {
        mounts.push(mount("store", STORE_MOUNT, true));
        volumes.push(store_volume());
    }
    Vendor {
        image: images
            .furiosa
            .map(str::to_string)
            .unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into()),
        note: "Furiosa: furiosa-llm serve. Store-backed artifacts use the HF id plus --fxb so \
               config/tokenizer still come from HF. Serving TP is PE count; resource devices \
               are RNGD count.",
        container: vec![
            ("args", args(serve_args)),
            ("ports", container_port(&plan.port)),
            (
                "env",
                seq(vec![
                    env_val("HF_HOME", "/model-cache"),
                    env_secret("HF_TOKEN", "hf-token", "HF_TOKEN", false),
                ]),
            ),
            (
                "resources",
                resources(plan.res_key, &plan.devices, "4", "16Gi"),
            ),
            ("volumeMounts", seq(mounts)),
        ],
        volumes,
        extra_doc: None,
    }
}

/// RBLN: with a vllm_rbln image, serve the compiled artifact directly. Without one, fall back
/// to the node's own RBLN stack via hostPath and the mounted serving recipe.
fn rbln(form: &DeployForm, plan: &ServePlan, images: &Images) -> Vendor {
    if let Some(img) = images.serving {
        return Vendor {
            image: img.to_string(),
            note: "RBLN: vllm_rbln runtime image from LMD_SERVING_IMAGE; loads the compiled \
                   artifact from model-store.",
            container: vec![
                (
                    "args",
                    args([
                        "serve".to_string(),
                        form.mount.clone(),
                        "--served-model-name".to_string(),
                        plan.served.clone(),
                        "--port".to_string(),
                        plan.port.clone(),
                        "--tensor-parallel-size".to_string(),
                        plan.devices.clone(),
                        "--max-num-seqs".to_string(),
                        "1".to_string(),
                    ]),
                ),
                ("ports", container_port(&plan.port)),
                (
                    "env",
                    seq(vec![env_val(
                        "VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK",
                        plan.devices.clone(),
                    )]),
                ),
                (
                    "resources",
                    resources(plan.res_key, &plan.devices, "8", "32Gi"),
                ),
                ("volumeMounts", seq(vec![mount("store", STORE_MOUNT, true)])),
            ],
            volumes: vec![store_volume()],
            extra_doc: None,
        };
    }

    // hostPath fallback: (volume name, host path, mount path).
    let host = [
        (
            "host-local-pkgs",
            "/home/gspark/.local/lib/python3.10/site-packages",
            "/home/gspark/.local/lib/python3.10/site-packages",
        ),
        (
            "host-sys-local-pkgs",
            "/usr/local/lib/python3.10/dist-packages",
            "/host-sys-local-pkgs",
        ),
        (
            "host-sys-pkgs",
            "/usr/lib/python3/dist-packages",
            "/host-sys-pkgs",
        ),
        ("host-libs", "/usr/lib/x86_64-linux-gnu", "/host-libs"),
        ("host-rbln-lib", "/usr/lib", "/host-rbln-lib"),
        ("host-rbln-bin", "/usr/bin", "/host-rbln-bin"),
    ];
    let cm_name = format!("{}-serve-script", plan.name);
    let mut volumes = vec![store_volume(), ymap! {
        "name" => s("script"),
        "configMap" => ymap! { "name" => s(cm_name.clone()) },
    }];
    volumes.extend(host.iter().map(|(name, path, _)| {
        ymap! {
            "name" => s(*name),
            "hostPath" => ymap! { "path" => s(*path), "type" => s("Directory") },
        }
    }));
    volumes.push(ymap! {
        "name" => s("shm"),
        "emptyDir" => ymap! { "medium" => s("Memory"), "sizeLimit" => s("16Gi") },
    });

    let mut mounts = vec![
        mount("store", STORE_MOUNT, true),
        mount("script", "/scripts", true),
    ];
    mounts.extend(host.iter().map(|(name, _, at)| mount(name, at, true)));
    mounts.push(mount("shm", "/dev/shm", false));

    Vendor {
        image: "ubuntu:22.04".to_string(),
        note: "RBLN: using host RBLN stack fallback on the target node; loads the compiled \
               artifact from model-store.",
        container: vec![
            ("command", args(["sh", "/scripts/serve.sh"])),
            ("ports", container_port(&plan.port)),
            (
                "env",
                seq(vec![
                    env_val("MOUNT", form.mount.clone()),
                    env_val("SERVED", plan.served.clone()),
                    env_val("PORT", plan.port.clone()),
                    env_val(
                        "PYTHONPATH",
                        "/home/gspark/.local/lib/python3.10/site-packages:/host-sys-local-pkgs:/host-sys-pkgs",
                    ),
                    env_val("PYTHONUNBUFFERED", "1"),
                    env_val("VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK", plan.devices.clone()),
                    env_val("LD_LIBRARY_PATH", "/host-rbln-lib:/host-libs"),
                    env_val(
                        "PATH",
                        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/host-rbln-bin",
                    ),
                ]),
            ),
            (
                "resources",
                resources(plan.res_key, &plan.devices, "8", "32Gi"),
            ),
            ("volumeMounts", seq(mounts)),
        ],
        volumes,
        extra_doc: Some(ymap! {
            "apiVersion" => s("v1"),
            "kind" => s("ConfigMap"),
            "metadata" => ymap! { "name" => s(cm_name), "namespace" => s(plan.ns) },
            "data" => ymap! { "serve.sh" => s(RECIPE_RBLN_SERVE) },
        }),
    }
}

/// GPU: vLLM serves the HF id or store path directly — no compile step involved.
fn gpu(form: &DeployForm, plan: &ServePlan, images: &Images) -> Vendor {
    Vendor {
        image: images
            .serving
            .map(str::to_string)
            .unwrap_or_else(|| "vllm/vllm-openai:latest".into()),
        note: "GPU: vLLM loads the model/store path directly; no compile step required.",
        container: vec![
            (
                "args",
                args([
                    "serve".to_string(),
                    form.mount.clone(),
                    "--served-model-name".to_string(),
                    plan.served.clone(),
                    "--port".to_string(),
                    plan.port.clone(),
                    "--tensor-parallel-size".to_string(),
                    plan.devices.clone(),
                ]),
            ),
            ("ports", container_port(&plan.port)),
            (
                "resources",
                resources(plan.res_key, &plan.devices, "4", "16Gi"),
            ),
            ("volumeMounts", seq(vec![mount("store", STORE_MOUNT, true)])),
        ],
        volumes: vec![store_volume()],
        extra_doc: None,
    }
}
