# lmd-top

> **[llm-d](https://llm-d.ai) 클러스터를 위한 터미널 관측·운영 도구.**
> 서빙 스택 전체 — Gateway, EPP 라우팅, 모델 서버, 이종 가속기 — 를 하나의 정적 바이너리로, 한 화면에서 봅니다.

[English](README.md) · **한국어**

[![release](https://img.shields.io/github/v/release/gspark-etri/lmd-top?logo=github)](https://github.com/gspark-etri/lmd-top/releases/latest)
[![license](https://img.shields.io/github/license/gspark-etri/lmd-top)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-000?logo=rust&logoColor=white)

`lmd-top` 은 llm-d 서빙 스택의 네 계층 — Gateway → EPP(Endpoint Picker) → 모델 서버 → 인프라 —
을 이종 가속기(NVIDIA GPU, Rebellions RBLN, Furiosa RNGD, 호스트 CPU) 환경에서 하나로 엮어
보여줍니다. 기존 Prometheus 와 Kubernetes 를 읽기만 할 뿐, 자체 데이터는 갖지 않습니다.

## 데모

*Demo recording omitted from the public history.*

*Demo recording omitted from the public export.*

## 주요 기능

- **네 계층을 한 화면에.** Gateway, EPP/InferencePool, 모델 서버, 하드웨어를 서로 엮어 보여주므로, 도구를 옮겨 다니지 않고도 *어떤 모델이 어디서 돌고, 요청이 어떻게 라우팅되며, 부하가 어떻게 분산되는지* 알 수 있습니다.
- **이종 가속기를 통합해서.** NVIDIA GPU, Rebellions RBLN, Furiosa RNGD 를 나란히 표시합니다. GPU 모델과 VRAM 은 자동으로 감지하고, 통합 메모리(GB10, GH200) 는 `∪` 로 표시하며, 노드별 디스크 사용량도 함께 봅니다.
- **EPP 를 이해합니다.** EPP `ConfigMap`(활성 scorer, 가중치, picker) 을 읽어 라우팅 결정과 파드별 큐를 시각화하고, HTTPRoute 가 실제로 InferencePool 을 거치는지 아니면 우회하는지를 진단합니다.
- **배포 라이프사이클.** Deploy 뷰는 모델별 컴파일 변형(텐서/파이프라인 병렬, 양자화, RBLN/Furiosa NPU 옵션) 을 묶어 보여 주고, 어느 노드·디스크에 저장돼 있는지, 그리고 어디에 여유 용량이 있어 배치할 수 있는지를 알려줍니다.
- **풍부한 터미널 UI.** LED 디바이스 그리드, 스택형 VRAM 바, braille 타임라인, 능동 알림, 로그 조회, `scale` 액션을 제공하고, 네 가지 테마와 은은한 애니메이션을 갖췄습니다. 이 모든 것이 C 의존성 없는 단일 정적 Rust 바이너리 하나에 들어 있습니다.

## 뷰

숫자키 `0`–`7` 로 전환하거나 `Tab` / `Shift+Tab` 으로 순환합니다.

| # | 뷰 | 내용 |
|---|---|---|
| 0 | **Overview** | 클러스터 요약, LED 그리드, VRAM 바, 종류/노드별 가속기, EPP 경로, 모델, 한 줄 진단 |
| 1 | **Nodes** 허브 | 노드 헬스(CPU/메모리/디스크/load + 디바이스). `w` 로 허브 순환: **노드 → 디바이스**(util/VRAM/온도/전력) **→ 서빙**(모델별 p95 QUEUE→PREFILL→DECODE→TPOT→E2E, tok/s, + SLO advisor) **→ 맵**(Canvas 토폴로지: Gateway→EPP→Pool 흐름 + 디바이스 pressure 히트맵) |
| 2 | **Models** | 모델별 가속기/노드, ready, running/waiting, KV%, tok/s, 경로, 상태. `⏎` 액션 메뉴(Info/Compile/Deploy/Stop/Scale/Restart/Logs/YAML/Objective) |
| 3 | **EPP** | scorer 와 가중치, picker, InferencePool 엔드포인트, 요청 분배 |
| 4 | **Flow** | Gateway → HTTPRoute → backend → 파드, InferencePool/EPP/SLO 와 EPP 우회 진단. `⏎` 로 backend 모델 이동 |
| 5 | **Pods** | `llm-serving` 파드(ready / phase / node / restarts). `⏎` 액션(Info/Logs/YAML/Delete) |
| 6 | **Deploy** | 모델 라이프사이클 — 컴파일 변형(계열→build·옵션·`@노드 /경로`), 노드별 디바이스 점유, 카탈로그 배치 가능성. `⏎` 로 선택 대상에 액션(Compile→RBLN/Furiosa·Deploy·노드 Cordon 등) |
| 7 | **Events** | Kubernetes + llm-d 이벤트(최신순). `⏎` 로 전체 메시지 |

리스트 헤더에 보이는 행(전체 또는 필터된 것)의 `Σ` 통합 메트릭이 표시됩니다. `y` 로 선택 리소스의 live YAML(읽기전용)을 봅니다.

## 설치

**빌드된 바이너리** (Linux x86_64):

```bash
VER=v0.32.0   # 최신 버전: https://github.com/gspark-etri/lmd-top/releases/latest
curl -fsSL "https://github.com/gspark-etri/lmd-top/releases/download/$VER/lmd-top-$VER-x86_64-linux.tar.gz" | tar xz
sudo install -m 0755 lmd-top /usr/local/bin/
```

릴리스마다 `.sha256` 체크섬이 함께 게시됩니다.

**소스 빌드** (Rust 툴체인과 C 링커 `cc`/`gcc` 만 있으면 됩니다):

```bash
git clone https://github.com/gspark-etri/lmd-top.git && cd lmd-top
./install.sh                 # 누락된 사전 요구사항을 설치한 뒤 `cargo install` 실행
#   ./install.sh --check     # 무엇이 있고 없는지만 확인(설치는 안 함)
#   ./install.sh --with-demo # agg 도 설치하고 데모 GIF 까지 생성
# 수동 설치: cargo install --path .
```

**실행 요구사항:**

- kubeconfig 접근 권한이 있는 `kubectl`, Prometheus 로의 네트워크 접근. 가속기 노드에 SSH 하지 않습니다.
- truecolor 를 지원하고 box-drawing·braille 글리프를 포함한 폰트를 쓰는 터미널 권장. 아니면 `LMD_THEME=default` 로 실행하세요.
- 바이너리는 glibc 만 링크합니다 — OpenSSL, pkg-config, cmake 불필요. `xdg-open` 은 `g` 키에만 쓰이는 선택 사항입니다.

## 사용법

```bash
lmd-top                      # TUI 실행 (권한 모드: observe)
lmd-top --mode admin         # scale / rollout 액션 허용
lmd-top --json               # 기계가 읽는 agent 상태(JSON) 출력
lmd-top --doctor             # Prometheus 전수조사: exporter, 지표 커버리지, 누락
lmd-top --audit              # 적용한 변경 작업 감사 로그 출력
lmd-top --snapshot | --render | --cast   # 헤드리스 텍스트 / CI 렌더 / 데모 asciicast
LMD_PROM=192.0.2.3:30090 LMD_NS=my-ns lmd-top   # 다른 클러스터 지정
```

**권한 모드**(`--mode`, 헤더에 배지로 표시) 는 변경 액션을 단계적으로 잠급니다.
`observe`(기본, 보기 전용) → `debug`(로그 `l` 추가) → `admin`(`scale` 추가, y/n 확인) →
`danger`(예약). admin 액션은 적용 전에 항상 확인을 받습니다.
적용된 모든 변경 작업(scale·stop·restart·cordon·delete·route 편집·apply)은 시각·모드·작업·
대상·결과와 함께 **감사 로그**(`~/.config/lmd-top/audit.log`, 또는 `$LMD_AUDIT`)에 남습니다 —
`lmd-top --audit` 로 확인.

**키.**

| | |
|---|---|
| 이동 | `↑↓`/`kj` 선택 · `⏎` 액션 메뉴(또는 상세) · `w` Nodes 허브/패널 포커스 · `←→` 항목 · `p i r e m` 크로스레이어 pivot |
| 액션 | `/` 필터 · `:` 커맨드 팔레트(뷰/표시 액션 퍼지 점프) · `o`/`O` 컬럼 정렬(컬럼 순환 / ▲▼ 방향 토글) · `y` live YAML · `l` 로그 · 액션 메뉴 → Compile/Deploy/Scale/Restart/Stop/Delete/Cordon/Objective (admin 게이팅·y/n 확인) |
| 표시 | `t` 테마 · `f` 애니메이션 · `z` zoom · `Space` 일시정지 · `g` Grafana · `A` 알림 · `?` 도움말 · `q` 종료 |

**환경 변수.**

- `LMD_PROM`, `LMD_NS`(기본 `llm-serving`), `LMD_GRAFANA` — 대상 클러스터 지정.
- `LMD_THEME` — 시작 테마: `soft`, `default`, `high-contrast`, `colorblind`.
- `LMD_AUDIT` — 감사 로그 경로(기본: `~/.config/lmd-top/audit.log`).
- `LMD_COMPILE_IMAGE_RBLN`, `LMD_COMPILE_IMAGE_FURIOSA`, `LMD_SERVING_IMAGE` — 생성되는 compile/deploy 매니페스트의 컨테이너 이미지. 지정 전에는 `TODO-…` placeholder 라 앱 내 apply(`a`)가 막히고, `w` 로 저장해 직접 편집·적용은 가능.
- `LMD_SAVE_DIR` — `w` 저장 위치(기본: 현재 디렉터리).
- `LMD_W` / `LMD_H` — `--render` 크기.
- 선택 사항인 `~/.config/lmd-top/lmd-top.yaml` 로 컬럼 순서를 바꿀 수 있습니다.

**색과 글리프.** 색은 심각도나 정체성을 나타내고, 상태는 별도의 글리프(`●` 정상, `○` 유휴,
`◐` 대기, `⚠` throttle, `⊘` cordon, `✗` 다운) 로 표현합니다. 그래서 색맹 테마에서도 읽힙니다.
아직 없는 지표는 `–` 로 표시되고, 워크로드가 올라오면 자동으로 채워집니다.

## 데이터 경로

lmd-top 은 기존 스택을 읽어 엮을 뿐, 자체 데이터를 갖지 않습니다.

| 계층 | 소스 | 예시 |
|---|---|---|
| 가속기 / 호스트 | Prometheus | `DCGM_FI_DEV_*`, `RBLN_DEVICE_STATUS:*`, `furiosa_npu_*`, `node_*` |
| 모델 서버 | Prometheus | `vllm:*_latency_seconds_bucket`, `vllm:num_requests_*`, `vllm:*kv_cache*` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| 토폴로지 / 상태 / 액션 | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool, InferenceObjective |

데이터는 두 계층으로 들어옵니다. 가속기와 노드는 약 1초 주기의 fast tier 로, 나머지는 약 3초
주기의 full 스냅샷으로 갱신됩니다. 전부 순수 Rust 로, Prometheus 는 raw `tokio` HTTP/1.0 으로,
Kubernetes 는 `kubectl` 로 조회합니다.

## 현황 & 로드맵

**지금 바로 되는 것 (트래픽 불필요).** 10개 뷰 전부, 자동 감지·통합 메모리를 포함한 GPU/RBLN/RNGD
및 노드/디스크 모니터링, Flow 토폴로지와 EPP 우회 진단, EPP ConfigMap 조회, 능동 알림,
`scale`·`logs` 액션, Deploy 뷰(컴파일 변형·저장 노드·배치 타깃), 헤드리스 `--json`·`--doctor`·
`--snapshot`·`--cast`, 그리고 테마·애니메이션·zoom·권한 모드.

**실제 트래픽이 EPP 를 거치고 vLLM 이 지표를 내보내면 채워지는 것.** 모델별 p95 지연 분해, tok/s,
파드별 큐 분배, KV%/TTFT/E2E, EPP 요청 분배. (EPP 가중치 `+`/`-` 는 로컬 시뮬레이션이며, 클러스터에
실제로 적용되지는 않습니다.)

**예정된 것.** 적용형 컨트롤 플레인 액션(엔드포인트 drain, 트래픽/정책 가중치 적용, rollout —
각각 dry-run → 확인 → 감사), EPP 엔드포인트별 점수 디버거, 그리고 **NPU 컴파일·배포 자동화** —
Deploy 뷰에서 모델을 RBLN/Furiosa 용으로 컴파일하고(벤더 툴체인을 도는 Kubernetes Job) ModelService
로 배포까지, 권한 모드로 게이팅. 자세한 내용은 [ROADMAP.md](ROADMAP.md) 와 [CHANGELOG.md](CHANGELOG.md) 참고.

## 성숙도

실제 이종 클러스터에서 검증했습니다(8개 노드, GB10·RBLN·RNGD 가속기, EPP·라우트·모델 모두 라이브).
아직 실험 단계(0.x) 라 인터페이스는 바뀔 수 있습니다.

## 기여 & 라이선스

이슈와 PR 을 환영합니다 — [CONTRIBUTING.md](CONTRIBUTING.md) 를 참고하세요.
라이선스는 [Apache-2.0](LICENSE) 입니다.
