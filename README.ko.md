# lmd-top

> **[llm-d](https://llm-d.ai) 클러스터를 위한 터미널 관측·운영 도구.**
> 서빙 스택 전체 — Gateway, EPP 라우팅, 모델 서버, 이종 가속기 — 를 한 화면, 하나의 정적 바이너리로.

[English](README.md) · **한국어**

![Rust](https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white)
![single static binary](https://img.shields.io/badge/single%20static%20binary-no%20C%20deps-success)
![for llm-d](https://img.shields.io/badge/for-llm--d-8839ef)
![views](https://img.shields.io/badge/correlated%20views-10-89b4fa)

`lmd-top` 은 llm-d 서빙 스택의 네 계층 — `Gateway → EPP(Endpoint Picker) → 모델 서버 → 인프라`
— 을 상관지어 보여줍니다. **이종 가속기**(NVIDIA GPU · Rebellions RBLN · Furiosa RNGD · 호스트 CPU)
지원. 기존 Prometheus + Kubernetes 를 읽을 뿐 자체 데이터는 없습니다.

## 데모

*Demo recording omitted from the public history.*

*Demo recording omitted from the public export.*

## 하이라이트

- **네 계층, 한 화면** — Gateway·EPP/InferencePool·모델 서버·하드웨어 상관: *어떤 모델이 어디서 돌고, 어떻게 라우팅되며, 부하가 어떻게 분산되나.*
- **이종 가속기 통합** — GPU(`DCGM_*`)·RBLN(`RBLN_DEVICE_STATUS:*`)·RNGD(`furiosa_npu_*`) 나란히; GPU 모델·VRAM **자동 감지**; **통합 메모리**(GB10/GH200) `∪` 표기; 노드 **disk** 도.
- **EPP 인지** — EPP `ConfigMap`(scorer/가중치/picker) 파악, 라우팅 결정·파드별 큐 시각화, **HTTPRoute→InferencePool 경유 vs 우회 진단**.
- **배포 라이프사이클** — 모델별 **컴파일 변형**(TP/PP·양자화·RBLN/Furiosa NPU 옵션), 어느 **노드/disk** 에 있는지, 여유 용량 **배치 타깃**.
- **풍부한 TUI** — LED 그리드·스택 VRAM 바·braille 타임라인·능동 알림(`A`)·로그·`scale`·4개 테마·은은한 애니메이션(`f`)·zoom(`z`); **순수 Rust 단일 정적 바이너리**.

## 뷰

숫자키 `0`–`9` 또는 `Tab` 으로 전환.

| # | 뷰 | 내용 |
|---|---|---|
| 0 | **Overview** | 클러스터 Σ · LED 그리드 · VRAM 바 · 종류/노드별 가속기 · EPP 경로 · 모델 · 한 줄 진단 |
| 1 | **Accel** | 디바이스별 util/VRAM/온도/전력 + 추세; `⏎` → util/VRAM 타임라인 |
| 2 | **Models** | 모델별 가속기/노드 · ready · running/waiting · KV% · tok/s · 경로 · 상태 |
| 3 | **EPP** | scorer·가중치 + picker + InferencePool 엔드포인트 + 요청 분배 |
| 4 | **Flow** | Gateway → HTTPRoute → backend → 파드, InferencePool/EPP/SLO, **EPP 우회 진단**; `⏎` → backend 모델 |
| 5 | **Pods** | `llm-serving` 파드(ready/phase/node/restarts) |
| 6 | **Perf** | 디바이스별 히스토리 + 모델별 p95 **QUEUE→PREFILL→DECODE→TPOT→E2E**, tok/s, 큐; `⏎` → p50/95/99 + 타임라인 |
| 7 | **Deploy** | **모델 라이프사이클** — 컴파일 변형(계열→build: 옵션·`@node /path`·상태) · 배치 타깃(노드별 여유) · 카탈로그 가능성 |
| 8 | **Events** | k8s + llm-d 이벤트(최신순); `⏎` → 전체 메시지 |
| 9 | **Nodes** | 노드 헬스 · CPU · mem · **disk** · load · 노드별 디바이스; `⏎` 후 `↑↓` 로 디바이스 선택 |

## 설치

**사전요구**(감사 완료 — 바이너리는 glibc 만 링크, **네이티브/C 라이브러리 의존 없음**): Rust 툴체인 +
C 링커(`cc`/`gcc`). 런타임: `kubectl`(kubeconfig) + **Prometheus** 접근(가속기 노드 SSH 불필요).
**truecolor** + box-drawing/braille 폰트 터미널 권장(아니면 `LMD_THEME=default`). `xdg-open` 은 선택(`g` 키).

```bash
git clone https://github.com/gspark-etri/lmd-top.git && cd lmd-top
./install.sh                 # 누락 사전요구 설치 후 `cargo install`
#   ./install.sh --check     # 리포트만   ·   --with-demo  GIF 도 생성
# 수동: cargo install --path .   (Rust 크레이트는 cargo 가 자동으로 받음)
```

## 사용법

```bash
lmd-top                      # TUI (권한 모드: observe)
lmd-top --mode admin         # scale/rollout 액션 허용
lmd-top --json               # 기계가독 agent 상태(JSON)
lmd-top --doctor             # Prometheus 전수조사: exporter·지표 커버리지·갭
lmd-top --snapshot | --render | --cast   # 헤드리스 텍스트 · CI 렌더 · 데모 asciicast
LMD_PROM=192.0.2.3:30090 LMD_NS=my-ns lmd-top   # 다른 클러스터
```

**권한 모드**(`--mode`, 헤더 배지): `observe`(기본, 보기) → `debug`(+로그 `l`) → `admin`(+`scale`, y/n 확인)
→ `danger`(예약). **키:** `↑↓/kj` 선택 · `Enter` 드릴다운 · `←→` 이전/다음 · `/` 필터 · `o` 정렬 ·
`l` 로그 · `s` scale · `A` 알림 · `t` 테마 · `f` 애니메이션 · `z` zoom · `Space` 일시정지 · `g` Grafana · `?` 도움말 · `q` 종료.

**환경변수:** `LMD_PROM` · `LMD_NS`(`llm-serving`) · `LMD_GRAFANA` · `LMD_THEME`
(`soft`/`default`/`high-contrast`/`colorblind`) · `LMD_W`/`LMD_H`(렌더 크기).
선택 YAML `~/.config/lmd-top/lmd-top.yaml` 로 컬럼 순서 지정.

**색**은 심각도/정체성, 상태는 별도 글리프(`●` 정상 · `○` 유휴 · `◐` 대기 · `⚠` throttle · `⊘` cordon · `✗` 다운)
— 색맹 테마에서도 읽힘. 없는 지표는 `–`, 워크로드 뜨면 자동 채움.

## 데이터 경로

기존 스택을 읽음(자체 데이터 없음):

| 계층 | 소스 | 예시 |
|---|---|---|
| 가속기/호스트 | Prometheus | `DCGM_FI_DEV_*`, `RBLN_DEVICE_STATUS:*`, `furiosa_npu_*`, `node_*` |
| 모델 서버 | Prometheus | `vllm:*_latency_seconds_bucket`, `vllm:num_requests_*`, `vllm:*kv_cache*` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| 토폴로지/상태/액션 | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool, InferenceObjective |

2계층: ~1초 fast tier(가속기+노드) + ~3초 full 스냅샷. 순수 Rust — Prometheus 는 raw `tokio` HTTP/1.0, k8s 는 `kubectl`.

## 현황 & 로드맵

**✅ 지금 됨(트래픽 불필요):** 10개 뷰 · GPU/RBLN/RNGD + 노드/disk 모니터링(자동감지·통합메모리) · Flow 토폴로지 +
EPP 우회 진단 · EPP ConfigMap 파악 · 능동 알림 · `scale`/`logs` · Deploy 뷰(컴파일 변형·저장 노드·배치 타깃) ·
헤드리스 `--json`/`--doctor`/`--snapshot`/`--cast` · 테마/애니메이션/zoom/권한모드.

**🟡 EPP 경로 트래픽 + vLLM 지표 필요:** 모델별 p95 지연 분해, tok/s, 파드별 큐 분배, KV%/TTFT/E2E, EPP 요청 분배.
(EPP 가중치 `+`/`-` 는 로컬 시뮬레이션, 실제 적용 아님.)

**🔴 예정:** 적용형 컨트롤플레인 액션(엔드포인트 drain·트래픽/정책가중치 적용·rollout — dry-run→확인→감사) ·
EPP 엔드포인트별 점수 디버거 · **NPU 컴파일 & 배포 자동화** — Deploy 뷰에서 모델을 RBLN/Furiosa 용으로 컴파일
(벤더 툴체인 k8s Job)하고 ModelService 로 배포까지, 권한모드 게이팅. `ROADMAP.md`/`CHANGELOG.md` 참고.

## 성숙도

실제 이종 클러스터에서 검증(8 노드; GB10·RBLN·RNGD; EPP/라우트/모델 라이브). 실험적(0.x) — 인터페이스는 바뀔 수 있음.
