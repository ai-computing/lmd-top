# lmd-top

> **llm-d 클러스터를 위한 터미널 관측·운영 도구.**
> `k9s`의 네비게이션 + `all-smi`의 가속기 카드 + **llm-d EPP 아키텍처에 대한 1급 이해**를 한 화면에.

`Gateway → EPP → Model Server → Infrastructure` 4계층을 하나의 TUI에서 **상관(correlate)** 해서 보여주는,
이종 가속기(NVIDIA GPU / Rebellions RBLN / Furiosa RNGD / CPU) llm-d 환경용 모니터입니다.

```
┌ lmd-top · llm-d · 5 nodes · gw 192.0.2.6 ● ─────────────────────────────┐
│ A100 0 RBLN 4 RNGD 8 · 0 busy │ models 1/6 │ ⚡390W                           │
│ 0:Overview  1:Accel  2:Models  3:EPP  4:Route  5:Pods                        │
├ Accelerators ────────────────────────────────────────────────────────────────┤
│ RBLN rbln0 ████░░░░ 12%  14/17G 42°C 19W  gemma4-rbln-…                       │
│ RNGD npu0  ░░░░░░░░  0%   0/51G 42°C 38W                                      │
├ Inference (EPP pools) ─────────────────────────────────────────────────────── ┤
│ llmd-router  ready 0  queue –  kv –  sat –                                     │
│ scorers: queue·2  kv-cache-utilization·2  prefix-cache·3  no-hit-lru·2         │
├ Models ────────────────────────────────────────────────────────────────────── ┤
│ gemma4-rbln       1/1  – – –  /gemma4  ● Running                               │
│ vllm-exaone       0/0  – – –  –        ○ Scaled-0                              │
├ Diagnosis ──────────────────────────────────────────────────────────────────  ┤
│ ● 1 모델 서빙 중, 가속기 여유                                                   │
└────────────────────────────────────────────────────────────────────────────── ┘
 ↑↓ select  Tab next  0-5 view  s scale  q quit
```

---

## 왜 llm-top 인가 — 차별점

| | 보는 것 | llm-d/EPP 이해 | 가속기 | K8s 액션 | 터미널 |
|---|---|---|---|---|---|
| `k9s` | K8s 오브젝트 | ❌ | ❌ | ✅ | ✅ |
| `all-smi` | Infra(가속기)만 | ❌ | ✅✅ | ❌ | ✅ |
| `llmtop` | 단일 호스트 psutil | ❌ | ⚠️ | ❌ | ✅ |
| Grafana | 전 계층 메트릭 | ⚠️ | ✅ | ❌ | ❌ 웹 |
| **lmd-top** | **4계층 상관** | ✅✅ EPP `Filter→Score→Pick` | ✅ | ✅ | ✅ |

llm-d 생태계에는 **라이브 운영용 터미널 도구가 없습니다** (Grafana 웹 대시보드 / Prism 벤치 / helm·kubectl 뿐).
lmd-top 은 그 빈자리를 채우며, 특히 **EPP 라우팅 의사결정**을 관측·설명합니다.

## 기능 (Phase 1: 모니터링)

여섯 개 뷰 — 상단 숫자키(`0`–`5`) 또는 `Tab` 으로 전환:

| # | 뷰 | 내용 |
|---|---|---|
| **0** | **Overview** | 가속기 요약 + EPP 풀 + 모델 + **cross-layer 1줄 진단** |
| 1 | **Accel** | 디바이스별 카드(util 바 / VRAM / 온도 / 전력) + util 스파크라인. NVIDIA·RBLN·Furiosa 통합 |
| 2 | **Models** | 모델별 가속기/노드(ACCEL) · ready · running/waiting · KV · t/s · 라우팅 path · 상태 |
| 3 | **EPP** | 활성 scorer·가중치(ConfigMap introspect) + picker + InferencePool endpoints + **요청 분배**(라우팅 결정) |
| 4 | **Topo** | **전체 구성 한눈에** — Gateway → HTTPRoute → backend(모델 상태/가속기/노드) + InferencePool/EPP/**SLO**(InferenceObjective) + *EPP 우회 진단* |
| 5 | **Pods** | llm-serving 파드 상태 |
| **6** | **Perf** | **EPP 정책용** — 구간별 지연 p50/p95/p99(queue/TTFT/TPOT/E2E) · 토큰 길이 분포 · tok/s · 요청 분배 + **timeline 스파크라인**(util/vram/tok·s/지연 추이) |

> **Topo 뷰**가 "어느 모델이 어디서 돌고, 어디로 라우팅되며, 요청이 어떻게 분배되는가"를 답하고,
> HTTPRoute 가 InferencePool(EPP) 을 경유하는지/우회하는지 자동 진단합니다.
> **Perf 뷰**는 EPP scorer 정책을 짜는 데 필요한 지연/토큰/분배 지표를 모읍니다(EPP 경유 트래픽 시 채워짐).

**디자인**: 둥근 테두리 · 라이브 스피너 · 상태 아이콘(●○◐⚠) · util/메모리 프랙셔널 바 ·
긴 이름은 선택 시 **가로 스크롤(마퀴) 애니메이션** · 폭(unicode-width) 안전 렌더.

### 색상 의미 (semantic colors)
색은 **심각도** 또는 **정체성**을, 상태는 **글리프**로 분리 표현합니다:

| 요소 | 의미 |
|---|---|
| 🟢 green | 정상 / 낮은 부하 / 서빙 중 |
| 🟡 yellow | 경고 / 중간 부하 / pending / 스로틀링 |
| 🔴 red | 위험 / 고부하 / 에러 / 장치 다운 |
| 🔵 cyan | 강조 / 헤더 / 상호작용 값 |
| ⚫ darkgray | idle / 부재(–) / 라벨 |
| vendor색 | GPU=green · **RBLN=magenta** · RNGD=cyan (가속기 종류 식별) |
| 글리프 | `●`up/healthy · `○`idle/scaled-0 · `◐`pending · `⚠`throttle · `✗`down |
| 임계값 | util >85🔴 >60🟡 · mem >90🔴 >70🟡 · temp >75🔴 >60🟡 |

- **2초 자동 갱신**, 가속기 util 히스토리 스파크라인
- **액션**: `s` = 선택 모델 scale up/down (꺼진 모델 켜기)
- 메트릭이 없으면(워크로드 off) `–`/`offline` 로 우아하게 표시 — 워크로드가 뜨면 자동으로 채워짐

## 설치

### 사전 요구
- **Rust** 툴체인 (`rustup`) + **C 링커**(`gcc`/`cc`) — Rust 링킹에 필요
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  sudo apt-get install -y build-essential      # gcc/cc 링커
  ```
- 런타임: `kubectl`(kubeconfig 접근) + Prometheus 도달성. 가속기 노드 SSH 불필요(Prometheus 경유).

### 빌드 & 설치
```bash
git clone <this-repo> lmd-top && cd lmd-top
cargo install --path .        # → ~/.cargo/bin/lmd-top (PATH 에 있으면 어디서든 실행)
# 또는 빌드만:
cargo build --release         # → target/release/lmd-top
```

## 사용법

```bash
lmd-top                       # TUI 실행
lmd-top --snapshot            # 1회 수집 후 텍스트 출력 (헤드리스/디버그)
lmd-top --render              # TestBackend 로 전 뷰를 텍스트 렌더 (CI/검증)

# 다른 클러스터/네임스페이스
LMD_PROM=192.0.2.3:30090 LMD_NS=my-ns lmd-top
```

| 키 | 동작 |
|---|---|
| `0`–`6` | 뷰 전환 |
| `Tab` | 다음 뷰 |
| `↑`/`↓` (또는 `k`/`j`) | 행 선택 |
| `Enter` | 선택 항목 **상세(drill-down)** — 가속기/모델/파드 |
| `o` | **정렬** 순환 (Accel: util/temp/mem/name · Models: name/status/ready · Pods: name/phase/restarts) |
| `s` | 선택 모델 scale (desired 0↔1 토글) |
| `Esc` | 상세 닫기 / (상세 아닐 때) 종료 |
| `q` | 종료 |

> 표시 컬럼/색 커스터마이징(설정 파일)은 로드맵(Phase 3) — 현재는 합리적 기본값.

### 환경 변수
| 변수 | 기본값 | 의미 |
|---|---|---|
| `LMD_PROM` | `192.0.2.5:30090` | Prometheus `host:port` (평문 HTTP) |
| `LMD_NS` | `llm-serving` | 대상 네임스페이스 |

## 메트릭 데이터 경로

lmd-top 은 데이터를 **소유하지 않고** 기존 스택을 읽어 상관·표시합니다.

| 계층 | 소스 | 예시 메트릭/리소스 |
|---|---|---|
| Infra (가속기) | Prometheus | `furiosa_npu_*`, `RBLN_DEVICE_STATUS:*`, `DCGM_FI_DEV_*`, `node_*` |
| Model Server | Prometheus | `vllm:num_requests_running/waiting`, `vllm:generation_tokens_total` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| 토폴로지/상태/액션 | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool |

> **전체 메트릭을 보려면** RBLN/EPP 의 exporter·ServiceMonitor 가 필요할 수 있습니다(Furiosa 는 기본 가동).
> 동반 셋업 레포(`llm-d-setup`)의 `manifests/epp-servicemonitor.yaml`,
> `manifests/rbln-metrics-servicemonitor.yaml` 참고. (향후 `lmd-top setup` 이 자동화 예정 — 로드맵)

## 아키텍처

```
 kubectl ─┐                                          ┌─ Overview ─┐
 Prom    ─┤→ collectors → metric bus(Snapshot) → panels ┤  Accel … │ → ratatui → 터미널
 (cm)    ─┘   (데이터 IN, 단방향)        (렌더 OUT)    └─ Pods ─────┘
```
- **순수 Rust** (C 라이브러리 의존 회피): Prometheus 는 `tokio` TCP 로 HTTP/1.0 직접 질의(TLS 불요),
  K8s 는 `kubectl` 셸링. → 단일 정적 바이너리, 무거운 TLS/HTTP 크레이트 불필요.
- `collectors` 는 Snapshot 에 **쓰기만**, `panels` 는 **읽기만** — 새 데이터=collector 추가, 새 화면=panel 추가.
- 의존: `ratatui` `tokio` `serde`/`serde_json`/`serde_yaml` `anyhow`.

소스 구조:
```
src/
  main.rs      진입점 · 이벤트 루프 · --snapshot/--render
  collect.rs   Snapshot 타입 + prom/kube 수집
  prom.rs      순수 tokio HTTP/1.0 Prometheus 클라이언트
  kube.rs      kubectl 셸링 + scale 액션
  app.rs       UI 상태(뷰/선택/스파크라인 히스토리)
  ui.rs        ratatui 렌더(헤더/탭/뷰별/footer)
```

## 로드맵

- **Phase 1 — Monitor** ✅ (현재) — 6뷰 관측 + scale 액션
- **Phase 2 — Launch** — 모델 카탈로그 → 배치 솔버 → **llm-d ModelService** 렌더로 배포 + 런북
- **Phase 3 — EPP Deep + Plugins** — 라우팅 결정 분포/큐 히트맵 + 선언적 TOML collector
- **Phase 4 — 심화** — Request Lifecycle(트레이스) + P/D 분리 뷰 + `lmd-top setup`/`doctor`

상세 설계: 동반 레포 `llm-d-setup` 의 `lmd-top-DESIGN.md`(명제·차별점·4계층·플러그인)와
`lmd-top-PLAN.md`(마일스톤·모듈설계·목업) 참조.

## 상태

Phase 1 동작 검증 완료(실 클러스터: 5 nodes, 12 accelerators, EPP/routes/models 라이브).
실험적 프로젝트 — 인터페이스는 변경될 수 있습니다.
