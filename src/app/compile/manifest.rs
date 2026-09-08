//! Kubernetes batch/v1 Job and ConfigMap manifest generation for NPU compilation.

use crate::app::compile_job_name;
use crate::collect::NodeInfo;
use crate::ops::CompileForm;
use crate::quote::{shq, valid_model_id, yamlq};

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

/// Construct the Kubernetes Job (and optional ConfigMap) YAML manifest for the compile job.
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
    let tp = form.get("tp");

    // Destination node: explicit picker selection takes precedence over any auto-selection
    let node_pick = if form.dest.is_empty() {
        "any"
    } else {
        &form.dest
    };
    let node_host = node_pick.split('(').next().unwrap_or("any").trim();

    // RBLN host stack fallback: when no registry image is configured, run on an RBLN node via hostPath
    let rbln_host_stack = vendor == "rbln" && img_rbln.is_none();
    let auto_rbln_node = nodes
        .iter()
        .find(|n| n.npu.to_uppercase().contains("RBLN"))
        .map(|n| n.name.clone());

    let image = if vendor == "rbln" {
        if rbln_host_stack {
            "ubuntu:22.04".to_string()
        } else {
            img_rbln.unwrap().to_string()
        }
    } else {
        img_furiosa
            .map(|s| s.to_string())
            .unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into())
    };

    let node_label = if node_host != "any" && !node_host.is_empty() {
        format!("kubernetes.io/hostname: {}", node_host)
    } else if rbln_host_stack {
        match &auto_rbln_node {
            Some(n) => format!("kubernetes.io/hostname: {}", n),
            None => "rebellions.ai/npu.product: RBLN-CA22".to_string(),
        }
    } else {
        String::new()
    };

    let resources_line = "resources: { requests: { cpu: \"8\", memory: \"16Gi\" } }";
    let envs = build_compile_env_vars(vendor, form, &repo_dir, &target);
    let opts_summary: String = form
        .fields
        .iter()
        .map(|f| format!("{}={}", f.key, f.value))
        .collect::<Vec<_>>()
        .join("  ");

    let outdir = format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target);

    let (volumes_extra, env_block, mounts_extra, command, note, extra_doc) = if vendor == "furiosa"
    {
        build_furiosa_container_spec(model_id, form, &tp, &outdir)
    } else {
        build_rbln_container_spec(&name, ns, rbln_host_stack, &envs)
    };

    let yaml = format!(
        "# Compile Job preview. Review, then apply with `kubectl apply -f -`.\n\
         # Model {model_id} -> {vendor} compile -> shared store compiled/{repo_dir}/{vendor}/{target}.\n\
         # Compile-time fixed options: {opts}\n\
         {extra_doc}\
         {note}\n\
         apiVersion: batch/v1\n\
         kind: Job\n\
         metadata: {{ name: {name}, namespace: {ns} }}\n\
         spec:\n\
         \x20 backoffLimit: 0\n\
         \x20 ttlSecondsAfterFinished: 3600\n\
         \x20 template:\n\
         \x20   spec:\n\
         \x20     restartPolicy: Never\n\
         \x20     nodeSelector: {{ {node_label} }}\n\
         \x20     volumes:\n\
         \x20       - {{ name: store, persistentVolumeClaim: {{ claimName: model-store }} }}\n\
         {volumes_extra}\
         \x20     containers:\n\
         \x20       - name: compile\n\
         \x20         image: {image}\n\
         \x20         {resources_line}\n\
         \x20         env:\n\
         {env_block}\
         \x20         volumeMounts:\n\
         \x20           - {{ name: store, mountPath: /mnt/store }}\n\
         {mounts_extra}\
         \x20         command: {command}\n",
        model_id = model_id,
        vendor = vendor,
        repo_dir = repo_dir,
        target = target,
        opts = opts_summary,
        extra_doc = extra_doc,
        note = note,
        name = name,
        ns = ns,
        node_label = node_label,
        image = image,
        resources_line = resources_line,
        volumes_extra = volumes_extra,
        env_block = env_block,
        mounts_extra = mounts_extra,
        command = command,
    );

    CompileManifestOutcome::Ready {
        title: format!("compile {} → {}", form.model, target),
        yaml,
    }
}

/// Map compile form fields to environment variables passed to compiler runners.
fn build_compile_env_vars(
    vendor: &str,
    form: &CompileForm,
    repo_dir: &str,
    target: &str,
) -> Vec<(String, String)> {
    let mut envs: Vec<(String, String)> = vec![
        ("MODEL_STORE".into(), "/mnt/store".into()),
        ("MODEL_ID".into(), form.model_id.clone()),
        (
            "OUTPUT".into(),
            format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target),
        ),
        ("HF_HOME".into(), "/mnt/store/hub".into()),
    ];

    for f in &form.fields {
        if f.value.is_empty() || f.value == "none" {
            continue;
        }
        let ek = match (vendor, f.key.as_str()) {
            ("rbln", "tp") => "RBLN_TENSOR_PARALLEL_SIZE",
            ("rbln", "max-len") => "RBLN_MAX_SEQ_LEN",
            ("rbln", "batch") => "RBLN_BATCH_SIZE",
            ("rbln", "attn") => "RBLN_ATTN_IMPL",
            ("rbln", "kvpart") => "RBLN_KVCACHE_PARTITION_LEN",
            ("rbln", "npu") => "RBLN_NPU",
            ("rbln", "quant") => "RBLN_QUANTIZATION",
            (_, "tp") => "TENSOR_PARALLEL_SIZE",
            (_, "pp") => "PIPELINE_PARALLEL_SIZE",
            (_, "max-len") => "MAX_SEQ_LEN_TO_CAPTURE",
            (_, "batch") => "BUCKET_BATCH_SIZE",
            (_, "chunk") => "PREFILL_CHUNK_SIZE",
            (_, "block") => "PAGED_ATTENTION_BLOCK_SIZE",
            (_, "quant") => "USE_ACTIVATION_DQ",
            _ => continue,
        };
        envs.push((ek.into(), f.value.clone()));
    }
    envs
}

/// Furiosa container execution configuration using `fxb build`.
fn build_furiosa_container_spec(
    model_id: &str,
    form: &CompileForm,
    tp: &str,
    outdir: &str,
) -> (String, String, String, String, &'static str, String) {
    let pp = {
        let p = form.get("pp");
        if p.is_empty() {
            "1".into()
        } else {
            p
        }
    };
    let ml = {
        let m = form.get("max-len");
        if m.is_empty() {
            "8192".into()
        } else {
            m
        }
    };
    let dashes = model_id.replace('/', "--");
    let cmd = format!(
        "set -e; apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq gcc-aarch64-linux-gnu build-essential >/dev/null 2>&1; \
         mkdir -p /work/hub/hub; \
         if [ -d {prefetched} ]; then echo 'reuse prefetched weights from store'; cp -r {prefetched} /work/hub/hub/ && export HF_HUB_OFFLINE=1; fi; \
         mkdir -p /work/out; fxb build {model_id} /work/out/model -tp {tp} -pp {pp} --max-model-len {ml} --concurrency 8; \
         mkdir -p {outdir}; cp -r /work/out/. {outdir}/; echo COMPILE_DONE; ls -la {outdir}",
        outdir = shq(outdir),
        model_id = shq(model_id),
        tp = shq(tp),
        pp = shq(&pp),
        ml = shq(&ml),
        prefetched = shq(&format!("/mnt/store/hub/hub/models--{}", dashes))
    );
    (
        "        - { name: work, emptyDir: {} }\n".to_string(),
        "            - { name: HF_HOME, value: /work/hub }\n            - { name: HF_TOKEN, valueFrom: { secretKeyRef: { name: hf-token, key: HF_TOKEN, optional: true } } }\n".to_string(),
        "            - { name: work, mountPath: /work }\n".to_string(),
        format!("[\"sh\", \"-c\", \"{}\"]", yamlq(&cmd)),
        "# Furiosa: run fxb build directly for furiosa-ai quantized checkpoints. Installs aarch64 cross-compiler, builds locally, then copies to model-store.",
        String::new(),
    )
}

/// Rebellions (RBLN) inline ConfigMap compile script and container execution configuration.
fn build_rbln_container_spec(
    name: &str,
    ns: &str,
    rbln_host_stack: bool,
    envs: &[(String, String)],
) -> (String, String, String, String, &'static str, String) {
    let env_lines: String = envs
        .iter()
        .map(|(k, v)| {
            format!(
                "            - {{ name: {}, value: \"{}\" }}\n",
                k,
                yamlq(v)
            )
        })
        .collect();
    let hf_token_env = "            - { name: HF_TOKEN, valueFrom: { secretKeyRef: { name: hf-token, key: HF_TOKEN, optional: true } } }\n";

    let script_doc = format!(
        "# RBLN compile script (inline) — create_runtimes=False, local build, then copy to model-store.\n\
         apiVersion: v1\n\
         kind: ConfigMap\n\
         metadata: {{ name: {name}-script, namespace: {ns} }}\n\
         data:\n\
         \x20 compile.py: |\n\
         \x20\x20\x20 import os, shutil\n\
         \x20\x20\x20 from optimum.rbln import RBLNAutoModelForCausalLM as M\n\
         \x20\x20\x20 g = os.environ.get; o = os.environ[\"OUTPUT\"]; loc = \"/work/out\"\n\
         \x20\x20\x20 cfg = dict(\n\
         \x20\x20\x20\x20\x20 rbln_npu=g(\"RBLN_NPU\", \"RBLN-CA22\"),\n\
         \x20\x20\x20\x20\x20 rbln_num_devices=int(g(\"RBLN_TENSOR_PARALLEL_SIZE\", \"1\")),\n\
         \x20\x20\x20\x20\x20 rbln_max_seq_len=int(g(\"RBLN_MAX_SEQ_LEN\", \"4096\")),\n\
         \x20\x20\x20\x20\x20 rbln_batch_size=int(g(\"RBLN_BATCH_SIZE\", \"1\")))\n\
         \x20\x20\x20 attn = g(\"RBLN_ATTN_IMPL\", \"flash_attn\")\n\
         \x20\x20\x20 if attn:\n\
         \x20\x20\x20\x20\x20 cfg[\"rbln_attn_impl\"] = attn\n\
         \x20\x20\x20 if attn == \"flash_attn\":\n\
         \x20\x20\x20\x20\x20 cfg[\"rbln_kvcache_partition_len\"] = int(g(\"RBLN_KVCACHE_PARTITION_LEN\", \"16384\"))\n\
         \x20\x20\x20 print(\"RBLN_CONFIG\", cfg)\n\
         \x20\x20\x20 m = M.from_pretrained(os.environ[\"MODEL_ID\"], export=True, rbln_create_runtimes=False, **cfg)\n\
         \x20\x20\x20 m.save_pretrained(loc)\n\
         \x20\x20\x20 os.makedirs(o, exist_ok=True)\n\
         \x20\x20\x20 for f in os.listdir(loc):\n\
         \x20\x20\x20\x20\x20 s = os.path.join(loc, f); d = os.path.join(o, f)\n\
         \x20\x20\x20\x20\x20 shutil.copytree(s, d, dirs_exist_ok=True) if os.path.isdir(s) else shutil.copy2(s, d)\n\
         \x20\x20\x20 print(\"COMPILE_DONE\", os.listdir(o))\n\
         ---\n",
        name = name,
        ns = ns
    );

    let cm_vol = format!(
        "        - {{ name: script, configMap: {{ name: {}-script }} }}\n        - {{ name: work, emptyDir: {{}} }}\n",
        name
    );

    if rbln_host_stack {
        let host_vols = format!(
            "{cm}\
             \x20       - {{ name: hp-local, hostPath: {{ path: /home/gspark/.local/lib/python3.10/site-packages, type: Directory }} }}\n\
             \x20       - {{ name: hp-sys, hostPath: {{ path: /usr/local/lib/python3.10/dist-packages, type: Directory }} }}\n\
             \x20       - {{ name: hp-lib, hostPath: {{ path: /usr/lib, type: Directory }} }}\n\
             \x20       - {{ name: hp-bin, hostPath: {{ path: /usr/bin, type: Directory }} }}\n",
            cm = cm_vol
        );
        let host_mounts = "            - { name: script, mountPath: /scripts, readOnly: true }\n            - { name: work, mountPath: /work }\n            - { name: hp-local, mountPath: /home/gspark/.local/lib/python3.10/site-packages }\n            - { name: hp-sys, mountPath: /host-sys }\n            - { name: hp-lib, mountPath: /host-lib }\n            - { name: hp-bin, mountPath: /host-bin }\n".to_string();
        let host_env = "            - { name: PYTHONPATH, value: \"/home/gspark/.local/lib/python3.10/site-packages:/host-sys:/host-lib/python3/dist-packages\" }\n            - { name: LD_LIBRARY_PATH, value: \"/host-lib:/host-lib/x86_64-linux-gnu\" }\n            - { name: PATH, value: \"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/host-bin\" }\n";
        let cmd = "set -e; export DEBIAN_FRONTEND=noninteractive; apt-get update -qq >/dev/null 2>&1; apt-get install -y -qq --no-install-recommends python3.10 libnuma1 libgomp1 ca-certificates tzdata >/dev/null 2>&1; ln -sf /usr/bin/python3.10 /usr/local/bin/python3; python3 /scripts/compile.py";
        (
            host_vols,
            format!("{}{}{}", host_env, hf_token_env, env_lines),
            host_mounts,
            format!("[\"bash\", \"-c\", \"{}\"]", cmd),
            "# RBLN: no registry image configured, so this uses the target node's host rebel-compiler stack via hostPath. create_runtimes=False.",
            script_doc,
        )
    } else {
        (
            cm_vol,
            format!("{}{}", hf_token_env, env_lines),
            "            - { name: script, mountPath: /scripts, readOnly: true }\n            - { name: work, mountPath: /work }\n".to_string(),
            "[\"python3\", \"/scripts/compile.py\"]".to_string(),
            "# RBLN: runs the inline optimum-rbln compile script inside LMD_COMPILE_IMAGE_RBLN. create_runtimes=False.",
            script_doc,
        )
    }
}
