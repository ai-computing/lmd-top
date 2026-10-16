# lmd-top

설치·초기 설정·업데이트는 [Quick start](docs/QUICKSTART.md)를 참고하세요. `--init`, `--check-setup`은 현재 소스에 추가됐으며 다음 릴리스에 포함될 예정입니다.

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
- **생애주기 분리 — 올리기 vs 돌보기.** **Serving**(섹션 2)은 돌아가는 쪽: 배포를 정렬 가능한 표로 보고, pods 교차로 상태(서빙/시도 중/degraded/실패)를 판정해 "서빙 중인지·시도 중인지·실패인지"를 바로 보여주며 scale/restart/stop 을 제공. **Deploy**(섹션 4)는 올리는 쪽: 배포 가능한 것들의 **Model List**(카탈로그 가능성 + 스토어 빌드)와, compile Job·deploy rollout 을 상태·진행률과 함께 묶은 **Activity** 피드.
- **풍부한 터미널 UI.** LED 디바이스 그리드, 스택형 VRAM 바, braille 타임라인, 능동 알림, 로그 조회, `scale` 액션을 제공하고, 네 가지 테마와 은은한 애니메이션을 갖췄습니다. 이 모든 것이 C 의존성 없는 단일 정적 Rust 바이너리 하나에 들어 있습니다.

## 뷰

네비게이션은 요청 경로(Gateway → EPP → Model → Infra)를 따르는 두 축입니다. 숫자키 `0`–`5`
(또는 `Tab` / `Shift+Tab`)로 **섹션**을 고르고, `←` / `→`(또는 `[` / `]`)로 그 안의 **서브탭**을 순환합니다.
멀티패널 뷰에서는 `Ctrl+w`로 패널 포커스 모드 진입(이후 `h`/`j`/`k`/`l` 또는 화살표로 이동, `Esc`로 종료) — vi/tmux 창 이동 방식.

| # | 섹션 | 서브탭 | 내용 |
|---|---|---|---|
| 0 | **Overview** | — | 클러스터 요약, LED 그리드, VRAM 바, 종류/노드별 가속기, EPP 경로, 모델, 한 줄 진단 |
| 1 | **Traffic** | Flow · EPP | **Flow**: Gateway → HTTPRoute → backend → 파드, InferencePool/EPP/SLO 와 EPP 우회 진단(`⏎` backend 모델). **EPP**: scorer/가중치, picker, InferencePool 엔드포인트, 요청 분배 |
| 2 | **Serving** | Serving · Perf · Pods | **Serving**: 돌아가는 배포를 정렬 가능한 표로 — 상태(`● 서빙`/`◑ 시도 중`/`⚠ degraded`/`✗ 실패`/`○ 정지`, pods 교차)·엔진·타깃·replica·`@노드`·tok/s; `o`/`O` 정렬, `⏎` → Scale/Restart/Stop/Objective/YAML/Logs. **Perf**: p95 QUEUE→PREFILL→DECODE→TPOT→E2E·tok/s·SLO advisor. **Pods**: `llm-serving` 파드 |
| 3 | **Infra** | Nodes · Devices · Topology | **Nodes**: 노드 헬스(CPU/메모리/디스크/load). **Devices**: 디바이스별 util/VRAM/온도/전력. **Topology**: Canvas Gateway→EPP→Pool 흐름 + pressure 히트맵 |
| 4 | **Deploy** | Library / Zoo | **Library** — 런타임이 아니라 프로비저닝 — 위/아래 2패널(`Ctrl+w` 로 포커스 전환). **Model List**(위): 배포 가능한 모든 것 — 카탈로그 가능성(`✓ ready`/`⚙ needs-compile`/`✗ no-capacity`) + 배치 타깃·스토어 빌드, family 로 묶음; `⏎` → Deploy/Compile. deploy 폼은 옵션을 먼저 고르고 `⏎` 누르면 **placement** 선택 화면(후보 노드의 유휴/전체 디바이스·util·mem·스케줄 가능)이 뜨며, 노드를 고르면 매니페스트가 생성됨. **Activity**(아래): compile Job(진행률 % 바)과 서빙 중·시도 중·실패 deploy rollout 을 STARTED(5m/3h/2d)·서빙 노드·결과와 함께 한 피드로; 끝난 compile 은 30분 뒤 자동 정리; `⏎` → Logs/Delete. **Zoo**: 벤더(Furiosa/Rebellions) 모델 zoo — [furiosa-ai HF org](https://huggingface.co/furiosa-ai)·[rbln-model-zoo](https://github.com/RBLN-SW/rbln-model-zoo) 에서 자동 수집(번들 베이스라인은 `scripts/fetch-zoo.sh`, `r` 로 Furiosa 목록 `curl` 라이브 갱신, `LMD_ZOO` 로 대체). 각 행에 컴파일 가능 벤더(NPU 호환 목록)·상황별 STATUS(`● built`/`◐ compiling`/`⇊ prefetch`/`⇩ gated`/`○ available`) 표시, `o` 정렬. 아래엔 동일한 **Activity** 피드(compile+**prefetch**+deploy). `⏎` → **Prefetch**(가중치를 공유 스토어 캐시로) / **Compile→벤더**, 산출물은 Library 에서 배포 |
| 5 | **Events** | — | Kubernetes + llm-d 이벤트(최신순). `⏎` 로 전체 메시지 |
| 6 | **Setup** | — | 새 클러스터 부트스트랩 **Doctor** — llm-d 플랫폼 전제조건(Gateway API·Inference-Extension CRD, `llm-d-gateway`, 공유 EPP Role, `model-store` PVC, `hf-token` secret, 가속기 device plugin, Prometheus)을 `✓`/`!`/`✗` 로 점검. 행에서 `⏎` 는 위험도에 따라: lmd-top 이 정확히 생성하는 오브젝트(namespace, 감지된 `gatewayClassName` 의 Gateway)는 미리보기→apply, CRD 는 `kubectl apply -f <상류 릴리스 URL>`(admin·확인), 사이트 특화·Helm 관리 항목(PVC·EPP Role·secret·device plugin)은 직접 실행할 명령만 안내. 확인 전까지 read-only |

리스트 헤더에 보이는 행(전체 또는 필터된 것)의 `Σ` 통합 메트릭이 표시됩니다. `y` 로 선택 리소스의 live YAML(읽기전용)을 봅니다.

## 설치

단일 정적 바이너리(glibc만)가 `kubectl`을 구동하는 구조라, 설치는 "바이너리를 PATH에 얹기"가 전부입니다. 아래 중 편한 걸 고르세요 — **모든 기능(컴파일/배포 포함)은 방식과 무관하게 동일**합니다.

**kubectl 플러그인** (클러스터 운영자 권장) — 풀 TUI 를 `kubectl lmd-top` 으로 실행:

```bash
# 중앙 krew-index 등록 전까지는 자체 매니페스트로:
kubectl krew install --manifest-url https://raw.githubusercontent.com/gspark-etri/lmd-top/main/plugins/lmd-top.yaml
kubectl lmd-top                      # 또는: kubectl lmd-top --mode admin
```

`kubectl` 아래서 돌므로 kubectl 존재가 보장되고, scale/stop/restart 와 RBLN/Furiosa 컴파일·배포(생성한 매니페스트를 `kubectl apply`)가 사용자 kubeconfig 권한으로 바로 동작합니다.

**원라이너 설치** (사전 빌드 바이너리 → `~/.local/bin`, Rust 툴체인 불필요):

```bash
curl -fsSL https://raw.githubusercontent.com/gspark-etri/lmd-top/main/install.sh | bash
#   ... | bash -s -- --version v0.34.0            # 버전 고정
#   ... | bash -s -- --bin-dir /usr/local/bin     # 시스템 전역(쓰기 권한 필요)
```

OS/arch(Linux/macOS · x86_64/aarch64) 자동 감지 → 릴리스 tarball 다운로드 → `.sha256` 검증 → 설치. 수동 등가:

```bash
VER=v0.34.0   # 최신: https://github.com/gspark-etri/lmd-top/releases/latest
curl -fsSL "https://github.com/gspark-etri/lmd-top/releases/download/$VER/lmd-top-$VER-x86_64-linux.tar.gz" | tar xz
sudo install -m 0755 "lmd-top-$VER-x86_64-linux/lmd-top" /usr/local/bin/
```

**소스 빌드** (개발자용; Rust 툴체인 + C 링커 필요):

```bash
git clone https://github.com/gspark-etri/lmd-top.git && cd lmd-top
./install.sh --from-source        # cargo install(+사전요구); --with-demo 는 GIF 까지 재생성
# 수동: cargo install --path .
```

**실행 요구사항:**

- kubeconfig 접근 권한이 있는 `kubectl`, Prometheus 로의 네트워크 접근. 가속기 노드에 SSH 하지 않습니다.
- truecolor 를 지원하고 box-drawing·braille 글리프를 포함한 폰트를 쓰는 터미널 권장. 아니면 `LMD_THEME=default` 로 실행하세요.
- 바이너리는 glibc 만 링크합니다 — OpenSSL, pkg-config, cmake 불필요. `xdg-open` 은 `g` 키에만 쓰이는 선택 사항입니다.

## 사용법

```bash
lmd-top                      # TUI 실행 (권한 모드: observe)
lmd-top --mode admin         # scale / restart / compile·deploy apply 등 운영 액션 허용
lmd-top --json               # 기계가 읽는 agent 상태(JSON) 출력
lmd-top --doctor             # Prometheus 전수조사: exporter, 지표 커버리지, 누락
lmd-top --audit              # 적용한 변경 작업 감사 로그 출력
lmd-top --snapshot | --render | --cast   # 헤드리스 텍스트 / CI 렌더 / 데모 asciicast
LMD_PROM=192.0.2.10:30090 LMD_NS=my-ns lmd-top   # 다른 클러스터 지정
```

**권한 모드**(`--mode`, 헤더에 배지로 표시) 는 액션을 단계적으로 잠급니다.
`observe`(기본, 보기 전용) → `debug`(로그 `l` 추가) → `admin`(scale·restart·stop·compile/deploy apply·cordon·route rename/retarget) →
`danger`(pod/job/route rule 삭제). 변경 액션의 확인 팝업은 기본값이 **No** 입니다.
적용된 모든 변경 작업(scale·stop·restart·cordon·delete·route 편집·apply)은 시각·모드·작업·
대상·결과와 함께 **감사 로그**(`~/.config/lmd-top/audit.log`, 또는 `$LMD_AUDIT`)에 남습니다 —
`lmd-top --audit` 로 확인.

**키.**

| | |
|---|---|
| 이동 | `0-5`/`Tab` 섹션 · `←`/`→` (`[ ]`) 서브탭 · `Ctrl+w` 후 `hjkl`/화살표 패널 포커스 · `↑↓`/`kj` 선택 · `g`/`G` 처음/끝 · `Ctrl+u`/`Ctrl+d` 반 페이지 · `Esc` 뒤로 |
| 액션 | `⏎`/`a` 액션 메뉴(없으면 상세) · `p i r e m` 크로스레이어 pivot(메뉴에 **Go: …** 로도) · `/` 필터 · `:` 커맨드 팔레트(뷰 점프 / 표시 액션 실행) · `o`/`O` 정렬 컬럼/방향 · `y` live YAML · `l` 로그 · 메뉴 → Compile/Deploy/Scale/Restart/Stop/Delete/Cordon/Objective (모드 게이팅 `⊘`·기본 No 확인) |
| 표시 | `t` 테마 · `f` 애니메이션 · `z` zoom · `Space` 일시정지 · `A` 알림 · `?` 도움말 · `q` 종료 · `:graf` Grafana · `R` 세션 에너지 리셋 |

수집 상태: `:` → **Data sources**에서 소스별 상태·마지막 성공 시각·오류를 확인할 수 있습니다. 수집에 실패하면 이전 값과 원래 수집 시각을 유지합니다. `–`와 대시 게이지는 0이 아닌 측정값 부재를 뜻합니다. JSON에는 `sources`, `source_states`, `warnings`가 포함됩니다. 조회창은 스냅샷이므로 다시 열면 갱신됩니다.

워크로드 판별: `:` → **Workload classification**에서 모델·Prefill·Decode·게이트웨이·미확인 배포와 판별 근거를 조회합니다. 이름만으로 모델을 추정하지 않습니다. 인식되지 않는 커스텀 추론 서버는 Deployment에 `lmd-top.io/workload-role: model` 주석을 지정할 수 있습니다(`prefill`, `decode`도 지원). 이 판별과 Ready 수는 실제 추론 성공을 보장하지 않습니다. JSON의 `workloads`에는 모델 외 배포도 포함됩니다.

성능 목표: `:` → **Performance assessment**에서 세션에 설정한 SLO의 충족·위반·판단 불가 사유를 조회합니다. 지표가 없거나 오래되었거나 요청 트래픽이 관측되지 않으면 목표 충족으로 판정하지 않습니다. 모델 이름이 모호하게 일치하면 더 구체적인 목표가 필요합니다. 단계별 p95는 독립적인 백분위수이므로 합산 지연이나 가속기 용량의 근거로 해석하지 않습니다.

운영 권한은 실행 함수에서도 다시 검사합니다. 스토어 삭제의 `danger` 요구 권한은 미리보기·매니페스트 편집 후에도 유지됩니다. CLI의 `--plan … --apply`에는 이제 `--mode admin` 또는 `--mode danger`가 필요합니다. 계획 생성과 서버 dry-run은 적용 권한을 요구하지 않습니다. 이 모드는 조작 실수 방지용이며 실제 접근 권한은 Kubernetes RBAC가 결정합니다.

감사 로그에는 고정된 컨텍스트, 요청·기본 네임스페이스, 확인한 UID·리소스 버전(있는 경우), 소요 시간이 추가됩니다. CLI 계획 적용도 기록됩니다. 작업 성공 후 로그 기록만 실패한 경우 별도로 표시하므로, 로그를 남기려고 작업을 다시 실행하지 마세요. 기존 로그도 그대로 조회할 수 있습니다.

Kubernetes 대상: 최초 클러스터 접근 시 현재 kubeconfig를 세션용으로 고정합니다. 이후 모든 kubectl 조회·변경이 이 설정을 사용하므로 외부에서 현재 컨텍스트를 바꿔도 실행 중인 lmd-top의 대상은 바뀌지 않습니다. 다른 컨텍스트를 선택하거나 초기 설정 실패를 복구하려면 프로그램을 다시 실행하세요. 확인창과 JSON의 `kubernetes_context`에서 선택된 컨텍스트를 확인할 수 있습니다. 세션 설정 파일은 소유자만 접근 가능하며 정상 종료 시 삭제됩니다.

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

**지금 바로 되는 것 (트래픽 불필요).** 12개 뷰 전부, 자동 감지·통합 메모리를 포함한 GPU/RBLN/RNGD
및 노드/디스크 모니터링, Flow 토폴로지와 EPP 우회 진단, EPP ConfigMap 조회, 능동 알림,
`scale`·`logs` 액션, Serving 섹션(서빙/시도 중/실패 상태의 돌아가는 배포)과 Deploy 섹션의
Model List + Activity(카탈로그 가능성 · RBLN/Furiosa 컴파일·배포 매니페스트 생성 + 권한 모드
게이팅 apply 를 한 작업 피드로), 헤드리스 `--json`·`--doctor`·
`--snapshot`·`--cast`, 그리고 테마·애니메이션·zoom·권한 모드.

**실제 트래픽이 EPP 를 거치고 vLLM 이 지표를 내보내면 채워지는 것.** 모델별 p95 지연 분해, tok/s,
파드별 큐 분배, KV%/TTFT/E2E, EPP 요청 분배. (EPP 가중치 `+`/`-` 는 로컬 시뮬레이션이며, 클러스터에
실제로 적용되지는 않습니다.)

**예정된 것.** 현재 apply 흐름을 넘어서는 적용형 컨트롤 플레인 액션(엔드포인트 drain, 트래픽/정책
가중치 적용, rollout — 각각 dry-run → 확인 → 감사)과 EPP 엔드포인트별 점수 디버거. (**NPU 컴파일·배포
자동화** — Deploy 뷰에서 RBLN/Furiosa 컴파일 Job·서빙 Deployment 를 생성하고 권한 모드로 게이팅 —
는 이미 반영됨; Highlights 참고.) 자세한 내용은 [ROADMAP.md](ROADMAP.md) 와 [CHANGELOG.md](CHANGELOG.md) 참고.

## 성숙도

실제 이종 클러스터에서 검증했습니다(8개 노드, GB10·RBLN·RNGD 가속기, EPP·라우트·모델 모두 라이브).
아직 실험 단계(0.x) 라 인터페이스는 바뀔 수 있습니다.

## 기여 & 라이선스

이슈와 PR 을 환영합니다 — [CONTRIBUTING.md](CONTRIBUTING.md) 를 참고하세요.
라이선스는 [Apache-2.0](LICENSE) 입니다.
