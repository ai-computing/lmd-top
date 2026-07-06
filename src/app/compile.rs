//! NPU compile flow — form construction, fit estimation, preflight checks, and
//! headless compile/deploy planning. Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    /// `[c] compile` — 선택 빌드의 NPU 컴파일 폼. 엔진이 NPU 면 그 벤더로,
    /// GPU/HF 라도 [[npu-compat]] 지원 목록에 있으면 해당 벤더로 컴파일 가능(GPU→NPU 경로).
    pub fn compile_preview(&mut self) {
        let owned;
        let a = if let Some(a) = self.selected_artifact() {
            a
        } else if let Some(cat) = self.selected_catalog_artifact() {
            owned = cat;
            &owned
        } else if let Some(z) = self.selected_zoo_artifact() {
            owned = z;
            &owned
        } else {
            return;
        };
        let model_id = Self::artifact_model_id(a);
        let vendor: Option<&'static str> = if a.engine.contains("RBLN") {
            Some("rbln")
        } else if a.engine.contains("Furiosa") {
            Some("furiosa")
        } else {
            crate::compat::compilable_vendors(&model_id)
                .first()
                .copied()
        };
        match vendor {
            Some(v) => {
                let form = self.build_compile_form(a, v);
                self.compile_form = Some(form);
            }
            None => {
                self.preview = Some((
                    format!("compile · {}", a.model),
                    format!(
                        "# {}\n# This model family is not in the NPU compile support list (RBLN/Furiosa).\n# Supported families: Llama, Qwen2/3, Gemma, Mistral, EXAONE, Phi, OPT, GPT2, SOLAR, DeepSeek, T5, ...\n# Source list: src/npu-compat.json (based on vendor documentation)\n",
                        model_id
                    ),
                ));
                self.preview_scroll = 0;
                self.preview_apply = false;
            }
        }
    }

    /// 특정 벤더로 컴파일 폼 열기(액션 메뉴의 'Compile → RBLN/Furiosa'용).
    pub fn compile_form_for(&mut self, vendor: &'static str) {
        let owned;
        let a = if let Some(a) = self.selected_artifact() {
            a
        } else if let Some(cat) = self.selected_catalog_artifact() {
            owned = cat;
            &owned
        } else if let Some(z) = self.selected_zoo_artifact() {
            owned = z;
            &owned
        } else {
            return;
        };
        let form = self.build_compile_form(a, vendor);
        self.compile_form = Some(form);
    }

    /// 선택 아티팩트를 주어진 벤더로 컴파일하는 옵션 폼 구성(순수). 초기값은 관측 opts.
    pub(super) fn build_compile_form(
        &self,
        a: &crate::collect::ModelArtifact,
        vendor: &'static str,
    ) -> CompileForm {
        let rbln = vendor == "rbln";
        let model_id = Self::artifact_model_id(a);
        let mkf =
            |key: &str, label: &str, def: &str, choices: &[&str], numeric: bool, help: &str| {
                CompileField {
                    key: key.into(),
                    label: label.into(),
                    value: Self::opt_or(a, key, def),
                    choices: choices.iter().map(|s| s.to_string()).collect(),
                    numeric,
                    help: help.into(),
                }
            };
        // Parameters are compile-time fixed: RBLN uses optimum-rbln config, Furiosa uses furiosa-llm build.
        // Deeper search/autotuning belongs in npu_aitune; this UI emits one compile Job.
        let mut fields = if rbln {
            // RBLNDecoderOnlyModelForCausalLMConfig. RBLN-CA22 has up to 4 chips, so TP<=4.
            vec![
                mkf("tp", "tensor-parallel", "4", &["1", "2", "4"], true, "Tensor parallel size = number of RBLN chips (rbln_tensor_parallel_size). CA22 max is 4."),
                mkf("max-len", "max-seq-len", "8192", &["2048", "4096", "8192", "16384", "32768"], true, "Compile-time maximum context length (rbln_max_seq_len). Larger values use more memory and compile time."),
                mkf("batch", "batch-size", "1", &["1", "2", "4", "8", "16"], true, "Static batch size (rbln_batch_size). RBLN fixes this at compile time."),
                mkf("attn", "attn-impl", "flash_attn", &["flash_attn", "eager"], false, "Attention implementation: flash_attn for SRAM optimized path, eager for PagedAttention."),
                mkf("kvpart", "kvcache-partition", "8192", &["4096", "8192", "16384", "32768"], true, "flash_attn only: KV tokens per SRAM partition. Must be a power of two AND divide max-seq-len (e.g. max-len 8192 → 4096/8192)."),
                mkf("quant", "quantization", "none", &["none", "w8a8", "w4a16"], false, "Weight/activation quantization format (RBLNQuantizationConfig); model support varies."),
                mkf("npu", "npu-chip", "RBLN-CA22", &["RBLN-CA22"], false, "Target RBLN chip (rbln_npu), detected from the cluster."),
            ]
        } else {
            // furiosa-llm ArtifactBuilder. One RNGD exposes full=8 / half=4 PE layouts.
            vec![
                mkf("tp", "tensor-parallel", "8", &["4", "8"], true, "Tensor parallel size in PE units. RNGD full=8, half=4."),
                mkf("pp", "pipeline-parallel", "1", &["1", "2"], true, "Number of pipeline-parallel stages (ParallelConfig)."),
                mkf("max-len", "max-seq-len", "8192", &["2048", "4096", "8192", "16384"], true, "max_seq_len_to_capture; longer buckets are excluded."),
                mkf("batch", "batch-size", "1", &["1", "2", "4", "8"], true, "Prefill/decode bucket batch size (BucketConfig)."),
                mkf("chunk", "prefill-chunk", "none", &["none", "512", "1024", "2048"], true, "Chunked prefill chunk size (prefill_chunk_size)."),
                mkf("block", "kv-block-size", "16", &["16", "32"], true, "Tokens per PagedAttention block (paged_attention_block_size)."),
                mkf("quant", "activation-dq", "none", &["none", "on"], false, "use_activation_dq: dynamic activation quantization to reduce memory and improve throughput."),
            ]
        };
        // Infrastructure placement: build device/node candidates from the live snapshot.
        let want_kind = if rbln {
            crate::collect::AccelKind::Rbln
        } else {
            crate::collect::AccelKind::Rngd
        };
        // Same-kind device count per node.
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
        let max_dev = per_node.values().copied().max().unwrap_or(4).max(1);
        // Default devices needed for compile: RBLN=TP, Furiosa=ceil(TP/8)*PP.
        let tp_v = fields
            .iter()
            .find(|f| f.key == "tp")
            .and_then(|f| f.value.parse::<i64>().ok())
            .unwrap_or(1)
            .max(1);
        let pp_v = fields
            .iter()
            .find(|f| f.key == "pp")
            .and_then(|f| f.value.parse::<i64>().ok())
            .unwrap_or(1)
            .max(1);
        let dev_default = if rbln {
            tp_v
        } else {
            ((tp_v as f64 / 8.0).ceil() as i64).max(1) * pp_v
        };
        let dev_choices: Vec<String> = (1..=max_dev.max(dev_default))
            .map(|i| i.to_string())
            .collect();
        fields.push(CompileField {
            key: "devices".into(),
            label: "devices".into(),
            value: dev_default.to_string(),
            choices: dev_choices,
            numeric: true,
            help: "Requested accelerator device count (resources.limits). Usually TP for Rebellions or ceil(TP/8)×PP for Furiosa.".into(),
        });
        // Add driver/SDK summaries to node choices.
        let node_drv = |n: &str| -> String {
            self.snap
                .nodes
                .iter()
                .find(|x| x.name == n)
                .map(|x| x.npu.clone())
                .filter(|s| !s.is_empty())
                .map(|s| format!(" {}", s))
                .unwrap_or_default()
        };
        let mut node_choices = vec!["any".to_string()];
        node_choices.extend(
            cand_nodes
                .iter()
                .map(|n| format!("{}({}){}", n, per_node[n], node_drv(n))),
        );
        fields.push(CompileField {
            key: "node".into(),
            label: "target-node".into(),
            value: "any".into(),
            choices: node_choices,
            numeric: false,
            help: "Compile execution node. any matches the product label; parentheses show device count and driver summary.".into(),
        });
        CompileForm {
            model: a.model.clone(),
            model_id,
            vendor,
            engine: a.engine.clone(),
            fields,
            cursor: 0,
            editing: false,
        }
    }

    /// RBLN(optimum-rbln) 파라미터 조합의 컴파일 실행가능성 사전 검증.
    /// 알려진 제약을 인코딩해 "될지 예측"한다. 위반 시 (설명+권장 수정값) 메시지, 아니면 None.
    ///  - flash_attn: max_seq_len 은 kvcache_partition_len 의 배수여야 하고 kvpart 는 2의 거듭제곱.
    ///    (실기 ValueError: "max_seq_len must be a multiple of kvcache_partition_len when using flash_attn")
    ///  - tensor-parallel 은 RBLN-CA22 에서 1/2/4.
    pub(super) fn rbln_param_issue(form: &CompileForm) -> Option<String> {
        let getn = |k: &str| form.get(k).parse::<i64>().ok();
        let tp = getn("tp").unwrap_or(1);
        if !matches!(tp, 1 | 2 | 4) {
            return Some(format!(
                "tensor-parallel {} — RBLN-CA22 supports 1/2/4",
                tp
            ));
        }
        if form.get("attn") == "flash_attn" {
            let max_len = getn("max-len").unwrap_or(0);
            let kvpart = getn("kvpart").unwrap_or(0);
            if kvpart <= 0 || (kvpart & (kvpart - 1)) != 0 {
                return Some(format!(
                    "kvcache-partition {} must be a power of two for flash_attn",
                    kvpart
                ));
            }
            if max_len < kvpart || max_len % kvpart != 0 {
                // 표준 후보 중 max_len 을 나누는 가장 큰 값을 권장.
                let fix = [32768i64, 16384, 8192, 4096]
                    .into_iter()
                    .find(|&k| k <= max_len && max_len % k == 0);
                return Some(match fix {
                    Some(k) => format!(
                        "flash_attn needs max-seq-len ({}) to be a multiple of kvcache-partition ({}) → set kvcache-partition to {}",
                        max_len, kvpart, k
                    ),
                    None => format!(
                        "flash_attn needs max-seq-len (≥4096) divisible by kvcache-partition; {} is too small → switch attn-impl to eager",
                        max_len
                    ),
                });
            }
        }
        None
    }

    /// 컴파일 폼 → 매니페스트 미리보기(dry-run) 생성. Enter 시 호출. 폼 값을 env·OUTPUT 에 반영.
    pub fn compile_form_submit(&mut self) {
        let Some(form) = self.compile_form.take() else {
            return;
        };
        let model_id = &form.model_id;
        let vendor = form.vendor;
        let target = form.target();
        // 사전 차단: 컴파일은 from_pretrained(MODEL_ID) 로 소스 가중치를 받는다. MODEL_ID 가
        // 유효한 HF repo id(org/name)도 로컬 경로(/…)도 아니면 HF 에서 404 → 25분짜리 Job 이
        // 무의미하게 죽는다(실기 원인). Job 을 띄우기 전에 명확히 안내하고 중단.
        if !model_id.contains('/') {
            self.preview = Some((
                format!("compile · {} — source unresolved", form.model),
                format!(
                    "# Cannot compile: MODEL_ID '{id}' is not a valid Hugging Face repo id (expected org/name)\n\
                     # and is not a local path. RBLN/Furiosa compile downloads the source weights via\n\
                     # from_pretrained(MODEL_ID), so it would 404 on https://huggingface.co/{id}.\n\
                     #\n\
                     # Fix: give this model a canonical HF source. In your catalog (catalog/models.yaml or LMD_CATALOG):\n\
                     #   - id: {id}\n\
                     #     source: <org>/<name>        # e.g. meta-llama/Llama-3.1-8B-Instruct\n\
                     # or add an `hf://<org>/<name>` placement. Alternatively pre-place the weights in the store.\n",
                    id = model_id
                ),
            ));
            self.preview_scroll = 0;
            self.preview_apply = false;
            return;
        }
        // 사전 차단(예측): RBLN(optimum-rbln) 파라미터 조합이 컴파일 시 죽는 걸 미리 잡는다.
        // 실기 원인: flash_attn 인데 max-seq-len(8192)이 kvcache-partition(16384)의 배수가 아님 →
        // ValueError 로 15분 후 실패. 폼을 열어둔 채 정확한 수정값을 안내하고 중단.
        if vendor == "rbln" {
            if let Some(issue) = Self::rbln_param_issue(&form) {
                self.notify(format!("compile blocked — {}", issue));
                self.compile_form = Some(form); // 폼 유지: 값만 고쳐 다시 Enter
                return;
            }
        }
        let repo_dir = model_id.replace('/', "--");
        // Job 이름 = 모델 + 타깃(vendor·chip·tp·pp·seq). 같은 모델을 다른 옵션으로 컴파일하면
        // 서로 다른 Job 이 되어 공존 가능(스토어 산출물 경로와 동일한 정체성). DNS-1123(≤63자) 보장.
        let name = compile_job_name(&repo_dir, &target);
        let tp = form.get("tp");
        // 디바이스 수·노드는 폼에서 선택한 값. 노드 라벨의 "(N)" 접미는 제거.
        let devices = {
            let d = form.get("devices");
            if d.is_empty() {
                tp.clone()
            } else {
                d
            }
        };
        let node_pick = form.get("node");
        let node_host = node_pick
            .split('(')
            .next()
            .unwrap_or("any")
            .trim()
            .to_string();
        // 컴파일은 AOT — 가속기 디바이스 예약 불필요(서빙이 칩을 다 써도 컴파일 가능).
        //   Furiosa: 공개 이미지(furiosaai/furiosa-llm:latest)에 toolchain 내장 → 아무 노드/CPU.
        //   RBLN   : 레지스트리 이미지(LMD_COMPILE_IMAGE_RBLN)가 있으면 그걸로, 없으면
        //            rebel-compiler 가 깔린 노드의 호스트 스택을 hostPath 로 사용(그 노드에 고정).
        let rbln_host_stack = vendor == "rbln" && self.img_rbln.is_none();
        let auto_rbln_node = self
            .snap
            .nodes
            .iter()
            .find(|n| n.npu.to_uppercase().contains("RBLN"))
            .map(|n| n.name.clone());
        let image = if vendor == "rbln" {
            if rbln_host_stack {
                "ubuntu:22.04".to_string()
            } else {
                self.img_rbln.clone().unwrap()
            }
        } else {
            self.img_furiosa
                .clone()
                .unwrap_or_else(|| "furiosaai/furiosa-llm:latest".into())
        };
        // 노드: 특정 선택이 최우선. any 면 — RBLN 호스트스택은 그 노드에 자동 고정(hostPath),
        // 그 외(furiosa AOT·RBLN 이미지)는 제약 없음(아무 노드/CPU 에서 실행).
        let node_label = if node_host != "any" && !node_host.is_empty() {
            format!("kubernetes.io/hostname: {}", node_host)
        } else if rbln_host_stack {
            match &auto_rbln_node {
                Some(n) => format!("kubernetes.io/hostname: {}", n),
                None => "rebellions.ai/npu.product: RBLN-CA22".to_string(),
            }
        } else {
            String::new() // AOT: 노드 제약 없음
        };
        // 컴파일 Job 리소스 — 디바이스 예약 없이 cpu/mem 만(AOT).
        let resources_line =
            "resources: { requests: { cpu: \"8\", memory: \"16Gi\" } }".to_string();
        let _ = devices;
        // 폼 값을 스크립트가 읽는 env 로 — 벤더별 파라미터 이름 대응.
        let mut envs: Vec<(String, String)> = vec![
            ("MODEL_STORE".into(), "/mnt/store".into()),
            ("MODEL_ID".into(), model_id.clone()),
            (
                "OUTPUT".into(),
                format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target),
            ),
            // HF 캐시를 공유 스토어에 둠 — 이미 받은 가중치 재사용(오프라인·재다운로드 방지),
            // 컴파일 간/노드 간 공유(PLAYBOOK: 가중치는 PVC 에 두는 게 안정). RBLN 경로에 반영.
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
        let opts_summary: String = form
            .fields
            .iter()
            .map(|f| format!("{}={}", f.key, f.value))
            .collect::<Vec<_>>()
            .join("  ");
        let outdir = format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target);
        // 벤더별 컨테이너: Furiosa 는 이미지에 든 `fxb build` 를 직접 호출(스크립트 불필요, 바로 실행 가능).
        // RBLN 은 optimum-rbln 커스텀 스크립트(compile-script ConfigMap)를 env 로 구동.
        // 벤더별 컨테이너: (volumes_extra, env_block, mounts_extra, command, note, extra_doc).
        // extra_doc = Job 앞에 붙는 별도 YAML 문서(RBLN 은 인라인 컴파일 스크립트 ConfigMap).
        let (volumes_extra, env_block, mounts_extra, command, note, extra_doc) = if vendor
            == "furiosa"
        {
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
            // 실기 검증 반영:
            //  - RNGD 는 ARM64 제어 프로세서라 EDF 최종 코드젠에 aarch64 크로스컴파일러 필요(gcc-aarch64-linux-gnu).
            //    furiosa-llm serve 이미지엔 없어 apt 로 설치(또는 build-complete 이미지 사용).
            //  - SMB 스토어는 컴파일 작업 I/O(mmap 등) 미지원(os error 95) → 로컬 emptyDir 에 빌드 후 스토어로 복사.
            //  - fxb 는 .fxb 아카이브 → {target}/model.fxb 로 디렉터리 안에 두어 discovery 레이아웃 유지.
            let cmd = format!(
                "set -e; apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq gcc-aarch64-linux-gnu build-essential >/dev/null 2>&1; \
                 mkdir -p /work/out; fxb build {model_id} /work/out/model -tp {tp} -pp {pp} --max-model-len {ml} --concurrency 8; \
                 mkdir -p {outdir}; cp -r /work/out/. {outdir}/; echo COMPILE_DONE; ls -la {outdir}",
                outdir = outdir, model_id = model_id, tp = tp, pp = pp, ml = ml
            );
            (
                "        - { name: work, emptyDir: {} }\n".to_string(),
                "            - { name: HF_HOME, value: /mnt/store/hub }\n            - { name: HF_TOKEN, valueFrom: { secretKeyRef: { name: hf-token, key: HF_TOKEN, optional: true } } }\n".to_string(),
                "            - { name: work, mountPath: /work }\n".to_string(),
                format!("[\"sh\", \"-c\", \"{}\"]", cmd),
                "# Furiosa: run fxb build directly for furiosa-ai quantized checkpoints. Installs aarch64 cross-compiler, builds locally, then copies to model-store.",
                String::new(),
            )
        } else {
            // RBLN: optimum-rbln 인라인 스크립트를 ConfigMap 으로 동봉(외부 의존 없음).
            //  - rbln_create_runtimes=False: 디바이스에 런타임을 올리지 않고 컴파일만 → 서빙이 칩을
            //    점유 중이어도 성공(실기 확인: create_runtimes=True 면 "Device 0 is not a valid NPU device").
            //  - SMB 스토어 I/O(os error 95) 회피: 로컬 /work 에 save 후 스토어로 복사.
            let env_lines: String = envs
                .iter()
                .map(|(k, v)| format!("            - {{ name: {}, value: \"{}\" }}\n", k, v))
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
                name = name, ns = self.ns
            );
            let cm_vol = format!("        - {{ name: script, configMap: {{ name: {}-script }} }}\n        - {{ name: work, emptyDir: {{}} }}\n", name);
            if rbln_host_stack {
                // 레지스트리 이미지 없음 → rebel-compiler 가 깔린 노드의 호스트 스택을 hostPath 로 사용.
                // ubuntu 베이스에 python3.10 설치 + 호스트 site-packages/libs 마운트(서빙과 동일 패턴).
                // sympy 등은 /usr/local dist-packages 에 있어 PYTHONPATH 에 포함.
                let host_vols = format!(
                    "{cm}\
                     \x20       - {{ name: hp-local, hostPath: {{ path: /home/gspark/.local/lib/python3.10/site-packages, type: Directory }} }}\n\
                     \x20       - {{ name: hp-sys, hostPath: {{ path: /usr/local/lib/python3.10/dist-packages, type: Directory }} }}\n\
                     \x20       - {{ name: hp-lib, hostPath: {{ path: /usr/lib, type: Directory }} }}\n\
                     \x20       - {{ name: hp-bin, hostPath: {{ path: /usr/bin, type: Directory }} }}\n",
                    cm = cm_vol
                );
                let host_mounts = "            - { name: script, mountPath: /scripts, readOnly: true }\n            - { name: work, mountPath: /work }\n            - { name: hp-local, mountPath: /home/gspark/.local/lib/python3.10/site-packages }\n            - { name: hp-sys, mountPath: /host-sys }\n            - { name: hp-lib, mountPath: /host-lib }\n            - { name: hp-bin, mountPath: /host-bin }\n".to_string();
                // markupsafe 등 apt 설치 파이썬 패키지는 /usr/lib/python3/dist-packages(=/host-lib/python3/dist-packages)
                // 에 있음 — jinja2(→transformers)가 markupsafe 를 못 찾던 실기 에러 해소. /usr/lib 는 이미 hp-lib 로 마운트됨.
                let host_env = "            - { name: PYTHONPATH, value: \"/home/gspark/.local/lib/python3.10/site-packages:/host-sys:/host-lib/python3/dist-packages\" }\n            - { name: LD_LIBRARY_PATH, value: \"/host-lib:/host-lib/x86_64-linux-gnu\" }\n            - { name: PATH, value: \"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/host-bin\" }\n";
                // tzdata: pandas→pytz 가 /usr/share/zoneinfo 를 요구(host site-packages 가 transformers→sklearn→pandas 를 끌어옴).
                // 최소 ubuntu 이미지엔 없어 apt 로 설치(없으면 FileNotFoundError: tzdata.zi).
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
            ns = self.ns,
            node_label = node_label,
            image = image,
            resources_line = resources_line,
            volumes_extra = volumes_extra,
            env_block = env_block,
            mounts_extra = mounts_extra,
            command = command,
        );
        // 자동화: YAML 덤프 대신 바로 apply 확인 팝업. (YAML 은 팝업에서 e=vi 편집·v=검증)
        self.confirm = Some(Pending::Apply {
            title: format!("compile {} → {}", form.model, target),
            yaml,
        });
        self.confirm_yes = false;
    }

    pub(super) fn apply_field_overrides(fields: &mut [CompileField], overrides: &[(String, String)]) {
        for (key, val) in overrides {
            if key == "mount" {
                continue;
            }
            if let Some(f) = fields.iter_mut().find(|f| f.key == *key || f.label == *key) {
                f.value = val.clone();
            }
        }
    }

    pub(super) fn override_value(overrides: &[(String, String)], key: &str) -> Option<String> {
        overrides
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    pub(super) fn take_apply_plan(&mut self) -> Result<(String, String), String> {
        match self.confirm.take() {
            Some(Pending::Apply { title, yaml }) => {
                self.confirm_yes = false;
                Ok((title, yaml))
            }
            _ => Err("operation did not produce an apply manifest".into()),
        }
    }

    pub(super) fn synthetic_artifact_for(
        model_id: &str,
        vendor: &'static str,
        mount: String,
        overrides: &[(String, String)],
    ) -> crate::collect::ModelArtifact {
        let model_name = model_id.rsplit('/').next().unwrap_or(model_id).to_string();
        let engine = match vendor {
            "rbln" => "vLLM-RBLN",
            "furiosa" => "Furiosa-LLM",
            _ => "vLLM",
        };
        let tp_default = match vendor {
            "rbln" => "4",
            "furiosa" => "8",
            _ => "1",
        };
        let mut opts = vec![(
            "tp".into(),
            Self::override_value(overrides, "tp").unwrap_or_else(|| tp_default.to_string()),
        )];
        if let Some(v) = Self::override_value(overrides, "max-len") {
            opts.push(("max-len".into(), v));
        }
        crate::collect::ModelArtifact {
            model: model_name.clone(),
            family: model_name.to_lowercase(),
            engine: engine.into(),
            node: String::new(),
            image: String::new(),
            source: model_id.into(),
            mount,
            opts,
        }
    }

    pub fn plan_compile_for_model(
        &mut self,
        model_id: &str,
        vendor: &'static str,
        overrides: &[(String, String)],
    ) -> Result<(String, String), String> {
        let a = Self::synthetic_artifact_for(model_id, vendor, String::new(), overrides);
        let mut form = self.build_compile_form(&a, vendor);
        Self::apply_field_overrides(&mut form.fields, overrides);
        self.compile_form = Some(form);
        self.compile_form_submit();
        self.take_apply_plan()
    }

    pub fn plan_deploy_for_model(
        &mut self,
        model_id: &str,
        vendor: &'static str,
        overrides: &[(String, String)],
    ) -> Result<(String, String), String> {
        let repo_dir = model_id.replace('/', "--");
        let default_mount = match vendor {
            "rbln" => {
                let target = {
                    let a =
                        Self::synthetic_artifact_for(model_id, vendor, String::new(), overrides);
                    let mut form = self.build_compile_form(&a, vendor);
                    Self::apply_field_overrides(&mut form.fields, overrides);
                    form.target()
                };
                format!("/mnt/store/compiled/{}/{}/{}", repo_dir, vendor, target)
            }
            "furiosa" => {
                Self::override_value(overrides, "mount").unwrap_or_else(|| model_id.to_string())
            }
            _ => model_id.to_string(),
        };
        let mount = Self::override_value(overrides, "mount").unwrap_or(default_mount);
        let old_view = self.view;
        let old_focus = self.panel_focus;
        let old_selected = self.selected;
        let old_len = self.snap.artifacts.len();
        let a = Self::synthetic_artifact_for(model_id, vendor, mount, overrides);
        self.snap.artifacts.push(a);
        self.view = View::Serving;
        self.panel_focus = 0;
        self.selected = old_len;
        self.open_deploy_form();
        self.snap.artifacts.truncate(old_len);
        self.view = old_view;
        self.panel_focus = old_focus;
        self.selected = old_selected;
        let Some(form) = self.deploy_form.as_mut() else {
            return Err("failed to create deploy form".into());
        };
        Self::apply_field_overrides(&mut form.fields, overrides);
        self.deploy_form_submit();
        self.take_apply_plan()
    }

    /// 모델 이름에서 파라미터 수(B) 추정 — "8B", "1.5b", "0.5B", "32b" 등 첫 매치.
    pub(super) fn est_params_b(name: &str) -> Option<f64> {
        let lower = name.to_lowercase();
        let bytes = lower.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                // 숫자 뒤 바로 'b' 이고, 앞이 알파벳이 아니어야(예: fp"8b" 방지 위해 직전 문자 체크)
                if i < bytes.len() && (bytes[i] == b'b') {
                    let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphabetic();
                    if before_ok {
                        if let Ok(v) = lower[start..i].parse::<f64>() {
                            if (0.1..=2000.0).contains(&v) {
                                return Some(v);
                            }
                        }
                    }
                }
            } else {
                i += 1;
            }
        }
        None
    }

    /// 선택 인프라(NPU 메모리) 대비 컴파일 옵션 적합성 추정 + 조정 제안. 대략치.
    pub fn compile_fit(&self, form: &CompileForm) -> FitEstimate {
        let rbln = form.vendor == "rbln";
        let params_b =
            Self::est_params_b(&form.model_id).or_else(|| Self::est_params_b(&form.model));
        let tp = form.get("tp").parse::<f64>().unwrap_or(1.0).max(1.0);
        let pp = form.get("pp").parse::<f64>().unwrap_or(1.0).max(1.0);
        let seq = form
            .get("max-len")
            .parse::<f64>()
            .unwrap_or(8192.0)
            .max(1.0);
        let batch = form.get("batch").parse::<f64>().unwrap_or(1.0).max(1.0);
        // dtype 바이트/파라미터 — 양자화·모델명 반영.
        let q = form.get("quant").to_lowercase();
        let name_l = form.model_id.to_lowercase();
        let dtype_bytes = if rbln {
            if q.contains("w4") {
                0.5
            } else if q.contains("w8") {
                1.0
            } else {
                2.0
            }
        } else if name_l.contains("fp8") || name_l.contains("-w8") {
            1.0
        } else if name_l.contains("int4") || name_l.contains("awq") || name_l.contains("gptq") {
            0.5
        } else {
            2.0
        };
        // 칩당 가용 메모리 — 스냅샷의 동종 디바이스 mem_total 평균, 없으면 표준값.
        let want_kind = if rbln {
            crate::collect::AccelKind::Rbln
        } else {
            crate::collect::AccelKind::Rngd
        };
        let mems: Vec<f64> = self
            .snap
            .accel
            .iter()
            .filter(|a| a.kind == want_kind && a.mem_total_gb > 0.0)
            .map(|a| a.mem_total_gb)
            .collect();
        let avail_gb = if mems.is_empty() {
            if rbln {
                15.7
            } else {
                48.0
            }
        } else {
            mems.iter().sum::<f64>() / mems.len() as f64
        };
        // 메모리 분산 칩 수: RBLN=TP, Furiosa≈ceil(tp/8)*pp.
        let chips = if rbln {
            tp
        } else {
            (tp / 8.0).ceil().max(1.0) * pp
        };
        let weight_gb = params_b.map(|p| p * dtype_bytes).unwrap_or(0.0);
        // KV: Llama-8B bf16 ≈ 0.25 MB/token 기준 스케일. (KV 양자화는 미반영 — 보수적)
        let kv_per_tok_mb = 0.25 * (params_b.unwrap_or(8.0) / 8.0);
        let kv_gb = batch * seq * kv_per_tok_mb / 1024.0;
        let overhead_gb = 2.0;
        let per_chip_gb = (weight_gb + kv_gb) / chips + overhead_gb;
        let ratio = per_chip_gb / avail_gb;
        let verdict = if params_b.is_none() {
            FitVerdict::Unknown
        } else if ratio > 1.0 {
            FitVerdict::Oom
        } else if ratio > 0.85 {
            FitVerdict::Tight
        } else {
            FitVerdict::Fits
        };
        // 조정 제안.
        let mut tips: Vec<String> = Vec::new();
        let max_chips = if rbln { 4.0 } else { 8.0 };
        if matches!(verdict, FitVerdict::Oom | FitVerdict::Tight) {
            if tp < max_chips {
                tips.push(format!(
                    "TP↑ {}→{} (칩 추가로 칩당 부담↓)",
                    tp as i64,
                    (tp * 2.0).min(max_chips) as i64
                ));
            }
            if seq > 2048.0 {
                tips.push(format!(
                    "max-seq-len↓ {}→{} (KV {:.1}GiB↓)",
                    seq as i64,
                    (seq / 2.0) as i64,
                    kv_gb / 2.0
                ));
            }
            if batch > 1.0 {
                tips.push(format!(
                    "batch↓ {}→{} (KV 절반)",
                    batch as i64,
                    (batch / 2.0) as i64
                ));
            }
            if dtype_bytes >= 2.0 {
                tips.push(if rbln {
                    "양자화 w4a16/w8a8 로 가중치↓".into()
                } else {
                    "FP8 체크포인트 사용 시 가중치 절반".into()
                });
            }
        } else if matches!(verdict, FitVerdict::Fits) && ratio < 0.4 {
            if batch < 8.0 {
                tips.push(format!(
                    "여유 있음 — batch↑ {}→{} 로 처리량 확보 여지",
                    batch as i64,
                    (batch * 2.0) as i64
                ));
            }
            if tp > 1.0 && rbln {
                tips.push(format!(
                    "TP↓ {}→{} 로 칩 절약(가능 시)",
                    tp as i64,
                    (tp / 2.0) as i64
                ));
            }
        }
        // 요청 디바이스 수가 물리 분산 칩 수보다 적으면 컴파일 불가.
        if let Ok(dev) = form.get("devices").parse::<f64>() {
            if dev > 0.0 && dev < chips {
                tips.push(format!(
                    "⚠ devices {} < 필요 {} 칩 — 이 TP/PP 로는 컴파일 불가",
                    dev as i64, chips as i64
                ));
            }
        }
        // RBLN kvcache_partition_len 은 2의 거듭제곱이어야.
        if rbln {
            if let Ok(k) = form.get("kvpart").parse::<u64>() {
                if k == 0 || (k & (k - 1)) != 0 {
                    tips.push(format!("⚠ kvcache-partition {} 는 2의 거듭제곱이 아님", k));
                }
            }
        }
        // 선택 노드가 이 NPU 드라이버를 갖고 있는지 — 컴파일은 드라이버 설치 노드에서만 가능.
        let node_host = form.get("node");
        let node_host = node_host.split('(').next().unwrap_or("any").trim();
        if node_host != "any" && !node_host.is_empty() {
            if let Some(nd) = self.snap.nodes.iter().find(|n| n.name == node_host) {
                let want = if rbln { "RBLN" } else { "RNGD" };
                if !nd.npu.to_uppercase().contains(want) {
                    tips.push(format!(
                        "⚠ 노드 {} 에 {} 드라이버 없음(accel: {}) — 컴파일 실패",
                        node_host,
                        want,
                        if nd.npu.is_empty() { "none" } else { &nd.npu }
                    ));
                }
            }
        }
        FitEstimate {
            params_b,
            weight_gb,
            kv_gb,
            overhead_gb,
            chips,
            per_chip_gb,
            avail_gb,
            verdict,
            tips,
        }
    }

    /// 컴파일 사전 점검(preflight) — 긴 컴파일 전에 전제조건 충족 여부를 미리 검사.
    /// 실기에서 발견한 함정(레지스트리 미등록·aarch64 툴체인·노드 드라이버·컴파일러 이미지)을 사전 방어.
    /// 반환: (충족?, 메시지). 하나라도 false 면 컴파일 실패 가능 → 폼에서 경고 표시.
    /// 이 폼의 컴파일 산출물이 이미 스토어에 있는지 — 있으면 그 상대경로. 재컴파일=덮어씀.
    /// 매칭: 컴파일 출력경로 `compiled/{repo_dir}/{vendor}/{target}` 와 인벤토리 path 정확 일치.
    pub fn compile_already_stored(&self, form: &CompileForm) -> Option<String> {
        let repo_dir = form.model_id.replace('/', "--");
        let expected = format!("compiled/{}/{}/{}", repo_dir, form.vendor, form.target());
        self.snap
            .stored
            .iter()
            .map(|s| s.path.trim_end_matches('/'))
            .find(|p| *p == expected)
            .map(|p| p.to_string())
    }

    pub fn compile_preflight(&self, form: &CompileForm) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        let mid = form.model_id.to_lowercase();
        // 이미 같은 옵션으로 컴파일된 산출물이 스토어에 있으면 안내(블로커 아님 — 재컴파일 시 덮어씀).
        if let Some(path) = self.compile_already_stored(form) {
            out.push((
                true,
                format!(
                    "⚠ 이미 컴파일됨 — {} (재컴파일 시 덮어씀; 불필요하면 취소하고 Deploy)",
                    path
                ),
            ));
        }
        if form.vendor == "furiosa" {
            // fxb build 는 furiosa-ai 조직의 양자화 체크포인트만 컴파일(실기 확인).
            let quant = ["fp8", "nvfp4", "-w8", "-w4", "awq", "gptq", "int4", "int8"]
                .iter()
                .any(|q| mid.contains(q));
            let org = mid.starts_with("furiosa-ai/");
            out.push((
                org && quant,
                if org && quant {
                    "registry: furiosa-ai 양자화 체크포인트 — fxb 등록 대상".into()
                } else {
                    format!("registry: fxb 는 furiosa-ai 양자화 모델만 빌드(예: furiosa-ai/Qwen3-4B-FP8) — '{}' 미등록 가능성", form.model_id)
                },
            ));
            // RNGD 는 ARM64 제어 프로세서 → EDF 최종 코드젠에 aarch64 크로스컴파일러 필요(매니페스트가 자동 설치).
            out.push((
                true,
                "toolchain: aarch64 크로스컴파일러 매니페스트가 자동 설치".into(),
            ));
            out.push((
                true,
                "build I/O: 로컬 emptyDir 빌드→스토어 복사(SMB os error 95 회피)".into(),
            ));
            // RBLN 은 create_runtimes=False 로 디바이스 없이 컴파일(서빙 중에도 가능).
            out.push((true, "compile-only: rbln_create_runtimes=False — 디바이스 점유 없이 컴파일(서빙 중에도 OK)".into()));
            let compat_ok = crate::compat::compilable_vendors(&form.model_id).contains(&"rbln");
            out.push((
                compat_ok,
                format!(
                    "registry: RBLN 지원 계열 {}",
                    if compat_ok {
                        "확인됨(npu-compat)"
                    } else {
                        "미확인"
                    }
                ),
            ));
        }
        // 컴파일 실행 노드 — AOT 라 가속기 드라이버는 불필요. 툴체인이 있는 노드만 있으면 됨.
        //   Furiosa: 이미지(furiosaai/furiosa-llm:latest, 공개)에 toolchain 포함 → 아무 노드(CPU 포함)에서 실행.
        //   RBLN   : 레지스트리 이미지(LMD_COMPILE_IMAGE_RBLN) 또는 rebel-compiler 가 깔린 노드의 호스트 스택(hostPath).
        if form.vendor == "furiosa" {
            out.push((true, "compile node: AOT — 가속기 불필요, 아무 노드(CPU 포함)에서 실행. 이미지에 toolchain 내장".into()));
        } else {
            let host_node = self
                .snap
                .nodes
                .iter()
                .find(|n| n.npu.to_uppercase().contains("RBLN"))
                .map(|n| n.name.clone());
            match (self.img_rbln.is_some(), host_node) {
                (true, _) => out.push((true, "compile node: LMD_COMPILE_IMAGE_RBLN 이미지로 실행(아무 노드)".into())),
                (false, Some(n)) => out.push((true, format!("compile node: {} 의 rebel-compiler 호스트 스택 사용(hostPath) — 레지스트리 이미지 불필요", n))),
                (false, None) => out.push((
                    false,
                    "compile node: rebel-compiler 이미지도, 그게 깔린 노드도 없음 — LMD_COMPILE_IMAGE_RBLN 지정 또는 RBLN 노드 필요".into(),
                )),
            }
        }
        out
    }
}

#[cfg(test)]
mod compile_tests {
    use super::*;
    use crate::catalog::{CatModel, CatPlacement};

    fn placement(engine: &str, resource: &str, uri: &str) -> CatPlacement {
        CatPlacement {
            engine: engine.into(),
            accel: String::new(),
            resource: resource.into(),
            count: 4,
            replicas: 1,
            uri: uri.into(),
            requires_artifact: uri.starts_with("pvc://"),
        }
    }
    fn model(id: &str, source: &str, placements: Vec<CatPlacement>) -> CatModel {
        CatModel {
            id: id.into(),
            display: String::new(),
            role: "chat".into(),
            source: source.into(),
            placements,
        }
    }

    // 실기 버그: RBLN placement(pvc://) 를 골라도 컴파일 소스는 형제 hf:// 가중치 id 여야 한다.
    #[test]
    fn hf_source_prefers_sibling_hf_placement_over_pvc() {
        let m = model(
            "llama3.1-8b",
            "",
            vec![
                placement("vllm", "nvidia.com/gpu", "hf://meta-llama/Llama-3.1-8B-Instruct"),
                placement("vllm-rbln", "rebellions.ai/ATOM", "pvc://rbln-artifacts/llama31-8b-tp4"),
            ],
        );
        assert_eq!(
            App::catalog_hf_source(&m),
            "meta-llama/Llama-3.1-8B-Instruct"
        );
        // 그리고 그 아티팩트의 컴파일 model_id 도 유효한 HF id 여야 함(예전엔 family "llama3.1-8b" 로 폴백해 404).
        let rbln_p = &m.placements[1];
        let art = App::catalog_artifact(&m, rbln_p);
        assert_eq!(App::artifact_model_id(&art), "meta-llama/Llama-3.1-8B-Instruct");
    }

    #[test]
    fn hf_source_explicit_field_wins() {
        let m = model(
            "qwen3-embedding-8b",
            "Qwen/Qwen3-Embedding-8B",
            vec![placement("furiosa", "furiosa.ai/rngd", "pvc://furiosa-artifacts/qwen3-embed")],
        );
        assert_eq!(App::catalog_hf_source(&m), "Qwen/Qwen3-Embedding-8B");
    }

    #[test]
    fn hf_source_falls_back_to_id_when_unknown() {
        // hf:// placement 도 source 도 없으면 id 로 폴백 → compile 가드가 이후 걸러낸다.
        let m = model(
            "koni-llama3.1-8b",
            "",
            vec![placement("vllm-rbln", "rebellions.ai/ATOM", "pvc://rbln-artifacts/koni-tp4")],
        );
        assert_eq!(App::catalog_hf_source(&m), "koni-llama3.1-8b");
    }

    // 사전 차단: 유효한 HF id(org/name)가 아니면 Job 매니페스트를 만들지 않는다.
    #[test]
    fn compile_blocks_invalid_hf_id() {
        let mut a = App::new();
        let r = a.plan_compile_for_model("llama3.1-8b", "rbln", &[]);
        assert!(r.is_err(), "bare name must not produce a compile manifest");
        assert!(a.preview.is_some(), "should surface a guidance preview");
    }

    #[test]
    fn compile_allows_valid_hf_id() {
        let mut a = App::new();
        let r = a.plan_compile_for_model("meta-llama/Llama-3.1-8B-Instruct", "rbln", &[]);
        let (_, yaml) = r.expect("valid HF id should produce a manifest");
        assert!(yaml.contains("meta-llama/Llama-3.1-8B-Instruct"));
        assert!(yaml.contains("/mnt/store/hub"), "HF cache on shared store");
    }

    fn ov(kvs: &[(&str, &str)]) -> Vec<(String, String)> {
        kvs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    // 실기 에러 재현·차단: flash_attn 인데 max-seq-len(8192)이 kvcache-partition(16384)의 배수가 아님.
    #[test]
    fn rbln_blocks_flash_attn_kvpart_mismatch() {
        let mut a = App::new();
        let r = a.plan_compile_for_model(
            "meta-llama/Llama-3.1-8B-Instruct",
            "rbln",
            &ov(&[("max-len", "8192"), ("kvpart", "16384"), ("attn", "flash_attn")]),
        );
        assert!(r.is_err(), "8192 not a multiple of 16384 must be blocked before the Job");
    }

    #[test]
    fn rbln_allows_valid_flash_attn_combo() {
        let mut a = App::new();
        let r = a.plan_compile_for_model(
            "meta-llama/Llama-3.1-8B-Instruct",
            "rbln",
            &ov(&[("max-len", "16384"), ("kvpart", "8192"), ("attn", "flash_attn")]),
        );
        assert!(r.is_ok(), "16384 % 8192 == 0 should compile");
    }

    #[test]
    fn rbln_eager_skips_kvpart_divisibility() {
        let mut a = App::new();
        let r = a.plan_compile_for_model(
            "meta-llama/Llama-3.1-8B-Instruct",
            "rbln",
            &ov(&[("max-len", "2048"), ("attn", "eager")]),
        );
        assert!(r.is_ok(), "eager path doesn't require kvpart divisibility");
    }

    #[test]
    fn default_rbln_form_is_valid() {
        // 기본값(attn=flash_attn, max-len 8192, kvpart 8192)은 곧바로 유효해야 한다.
        assert!(App::rbln_param_issue(&App::new().build_compile_form(
            &App::synthetic_artifact_for("meta-llama/Llama-3.1-8B-Instruct", "rbln", String::new(), &[]),
            "rbln",
        ))
        .is_none());
    }
}
