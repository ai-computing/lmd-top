# Changelog

[Semantic Versioning](https://semver.org). 0.x = 실험적(인터페이스 변경 가능).

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
