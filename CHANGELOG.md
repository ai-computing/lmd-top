# Changelog

[Semantic Versioning](https://semver.org). 0.x = 실험적(인터페이스 변경 가능).

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
