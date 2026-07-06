//! Serving deploy flow — deploy-spec derivation, form construction, capacity
//! fit, preflight, manifest rendering, and llm-d routing docs. Split out of
//! `app.rs` (see `impl App`).

use super::*;

impl App {
    pub(super) fn selected_deploy_spec(
        &self,
    ) -> Option<(
        String,
        String,
        String,
        &'static str,
        String,
        String,
        Option<String>,
    )> {
        if let Some(a) = self.selected_artifact() {
            let model_id = Self::artifact_model_id(a);
            let repo_dir = model_id.replace('/', "--");
            let vendor = if a.engine.contains("RBLN") {
                "rbln"
            } else if a.engine.contains("Furiosa") {
                "furiosa"
            } else {
                "gpu"
            };
            let mount = if a.mount.is_empty() {
                format!("/mnt/store/compiled/{}", repo_dir)
            } else {
                a.mount
                    .split(" ← ")
                    .next()
                    .unwrap_or("/mnt/store")
                    .to_string()
            };
            let tp = Self::opt_or(a, "tp", if vendor == "furiosa" { "8" } else { "1" });
            let dev_default = if vendor == "furiosa" {
                let pe = tp.parse::<i64>().unwrap_or(8).max(1);
                ((pe as f64 / 8.0).ceil() as i64).max(1).to_string()
            } else {
                tp.clone()
            };
            return Some((
                a.model.clone(),
                model_id,
                a.engine.clone(),
                vendor,
                mount,
                dev_default,
                if vendor == "furiosa" { Some(tp) } else { None },
            ));
        }
        // Library 패널0: 스토어 컴파일본을 바로 배포 — repo/포맷/타깃(compiled_for)에서 spec 유도.
        if let Some(s) = self.selected_stored() {
            let vendor = match s.format.as_str() {
                "rbln" => "rbln",
                "furiosa" => "furiosa",
                _ => "gpu",
            };
            let engine = match vendor {
                "rbln" => "vLLM-RBLN",
                "furiosa" => "Furiosa-LLM",
                _ => "vLLM",
            };
            // compiled_for(예: RBLN-CA22-tp4-s8192) 에서 tp 추출.
            let tp = s
                .compiled_for
                .split(['-', '_', ' '])
                .find_map(|t| {
                    let t = t.to_lowercase();
                    t.strip_prefix("tp").and_then(|n| n.parse::<i64>().ok())
                })
                .unwrap_or(if vendor == "furiosa" { 8 } else { 1 });
            let dev_default = if vendor == "furiosa" {
                ((tp as f64 / 8.0).ceil() as i64).max(1).to_string()
            } else {
                tp.to_string()
            };
            let mount = if s.path.is_empty() {
                "/mnt/store".to_string()
            } else if s.path.starts_with('/') {
                s.path.clone()
            } else {
                format!("/mnt/store/{}", s.path)
            };
            let model = s.repo.rsplit('/').next().unwrap_or(&s.repo).to_string();
            return Some((
                model,
                s.repo.clone(),
                engine.to_string(),
                vendor,
                mount,
                dev_default,
                if vendor == "furiosa" {
                    Some(tp.to_string())
                } else {
                    None
                },
            ));
        }
        let m = self.selected_catalog_model()?;
        let p = self.preferred_catalog_placement(m)?;
        let model_id = Self::placement_model_id(m, p);
        let model = if m.display.is_empty() {
            m.id.clone()
        } else {
            m.display.clone()
        };
        let vendor = Self::placement_vendor(p);
        Some((
            model,
            model_id,
            Self::placement_engine(p).to_string(),
            vendor,
            Self::placement_mount(m, p),
            p.count.max(1).to_string(),
            if vendor == "furiosa" {
                Some("8".to_string())
            } else {
                None
            },
        ))
    }

    /// `[d] deploy` — 선택 모델의 배포(서빙) 옵션 편집 폼을 연다. replicas·디바이스·노드 배치.
    pub fn open_deploy_form(&mut self) {
        let Some((model, model_id, engine, vendor, mount, dev_default, serve_tp_default)) =
            self.selected_deploy_spec()
        else {
            return;
        };
        let want_kind = match vendor {
            "rbln" => crate::collect::AccelKind::Rbln,
            "furiosa" => crate::collect::AccelKind::Rngd,
            _ => crate::collect::AccelKind::Gpu,
        };
        let mut per_node: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for ac in self
            .snap
            .accel
            .iter()
            .filter(|x| x.kind == want_kind && !x.node.is_empty())
        {
            *per_node.entry(ac.node.clone()).or_insert(0) += 1;
        }
        let mut cand_nodes: Vec<String> = per_node.keys().cloned().collect();
        cand_nodes.sort();
        let mut place_choices = vec!["any".to_string(), "spread".to_string()];
        place_choices.extend(cand_nodes.iter().map(|n| format!("{}({})", n, per_node[n])));
        let mut fields = vec![CompileField {
            key: "replicas".into(),
            label: "replicas".into(),
            value: "1".into(),
            choices: ["1", "2", "3", "4", "6", "8"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            numeric: true,
            help: "Number of serving instances. Total device demand = replicas × devices.".into(),
        }];
        if vendor == "furiosa" {
            fields.push(CompileField {
                key: "tp".into(),
                label: "serve TP (PE)".into(),
                value: serve_tp_default.unwrap_or_else(|| "8".to_string()),
                choices: ["4", "8", "16"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Furiosa serving tensor parallel size in PE units. Device request stays separate.".into(),
            });
        }
        fields.extend(vec![
            CompileField {
                key: "devices".into(),
                label: "devices/replica".into(),
                value: dev_default,
                choices: ["1", "2", "4", "8"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Accelerators requested per replica (resources.limits). For Furiosa, this is RNGD count, not PE TP.".into(),
            },
            CompileField {
                key: "port".into(),
                label: "port".into(),
                value: "8000".into(),
                choices: ["8000", "8080"].iter().map(|s| s.to_string()).collect(),
                numeric: true,
                help: "Serving container port.".into(),
            },
            CompileField {
                key: "place".into(),
                label: "placement".into(),
                value: "any".into(),
                choices: place_choices,
                numeric: false,
                help: "Node placement: any=no extra constraint, spread=topology spread, hostname=pinned node.".into(),
            },
            CompileField {
                key: "routing".into(),
                label: "routing".into(),
                value: "llm-d".into(),
                choices: ["llm-d", "direct"].iter().map(|s| s.to_string()).collect(),
                numeric: false,
                help: "llm-d also creates InferencePool, EPP, and HTTPRoute; direct creates only Deployment.".into(),
            },
        ]);
        self.deploy_form = Some(DeployForm {
            model,
            model_id,
            engine,
            vendor,
            mount,
            fields,
            cursor: 0,
            editing: false,
        });
    }

    /// 배포 용량 판정 — 총 디바이스 수요 대 클러스터 동종 가속기(총/유휴).
    pub fn deploy_fit(&self, form: &DeployForm) -> DeployFit {
        let want_kind = match form.vendor {
            "rbln" => crate::collect::AccelKind::Rbln,
            "furiosa" => crate::collect::AccelKind::Rngd,
            _ => crate::collect::AccelKind::Gpu,
        };
        let devs: Vec<&crate::collect::Accel> = self
            .snap
            .accel
            .iter()
            .filter(|x| x.kind == want_kind)
            .collect();
        let total = devs.len() as i64;
        // 노드별 유휴(살아있고 미점유) — replica 는 한 노드에 per 개가 모여야 배치 가능(패킹).
        let mut free_by_node: std::collections::BTreeMap<&str, i64> =
            std::collections::BTreeMap::new();
        for d in &devs {
            if !d.node.is_empty() {
                let e = free_by_node.entry(d.node.as_str()).or_insert(0);
                if d.alive && d.busy_model.is_empty() {
                    *e += 1;
                }
            }
        }
        let free: i64 = free_by_node.values().sum();
        let nodes = free_by_node.len() as i64;
        let max_node_free = free_by_node.values().copied().max().unwrap_or(0);
        let replicas = form.get("replicas").parse::<i64>().unwrap_or(1).max(1);
        let per = form.get("devices").parse::<i64>().unwrap_or(1).max(1);
        let demand = replicas * per;
        // k8s 리소스 관점 유휴 = allocatable - requested (스케줄러가 실제로 보는 값).
        // metric busy_model 로는 유휴여도, 다른 배포가 리소스를 예약(request)했으면 스케줄 불가.
        let res_key = match form.vendor {
            "rbln" => "rebellions.ai/ATOM",
            "furiosa" => "furiosa.ai/rngd",
            _ => "nvidia.com/gpu",
        };
        let resource_free = self
            .snap
            .inventory
            .iter()
            .find(|(k, _, _)| k == res_key)
            .map(|(_, alloc, req)| (alloc - req).max(0))
            .unwrap_or(free); // inventory 없으면 metric 값으로 폴백
                              // 실제 배치 가능 replica 수 = Σ floor(node_free / per) (한 노드 안에 per 개가 모여야).
        let placeable: i64 = free_by_node.values().map(|f| f / per).sum();
        let verdict = if total == 0 {
            FitVerdict::Unknown
        } else if demand > resource_free {
            // 스케줄러 관점 리소스 부족 — metric 유휴여도 예약돼 있으면 못 뜸(우선).
            FitVerdict::Oom
        } else if per > max_node_free {
            FitVerdict::Oom // replica 하나도 어느 노드에도 안 들어감(조각난 여유)
        } else if placeable < replicas {
            FitVerdict::Tight // 총량은 되지만 노드 패킹으로 일부만 배치
        } else {
            FitVerdict::Fits
        };
        let mut tips: Vec<String> = Vec::new();
        if demand > resource_free {
            tips.push(format!(
                "리소스 예약 기준 유휴 {} < 수요 {} — 다른 배포가 {} 를 점유(request)함. 그 서빙을 stop 하거나 replicas/devices↓",
                resource_free, demand, res_key
            ));
        } else if matches!(verdict, FitVerdict::Oom) {
            tips.push(format!("replica당 {}개가 단일 노드에 안 들어감(최대 유휴 {}/노드) — devices/replica↓ 또는 서빙 정리", per, max_node_free));
        } else if matches!(verdict, FitVerdict::Tight) {
            tips.push(format!("노드 패킹상 {}/{} replica 만 배치 가능(유휴 {}, 노드별 조각) — replicas↓ 또는 노드 확보", placeable, replicas, free));
        }
        // metric 유휴와 리소스 유휴가 어긋나면 명시(오해 방지).
        if resource_free != free {
            tips.push(format!(
                "(metric 유휴 {} ≠ 리소스 유휴 {} — 예약됐지만 idle 인 디바이스 있음)",
                free, resource_free
            ));
        }
        if form.get("place") == "spread" && replicas > nodes && nodes > 0 {
            tips.push(format!(
                "⚠ spread: replicas {} > 노드 {} — 일부는 같은 노드로",
                replicas, nodes
            ));
        }
        if per > 1 && form.vendor == "rbln" {
            tips.push("replica당 다중 칩은 컴파일 TP 와 일치해야 함".into());
        }
        DeployFit {
            demand,
            total,
            free,
            resource_free,
            nodes,
            verdict,
            tips,
        }
    }

    /// 배포 사전 점검(preflight) — apply 전에 서빙 전제조건 확인(사전 방어).
    pub fn deploy_preflight(&self, form: &DeployForm) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        // 이미지 — deploy_form_submit 의 벤더별 기본값과 동일 판정(불일치로 오탐 방지).
        //   furiosa=furiosaai/furiosa-llm:latest, gpu=vllm/vllm-openai:latest 기본 존재 → OK.
        //   rbln 은 vllm_rbln 런타임이 든 이미지가 필요(기본 없음) → LMD_SERVING_IMAGE 미지정이면 차단.
        let (img_ok, img_msg) = match form.vendor {
            "furiosa" => (
                true,
                "image ready: furiosaai/furiosa-llm:latest (furiosa-llm serve)".to_string(),
            ),
            "gpu" => {
                let img = self
                    .img_serving
                    .clone()
                    .unwrap_or_else(|| "vllm/vllm-openai:latest".into());
                (true, format!("image ready: {} (vLLM serve)", img))
            }
            _ => match &self.img_serving {
                Some(img) => (true, format!("image ready: {} (vllm_rbln runtime)", img)),
                None => (
                    true,
                    "image fallback: ubuntu:22.04 with host RBLN stack on the target node".into(),
                ),
            },
        };
        out.push((img_ok, format!("1. serving image: {}", img_msg)));
        // 2. Model artifact path — serving must load an HF id or compiled store path.
        out.push((
            !form.mount.is_empty(),
            format!(
                "2. model location: {}",
                if form.mount.is_empty() {
                    "unknown path: compile first or choose a store artifact".into()
                } else {
                    form.mount.clone()
                }
            ),
        ));
        // 3. NPU vendors need a node with the corresponding driver/resource plugin.
        if form.vendor != "gpu" {
            let want = if form.vendor == "rbln" {
                "RBLN"
            } else {
                "RNGD"
            };
            let any = self
                .snap
                .nodes
                .iter()
                .any(|n| n.npu.to_uppercase().contains(want));
            out.push((
                any,
                format!(
                    "3. accelerator driver: {} node {}",
                    want,
                    if any {
                        "exists: schedulable"
                    } else {
                        "missing: pods will stay Pending"
                    }
                ),
            ));
        }
        // 4. Capacity — requested devices must fit scheduler-visible free resources.
        let fit = self.deploy_fit(form);
        let cap_ok = matches!(fit.verdict, FitVerdict::Fits);
        out.push((
            cap_ok,
            format!(
                "4. capacity: needs {} device(s), {} free -> {}{}",
                fit.demand,
                fit.resource_free,
                fit.verdict.label(),
                if cap_ok {
                    ""
                } else {
                    " (stop another serving workload or lower replicas/devices)"
                }
            ),
        ));
        out
    }

    /// 배포 폼 → Deployment 매니페스트 미리보기(dry-run). Enter 시 호출.
    pub fn deploy_form_submit(&mut self) {
        let Some(form) = self.deploy_form.take() else {
            return;
        };
        let name = form.model_id.replace(['/', '.'], "-").to_lowercase();
        let name = format!("serve-{}", name);
        let replicas = form.get("replicas");
        let devices = form.get("devices");
        let serve_tp = if form.vendor == "furiosa" {
            let tp = form.get("tp");
            if tp.is_empty() {
                "8".to_string()
            } else {
                tp
            }
        } else {
            devices.clone()
        };
        let port = {
            let p = form.get("port");
            if p.is_empty() {
                "8000".to_string()
            } else {
                p
            }
        };
        let (res_key, product_label) = match form.vendor {
            "rbln" => ("rebellions.ai/ATOM", "rebellions.ai/npu.product: RBLN-CA22"),
            "furiosa" => ("furiosa.ai/rngd", "furiosa.ai/npu.product: rngd"),
            _ => ("nvidia.com/gpu", ""),
        };
        let place = form.get("place");
        let place_host = place.split('(').next().unwrap_or("any").trim().to_string();
        // 배치 스펙 — spread=topologySpread, 특정=nodeSelector hostname, any=제약 없음(디바이스 resource 로 스케줄).
        let placement_yaml = if place_host == "spread" {
            format!(
                "\x20     topologySpreadConstraints:\n\
                 \x20       - {{ maxSkew: 1, topologyKey: kubernetes.io/hostname, whenUnsatisfiable: DoNotSchedule, labelSelector: {{ matchLabels: {{ app: {name} }} }} }}\n",
                name = name
            )
        } else if place_host != "any" && !place_host.is_empty() {
            format!(
                "\x20     nodeSelector: {{ kubernetes.io/hostname: {} }}\n",
                place_host
            )
        } else if !product_label.is_empty() {
            format!("\x20     nodeSelector: {{ {} }}\n", product_label)
        } else {
            String::new()
        };
        // Vendor-specific serving specs. NPU engines do not accept the generic vLLM `--model` form.
        let served = if form.vendor == "furiosa" && form.mount.starts_with("/mnt/store/") {
            form.mount.clone()
        } else {
            form.model_id.clone()
        };
        let (image, note, container_spec, volumes_block) = match form.vendor {
            "furiosa" => {
                let img = self
                    .img_furiosa
                    .clone()
                    .unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into());
                let store_backed = form.mount.starts_with("/mnt/store/");
                let store_mount = if store_backed {
                    "\x20           - { name: store, mountPath: /mnt/store, readOnly: true }\n"
                } else {
                    ""
                };
                let fxb_args = if store_backed {
                    format!(
                        ", \"--fxb\", \"{}/model.fxb\"",
                        form.mount.trim_end_matches('/')
                    )
                } else {
                    String::new()
                };
                let spec = format!(
                    "\x20         args: [\"serve\", \"{model}\", \"--served-model-name\", \"{served_name}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{serve_tp}\"{fxb_args}]\n\
                     \x20         ports: [{{ containerPort: {port} }}]\n\
                     \x20         env:\n\
                     \x20           - {{ name: HF_HOME, value: /model-cache }}\n\
                     \x20           - {{ name: HF_TOKEN, valueFrom: {{ secretKeyRef: {{ name: hf-token, key: HF_TOKEN }} }} }}\n\
                     \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"4\", memory: \"16Gi\", {res_key}: {devices} }} }}\n\
                     \x20         volumeMounts:\n\
                     \x20           - {{ name: cache, mountPath: /model-cache }}\n\
                     {store_mount}",
                    model = form.model_id,
                    served_name = form.model_id,
                    port = port,
                    devices = devices,
                    serve_tp = serve_tp,
                    fxb_args = fxb_args,
                    res_key = res_key,
                    store_mount = store_mount
                );
                let vols = if store_backed {
                    "\x20     volumes:\n\x20       - { name: cache, emptyDir: {} }\n\x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n".to_string()
                } else {
                    "\x20     volumes:\n\x20       - { name: cache, emptyDir: {} }\n".to_string()
                };
                (img, "# Furiosa: furiosa-llm serve. Store-backed artifacts use the HF id plus --fxb so config/tokenizer still come from HF. Serving TP is PE count; resource devices are RNGD count.".to_string(), spec, vols)
            }
            "rbln" => {
                if let Some(img) = self.img_serving.clone() {
                    let spec = format!(
                        "\x20         args: [\"serve\", \"{mount}\", \"--served-model-name\", \"{served}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{devices}\", \"--max-num-seqs\", \"1\"]\n\
                         \x20         ports: [{{ containerPort: {port} }}]\n\
                         \x20         env:\n\
                         \x20           - {{ name: VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK, value: \"{devices}\" }}\n\
                         \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"8\", memory: \"32Gi\", {res_key}: {devices} }} }}\n\
                         \x20         volumeMounts:\n\
                         \x20           - {{ name: store, mountPath: /mnt/store, readOnly: true }}\n",
                        mount = form.mount, served = served, port = port, devices = devices, res_key = res_key
                    );
                    let vols = "\x20     volumes:\n\x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n".to_string();
                    (img, "# RBLN: vllm_rbln runtime image from LMD_SERVING_IMAGE; loads the compiled artifact from model-store.".to_string(), spec, vols)
                } else {
                    let cmd = format!(
                        "set -eux\n\
                         export DEBIAN_FRONTEND=noninteractive\n\
                         apt-get update -qq\n\
                         apt-get install -y -qq --no-install-recommends python3.10 python3.10-dev python3-pip libdrm2 libnuma1 libgomp1 ca-certificates tzdata g++ libc6-dev >/dev/null\n\
                         ln -sf /usr/bin/python3.10 /usr/local/bin/python3\n\
                         python3 -m pip install -q --target=/opt/py-overrides --upgrade prometheus-fastapi-instrumentator\n\
                         export PYTHONPATH=\"/opt/py-overrides:${{PYTHONPATH}}\"\n\
                         exec python3 -m vllm.entrypoints.openai.api_server --model={mount} --served-model-name={served} --enforce-eager --max-num-seqs 1 --host=0.0.0.0 --port={port}\n",
                        mount = form.mount,
                        served = served,
                        port = port
                    );
                    let spec = format!(
                        "\x20         command: [\"bash\", \"-c\"]\n\
                         \x20         args:\n\
                         \x20           - |-\n\
                         {cmd_indented}\
                         \x20         ports: [{{ containerPort: {port} }}]\n\
                         \x20         env:\n\
                         \x20           - {{ name: PYTHONPATH, value: \"/home/gspark/.local/lib/python3.10/site-packages:/host-sys-local-pkgs:/host-sys-pkgs\" }}\n\
                         \x20           - {{ name: PYTHONUNBUFFERED, value: \"1\" }}\n\
                         \x20           - {{ name: VLLM_RBLN_NUM_DEVICES_PER_LOCAL_RANK, value: \"{devices}\" }}\n\
                         \x20           - {{ name: LD_LIBRARY_PATH, value: \"/host-rbln-lib:/host-libs\" }}\n\
                         \x20           - {{ name: PATH, value: \"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/host-rbln-bin\" }}\n\
                         \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"8\", memory: \"32Gi\", {res_key}: {devices} }} }}\n\
                         \x20         volumeMounts:\n\
                         \x20           - {{ name: store, mountPath: /mnt/store, readOnly: true }}\n\
                         \x20           - {{ name: host-local-pkgs, mountPath: /home/gspark/.local/lib/python3.10/site-packages, readOnly: true }}\n\
                         \x20           - {{ name: host-sys-local-pkgs, mountPath: /host-sys-local-pkgs, readOnly: true }}\n\
                         \x20           - {{ name: host-sys-pkgs, mountPath: /host-sys-pkgs, readOnly: true }}\n\
                         \x20           - {{ name: host-libs, mountPath: /host-libs, readOnly: true }}\n\
                         \x20           - {{ name: host-rbln-lib, mountPath: /host-rbln-lib, readOnly: true }}\n\
                         \x20           - {{ name: host-rbln-bin, mountPath: /host-rbln-bin, readOnly: true }}\n\
                         \x20           - {{ name: shm, mountPath: /dev/shm }}\n",
                        cmd_indented = cmd.lines().map(|l| format!("             {}\n", l)).collect::<String>(),
                        port = port,
                        devices = devices,
                        res_key = res_key
                    );
                    let vols = "\x20     volumes:\n\
                                \x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n\
                                \x20       - { name: host-local-pkgs, hostPath: { path: /home/gspark/.local/lib/python3.10/site-packages, type: Directory } }\n\
                                \x20       - { name: host-sys-local-pkgs, hostPath: { path: /usr/local/lib/python3.10/dist-packages, type: Directory } }\n\
                                \x20       - { name: host-sys-pkgs, hostPath: { path: /usr/lib/python3/dist-packages, type: Directory } }\n\
                                \x20       - { name: host-libs, hostPath: { path: /usr/lib/x86_64-linux-gnu, type: Directory } }\n\
                                \x20       - { name: host-rbln-lib, hostPath: { path: /usr/lib, type: Directory } }\n\
                                \x20       - { name: host-rbln-bin, hostPath: { path: /usr/bin, type: Directory } }\n\
                                \x20       - { name: shm, emptyDir: { medium: Memory, sizeLimit: 16Gi } }\n"
                        .to_string();
                    ("ubuntu:22.04".to_string(), "# RBLN: using host RBLN stack fallback on the target node; loads the compiled artifact from model-store.".to_string(), spec, vols)
                }
            }
            _ => {
                let img = self
                    .img_serving
                    .clone()
                    .unwrap_or_else(|| "vllm/vllm-openai:latest".into());
                let spec = format!(
                    "\x20         args: [\"serve\", \"{mount}\", \"--served-model-name\", \"{served}\", \"--port\", \"{port}\", \"--tensor-parallel-size\", \"{devices}\"]\n\
                     \x20         ports: [{{ containerPort: {port} }}]\n\
                     \x20         resources: {{ limits: {{ {res_key}: {devices} }}, requests: {{ cpu: \"4\", memory: \"16Gi\", {res_key}: {devices} }} }}\n\
                     \x20         volumeMounts:\n\
                     \x20           - {{ name: store, mountPath: /mnt/store, readOnly: true }}\n",
                    mount = form.mount, served = served, port = port, devices = devices, res_key = res_key
                );
                let vols = "\x20     volumes:\n\x20       - { name: store, persistentVolumeClaim: { claimName: model-store } }\n".to_string();
                (img, "# GPU: vLLM loads the model/store path directly; no NPU compile step required.".to_string(), spec, vols)
            }
        };
        let yaml = format!(
            "# Deployment manifest preview. Review, then apply with `kubectl apply -f -`.\n\
             # Serving model {model_id}. Engine: {engine}.\n\
             # Placement: {place}. Total device demand = {replicas} x {devices}.\n\
             # If the image contains a TODO- placeholder, set LMD_SERVING_IMAGE before applying.\n\
             {note}\n\
             apiVersion: apps/v1\n\
             kind: Deployment\n\
             metadata: {{ name: {name}, namespace: {ns} }}\n\
             spec:\n\
             \x20 replicas: {replicas}\n\
             \x20 selector: {{ matchLabels: {{ app: {name} }} }}\n\
             \x20 template:\n\
             \x20   metadata: {{ labels: {{ app: {name} }} }}\n\
             \x20   spec:\n\
             {placement}\
             {volumes_block}\
             \x20     containers:\n\
             \x20       - name: server\n\
             \x20         image: {image}   # {engine}\n\
             {container_spec}",
            model_id = form.model_id,
            engine = form.engine,
            note = note,
            place = place,
            name = name,
            ns = self.ns,
            replicas = replicas,
            devices = devices,
            placement = placement_yaml,
            volumes_block = volumes_block,
            container_spec = container_spec,
            image = image,
        );
        // routing=llm-d → 게이트웨이 라우팅 리소스(InferencePool+EPP+HTTPRoute)를 뒤에 동봉.
        let yaml = if form.get("routing") == "llm-d" {
            format!("{}{}", yaml, self.routing_docs(&name, form.vendor, &served))
        } else {
            yaml
        };
        // 자동화: YAML 을 덤프하지 않고 바로 apply 확인 팝업. (YAML 은 팝업에서 e=vi 편집·v=검증)
        self.confirm = Some(Pending::Apply {
            title: format!("deploy {} ×{}", form.model, replicas),
            yaml,
        });
        self.confirm_yes = false;
    }

    /// llm-d 게이트웨이 라우팅 리소스 문서들(서빙 Deployment 뒤에 붙임).
    /// 실기 배선 그대로: SA, RoleBinding×2(공유 Role llmd-router-epp-sa/-non-sa 참조),
    /// plugins ConfigMap, EPP Deployment/Service, InferencePool(app={name} 선택), HTTPRoute.
    /// 경로 = /accel/model. EPP 는 pool 멤버 파드로 model/부하 인지 라우팅.
    pub(super) fn routing_docs(&self, name: &str, vendor: &str, served: &str) -> String {
        let accel = match vendor {
            "furiosa" => "rngd",
            "rbln" => "atom",
            _ => "gpu",
        };
        let slug = served
            .rsplit('/')
            .next()
            .unwrap_or(served)
            .to_lowercase()
            .replace(['.', '_'], "-");
        let path = format!("/{}/{}", accel, slug);
        let ns = &self.ns;
        format!(
            "---\n\
             # ── llm-d 라우팅: 게이트웨이 {path} → InferencePool({name}-pool) → EPP → 이 서빙 ──\n\
             # (공유 Role llmd-router-epp-sa/-non-sa 는 클러스터에 이미 존재한다고 가정)\n\
             apiVersion: v1\n\
             kind: ServiceAccount\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             ---\n\
             apiVersion: rbac.authorization.k8s.io/v1\n\
             kind: RoleBinding\n\
             metadata: {{ name: {name}-epp-sa, namespace: {ns} }}\n\
             roleRef: {{ apiGroup: rbac.authorization.k8s.io, kind: Role, name: llmd-router-epp-sa }}\n\
             subjects:\n\
             \x20 - {{ kind: ServiceAccount, name: {name}-epp, namespace: {ns} }}\n\
             ---\n\
             apiVersion: rbac.authorization.k8s.io/v1\n\
             kind: RoleBinding\n\
             metadata: {{ name: {name}-epp-non-sa, namespace: {ns} }}\n\
             roleRef: {{ apiGroup: rbac.authorization.k8s.io, kind: Role, name: llmd-router-epp-non-sa }}\n\
             subjects:\n\
             \x20 - {{ kind: ServiceAccount, name: {name}-epp, namespace: {ns} }}\n\
             ---\n\
             apiVersion: v1\n\
             kind: ConfigMap\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             data:\n\
             \x20 default-plugins.yaml: |\n\
             \x20\x20\x20 apiVersion: inference.networking.x-k8s.io/v1alpha1\n\
             \x20\x20\x20 kind: EndpointPickerConfig\n\
             \x20\x20\x20 plugins:\n\
             \x20\x20\x20 - type: queue-scorer\n\
             \x20\x20\x20 - type: kv-cache-utilization-scorer\n\
             \x20\x20\x20 - type: prefix-cache-scorer\n\
             \x20\x20\x20 schedulingProfiles:\n\
             \x20\x20\x20 - name: default\n\
             \x20\x20\x20\x20\x20 plugins:\n\
             \x20\x20\x20\x20\x20 - {{ pluginRef: queue-scorer, weight: 2 }}\n\
             \x20\x20\x20\x20\x20 - {{ pluginRef: kv-cache-utilization-scorer, weight: 2 }}\n\
             \x20\x20\x20\x20\x20 - {{ pluginRef: prefix-cache-scorer, weight: 3 }}\n\
             ---\n\
             apiVersion: apps/v1\n\
             kind: Deployment\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             spec:\n\
             \x20 replicas: 1\n\
             \x20 selector: {{ matchLabels: {{ app: {name}-epp }} }}\n\
             \x20 template:\n\
             \x20   metadata: {{ labels: {{ app: {name}-epp }} }}\n\
             \x20   spec:\n\
             \x20     serviceAccountName: {name}-epp\n\
             \x20     containers:\n\
             \x20       - name: epp\n\
             \x20         image: ghcr.io/llm-d/llm-d-router-endpoint-picker-dev:main\n\
             \x20         args: [\"--pool-name\", \"{name}-pool\", \"--pool-namespace\", \"{ns}\", \"--pool-group\", \"inference.networking.k8s.io\", \"--config-file\", \"/config/default-plugins.yaml\", \"--zap-encoder\", \"json\", \"--tracing=false\"]\n\
             \x20         ports:\n\
             \x20           - {{ name: grpc, containerPort: 9002 }}\n\
             \x20           - {{ name: grpc-health, containerPort: 9003 }}\n\
             \x20           - {{ name: metrics, containerPort: 9090 }}\n\
             \x20         env:\n\
             \x20           - {{ name: NAMESPACE, valueFrom: {{ fieldRef: {{ fieldPath: metadata.namespace }} }} }}\n\
             \x20           - {{ name: POD_NAME, valueFrom: {{ fieldRef: {{ fieldPath: metadata.name }} }} }}\n\
             \x20         volumeMounts:\n\
             \x20           - {{ name: plugins, mountPath: /config }}\n\
             \x20     volumes:\n\
             \x20       - {{ name: plugins, configMap: {{ name: {name}-epp }} }}\n\
             ---\n\
             apiVersion: v1\n\
             kind: Service\n\
             metadata: {{ name: {name}-epp, namespace: {ns} }}\n\
             spec:\n\
             \x20 selector: {{ app: {name}-epp }}\n\
             \x20 ports:\n\
             \x20   - {{ name: grpc-ext-proc, port: 9002, targetPort: 9002 }}\n\
             \x20   - {{ name: http-metrics, port: 9090, targetPort: 9090 }}\n\
             ---\n\
             apiVersion: inference.networking.k8s.io/v1\n\
             kind: InferencePool\n\
             metadata: {{ name: {name}-pool, namespace: {ns} }}\n\
             spec:\n\
             \x20 selector: {{ matchLabels: {{ app: {name} }} }}\n\
             \x20 targetPorts:\n\
             \x20   - {{ number: 8000 }}\n\
             \x20 endpointPickerRef: {{ group: \"\", kind: Service, name: {name}-epp, port: {{ number: 9002 }}, failureMode: FailClose }}\n\
             ---\n\
             apiVersion: gateway.networking.k8s.io/v1\n\
             kind: HTTPRoute\n\
             metadata: {{ name: {name}-route, namespace: {ns} }}\n\
             spec:\n\
             \x20 parentRefs:\n\
             \x20   - {{ name: llm-d-gateway }}\n\
             \x20 rules:\n\
             \x20   - matches:\n\
             \x20       - {{ path: {{ type: PathPrefix, value: {path} }} }}\n\
             \x20     filters:\n\
             \x20       - {{ type: URLRewrite, urlRewrite: {{ path: {{ type: ReplacePrefixMatch, replacePrefixMatch: /v1 }} }} }}\n\
             \x20     backendRefs:\n\
             \x20       - {{ group: inference.networking.k8s.io, kind: InferencePool, name: {name}-pool }}\n",
            name = name, ns = ns, path = path
        )
    }
}
