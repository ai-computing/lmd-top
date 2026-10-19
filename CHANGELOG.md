# Changelog

[Semantic Versioning](https://semver.org). 0.x = 실험적(인터페이스 변경 가능).

## [Unreleased]

### ai-rm protocol v0.1 (read-only, 2026-10-19)
- Infra ▸ Devices shows an ai-rm panel when the cluster has any ai-rm source: `AcceleratorProfile` (`airm.ai-computing.io/v1alpha1`) vendor·model·memory·peak with its source (the selected device's profile is marked), `airm_*` Prometheus series with rm-core v0.1 names and units (vendor-derived `tok_s`/`req_s`/`power_w`/`accelerators` shown as `[vendor]` when no airm series exists), and the decisions in ConfigMap `airm-placement` (`LMD_AIRM_NS`, default `llm-d`). The device PWR column prefers `airm_power_w{scope="card"}`. Without the CRD, series and ConfigMap the view is unchanged and nothing is warned. Write actions are unchanged.

### Failing containers explain themselves (2026-10-19)
- Containers that are crash-looping, failing to pull, OOMKilled, or restarted in the last 10 minutes are found cluster-wide (from the pod list the inventory already reads) and shown with the line that explains them: the waiting message, the termination message, or — for a few pods per cycle, cached per restart — `logs --previous`. Fatal/panic lines win over later noise; logfmt `msg=… error=…` is reduced to its message.
- Known causes get a fix: image without a build for the node's architecture (`exec format error`), inotify instances exhausted, disk full, config apiVersion or CLI flag rejected by the image (noting a floating tag), the node unable to reach the API server, OOMKilled, missing tag / denied pull. Failures on a NotReady node blame the node and sort last.
- Floating image tags (`:latest`, `:main`, untagged …) running as more than one digest are reported as image drift.
- Generated EPP Deployments use the endpoint-picker image pinned by digest (the build the cluster runs and the emitted `llm-d.ai/v1alpha1` config was verified against) instead of the moving `:main` tag.
- Shown in the Status band (actionable failures only), a Pods view panel covering all namespaces, alerts, `--snapshot` (`[failing containers]`) and `--json` (`failing_containers`, `image_drift`). Regression tests replay the 2026-10-14 EPP, inotify and arm64-operator incidents and the 2026-10-19 cilium/NotReady case.

### Furiosa SDK 2026.4 readiness (2026-10-16)
- The default furiosa-llm image for compile, serve and training Jobs is pinned to `furiosaai/furiosa-llm:2026.3.0` (`accel::furiosa::DEFAULT_IMAGE`) instead of `:latest`. 2026.4 removed `serve --tensor-parallel-size` and legacy artifacts, so a floating tag would break existing manifests on their next restart.
- Furiosa serve manifests pass `--tensor-parallel-size` only to images before 2026.4; for 2026.4+ (and unversioned tags, which will become 2026.4) TP comes from the FXB artifact. Measured on one RNGD with Qwen3-4B-FP8: 2026.4 serves +27–29% tok/s over 2026.3 (106→137 c1, 1146→1459 c16), so moving to it is a matter of setting `LMD_COMPILE_IMAGE_FURIOSA` once a 2026.4 image is published.

### Accelerator coverage: unknown is not zero (2026-10-16)
- A Prometheus outage no longer reads as an empty or idle fleet. Overview shows `accel unknown` with the cause, aggregates render as `–` instead of `util 0% temp 0°C ⚡0W`, the Status band and `--snapshot` say the state is unknown, not idle.
- Devices a node declares (feature-discovery `*.npu.count`/`nvidia.com/gpu.count` labels, plus allocatable) are compared with exporter samples. A shortfall is shown in the accelerator panel, the Status band, alerts, `--snapshot` and `--json` (`accelerator_coverage.gaps`) with its likely cause: node NotReady, node pressure (evicted vendor pods), device plugin not advertising, or exporter not reporting. Families whose metrics source is itself failing are reported as unknown, not as missing devices.
- Collector errors that share one cause are collapsed (`prometheus unreachable … ×51 queries (first: …)`) instead of being listed per query.
- Accelerator packs gain an optional `count_label` (also accepted in runtime pack YAML).
- Regression tests replay the 2026-10-14 incidents: Prometheus down, and RBLN exporter/device plugin evicted under DiskPressure while RNGD keeps reporting.

### Operation correctness (2026-09-28)
- Compile history keys include namespace/name/Job UID; recreated Jobs are recorded separately and log Pods are matched by owner UID.
- HTTPRoute edits preserve sibling matches, filters and backend weights, resolve Service ports, reject ambiguous edits, and use resourceVersion conflicts instead of forced ownership changes.
- Endpoint exclusion derives label changes from live Service/InferencePool selectors, verifies Service EndpointSlice convergence, records original labels and attempts rollback on verification failure. EPP cache convergence and active-stream completion remain explicitly unverified.
- kubectl execution shares bounded stdin/stdout/stderr handling with child termination/reaping; server dry-run runs outside the UI loop. Real manifest application validates before replacement, refuses to replace nonterminal Jobs and reports deletion failures.
- Compile form options contribute to artifact fingerprints. Completed builds publish through hidden staging directories and same-filesystem rename; existing builds are never merged/overwritten. Old store paths remain usable by explicit selection/mount.
- Added regression coverage for recreated Job identities, route edits, selector-based exclusion, subprocess timeouts, build fingerprints and publication failures. Real-cluster/NPU validation is still required before deployment.

### Routing policy development
- `heterogeneous-capacity` EPP filter/scorer plus independent complete-load extractor, pinned router adapter, configuration-load tests and offline decision replay.
- `--policy init/validate/render/plan/test/replay/build`: bundled developer workspace, schema validation, existing-config merge and immutable ConfigMap rollout planning.
- TUI `:Routing policy`: load policy, edit image/profile/weight/freshness/capacity, asynchronously preview, then existing admin confirmation/audit applies the plan. Old config retained for rollout undo.
- `:EPP capabilities`, version-scoped feature guide and source-derived inventory of 72 upstream registered plugin types (includes aliases/test types).
- Plugin CI checks Rust-generated config against the pinned EPP loader and builds the Go binary. Live routing rollout/benchmark not performed.

### Setup and maintenance
- Guided `--init` writes namespace/Prometheus/Grafana settings without overwriting existing configuration. `LMD_CONFIG` selects a separate file; malformed config now produces a warning.
- Read-only `--check-setup` exposes the TUI prerequisite checks to the shell (exit 1 on required missing checks); `--version` identifies the binary.
- Installer fixes Bash invocation and argument parsing, honors source-build destination, requires SHA-256 verification, and stages replacement in the destination directory.
- PR CI runs formatting, strict Clippy, Rust tests and offline installer regression tests. Existing Rust formatting normalized to establish the formatting gate.
- Release builds validate tag/version identity, use locked dependencies, test all four platforms, and publish only after all builds succeed. Intel macOS runner moved to macos-15-intel.
- Added quick-start/update/recovery guide and a prioritized heterogeneous GPU/NPU analysis proposal (planned, not implemented).

## [0.36.0]
### Added — Track A 마무리(drain·rollback) + Track B 착수(EPP decision debugger)
- **endpoint drain** (killer #5): Pods▸Enter▸Drain. 파드를 라우팅 selector(`app`/`llm-d.ai/model`)에서 relabel(`<key>=<val>-drained`)로 이탈 → **신규 요청 차단·in-flight stream 유지**, ReplicaSet 이 대체본 생성. 되돌리려면 라벨 원복, 스트림 종료 후 Pods▸Delete. 실기 검증: 대체본 생성·drained 파드 Running 유지·Service 엔드포인트에서 제거 확인.
- **rollback**: Serving/Overview▸Enter▸Rollback = `kubectl rollout undo`(직전 리비전). 기존 Restart 와 짝. 실기 검증(이미지 rev2→undo→rev1).
- **EPP decision debugger**: EPP 뷰 하단이 per-endpoint 표 — `pick%`·`queue`(관측값) + **`score`**(아래 플러그인이 노출하는 실제 composite score, 없으면 `–`) + "왜 이 pick" 추론 힌트. scorer weight `+/-` what-if 는 기존 동작.
- **`contrib/epp-score-observer`**: 커스텀 llm-d EPP picker 플러그인(Go). 라우터의 실제 per-endpoint composite score 를 `epp_endpoint_score{pod}` prometheus 메트릭으로 export 후 stock max-score-picker 로 위임(선택 동작 불변). 포크 없이 thin `main.go` 가 플러그인 등록 후 표준 runner 실행. lmd-top 이 PromQL 로 조인해 score 열 자동 충전. 빌드/배포 가이드 포함(Go 박스 필요).
- 두 액션 모두 Admin 게이트 + confirm + audit. agent JSON(`--json`)에 `rollback:*` 액션 추가.
### Fixed — occupancy 스케줄가능 유휴 정확화
- `deploy_fit` 의 노드별 패킹(`max_node_free`/`placeable`)이 metric `busy_model` 기반이라 **예약(request)됐지만 idle 인 디바이스를 유휴로 오산**하던 것 수정 → 노드별 유휴 = 노드 디바이스수 − `node_alloc`(k8s requests). metric idle 은 `metric_free` 로 분리 표시(다르면 `idle N ≠ 스케줄가능 M`). 실기 검증: npu-node-1 RNGD allocatable 4·requested 1(idle 서빙 예약)·metric-busy 0 → 유휴 3 판정(4 아님).
### Docs
- ROADMAP: Track A(M1/M2/M3) ✅ 완료 표기, 다음 앵커=Track B. traffic/policy weight 는 단일 백엔드 InferencePool 특성상 EPP scorer what-if 로 귀속.
- 테스트 108개(occupancy 노드별·drain_label_key 우선순위·EPP endpoint 조인/힌트·action verb/mode).

## [0.35.0]
### Added — 벤더 모델 zoo(browse → prefetch/compile → deploy)
- **Deploy▸Zoo 뷰**: Furiosa/Rebellions 공개 HF 모델(76+) 브라우징. `⏎` → Prefetch(가중치 사전 다운로드)·Compile→벤더. 컴파일 가능 벤더는 `npu-compat.json` 에서 자동 판정. 벤더 zoo 자동 수집(curl) + 정렬 + 상황별 STATUS(built/compiling/prefetch/gated/available).
- **Prefetch 폼**: 대상 PVC(RWX)·캐시 경로·리비전 선택 → `snapshot_download` Job. **실 다운로드 % 표시**(downloaded/total bytes)·속도·live phase 를 Activity 패널에.
- **PVC 검증**: 관측된 PVC 만 후보로 제시·없는 PVC 는 Job 생성 차단(영구 Pending/FailedScheduling 회귀 방지).
### Fixed
- compile: 잘못된 RBLN 파라미터 조합 예측·차단 + rbln-model-zoo fetch 정정. deploy: 생성 EPP 를 llm-d 표준(GIE)에 정렬(실 스코어링 활성).

## [0.34.0]
### Added — 관리(k9s 식) + SLO 어드바이저 + NPU 컴파일 가능성
- **Enter 액션 메뉴**: Deploy 변형/노드/카탈로그·Models·Pods 에서 Enter → 컨텍스트 액션(Info/Compile→RBLN·Furiosa/Deploy/Stop/Scale/Restart/Logs/YAML/Delete/Cordon/Objective). 단축키를 몰라도 발견 가능.
- **옵션 그리드 폼**: compile/deploy 옵션을 모든 후보 인라인 표시(선택 강조), `e` 커스텀 입력. compile 은 fit(OOM/tight) 추정, deploy 는 노드 패킹 기반 용량 판정.
- **디바이스 점유**: Deploy targets 에 노드별 `[●●●○]` LED(모델색). Topology 맵(Canvas): Gateway→EPP→Pool + 서빙 노드 흐름선 + util pressure.
- **NPU 컴파일 가능성**: `src/npu-compat.json`(벤더 공식 목록) 으로 HF/GPU 모델도 RBLN/Furiosa 컴파일 가능 여부 판별 → GPU→NPU 경로 노출.
- **서빙 목표(SLO)**: 모델별 TTFT/TPOT/E2E/tok·s 목표 → Perf 뷰가 충족/위반 + 관측 병목 기반 조정 제안(값싼 런타임 노브).
- **리소스 YAML(`y`)**·매니페스트 저장(`w`)·apply(`a`, admin)·검증(`v`).
### Changed — 뷰 통합 + CLI
- Accel·Perf·Nodes·Topology → **Nodes 허브**(슬롯 1, `w` 로 전환). 탭 10→8. 리스트 뷰에 Σ 집계.
- `--help`/`-h`·잘못된 플래그 처리(exit 2 + help).
### Fixed — 하드닝
- 매니페스트 namespace 를 LMD_NS 에서 주입(하드코딩 제거). 이미지 env(`LMD_COMPILE_IMAGE_*`/`LMD_SERVING_IMAGE`) → apply 도달 가능.
- kubectl `--request-timeout`(UI 프리즈 방지), 사이드카 탐지, prom HTTP/1.1+chunked+재시도, `dechunk` 패닉 안전, `prom_ok` 프로브.
### Internal
- 모듈 분리: `src/ops.rs`(폼/액션 타입), `src/ui/overlays.rs`, `collect_kube` leaf 파서 추출. 테스트 24개.

## [0.26.1]
### Changed — 데모 GIF 재촬영(느린 전환 + 새 뷰)
- cast 전환 속도 완화(DT 0.10→0.13)로 각 뷰를 더 느긋하게. 스토리보드에 Nodes(disk)·Deploy 추가.

## [0.26.0]
### Changed — Launch → Deploy 라이프사이클 뷰(모델 관점 통합)
- 탭 7 을 **Deploy** 로: (1) 컴파일 변형 트리(모델 계열→build: 상태·엔진·컴파일 옵션·`@node /path`), (2) **배치 타깃**(노드별 여유 가속기 = 어디 배포 가능한지), (3) 카탈로그(배포 가능 모델 × 재고 가능성). `⏎` 로 아티팩트 상세(family·source·저장 노드/경로·전체 컴파일 옵션).
- Models 는 serving 전용으로 되돌림(artifacts 는 Deploy 로 통합) → 뷰 10개 유지.
- 컴파일/배포 실행(`[c]/[d]`)은 Phase 2(게이팅) 예정으로 명시.

## [0.25.0]
### Changed — Store 뷰를 Models 관점 토글로 통합(뷰 10개로 복귀)
- 별도 Store 탭 제거 → Models 뷰에 **관점 토글 `v`** 추가: **serving**(런타임: ready/run/wait/kv/tps) ⇄ **artifacts**(모델 정체성 중심: HF/local 소스·format(quant/dtype)·컴파일 옵션·저장 노드).
- 번호키 0-9 로 정리(11번째 뷰 없앰). `⏎` 는 관점에 따라 모델 상세 / 아티팩트 상세.
- artifacts 관점은 HF 여부 태그·포맷을 앞세워 "어떤 모델인지"를 강조.

## [0.24.1]
### Changed — 넘치는 뷰 오버플로 처리(무언의 잘림 제거)
- Flow 트리에 선택 route 로의 세로 스크롤 추가(길어도 잘리지 않음).
- Overview '가속기(종류/노드)' 패널이 넘치면 마지막 줄 `+N more` 로 표시.
- (기존) 테이블은 스크롤바, Nodes/Store 트리는 선택 스크롤, Perf 디바이스 패널은 가변높이+`+N more` — 모든 뷰가 넘침을 다룸.

## [0.24.0]
### Changed — Store 뷰를 모델 계열 트리 + 저장 노드 + NPU 컴파일 옵션으로
- **트리**: 모델 계열(source/이름 정규화)로 그룹 → 그 아래 변형(deploy)들을 나열. 같은 모델의 여러 배치(다른 TP/양자화/HW)를 한눈에.
- **저장 노드**: 각 변형이 어느 노드에 저장/구동되는지(`@node`) 표시.
- **NPU 컴파일 옵션 확장**: 공통(TP/PP/DP·max-len·batch·dtype·quant·kv-dtype·block) + RBLN(`RBLN_*`, `--rbln*`, 타깃 NPU) + Furiosa(`--bucket-config`/prefill·decode bucket, `--devices`, `-tp`/`-pp`/`-dp`) 를 컨테이너 args/env 에서 추출. `⏎` 로 전체.

## [0.23.0]
### Added — Store 뷰(모델 저장 위치 + 컴파일/서빙 옵션)
- 새 탭 **Store**(Tab 접근) — deploy 컨테이너 spec 에서 모델 소스(HF id/경로), 저장 위치(mountPath ← PVC/host/emptyDir), 컴파일/서빙 옵션(TP·max-len·batch·dtype·quant·NPU 등) 추출·표시. `⏎` 로 전체 옵션 상세.

## [0.22.0]
### Added — 노드 디스크 사용량 모니터링
- 루트 파일시스템(`mountpoint="/"`) 사용량을 `node_filesystem_*` 로 수집(NodeInfo.disk_used/total).
- 표시: Nodes 행(`disk NN%`), Node 상세 게이지 + 요약 타임라인(cpu/mem/disk 3-up), Overview 클러스터 라인 집계(`disk NN%`).
- 디스크 90% 초과 시 알림(`node X disk NN% full`). 히스토리 키 `nod:{node}:disk`.

## [0.21.1]
### Added — Events 상세(Enter)
- 이벤트 표에서 잘리던 **전체 메시지**를 Enter 로 열어 읽음: type/count/reason/object + 전체 message(줄바꿈). 깊이 갭 보완.

## [0.21.0]
### Added — 모델 상세 pivot 미리보기 + 데모 asciicast 생성
- **pivot 미리보기**: 모델 상세에서 pivot(p/i/r/e)이 이제 키 힌트뿐 아니라 **대상의 핵심 정보를 인라인 preview**로 — pods(수/running/이름), infra(디바이스·util), route, epp 경유 여부. 키 누르면 이동.
- **`--cast [out.cast]`**: 합성으로 값이 움직이는 프레임을 asciicast v2 로 출력(외부 녹화 도구 불필요). soft 테마·실측 구조 기반. `agg out.cast out.gif` 로 GIF 변환.
- Model 상세 텍스트 패널 높이를 내용에 맞춰 가변화.

## [0.20.0]
### Changed — 기본 테마를 soft(Catppuccin)로 승격
- 거친 ANSI-16 대신 **soft(Catppuccin Mocha)를 기본 테마**로 — "기본 화면이 이쁘게". ANSI-16 원하면 `LMD_THEME=default` 또는 실행 중 `t` 로 즉시 복귀(무손실).

## [0.19.1]
### Added — 시작 테마 지정(LMD_THEME)
- `LMD_THEME`(env) 또는 yaml `theme` 로 시작 테마 영구 지정 — 이름(soft/colorblind/high-contrast/default) 또는 번호(0~3). 예: `LMD_THEME=soft lmd-top`. soft 테마를 매번 `t` 안 누르고 기본으로 쓸 수 있음(기본값은 여전히 default라 무회귀).

## [0.19.0]
### Added — soft(Catppuccin Mocha) 테마
- 거친 ANSI-16(순수 Red/Yellow) 대신 검증된 저채도 팔레트(Catppuccin Mocha)를 **테마 3 'soft'** 로 추가 — 눈편함·미감. 기본 테마(0)는 그대로라 무회귀. `t` 로 순환(default→high-contrast→colorblind→soft). truecolor 터미널 권장.
- severity/chrome/prefill·decode/vendor(kind) 색 전반에 soft 팔레트 매핑.

## [0.18.5]
### Changed — Perf 디바이스 패널 가변 높이 + 무언의 잘림 방지
- 상단 디바이스 패널 높이를 대수에 맞춰 가변(6~18줄): 소형 클러스터는 컴팩트, 대형은 상한.
- 상한 초과 시 마지막 줄을 `… +N more (see Accel tab)` 로 명시 — 말없이 잘리던 문제 해소.

## [0.18.4]
### Added — 애니메이션 런타임 토글(f)
- `f` 키로 애니메이션 On/Off 즉시 전환(끄면 진행 중 이펙트도 제거). 느린 SSH·취향에 맞춰 사용자가 직접 강도(존재 여부)를 캘리브레이션. 도움말(?)에 추가.

## [0.18.3]
### Changed — EPP 패널 제목 길이
- 좌측 scorers 패널 제목이 좁은 폭에서 잘려 카운트가 안 보이던 것 → 제목 축약(`+/- weight · infl=share`)해 `· n/m` 카운트까지 노출.

## [0.18.2]
### Changed — Nodes/device 행 컬럼 정렬
- device 행(accel_brief) GB 필드 고정폭 + unified(∪) 슬롯 예약 → 온도/전력/모델이 열 정렬(Nodes 트리·Node 상세 공통).
- 노드 부모 행 mem 필드 고정폭 → load 열 정렬.

## [0.18.1]
### Changed — Overview 가속기 행 정렬
- 벤더/노드별 가속기 행의 VRAM(GB) 필드를 고정폭(우측정렬)으로 → 뒤따르는 인라인 트렌드 스파크라인이 열 정렬. `trend` 라벨 추가.

## [0.18.0]
### Changed — Overview 정보 위계 재배치
- 판정(Diagnosis)을 **히어로 카드 바로 밑(Status)** 으로 이동 — 맨 아래에서 위로. "지금 문제 있나?"에 즉답.
- 배치 순서를 위계대로: **Cluster(용량·부하) → Status(판정) → Accelerators → Inference(서빙경로) → Models(리스트)**.
- Overview 의 Models 표에 `⏎ detail` 힌트.

## [0.17.0]
### Changed — 은은한 애니메이션 + 패널 시스템 기반
- **애니메이션 톤다운·믹스**(눈 안 아프게, 테마-안전한 전경색 페이드): 뷰 전환=텍스트 dim→원색 잉크인(150ms), 상세 진입=짧은 coalesce(줌인 느낌, 180ms), 신규 알림=요약 바 전경 빨강→원색(450ms). 배경 전체 플래시/스캐터 제거.
- **패널 시스템 기반(Dashboard)**: 반응형 타일 배치 + 렌더 클로저(`.cell`) 추상화 도입. Perf 드릴/Model 상세 지표 그리드를 이관. (테이블+스크롤바 등 자체 chrome 위젯과도 마찰 없이 합성.)

## [0.16.0]
### Added — tachyonfx 애니메이션(순수 Rust)
- **뷰 전환 트랜지션**: 탭/뷰 전환·상세 진입 시 본문이 coalesce(흩어진 셀→완성) 로 부드럽게 나타남.
- **신규 알림 플래시**: 알림 발생 시 요약 바가 빨강에서 원색으로 페이드(펄스).
- 애니메이션 중엔 렌더 폴링을 16ms 로 좁혀 부드럽게, 평시엔 100ms 유지(CPU 절약).
- tachyonfx 0.16(ratatui 0.29 호환 마지막)로 핀. `--render` 는 이펙트 비활성(텍스트 덤프 오염 방지).

## [0.15.0]
### Changed — Perf 개요·정렬, 자연스러운 y축, 타일링
- **Perf 상단**: 종류별 집계 타임라인 박스 → **디바이스별 컴팩트 한 줄** util/VRAM 컬러 스파크라인(오른쪽=now). 한눈에 장비별 추이.
- **Per-model 표**: 지금 **서빙 중(active)** 모델만 표시 + **정렬**(o: tok/s·E2E·TTFT·queue·name). 표가 order() 경유로 정렬/선택 일치.
- **y축 자동 스케일**: nice_ceil 계단을 세밀화(1·1.5·2·2.5·3·4·5·6·8·10)하고 헤드룸 축소 → 피크가 높이의 ~80~90% 를 채워 그래프가 납작하지 않음(특히 상세의 지연 타임라인).
- **타일링 배치**: 타임라인 그리드(Perf 드릴/Model 상세)를 반응형 `tile_rects` 로 균형 배치.
- **Tab 표시**: 탭 바 앞에 `⇥` 마커 + 푸터에 `⇥/0-9 view` 힌트(전환 방법 노출). Flow 의 Enter 는 `⏎ model`.

## [0.14.1]
### Fixed — Flow 에서 route Enter 시 네비게이션 잠김
- Flow(Routing)엔 상세 패널이 없는데 Enter 가 detail=true 로 만들어 ↑↓ 가 스크롤로 빠지며 route 이동이 잠기던(멈춘 것처럼 보이던) 버그 수정.
- toggle_detail 은 이제 상세 패널이 있는 뷰에서만 동작. Flow 의 Enter 는 backend 모델 상세로 드릴(esc 로 복귀).

## [0.14.0]
### Added — 상세 뷰의 개체별 시계열 히스토리
- **Node 상세**: device 목록에서 ↑↓ 로 device 선택 → 그 device 의 util/VRAM 타임라인. 미선택(0)일 땐 노드 host cpu/mem 요약(기존). 선택 행은 ▸ 로 강조. (←→ 는 이전/다음 노드.)
- **Perf 드릴(⏎)**: percentile 표 + **모델별 지표 타임라인 그리드**(tok/s·TTFT·QUEUE·PREFILL·DECODE·E2E) + E2E 버킷 히스토그램. 컬럼 값들을 시간축으로.
- **Model 상세**: 정보 + 매칭되는 per-model perf 시계열(tok/s·TTFT·DECODE·E2E) 타임라인.
- per-model perf 시계열을 히스토리에 기록(`mperf:{model}:*`).

## [0.13.0]
### Changed — 타임라인을 htop 식 braille 영역 그래프로
- 블록 채움 + 가로 눈금(클러터) → **braille 점 영역 그래프**(셀당 2×4 점 = 가로2·세로4 고해상도, htop graph 모드 식)로 교체. 훨씬 부드럽고 깔끔.
- 각 시점(braille 서브열) 값에 따라 severity 색(초록 여유→빨강 과도). now/max 는 제목에.

## [0.12.3]
### Changed — 타임라인 개선(Perf/infra): 전체폭 10% 눈금 + 값별 색
- 가로 10% 눈금을 **차트 전체 폭에** 깔아(데이터 없는 왼쪽도) 스케일이 항상 보이게, 밝은 회색(244). 그 위에 시간 바를 얹음.
- **각 time bar 는 그 시점 값에 따른 severity 색**(높으면 빨강, 낮으면 초록) — 열마다 색이 다름.
- Perf 타임라인 차트 높이 확대(12→16)로 추세·눈금 가독성 ↑.

## [0.12.2]
### Changed — 눈금 10% 단위 + infra 상세 게이지 통일
- VRAM 구성 바 눈금 **5% → 10%**(`█████│████│…`, 훨씬 깔끔).
- **infra 상세(Accel 드릴인) 게이지**(compute/VRAM/temp)도 dot 게이지 + **10% `│` 눈금** 으로 통일(넓은 바에서만 눈금, 좁은 표 바는 점만).

## [0.12.1]
### Fixed — 눈금 안 보임(글리프) + 바를 색 점으로
- **원인**: VRAM 5% 눈금 문자 `┊`(U+250A)가 많은 터미널 폰트에 글리프가 없어 **공백으로 렌더**(→ 안 보임). `--render`(TestBackend)는 코드포인트를 그대로 보여줘서 진단에서 놓침. → **`│`(U+2502, 모든 폰트 지원) + 밝은 회색**으로 교체.
- **로드 바를 색 점(●/·) 게이지로**(요청 b): 채움=색 `●`, 여백=흐린 `·`. `●`는 이미 LED/상태에 쓰던 폰트-안전 글리프.

## [0.12.0]
### Changed — 바 색 통일 + VRAM 5% 눈금 edge
- 모든 로드 바(util/mem/cpu/게이지)를 **단일 severity 색**으로 통일(값 하나로 색 결정: util→util_color, mem→mem_color). per-cell 그라디언트 폐기, `grad_bar` 제거.
- **VRAM 구성 바에 5% 눈금 edge(┊)** — 셀 단위 세그먼트(벤더 색) + 5%마다 보이는 tick(회색) → stacked 느낌으로 구성/비율 눈금 읽힘. (좁은 바는 폭 부족으로 tick 생략.)

## [0.11.1]
### Changed — 타임라인: 가로 10% 그리드 + 바 전체 severity 색 + 간격 제거
- 바 색을 per-cell 그라디언트 → **바 전체 하나의 severity 색**(값이 threshold 넘으면 색 전체가 초록→노랑→빨강으로 바뀜).
- 세로 edge 제거, **가로 그리드라인을 10% 간격**으로(보이는 회색 `─`) → 값 높이를 눈금으로 읽음.
- **바 간 간격 제거**(인접) — 더 많은 히스토리, 그리드라인이 레벨 기준 제공.

## [0.11.0]
### Changed — 레인보우 철회 → severity(여유↔과도) + 바 edge
- **레인보우 제거**, 전 바/타임라인/LED 를 **초록(여유)→노랑→빨강(과도)** severity 색으로 복귀 — 부하가 색으로 바로 읽힘. (rainbow/hsl_to_rgb 헬퍼 삭제.)
- **타임라인 바에 세로 edge(│, 어두운 라인)** 추가 → 각 바가 경계로 구분(여백/점 대신). 순수 black 은 다크 터미널에서 배경에 묻혀 near-black(Indexed 234) 사용.
- (유지) 바 간격, Overview Σ 정리·평균온도·MEM 바.

## [0.10.4]
### Changed — Overview 한눈에 개선 (all-smi 참고)
- 클러스터 Σ 줄을 **라벨된 그룹**으로 정리: `N accel · 벤더×n │ util·temp │ VRAM·W │ models`. 평균 **온도** 추가, 상단바와 중복인 req/s·TTFT 제거.
- **LED 그리드 점을 util 히트(레인보우)** 로 채색 — fleet 핫스팟이 한눈에(파랑 저 → 빨강 고).

## [0.10.3]
### Changed — 레인보우 바 전면 통일 + Overview MEM 바
- 모든 load 바(Accel UTIL/MEM · Overview util · Nodes cpu · 상세 게이지 compute/VRAM/temp)를 **레인보우 progressbar 로 통일**. 의미(심각도)는 옆 수치 색으로 유지. (사용 안 하게 된 `grad_bar` 제거; `grad_color` 는 mem-bw 색에 잔존.)
- **Overview 가속기 행에 MEM 레인보우 바 추가** — util 이 0(유휴)이어도 채워진 progressbar 가 보임(예: GB10 `█████████▌ 124/131GB`).

## [0.10.2]
### Changed — 타임라인을 채움+track(████░░░░) 로
- 타임라인 각 세로 컬럼을 값까지 █(세로 레인보우) 채우고 나머지는 ░ track 으로 → `████░░░░` 느낌. 컬럼 사이 1칸 간격 유지(구분). (점 scatter 시안은 폐기.)

## [0.10.1]
### Changed — 타임라인 바 간격(가독성)
- 레인보우 타임라인 컬럼들이 붙어 개별 바 구분이 어렵던 것을, **바 사이 1칸 간격**(bar-chart 식, 바+간격=2칸)으로 분리. 높이가 충분(≥4행)한 타임라인은 **상단 1행 헤드룸**을 둬 바가 테두리에 안 붙게(위아래 간격). 작은 타임라인은 전체 높이 유지.

## [0.10.0]
### Added — 레인보우 바/타임라인 (tui-bar-graph·hedzr/progressbar 참고, 자체 구현)
- **레인보우 타임라인**: area-fill 타임라인 셀을 **세로 위치별 레인보우 그라디언트**(아래 파랑=저부하 → 위 빨강=고부하)로 채색(tui-bar-graph VerticalGradient 식). 높은 컬럼일수록 빨강까지 닿아 부하가 직관적.
- **레인보우 오버뷰 바**: Overview 가속기(by-kind/node) 바를 `rainbow_bar`(파랑→빨강 스펙트럼 채움)로 — progressbar 식 장식. 옆 수치는 severity 색 유지 → 의미 보존.
- HSL→RGB `rainbow(t)` 자체 구현(외부 크레이트 없음, 순수 Rust 원칙). 타임라인 now 값도 레인보우.

## [0.9.4]
### Changed — 리팩토링 R1: ui.rs 모듈 분해
- 단일 2052줄 `ui.rs` 를 **`ui/` 모듈**로 분해: `ui/theme.rs`(팔레트·심각도 임계치·색 로직 117줄) + `ui/widgets.rs`(재사용 렌더 헬퍼: 바/게이지/테이블/블록/문자열 절단 246줄) + `ui/mod.rs`(뷰·크롬·타임라인 1703줄). 뷰 추가 시 헬퍼가 discoverable 한 곳에. (렌더 구조 골든 대비 IDENTICAL — 동작 완전 보존. per-view 세분화는 후속 여지.)

### Refactor 요약 (R1~R5)
- R1 ui.rs 분해 · R2 metrics 단일출처 · R3 collect 소스별 수집기 · R4 테이블 헬퍼 · R5 config 외부화. 전 단계 동작 보존(테스트·렌더 골든 동일).

## [0.9.3]
### Changed — 리팩토링 R3: collect_fast 소스별 수집기로 해체
- `collect_fast` 의 **25-요소 `tokio::join!` 위치 튜플**(메트릭 추가 때마다 튜플/구조분해 동기화 필요 → 반복적으로 삐끗)을 제거. **소스별 수집기 4개**(`collect_furiosa`/`collect_rbln`/`collect_gpu`/`collect_nodes`)로 분리 — 각자 자기 메트릭만 작은 join! 로 처리하고 Vec 반환. `collect_fast` 는 4개를 병렬 실행 후 합치고 통합-메모리 backfill 만. 메트릭 추가 시 해당 함수 1곳만 수정.

## [0.9.2]
### Changed — 리팩토링 R4: 반복 테이블 렌더 헬퍼
- Accel/Pods/Events 의 Table+헤더+선택 하이라이트+스크롤바+위치카운터 보일러플레이트(3회 중복)를 **`render_list_table` 헬퍼 1곳**으로 통합. 뷰는 rows/widths/headers/title 만 넘김.

## [0.9.1]
### Changed — 리팩토링 R5: 튜닝값 config 외부화
- 흩어져 하드코딩되던 튜닝값을 **`config.rs` 단일 모듈**로: prom/ns/grafana/수집주기(full·fast). 우선순위 **env > `~/.config/lmd-top/lmd-top.yaml` > 기본값**. main.rs 의 하드코딩 인터벌·Grafana URL 제거, `ui_loop` 는 `Config` 를 받음. `Config` 를 collect→config 로 이관, columns 로더도 `config::load_yaml` 재사용.

## [0.9.0]
### Changed — 리팩토링 R2: 메트릭 이름 단일 출처
- 모든 프로메테우스 메트릭 이름을 **`metrics.rs` 단일 출처**로 모음(그룹별 `pub const` + doctor 커버리지 `DEPS`). `collect.rs`와 `doctor.rs`가 같은 상수를 참조 → 이름 drift 불가, 메트릭 추가는 한 곳만 수정. collect 의 bare 이름 21개를 상수로 치환. (동작 보존: doctor 33/40 동일, 테스트 통과.)

## [0.8.8]
### Changed — Flow(Topo) 인터랙티브화 (IA 리뷰 P1 핵심)
- **"correlation이 텍스트일 뿐 내비게이션이 아니다"** 해소: Topo→**Flow** 로 개명하고 route 경로를 **선택 가능**(↑↓, 배경 하이라이트)하게 + 선택 route 에서 **`p`/`i`/`m`/`e` pivot**(pods/infra/model/epp)으로 레이어 횡단. 이제 경로 위에서 손으로 상관관계를 넘나듦.
- *남은 P1(기본화면 Flow 플립·탭 그룹화·Events 오버레이)은 근육기억을 바꾸는 부분이라 실기 반복 후 진행.*

## [0.8.7]
### Added — safe action: rollout restart (Control-plane M3)
- **`S`**: 선택 모델 **rollout restart**(`kubectl rollout restart`, 롤링 재기동) — admin+ 게이트 + y/n 확인(Pending::Restart). scale 과 동일한 dry-run→confirm 경로. (endpoint drain / traffic·policy weight 는 endpoint/CR 조작이 필요해 후속.)

## [0.8.6]
### Fixed — 재평가 리뷰 반영(견고성/정직성)
- **패닉 시 터미널 복원**(#1, 높음): `panic::set_hook` 으로 패닉해도 raw mode/alt-screen 해제 → 셸 안 망가짐.
- **EPP 과대약속 수정**(#2, 높음): "+/- what-if (sim)" 문구가 라우팅 예측처럼 오해 → **"+/- weight · infl=weight share (not applied)"** 로 정직하게 낮춤(실제 재시뮬은 per-endpoint score 필요 → 인프라 대기).
- **serving 에러 글리프 과민**(#4): `err>0` 이면 무조건 노랑이던 것을 **비율(err/req>1%)** 기준으로 → 4xx 하나에 상단이 상시 노랑 안 됨.
- **pivot 빈-착지 방지**(#3): 매칭 0건이면 막다른 화면 대신 되짚고 안내.
- **미지원 pivot 키 힌트**(#8): 죽은 입력 대신 해당 뷰의 가능한 pivot 안내.
- **"busy" 정의 통일**(#5): 요약바 busy 기준을 `util>5` → `IDLE_UTIL` 로(LED/util_color 와 일치).
- **상태 리셋 일관화**(#6): `epp_weights` 를 pivot/nav_back 에서도 클리어.
- **selected_perf_model sel_orig 경유**(#7): Perf 정렬/필터 대비.
- **단위테스트 추가**(#C): pivot 왕복·빈-착지·미지원키 불변식.

## [0.8.5]
### Added — 세션 에너지 추적 (all-smi 식)
- `DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION`(누적 mJ)로 **세션 에너지(Wh)** 를 디바이스별 기준선 대비 추적. Accel 상세에 `energy X.XX Wh (session · avg N W)`, Overview Σ 에 클러스터 총 `E N Wh`. **`R`** 로 세션 리셋.

## [0.8.4]
### Changed — 모든 바/타임라인에 track(░) 표시 (대략 % 가늠)
- track 없이 `█████`만 그리던 바들(EPP WEIGHT, Perf 지연 히스토그램)을 **고정폭 + track(░)** 으로 → `████████░░░░░░░░` 형태로 상한 대비 대략 %가 보임(`bar_line` 재사용).
- **area-fill 타임라인**: 채워지지 않은 칸을 은은한 track(░, C_TRACK)으로 채워 **천장(ymax) 대비 현재 높이(%)** 를 한눈에. (grad_bar/bar_line/stacked_bar 는 이미 track 보유.)

## [0.8.3]
### Added — EPP scorer what-if + Perf pivot (인터랙티브)
- **EPP scorer 가중치 what-if**: 선택 scorer를 `+`/`-`로 조정 → 정규화 WEIGHT 바 + **상대 영향도(infl%)**가 실시간 재계산(로컬 시뮬, 클러스터 무변경). "이 가중치를 올리면 밸런스가 어떻게 바뀌나"를 탐색 — EPP 정책 설계용. EPP 떠나면 리셋.
- **Perf 크로스레이어 pivot**: 선택 모델 행에서 `p`(pods)/`i`(infra)/`e`(epp) 점프 — Models 와 일관.

## [0.8.2]
### Changed — 컨텍스트 푸터 (IA 리뷰 P2)
- 푸터가 12개 힌트를 뷰 무관하게 항상 나열하던 것(예: Events 에 `s scale` no-op)을, **현재 뷰가 실제 할 수 있는 액션만** 표시하도록 재구성. detail/filter/sort/pivot/logs/scale 을 뷰별로 노출, 전역 키(A/t/z/g/?/q)만 상시. (←→ 패널 포커스는 현재 다중 선택 패널이 없어 보류.)

## [0.8.1]
### Added — Perf 인터랙티브 드릴 (p50/p95/p99 + 히스토그램)
- Perf per-model 표를 **선택 가능**하게 하고 **`⏎`** 로 드릴다운: 선택 모델의 **TTFT/TPOT/E2E p50·p95·p99** 백분위 표 + **E2E 지연 버킷 히스토그램**(rate by bucket)을 프로메테우스에서 온디맨드 조회. vLLM/ds4-proxy 엔진 구분. `esc` 로 복귀. (유휴 시 "no samples" — 트래픽 발생 시 채워짐.)

## [0.8.0]
### Added — 크로스레이어 드릴 내비게이션 (IA 리뷰 P0, crown jewel)
- 선택 엔티티에서 **관련 레이어로 점프**(collector 는 이미 상관 연결됨, UI 내비만 없었음): Models/Overview 에서 **`p`**(pods)·**`i`**(infra/accel)·**`r`**(route/topo)·**`e`**(epp), Accel 에서 `p`·`m`(model)·`n`(node), Pods 에서 `i`·`m`. 점프 = 뷰 전환 + 상관 필터.
- **브레드크럼 스택**: pivot 시 현재 위치를 쌓고 **`esc` 로 되짚음**(상세→브레드크럼→필터→줌 순). 수동 뷰 전환 시 초기화.
- 발견성: Model/Pod 상세에 `pivot [p] pods [i] infra …` 안내 줄 + 뷰별 footer 힌트 + help 항목.
- "정적 카드 N장"을 **손으로 레이어를 넘나드는 도구**로 — k9s+all-smi 대비 이 도구의 존재이유.

## [0.7.4]
### Changed — 상단 요약바를 서빙/SLO 우선으로 (IA 리뷰 P0)
- 항상 보이는 요약바가 하드웨어(GPU/RBLN/RNGD 개수·와트)로 시작하던 것을 **서빙 건강 우선**으로 재조합: `● SERVING n/N · req/s · err · TTFT · E2E │ accel busy · VRAM% · ⚡W · ⚠alert`. 운영자의 첫 질문("서빙 정상인가/지연 건강한가")이 0 키스트로크에 보이고, 인프라 재고는 뒤로.

## [0.7.3]
### Changed — Nodes 뷰를 all-smi 식 트리로
- **Nodes 오버뷰가 트리**: 노드마다 헤더(status·cpu 바·mem·load) + **자식으로 그 노드의 모든 디바이스**를 인라인 메트릭(util 바·mem·temp·pwr·`∪`통합표식·busy_model)으로 한눈에. 선택 노드 하이라이트 + 자동 스크롤.
- **Nodes 상세(⏎)**: 호스트 게이지 + **그 노드의 모든 디바이스 full 라인**(mem-bw·clock 포함) + 호스트 cpu/mem 타임라인.

## [0.7.2]
### Added — 가속기 포화 신호 (mem-bandwidth/clock/mem-temp)
- GPU 에 **`DCGM_FI_DEV_MEM_COPY_UTIL`(메모리 대역폭 압박)·`SM_CLOCK`(MHz)·`MEMORY_TEMP`** 수집 → Accel 상세에 mem-bw 게이지 + clock/mem-temp 줄. **통합 메모리(GB10)에선 compute util·VRAM% 가 오해를 주므로 메모리 대역폭이 진짜 병목 신호** — 이 blind spot 을 메움. agent JSON 에 `mem_bw_pct`/`clock_mhz`/`mem_temp_c` 추가. (미지원 벤더/필드는 `–`/null.)

## [0.7.1]
### Fixed — ds4 perf 미표시 (커스텀 엔진 메트릭)
- ds4(deepseek-v4-flash)는 vLLM 이 아니라 **`ds4_proxy_*` 커스텀 프록시 메트릭**으로 노출해 `vllm:*` 쿼리에 안 잡혔음. `--doctor`/live 조사로 규명 → `ds4_proxy_requests_total`·`_ttft_seconds`·`_request_duration_seconds`(E2E)·`_output_tokens_total` 을 수집해 Perf 에 **집계 행 `ds4-proxy`** 로 반영(프록시가 백엔드 라벨 없이 집계 → 집계 1행). 트래픽 발생 시 TTFT/E2E/req·tok/s 채워짐.

## [0.7.0]
### Added — `--doctor` metric survey (자동 전수조사)
- **`lmd-top --doctor`**: Prometheus 메트릭을 전수조사해 (1) 감지된 exporter(job), (2) lmd-top 이 읽는 40개 메트릭의 **존재/부재 + 부재 시 영향**(예: `FB_TOTAL` 없음 → unified-mem 은 host 로 fallback / EPP 메트릭 없음 → EPP 뷰 빔), (3) **미사용 가속기 메트릭(=새 신호 후보)** 을 자동 리포트. "왜 이 뷰가 비었나"·"이 클러스터에 새 메트릭이 있나"를 한 번에 진단(수동 PromQL 조사 불필요).
- `prom::label_values`(`/api/v1/label/<l>/values`) 추가 — 메트릭 이름·job 목록 조회.

## [0.6.4]
### Added — GB10 exporter labels (accel↔model correlation)
- GB10 전용 `dcgm-exporter-gb10` 가 붙이는 **`exported_pod` 라벨**을 GPU `busy_model` 로 반영(기존 GPU 는 항상 빈 값이었음). → GPU 카드가 점유 모델서버 파드를 표시하고, **모델↔가속기 매칭(Models ACCEL 열)** · Accel 뷰 로그(`l`) 가 GB10 에서도 동작.
- `accel_for` 가 벤더 라벨 대신 **감지 모델**을 사용 → Models ACCEL 열이 `GPU×2` 대신 `GB10×2 <node>` 로 표시.

## [0.6.3]
### Fixed — unified-memory accelerators (GB10 등)
- GB10 같은 **통합 메모리**(Grace 계열 superchip: GB10/GH200/GB200) 는 별도 VRAM 이 없어 DCGM `FB_TOTAL` 이 0 → mem 이 `0/0` 로 표시되던 문제. 이제 통합 메모리 장치는 **호스트(노드) 메모리 풀**로 backfill 하고 **`∪` 표식 + 상세에 "unified w/ host"** 로 명시. agent JSON 에 `unified_mem: bool` 추가.
- 감지: 모델명이 GB10/GH200/GB200/GB300 계열이면 통합으로 판정.

## [0.6.2]
### Added — agent JSON state (Control-plane M2)
- **`lmd-top --json`** (또는 `--snapshot --json`): 화면 파싱 없이 AI agent 가 소비할 **기계가독 상태 트리**를 stdout 으로 출력. 큐레이트된 안정 스키마(`schema: "lmd-top/agent-state/v1"`) — 내부 Snapshot 과 분리:
  - `cluster`(집계) · `accelerators`(kind+감지모델) · `models` · `pools` · `per_model_perf` · `diagnosis`(message+severity) · `alerts`(snapshot 조건) · **`actions`**(모델별 scale, `risk`/`requires_confirmation` 포함).
  - NaN → `null` 정규화. 진단·알림 판정 로직은 UI 와 공유(`app::diagnose`/`snapshot_alerts`).

## [0.6.1]
### Fixed — GPU model/VRAM auto-detection (no more hardcoded "A100"/80GB)
- GPU 계열 라벨이 **하드코딩 "A100"** 이라 실제 장치(예: GB10)가 틀리게 표시되던 문제. 이제 DCGM `modelName` 라벨에서 **실제 모델을 자동 감지**(`"NVIDIA GB10"→"GB10"`, `"NVIDIA A100-SXM4-40GB"→"A100"`) → Accel KIND 열·Overview(Σ/그룹/LED)·상세·`--snapshot` 에 반영. 감지 실패 시 벤더 계열 `GPU` 로 fallback(더는 오답 "A100" 아님).
- GPU **총 VRAM 도 하드코딩 80GB** 였음 → `DCGM_FI_DEV_FB_TOTAL` 로 자동 산출(GB10 등 정확).

## [0.6.0]
### Added — permission modes (Control-plane M1)
- **권한 모드**(운영 사고 방지): 기동 시 `--mode observe|debug|admin|danger`(기본 observe), 헤더에 상시 배지(observe=은은, 상승 권한=색+굵게). 변경 작업을 권한 레벨로 게이트:
  - `l`(logs) → **debug+**, `s`(scale) → **admin+**. 부족하면 토스트로 안내.
- **변경 작업 확인(y/n)**: `s`(scale)는 즉시 실행 대신 footer 확인 프롬프트(`scale X → N replica(s)? y/n`) → `y`에만 실행. 이후 drain/weight/rollout 도 동일 경로(Pending) 로 확장 예정.

## [0.5.1]
### Fixed
- **Perf 에 런칭된 모델이 다 안 보이던 문제**: per-model perf 표가 `by (model_name)` 트래픽 시계열로만 채워져, **최근 1분 트래픽이 없는(유휴) 배포는 표에서 누락**됐음(예: 갓 런칭한 ds4). 이제 (1) 병합 키를 `model_name`→**`service`(=Deployment 이름, Models 뷰와 동일)** 로 바꾸고, (2) collect_kube 이후 **런칭된 모든 모델(snap.models)을 seed** 로 깔아 vLLM 메트릭을 좌조인 → 트래픽이 없어도 `–` 로 항상 표시. 표시명도 배포명으로 통일.

## [0.5.0]
### Added — 능동 알림 + 반응형 + 메모리 구성
- **능동 알림 시스템**: 스냅샷마다 임계/상태 이상(가속기 not-alive·throttle·고온 >80°C, 노드 cordon·NotReady·pressure, pod 재시작 증가·Failed)을 감지 → **신규 발생분만**(엣지 검출) 요약바 플래시(~3s 빨강 반전) + 만료형 토스트(심각=빨강). `A` 로 **알림 히스토리 오버레이**(최신순 50건, 상대시각). 요약바에 `⚠N alert (A)` 상시 카운터.
- **VRAM 구성 스택 바**: Overview Cluster 카드에 클러스터 VRAM을 **벤더별(GPU/RBLN/RNGD) 세그먼트 + free** 스택 바로 — 이종 가속기 메모리 점유 한눈에(all-smi multi-segment 식). *weights/KV 분해는 메트릭에 없어 미구현.*

### Changed
- **탭 바 반응형**: 전체 라벨이 화면 폭을 넘으면 비활성 탭은 번호만(활성 탭은 라벨 유지) — 좁은 터미널 잘림 해소.
- **토스트 만료**: 액션/알림 토스트에 5초 만료 부여(`notify`) — 키 입력 없이도 사라짐.

## [0.4.1]
### Fixed
- **스크롤바 off-by-one**: 뷰포트 계산이 테이블 헤더 행을 빼먹어 경계값(항목 수 == 화면 높이-2)에서 마지막 행이 가려져도 스크롤바를 안 그리던 문제. `list_scrollbar` 에 `header` 인자 추가(테이블=1, 로그=0).

### Changed
- **LED 그리드 줄바꿈**: Overview Cluster 카드의 LED 그리드가 단일 줄이라 디바이스가 많으면(대형 fleet) 가로로 잘리던 것을, 폭에 맞춰 줄바꿈(라벨 폭만큼 들여쓰기, 최대 8줄) + 카드 높이를 LED 줄 수에 맞춰 가변화.
- **헤더 데이터 신선도**: 타이틀 바에 `updated Ns ago`(수집 주기 3s) 표시 — 10s 초과 시 노랑으로 stale 경고, 스냅샷 전엔 `connecting…`.
- **PFILL/DECODE 테마 대응**: Perf 테이블의 prefill(cyan)·decode(magenta) 하드코딩 색을 테마·색맹(Okabe-Ito) 대응 헬퍼(C_PREFILL/C_DECODE)로 교체.

## [0.4.0]
### Added — polish pass (btop/all-smi/bottom 벤치마크)
- **채움(area-fill) 타임라인**: 타임라인을 세로 블록(▁▂▃▄▅▆▇█) 채움 + 값 높이별 green→yellow→red 심각도색으로(btop/bottom식). 외부 크레이트 없이 프레임 버퍼 직접 렌더(tui-bar-graph는 ratatui 0.29 비호환이라 자체 구현).
- **스크롤바 + 위치 카운터**: Accel/Models/Pods/Nodes/Events/Logs/EPP/Launch 테이블에 ratatui Scrollbar(오버플로 표시) + 블록 타이틀에 `· sel/total`.
- **활성 패널 포커스**: 멀티패널 뷰(EPP·Launch)에서 ↑↓가 움직이는 패널만 밝은 C_ACC 테두리 + `▸` 표식.
- **all-smi LED 그리드**: Overview Cluster 카드에 디바이스 1개=글리프 1개(vendor=색, util=●채움/○유휴, dead=✗, throttle=⚠) — fleet 밀도 한눈에.

### Changed — 일관성 폴리시
- 심각도 임계치를 단일 상수(UTIL_WARN/BAD·MEM_WARN/BAD·TEMP_WARN/BAD)로 통일 → 바 색과 값 색이 어긋나지 않음.
- 메모리 단위 `G`→`GB` 통일. help 범례에 `⊘ cordoned` 추가(실사용 글리프와 일치).

## [0.3.0]
### Fixed — serving metrics actually populate now
- **핵심 버그**: Perf/Models 의 req/s·TTFT·TPOT·E2E·in/out tokens 가 존재하지 않는 `inference_objective_*`(EPP 전용, 이 클러스터는 EPP 우회) 를 쿼리해서 항상 비어 있었음. → 전부 네이티브 **`vllm:*`** 로 교체(request_success_total / e2e_request_latency / request_time_per_output_token / request_prompt·generation_tokens). KV=`kv_cache_usage_perc`, prefix-hit=hits/queries, err=abort finished_reason.
- **조인 수정**: vLLM 메트릭을 fuzzy model_name 대신 **`service` 라벨(=Deployment 이름)** 로 정확 조인 → gemma4-rbln 등 run/wait/tps/kv 표시.
- **반응성**: 지연 히스토그램 rate 윈도우 `[5m]→[1m]` (생성 중 ~5배 빠르게 반영). 하한선은 Prometheus 스크랩 간격(vLLM 15s / HW 30s).

### Added
- **Perf per-model 에 P/D 분리 지표**: QUEUE(스케줄 대기) → PFILL(prefill, P) → DECODE(decode, D) p95 + preempt rate. prefill=cyan / decode=magenta 로 PD-disaggregation 가시화.
- **all-smi 식 현재-포션 게이지**: Accel/Node 상세를 게이지 행(util/VRAM/temp 현재값 바) + 넓은 반응형 타임라인 2개로 재구성(기존 좁은 3분할 개선).
- **Overview 클러스터 요약 카드**: 종류별 수·평균 util·VRAM·전력·모델 ready·req/s·TTFT 한 줄 + 가속기 그룹별 인라인 util 스파크라인.
- **Logs 뷰(l)**: 선택 pod/model 로그 오버레이(스크롤·r 새로고침·esc/q 닫기, error/warn 색상).

### Changed
- line_chart: 최신값을 **오른쪽(now)에 고정**(히스토리 짧을 때 왼쪽 뭉침 해소), 제목에 현재값 강조색.
- detail prev/next 를 **화살표 전용**으로(문자 h/l 해제 → l 은 Logs).

### Notes
- **EPP 우회 수정 시도 → 보류**: `openai-route` backendRef 를 InferencePool 로 바꾸면 EPP 경유하나, **Envoy Gateway v1.3.0 이 InferencePool 백엔드 미지원**(`ResolvedRefs=False: Unsupported backend kind`, /gemma4 HTTP 500). 즉시 롤백(HTTP 200 복구). 활성화하려면 EG 1.4+ (Inference Extension) 업그레이드 필요 = 컨트롤플레인 변경. 단, 코어 서빙 지표는 vLLM 직접 수집이라 EPP 없이도 전부 동작.

## [Unreleased]
### Added
- **Nodes 뷰(9)** · **Events 뷰(8)** · **Perf 종류별 동적 그리드**(존재하는 가속기만 이름+수) · **Topology 트리**(Gateway→HTTPRoute→route→pods) · **entity drill-down 멀티지표 타임라인**.
- **all-smi식 인라인 트렌드**: Accel 뷰 각 디바이스에 util 히스토리 스파크라인(▁▂▃▅▇) 컬럼.
- **detail 네비게이션**: ←→ 이전/다음 항목, ↑↓ 내부 스크롤, 제목에 `◂ i/n ▸` 위치.
- **btop 그라디언트 바** · **반응형 레이아웃**(two_panes, 폭<100 세로) · **zoom(z)** · **일시정지(space)** · **테마(t)** · **필터(/)** · **도움말(?)**.
- **추론 엔진** 컬럼(vLLM/SGLang/vLLM-RBLN/Ollama/…).
- Events 뷰(8): k8s + llm-d 이벤트 통합.

### Changed
- Esc는 **뒤로가기만**(종료는 q). 애니메이션 100ms(부드럽게). 타임라인 축(nice_ceil)+현재값.
- 경량화: fast tier(가속기+노드 1s, tokio::join! 병렬) + full 3s, Perf 쿼리 병렬/축소.
- **Nodes 뷰(9)**: 노드 health(ready/cordoned/pressure)·kubelet·CPU%·load·mem·노드별 가속기(전체 노드, cordoned 포함).
- **Entity drill-down timeline**: 가속기/노드 선택→⏎ → 그 대상의 다중 지표 타임라인(가속기 util/mem/temp · 노드 cpu/mem/load 스파크라인).
- summary/diagnosis 에 Warning 이벤트 반영.

- **추론 엔진 표시**: Models 뷰/상세에 ENGINE 컬럼(vLLM/SGLang/vLLM-RBLN/Ollama/Furiosa/custom, deploy command 감지).
- **그래프 개선(all-smi식)**: 타임라인을 축 라벨(X=시간 -Ns~now, Y=값+단위) + 현재/최대값 제목 표기하는 라인차트로. 드릴다운(가속기/노드 ⏎)은 util/mem/temp·cpu/mem/load 각각 라인차트. Perf 메인 타임라인 확대.

### Changed
- **성능(경량화)**: 가속기+노드 fast tier(collect_fast, tokio::join! 병렬) 1초 + full collect 3초. Perf 전역쿼리 병렬+축소(15→6). util/mem 반응성↑.
- **라벨 명확화**: Accel UTIL=compute%, MEM=VRAM. Perf timeline=accel util%/VRAM%(cluster avg). Node=host CPU/mem.
- Prometheus는 HTTP 전용 → 최적화는 병렬/쿼리수 축소(별도 경량 프로토콜 없음).
- (다음) PD 뷰, EPP decision score table, cache 뷰, ModelService 단위 — [ROADMAP.md](./ROADMAP.md)

## [0.2.0] — 2026-06-30
llm-d serving 운영 콘솔로 확장. 8개 뷰 + 시각화/상호작용/런처.

### Added
- 뷰 확장: **Topology**(Gateway→HTTPRoute→backend + InferencePool/EPP/SLO + KEDA autoscale + EPP-bypass 진단),
  **Perf**(구간별 지연 p50/p95/p99 · 토큰 분포 · per-model 테이블 · timeline 라인 차트), **Launch**(카탈로그 × 라이브 재고, read-only)
- **EPP Inspector**: 선택 가능 scorer + 설명 패널 + 요청 분배(scheduler_attempts) + prefix indexer
- NPU health/throttling, 오토스케일(KEDA), 절대 메모리(GB)
- 상호작용: `/` 필터 · `?` 도움말(색 범례) · `o` 정렬 · `t` 테마(default/high-contrast/colorblind) · `g` Grafana · 마우스 스크롤 · 컬럼 커스터마이징(`~/.config/lmd-top/lmd-top.yaml`)
- 디자인: 라인 차트 · 둥근 테두리 · 라이브 스피너 · 마퀴 가로스크롤 · semantic colors
- `ROADMAP.md`(LLM serving 운영 콘솔 비전), `CHANGELOG.md`

### Fixed
- 메모리 단위(RBLN/Furiosa/node DRAM 은 bytes → 항상 /1e9)
- RBLN health 반전(HEALTH=0 이 정상)
- 하이라이트 시 CJK 폭 오계산으로 글자 깨짐 → unicode-width 기반 절단 + TableState 배경 하이라이트
- Grafana(`g`)의 xdg-open 이 터미널 화면 깨뜨리던 문제 → stdio null

### Changed
- UI 전체 **영어화**

## [0.1.0] — 2026-06-30
Phase 1: 모니터링 TUI (Rust/ratatui).

### Added
- 6개 뷰: Overview/Accel/Models/EPP/Routing/Pods, 2초 자동 갱신
- 가속기(NVIDIA/RBLN/Furiosa) util/mem/temp/power, EPP scorer(ConfigMap), 모델·라우팅·파드
- 순수 Rust: Prometheus(tokio TCP HTTP/1.0) + kubectl 셸링, scale 액션
- `--snapshot` / `--render` 검증 모드

[Unreleased]: https://example/compare/v0.2.0...HEAD
[0.2.0]: https://example/compare/v0.1.0...v0.2.0
[0.1.0]: https://example/releases/tag/v0.1.0
