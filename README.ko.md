# lmd-top

> **[llm-d](https://llm-d.ai) 클러스터를 위한 터미널 관측·운영 도구.**
> 서빙 스택 전체 — Gateway, EPP 라우팅, 모델 서버, 이종 가속기 — 를 한 화면, 하나의 정적 바이너리로.

[English](README.md) · **한국어**

![Rust](https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white)
![single static binary](https://img.shields.io/badge/single%20static%20binary-no%20C%20deps-success)
![for llm-d](https://img.shields.io/badge/for-llm--d-8839ef)
![views](https://img.shields.io/badge/correlated%20views-10-89b4fa)

`lmd-top` 은 llm-d 서빙 스택의 **네 계층을 상관지어** 보여줍니다 —
`Gateway → EPP(Endpoint Picker) → 모델 서버 → 인프라` — **이종 가속기**
(NVIDIA GPU · Rebellions RBLN · Furiosa RNGD · 호스트 CPU) 환경을 위해.
기존 Prometheus + Kubernetes 를 읽을 뿐, 자체 데이터는 소유하지 않습니다.

## 데모

*Demo recording omitted from the public history.*

*Demo recording omitted from the public export.*

```
⠙ lmd-top [observe]   llm-d · 8 nodes   ⌂ gw 192.0.2.6 ●   · updated 2s ago
● SERVING 5/11   req/s 6.2  TTFT 92ms  E2E 0.8s  │ accel 9/14 busy  VRAM 67%  ⚡409W  ⚠1 alert
⇥  0:Overview  1:Accel  2:Models  3:EPP  4:Flow  5:Pods  6:Perf  7:Launch  8:Events  9:Nodes
╭ Cluster ───────────────────────────────────────────────────────────────────────────────╮
│ 14 accel  GB10×2 RBLN×4 RNGD×8 │ util 41% temp 52°C │ VRAM 489/735GB 67% ⚡409W │ 5/11    │
│ VRAM  █████│████│████│████│████│████│█░░░│░░░░│░░░░│░░  489/735GB used                     │
│ GB10 ● ●   RBLN ● ● ● ●   RNGD ● ● ● ● ● ● ● ●                                             │
╰───────────────────────────────────────────────────────────────────────────────────────────╯
╭ Status ──────────────────────────────────────────────────────────────────────────────────╮
│ ● 5 models serving, accelerators have headroom                                            │
╰───────────────────────────────────────────────────────────────────────────────────────────╯
╭ Accelerators (by kind / node) ────────────────────────────────────────────────────────────╮
│ ● GB10×1 @gb10-node-1   ●●●●●○○○○○  47%  mem ●●●●●●●●●●  124/131GB  trend ▁▂▄▅▆▅▄▃            │
│ ● RBLN×4 @npu-node-1     ●●●○○○○○○○  31%  mem ●●●●●●●●··   54/ 68GB  trend ▂▃▃▄▃▂▁▁            │
╰───────────────────────────────────────────────────────────────────────────────────────────╯
 ↑↓ sel  ⏎ detail  / filter  o sort  l logs  t theme  f anim  z zoom  ? help  q quit
```

---

## 왜 lmd-top 인가?

llm-d 클러스터를 위한 **실시간·운영자 관점의 터미널 뷰**입니다. 서빙 네 계층
(Gateway → EPP(Endpoint Picker) → 모델 서버 → 가속기)을 **상관지어** 보여주고,
**EPP 라우팅 결정을 관측·설명**합니다. 터미널을 벗어나지 않고 *어떤 모델이 어디서 돌고,
요청이 어떻게 라우팅되며, 부하가 어떻게 분산되는지* 답할 수 있습니다.

---

## 하이라이트

- **네 계층, 한 화면.** Gateway · EPP/InferencePool · 모델 서버 · 하드웨어를 상관지어
  *"어떤 모델이 어디서 돌고, 요청이 어떻게 라우팅되며, 부하가 어떻게 분산되는가"* 에 답합니다.
- **이종 가속기, 통합 뷰.** NVIDIA GPU(`DCGM_*`), Rebellions RBLN(`RBLN_DEVICE_STATUS:*`),
  Furiosa RNGD(`furiosa_npu_*`)를 나란히 표시 — 벤더는 색, 상태는 글리프로 구분. 정확한 GPU
  모델(A100 / GB10 / H100 …)과 총 VRAM 을 DCGM(`modelName` / `FB_TOTAL`)에서 **자동 감지**
  (하드코딩 아님). **통합 메모리**(GB10 / GH200 / GB200)를 인식해 호스트 공유 풀 기준으로
  표시하고 `∪` 로 표기.
- **EPP 인지.** EPP `ConfigMap`(활성 scorer·가중치·picker)을 파악하고, 라우팅 결정과 파드별
  큐를 시각화하며, **HTTPRoute 가 InferencePool(EPP)을 경유하는지 우회하는지 자동 진단**
  (EPP 지표가 비는 흔한 오설정).
- **풍부한 가속기 시각화.** 디바이스별 게이지, 인라인 스파크라인, braille **영역-채움 타임라인**,
  한눈에 보는 **LED 디바이스 그리드**, 벤더별 **스택형 VRAM 구성 바**.
- **능동 알림.** 임계/헬스 조건(throttle, not-alive, hot, 노드 NotReady/cordon/pressure,
  파드 재시작/Failed)이 요약 바 플래시 + 토스트를 띄우고 **알림 히스토리**(`A`)에 모입니다.
- **운영 편의.** 스크롤바·위치 카운터가 있는 행 선택, 부분일치 필터, 정렬, 드릴다운 상세,
  파드/모델 **로그 오버레이**, `scale` 액션, **데이터 신선도 시계**, 반응형 탭, 활성 패널 포커스
  강조, **zoom/focus** 모드, 은은한 **애니메이션**(`f` 토글), 4개 테마 —
  **soft(Catppuccin, 기본)** / classic / high-contrast / **색맹 안전**.
- **순수 Rust, 단일 정적 바이너리.** TLS/무거운 HTTP 크레이트 없음: Prometheus 는 raw `tokio`
  HTTP/1.0 로, Kubernetes 는 `kubectl` 로 조회. GPU 노드에 설치할 것이 없습니다.

---

## 뷰

상단 숫자키(`0`–`9`) 또는 `Tab` 으로 전환하는 10개의 상관 뷰:

| # | 뷰 | 내용 |
|---|---|---|
| **0** | **Overview** | 클러스터 Σ 요약 · **LED 디바이스 그리드** · **VRAM 구성 바** · 종류/노드별 가속기 · EPP 경로 & 풀 · 모델 표 · **한 줄 계층-간 진단** |
| 1 | **Accel** | 디바이스별 행: util 바 / VRAM / 온도 / 전력 + 인라인 util 추세. GPU · RBLN · RNGD 통합. `⏎` → 전체 util/VRAM 타임라인 |
| 2 | **Models** | 모델별 가속기/노드 · ready · running/waiting · KV% · tok/s · 라우팅 경로 · 상태 |
| 3 | **EPP** | 활성 scorer·가중치(ConfigMap 파악) + picker + InferencePool 엔드포인트 + **요청 분배**(라우팅 결정) |
| 4 | **Flow** | **토폴로지 한눈에** — Gateway → HTTPRoute → backend(모델 상태/가속기/노드) + InferencePool/EPP/**SLO**(InferenceObjective) + autoscaler + **EPP 우회 진단**. `⏎` → backend 모델 상세 |
| 5 | **Pods** | `llm-serving` 파드 상태(ready / phase / node / restarts) |
| 6 | **Perf** | **EPP 정책 튜닝용** — 시스템 타임라인 + 모델별 p95 지연 분해 **QUEUE → PREFILL(P) → DECODE(D) → TPOT → E2E** + preemption, tok/s, 파드별 큐 분배. *서빙 중(active) 모델만, `o` 로 정렬.* |
| 7 | **Launch** | 모델 **카탈로그** × 실시간 가속기 재고 → 배치 가능성(`✓` ready / `⚙` 아티팩트 필요 / `✗` 용량부족). 읽기전용; 카탈로그 = `catalog/models.yaml` |
| 8 | **Events** | k8s + llm-d 이벤트 통합(최신순), 경고 강조. `⏎` → 전체 메시지 상세 |
| 9 | **Nodes** | 노드 헬스/배치 — 상태 · kubelet · CPU · load · 메모리 · 노드별 가속기. `⏎` 후 `↑↓` 로 디바이스 선택 → 해당 디바이스 히스토리 |

> **Flow** 는 *각 모델이 어디서 돌고, 어떻게 라우팅되며, 트래픽이 실제로 EPP 를 지나는가?* 에
> 답합니다. **Perf** 는 EPP scorer 정책 설계에 필요한 지연/토큰/분배 신호를 모읍니다
> (EPP 경로 트래픽 + vLLM 지표가 있어야 채워짐).

---

## 설치

### 사전 요구사항

**빌드** (감사 완료 — 바이너리는 glibc 만 링크하며 **네이티브/C 라이브러리 의존이 없음**,
OpenSSL/pkg-config/cmake 불필요):

- **Rust** 툴체인(`rustup`) + **C 링커**(`gcc`/`cc`, libc 링크용):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  sudo apt-get install -y build-essential      # cc/gcc 링커 제공
  ```
  첫 빌드 때 crates.io 에서 Rust 크레이트를 받습니다(1회 네트워크 필요). 이후엔 오프라인.

**런타임:**

- `PATH` 에 `kubectl` + kubeconfig 접근 권한(토폴로지 / 상태 / `scale` 액션).
- **Prometheus** 네트워크 접근(지표). **가속기 노드 SSH 불필요** — 모두 Prometheus 경유.
- **truecolor**(24bit) 지원 터미널 + **box-drawing·braille** 글리프를 포함한 고정폭 폰트
  (대부분의 최신 폰트 / Nerd Font; 예: DejaVu Sans Mono). soft 테마 + 타임라인 그래프에 필요 —
  아니면 `LMD_THEME=default` 로 전환(그래도 일부 글리프는 빈칸일 수 있음).
- *선택:* `xdg-open` — `g` 키(브라우저로 Grafana 열기)에만 사용; 없어도 무해.

그 외(Prometheus HTTP 클라이언트, 렌더링, 애니메이션)는 전부 단일 바이너리 안의 순수 Rust.

### 빌드 & 설치

```bash
git clone https://github.com/gspark-etri/lmd-top.git && cd lmd-top

./install.sh          # 누락된 사전요구사항(Rust, cc) 설치 후 `cargo install`
#   ./install.sh --check       # 무엇이 있고 없는지만 리포트(설치 안 함)
*Demo recording omitted from the public export.*
```

또는 수동으로 (Rust 크레이트 의존성은 `cargo` 가 자동으로 받으므로 수동 설치할 것 없음):

```bash
cargo install --path .        # → ~/.cargo/bin/lmd-top
cargo build --release         # 빌드만 → target/release/lmd-top
```

---

## 사용법

```bash
lmd-top                       # TUI 실행 (권한 모드: observe)
lmd-top --mode admin          # scale/rollout 액션 허용 (권한 모드 참고)
lmd-top --snapshot            # 1회 수집 후 텍스트 출력 (헤드리스 / 디버그)   [별칭: -s]
lmd-top --json                # 1회 수집 후 기계가독 agent 상태(JSON) 출력
lmd-top --doctor              # Prometheus 전수조사: exporter, 지표 커버리지, 갭, 새 신호
lmd-top --render              # 모든 뷰를 TestBackend 로 텍스트 렌더 (CI / 검증)
lmd-top --cast                # 데모 asciicast 생성 (→ agg 로 GIF 변환)

# 다른 클러스터 / 네임스페이스 지정
LMD_PROM=192.0.2.3:30090 LMD_NS=my-ns lmd-top
```

### 권한 모드

변경(mutating) 액션은 시작 모드(`--mode observe|debug|admin|danger`, 기본 `observe`)로
게이팅되며 헤더에 배지로 표시됩니다. 공유 클러스터에서의 실수를 방지합니다.

| 모드 | 허용 | 게이팅 키 |
|---|---|---|
| **observe** *(기본)* | 보기 전용 | — |
| **debug** | + 로그 / dry-run | `l` |
| **admin** | + scale / rollout | `s` |
| **danger** | + delete / force | *(예정)* |

admin+ 변경 액션(예: `s` scale)은 적용 전 `y`/`n` 확인을 받습니다.

### 키 바인딩

| 키 | 동작 |
|---|---|
| `0`–`9` | 뷰 전환 |
| `Tab` | 다음 뷰 |
| `↑`/`↓` (또는 `k`/`j`), 마우스 스크롤 | 행 선택 |
| `Enter` | **드릴다운 상세** (accel · model · pod · node · event; **Flow** → backend 모델; **Perf** → p50/p95/p99 + 타임라인) |
| `←`/`→` | 이전 / 다음 항목 (노드 상세에선 `↑`/`↓` 가 디바이스 선택) |
| `/` | **필터**(부분일치) — 입력 후 Enter/Esc |
| `o` | **정렬** 순환 (Accel: util/temp/mem/name · Models: name/status/ready · Pods · Perf: tok/s/E2E/TTFT/queue/name) |
| `l` | 선택 파드/모델 **로그** 오버레이 (스크롤, `r` 새로고침) |
| `s` | 선택 모델 **scale** (desired 0↔1 토글) |
| `A` | **알림 히스토리** 오버레이 (임계 / 헬스 이벤트) |
| `t` | **테마** 순환 (soft / classic / high-contrast / 색맹안전) |
| `f` | **애니메이션** on/off 토글 |
| `g` | 브라우저로 **Grafana** 열기 |
| `z` | **zoom / focus** (헤더+탭 숨기고 본문 최대화) |
| `Space` | 갱신 **일시정지** (읽기용으로 데이터 고정) |
| `Esc` | **뒤로만** (상세 / 필터 / zoom 닫기 — 종료 아님) |
| `?` | 도움말 / 색 범례 오버레이 |
| `q` | 종료 |

### 의미 색상 & 글리프

색은 **심각도** 또는 **정체성**을 인코딩하고, 상태는 별도 **글리프**로 표현합니다
(둘이 충돌하지 않고 색맹 테마에서도 읽힘):

| 요소 | 의미 |
|---|---|
| 🟢 초록 | 정상 / 저부하 / 서빙 중 |
| 🟡 노랑 | 경고 / 중부하 / 대기 / throttling |
| 🔴 빨강 | 위험 / 고부하 / 오류 / 디바이스 다운 / **활성 알림** |
| 🔵 청록 | 강조 / 헤더 / 상호작용 값 |
| ⚫ 진회색 | 유휴 / 없음(`–`) / 라벨 |
| 벤더 색 | GPU · RBLN · RNGD 를 서로 다른 색으로(테마별로 팔레트 조화) |
| 글리프 | `●` 정상 · `○` 유휴/scaled-0 · `◐` 대기 · `⚠` throttle · `⊘` cordon · `✗` 다운 |
| 임계치 | util `>85`🔴 `>60`🟡 · mem `>90`🔴 `>70`🟡 · temp `>80`🔴 `>60`🟡 |

아직 없는 지표(워크로드 꺼짐)는 `–`/`offline` 로 표시되고 워크로드가 뜨면 자동으로 채워집니다.
헤더의 **신선도 시계**(`updated Ns ago`)는 데이터가 오래되면 노랑으로 바뀝니다.

### 설정 (선택) — `~/.config/lmd-top/lmd-top.yaml`

컬럼 표시/순서 커스터마이즈:

```yaml
columns:
  models: [name, accel, status, tps]   # 이 컬럼만, 이 순서로 (기본: 전체)
```

### 환경 변수

| 변수 | 기본값 | 의미 |
|---|---|---|
| `LMD_PROM` | `192.0.2.5:30090` | Prometheus `host:port` (평문 HTTP) |
| `LMD_NS` | `llm-serving` | 대상 네임스페이스 |
| `LMD_GRAFANA` | `http://192.0.2.5:30300` | `g` 가 여는 Grafana base URL |
| `LMD_THEME` | `soft` | 시작 테마: `soft` / `default` / `high-contrast` / `colorblind` (또는 `0`–`3`) |
| `LMD_W` / `LMD_H` | `100` / `26` | `--render` 렌더 크기 |

---

## 데이터 경로

lmd-top 은 **자체 데이터를 소유하지 않고** 기존 스택을 읽어 상관짓습니다.

| 계층 | 소스 | 예시 지표 / 리소스 |
|---|---|---|
| 인프라(가속기) | Prometheus | `furiosa_npu_*`, `RBLN_DEVICE_STATUS:*`, `DCGM_FI_DEV_*`, `node_*` |
| 모델 서버 | Prometheus | `vllm:num_requests_running/waiting`, `vllm:*_latency_seconds_bucket`, `vllm:generation_tokens_total`, `vllm:kv_cache_usage_perc` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| 토폴로지 / 상태 / 액션 | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool, InferenceObjective |

데이터는 두 계층으로 들어옵니다: **~1초 fast tier**(가속기 + 노드)와 **~3초 full 스냅샷**
(그 외 전부). 모델별 perf 는 vLLM 지표를 `service` 라벨(= Deployment 이름)로 조인하는데,
이는 Models 뷰가 쓰는 키와 동일합니다.

> **모든 지표를 보려면** 일부 exporter/ServiceMonitor 가 필요할 수 있습니다(RBLN·EPP;
> Furiosa 는 기본 on). 동반 설정 리포(`llm-d-setup`) 참고:
> `manifests/epp-servicemonitor.yaml`, `manifests/rbln-metrics-servicemonitor.yaml`.

---

## 아키텍처

```
 kubectl ─┐                                              ┌─ Overview ─┐
 Prom    ─┤→ collectors → Snapshot (metric bus) → panels ┤  Accel …   │ → ratatui → terminal
 (cm)    ─┘   (데이터 IN, 단방향)          (렌더 OUT)     └─ Nodes ────┘
```

- **순수 Rust, C 라이브러리 의존 없음.** Prometheus 는 `tokio` TCP(HTTP/1.0, TLS 없음)로 직접
  조회; Kubernetes 는 `kubectl` 셸아웃. 결과: 무거운 TLS/HTTP 크레이트 없는 단일 정적 바이너리.
- `collectors` 는 `Snapshot` 에 **쓰기만**, `panels` 는 **읽기만**. 새 데이터 = collector 추가,
  새 화면 = panel 추가.
- 의존성: `ratatui`, `tokio`, `serde`/`serde_json`/`serde_yaml`, `anyhow`, `unicode-width`,
  `tachyonfx`(애니메이션).

```
src/
  main.rs      진입점 · 이벤트 루프 · --snapshot/--json/--doctor/--render/--cast
  collect.rs   Snapshot 타입 + prom/kube 수집          config.rs   env/yaml 설정
  prom.rs      순수-tokio HTTP/1.0 Prometheus 클라이언트  metrics.rs  지표명 레지스트리
  kube.rs      kubectl 셸아웃 + scale 액션              catalog.rs  Launch 모델 카탈로그
  app.rs       UI 상태 (뷰 / 선택 / 히스토리 / 알림 / 권한모드)
  agent.rs     --json agent 상태    doctor.rs   --doctor 조사    cast.rs   --cast 데모
  ui/          mod.rs(뷰) · theme.rs(팔레트) · widgets.rs · panel.rs · fx.rs(애니메이션)
```

---

## 현황 & 로드맵 — 지금 되는 것

### ✅ 지금 됨 (트래픽 불필요)
- **10개 뷰** 전부 — 내비게이션·필터·정렬·드릴다운 상세(모델 상세의 **pivot 미리보기**,
  노드 상세의 디바이스별 히스토리, Enter 로 여는 이벤트 전체 메시지 포함).
- **가속기 모니터링** — NVIDIA GPU / Rebellions RBLN / Furiosa RNGD 나란히; GPU 모델+VRAM
  DCGM 에서 **자동 감지**; **통합 메모리** `∪` 표기. LED 그리드, 스택 VRAM 바, 타임라인, 스파크라인.
- **노드 모니터링** — 상태 / kubelet / CPU / load / 메모리 + 노드별 디바이스.
- **토폴로지(Flow)** — Gateway → HTTPRoute → backend → 파드, InferencePool/EPP,
  **EPP 우회 진단**(HTTPRoute 가 InferencePool 대신 Service 를 가리킴).
- **EPP 파악** — ConfigMap 에서 활성 scorer·가중치·picker.
- **능동 알림** (throttle / not-alive / hot / 노드 NotReady·cordon·pressure / 파드 재시작·Failed)
  + 플래시·토스트·**알림 히스토리**(`A`).
- **액션**: 모델 `scale`(admin 모드, `y/n` 확인); **로그** 오버레이(debug 모드).
- **헤드리스 / 에이전트**: `--snapshot`, `--json`(agent 상태), `--doctor`(지표 커버리지 조사),
  `--render`, `--cast`(데모).
- **UX**: 4개 테마, 은은한 애니메이션(`f`), zoom(`z`), 일시정지, 신선도 시계, 권한 모드, Grafana 열기(`g`).

### 🟡 워크로드 + EPP 경로가 살아있어야 됨 (지표 의존)
실제 요청이 **InferencePool/EPP 를 경유**하고 vLLM 이 지표를 노출해야 채워지며, 그 전엔 `–`/"no data":
- **모델별 성능**(Perf) — p95 지연 분해 QUEUE → PREFILL → DECODE → TPOT → E2E, tok/s,
  preemption, 파드별 큐 분배.
- **EPP 요청 분배** — 파드별 라우팅 결정 점유율(EPP 경로 트래픽 필요).
- Models/Overview 의 **KV cache %, TTFT / E2E, running/waiting**.
- **EPP 가중치 what-if**(`+`/`-`)는 **가중치 점유율의 로컬 시뮬레이션일 뿐** — 클러스터에
  적용되거나 실제 라우팅을 재실행하지 않습니다.

### 🔴 아직 안 됨 (예정)
- scale 외 **적용형 컨트롤플레인 액션** — 엔드포인트 **drain**, **트래픽/정책-가중치 적용**,
  **rollout** (dry-run → 확인 → 감사). *(danger 모드 delete/force 는 예약.)*
- **EPP 결정 디버거** — 엔드포인트별 `Filter→Score→Pick` 점수 표(엔드포인트별 스코어링 지표 필요).
- **PD 인지 대시보드**, KV/prefix 캐시 지역성, SLO/goodput 진단.
- **ModelService 네이티브** — Launch 는 현재 **읽기전용**(가능성 판정만); 카탈로그에서 실제 배포는
  llm-d ModelService CRD 연동 후.

상세 계획은 `ROADMAP.md`, 릴리스 이력은 `CHANGELOG.md` 참고.

## 성숙도

실제 이종 클러스터에서 검증됨(8 노드; GB10 · RBLN · RNGD 가속기; EPP / 라우트 / 모델 라이브).
실험적(0.x) — 인터페이스는 바뀔 수 있습니다.
