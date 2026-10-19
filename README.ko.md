<div align="center">

<h1>lmd-top</h1>

<h3>이기종 Kubernetes + llm-d 클러스터를 터미널에서 관측하고 운영합니다</h3>

<p>
  <a href="README.md">English</a> · <strong>한국어</strong>
</p>

<p>
  <a href="https://github.com/ai-computing/lmd-top/actions/workflows/ci.yml"><img src="https://github.com/ai-computing/lmd-top/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/ai-computing/ai-rm"><img src="https://img.shields.io/badge/ai--rm-L5%20observe%20%26%20operate-f472b6" alt="ai-rm 계층: L5 관측·운영"></a>
  <a href="https://github.com/ai-computing/rm-core/blob/v0.1.1/PROTOCOL.md"><img src="https://img.shields.io/badge/protocol-v0.1-7c5cff" alt="Protocol v0.1"></a>
  <a href="https://github.com/ai-computing/lmd-top/releases/latest"><img src="https://img.shields.io/github/v/release/ai-computing/lmd-top?logo=github" alt="최신 릴리스"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/Rust-single%20binary-000000?logo=rust&amp;logoColor=white" alt="Rust 단일 바이너리"></a>
  <a href="#-아키텍처"><img src="https://img.shields.io/badge/target-Kubernetes%20%2B%20llm--d-326ce5?logo=kubernetes&amp;logoColor=white" alt="대상: Kubernetes와 llm-d"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-64748b" alt="라이선스: Apache-2.0"></a>
</p>

<p>
  <a href="#-빠른-시작"><b>설치</b></a> ·
  <a href="#-사용법"><b>사용법</b></a> ·
  <a href="#-ai-rm-연동"><b>ai-rm</b></a> ·
  <a href="#-아키텍처"><b>아키텍처</b></a> ·
  <a href="#-문서"><b>문서</b></a> ·
  <a href="#-로드맵"><b>로드맵</b></a>
</p>

</div>

---

**lmd-top**은 [llm-d](https://llm-d.ai) 서빙 스택의 네 계층(Gateway → EPP(Endpoint Picker) → 모델 서버 → 인프라)을
이종 가속기(NVIDIA GPU, Rebellions RBLN, Furiosa RNGD, 호스트 CPU) 환경에서 하나로 엮어 보여주는 터미널 UI입니다. 기존
Prometheus와 Kubernetes를 읽기만 하고 자체 데이터는 갖지 않습니다. 운영 액션은 권한 모드와 감사 로그 아래에서 `kubectl`로
실행합니다. [ai-rm](https://github.com/ai-computing/ai-rm)에서는 **L5 관측·운영** 콘솔이며, ai-rm protocol v0.1 객체
(AcceleratorProfile, `airm_*` 지표, 배치 결정)를 벤더 데이터 옆에 함께 보여줍니다.

아직 실험 단계(0.x)라 인터페이스는 바뀔 수 있습니다.

## ✨ 주요 기능

<table>
<tr>
<td width="33%" valign="top">
<b>네 계층을 한 화면에</b><br>
Gateway, EPP/InferencePool, 모델 서버, 하드웨어를 서로 엮어 보여주므로, 도구를 옮겨 다니지 않고도 어떤 모델이 어디서 돌고, 요청이 어떻게 라우팅되며, 부하가 어떻게 분산되는지 알 수 있습니다.
</td>
<td width="33%" valign="top">
<b>이종 가속기를 하나로</b><br>
NVIDIA GPU, Rebellions RBLN, Furiosa RNGD를 나란히 표시합니다. GPU 모델과 VRAM은 자동 감지하고, 통합 메모리(GB10, GH200)는 <code>∪</code>로 표시하며, 노드별 디스크 사용량도 봅니다.
</td>
<td width="33%" valign="top">
<b>EPP를 이해합니다</b><br>
EPP <code>ConfigMap</code>(활성 scorer, 가중치, picker)을 읽어 라우팅 결정과 파드별 큐를 보여주고, HTTPRoute가 실제로 InferencePool을 거치는지 우회하는지 진단합니다.
</td>
</tr>
<tr>
<td width="33%" valign="top">
<b>올리기와 돌보기</b><br>
<b>Serving</b>은 돌아가는 배포를 실시간 상태(서빙, 시도 중, degraded, 실패)와 scale·restart·stop으로 보여줍니다. <b>Deploy</b>는 배포 가능한 목록과 compile Job·rollout 작업 피드를 보여줍니다.
</td>
<td width="33%" valign="top">
<b>안전장치가 있는 운영</b><br>
권한 모드(<code>observe</code> → <code>debug</code> → <code>admin</code> → <code>danger</code>), 기본값이 No인 확인 팝업, 적용한 모든 변경 작업의 감사 로그.
</td>
<td width="33%" valign="top">
<b>단일 정적 바이너리</b><br>
LED 디바이스 그리드, 스택형 VRAM 바, braille 타임라인, 알림, 로그 조회, 네 가지 테마를 glibc만 링크하는 Rust 바이너리 하나에 담았습니다. 헤드리스 <code>--json</code>, <code>--doctor</code>, <code>--snapshot</code>도 있습니다.
</td>
</tr>
</table>

공개 레포는 정리된 export라 데모 녹화는 들어 있지 않습니다. 데모는 로컬에서
`lmd-top --cast && agg docs/demo.cast lmd-top.gif`로 만듭니다(soft 테마, 실시간 braille 타임라인, 계층 간 드릴다운, 합성 데이터).

## 🚀 빠른 시작

lmd-top은 `kubectl`을 구동하는 단일 바이너리라, 설치는 바이너리를 `PATH`에 두는 것이 전부입니다. 어떤 방식으로 설치해도
컴파일·배포를 포함한 모든 기능이 같습니다.

**릴리스 바이너리** (Rust 툴체인 불필요). 최신 릴리스: v0.35.0, 대상 `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`.

```bash
VER=v0.35.0   # 최신: https://github.com/ai-computing/lmd-top/releases/latest
curl -fsSLO "https://github.com/ai-computing/lmd-top/releases/download/$VER/lmd-top-$VER-x86_64-linux.tar.gz"
curl -fsSLO "https://github.com/ai-computing/lmd-top/releases/download/$VER/lmd-top-$VER-x86_64-linux.tar.gz.sha256"
sha256sum -c "lmd-top-$VER-x86_64-linux.tar.gz.sha256" && tar xzf "lmd-top-$VER-x86_64-linux.tar.gz"
sudo install -m 0755 "lmd-top-$VER-x86_64-linux/lmd-top" /usr/local/bin/
```

**소스 빌드** (Rust 툴체인 + C 링커 필요). [Quick start 가이드](docs/QUICKSTART.md)에 나오는 미릴리스 명령
(`--init`, `--check-setup`, `--version`)도 이 방법으로 씁니다.

```bash
git clone https://github.com/ai-computing/lmd-top.git && cd lmd-top
./install.sh --from-source        # cargo install(+사전요구); --with-demo 는 데모 GIF까지 재생성
# 수동: cargo install --locked --path .
```

**실행** (클러스터 지정):

```bash
lmd-top                                            # observe 모드
LMD_PROM=192.0.2.10:30090 LMD_NS=my-ns lmd-top     # 다른 클러스터 지정
```

<details>
<summary><b>설치 스크립트와 kubectl 플러그인</b></summary>
<br>

> [!NOTE]
> `install.sh`와 `plugins/lmd-top.yaml`은 아직 `ai-computing/lmd-top`이 아니라 프로젝트의 이전 저장소 위치에서 릴리스
> 파일을 받습니다. 갱신되기 전까지는 위의 릴리스 바이너리나 소스 빌드를 쓰세요.

**kubectl 플러그인** (클러스터 운영자용): 풀 TUI를 `kubectl lmd-top`으로 실행합니다.

```bash
# 중앙 krew-index 등록 전까지는 자체 매니페스트로:
kubectl krew install --manifest-url https://raw.githubusercontent.com/ai-computing/lmd-top/main/plugins/lmd-top.yaml
kubectl lmd-top                      # 또는: kubectl lmd-top --mode admin
```

`kubectl` 아래서 돌므로 kubectl 존재가 보장되고, scale/stop/restart와 RBLN/Furiosa 컴파일·배포(생성한 매니페스트를
`kubectl apply`)가 사용자 kubeconfig 권한으로 동작합니다.

**원라이너 설치** (사전 빌드 바이너리 → `~/.local/bin`):

```bash
curl -fsSL https://raw.githubusercontent.com/ai-computing/lmd-top/main/install.sh | bash
#   ... | bash -s -- --version v0.35.0            # 버전 고정
#   ... | bash -s -- --bin-dir /usr/local/bin     # 시스템 전역(쓰기 권한 필요)
```

OS/arch(Linux/macOS · x86_64/aarch64)를 자동 감지해 릴리스 tarball을 받고, `.sha256`을 검증한 뒤 설치합니다.
</details>

**실행 요구사항**

- kubeconfig 접근 권한이 있는 `kubectl`, Prometheus로의 네트워크 접근. 가속기 노드에 SSH 하지 않습니다.
- truecolor를 지원하고 box-drawing·braille 글리프를 포함한 폰트를 쓰는 터미널 권장. 아니면 `LMD_THEME=default`로 실행하세요.
- 바이너리는 glibc만 링크합니다. OpenSSL, pkg-config, cmake가 필요 없습니다. `xdg-open`은 `g` 키에만 쓰이는 선택 사항입니다.

설치·연결·유지보수: [Quick start 가이드](docs/QUICKSTART.md).

## 🧭 사용법

```bash
lmd-top                      # TUI 실행 (권한 모드: observe)
lmd-top --mode admin         # scale / restart / apply 등 운영 액션 허용
lmd-top --json               # 기계가 읽는 agent 상태(JSON, pods와 스토어 용량 포함) 출력
lmd-top --doctor             # Prometheus 전수조사: exporter, 지표 커버리지, 누락
lmd-top --audit              # 적용한 변경 작업 감사 로그 출력
lmd-top --snapshot | --render | --cast   # 헤드리스 텍스트 / CI 렌더 / 데모 asciicast
```

네비게이션은 요청 경로(Gateway → EPP → Model → Infra)를 따릅니다. `0`–`6`(또는 `Tab` / `Shift+Tab`)으로 **섹션**을 고르고,
`←` / `→`(또는 `[` / `]`)로 그 안의 **서브탭**을 순환합니다. 멀티패널 뷰에서는 `Ctrl+w`로 패널 포커스 모드에 들어가며
(`h`/`j`/`k`/`l` 또는 화살표로 이동, `Esc`로 종료) vi/tmux 창 이동 방식과 같습니다.

| # | 섹션 | 서브탭 |
|---|---|---|
| 0 | **Overview** | — |
| 1 | **Traffic** | Flow · EPP |
| 2 | **Serving** | Serving · Perf · Pods |
| 3 | **Infra** | Nodes · Devices · Topology |
| 4 | **Deploy** | Library / Zoo |
| 5 | **Events** | — |
| 6 | **Setup** | — |

**권한 모드**(`--mode`, 헤더에 배지로 표시)는 액션을 단계적으로 잠급니다: `observe`(기본, 보기 전용) → `debug`(로그 `l` 추가)
→ `admin`(scale, restart, stop, compile/deploy apply, cordon, route rename/retarget, 스토어 move/rescan) → `danger`
(pod/job/route rule 삭제, 스토어에서 빌드 제거). 변경 액션의 확인 팝업은 기본값이 **No**입니다. 적용된 모든 변경 작업(scale,
stop, restart, cordon, delete, route 편집, apply)은 시각·모드·작업·대상·결과와 함께 **감사 로그**
(`~/.config/lmd-top/audit.log`, 또는 `$LMD_AUDIT`)에 남고, `lmd-top --audit`로 확인합니다.

<details>
<summary><b>뷰별 내용</b></summary>
<br>

| # | 섹션 | 서브탭 | 내용 |
|---|---|---|---|
| 0 | **Overview** | — | 클러스터 요약, LED 디바이스 그리드, VRAM 바, 종류/노드별 가속기, EPP 경로, 모델, 한 줄 진단 |
| 1 | **Traffic** | Flow · EPP | **Flow**: Gateway → HTTPRoute → backend → 파드, InferencePool/EPP/SLO와 EPP 우회 진단(`⏎`로 backend 모델로 이동). **EPP**: scorer/가중치, picker, InferencePool 엔드포인트, 요청 분배 |
| 2 | **Serving** | Serving · Perf · Pods | **Serving**: 돌아가는 배포를 정렬 가능한 표로 — 상태(`● serving` / `◑ starting` / `⚠ degraded` / `✗ failed` / `○ stopped`, pods 교차 확인), 엔진, 타깃, replica, `@노드`, tok/s; `o`/`O` 정렬, `⏎` → Scale/Restart/Stop/Objective/YAML/Logs, 노드 로컬 아티팩트에는 **Provenance**(`P`)도 — 그 노드에서 읽기 전용 프로브 Job이 아티팩트가 기록한 toolchain을 읽으며, 손으로 컴파일한 `hostPath` 빌드는 이것이 유일한 확인 방법. **Perf**: p95 QUEUE→PREFILL→DECODE→TPOT→E2E, tok/s, SLO advisor. **Pods**: `llm-serving` 파드(ready/phase/node/restarts) |
| 3 | **Infra** | Nodes · Devices · Topology | **Nodes**: 노드 헬스(CPU/메모리/디스크/load + 디바이스). **Devices**: 디바이스별 util/VRAM/온도/전력과 ai-rm 패널([아래](#-ai-rm-연동)). **Topology**: Canvas Gateway→EPP→Pool 흐름 + 디바이스 pressure 히트맵 |
| 4 | **Deploy** | Library / Zoo | **Library** — 런타임이 아니라 프로비저닝 — 위/아래 2패널(`Ctrl+w`로 포커스 전환). 빌드를 만들고 지우는 곳이라 헤더에 공유 스토어의 남은 용량을 표시. **Model List**(위): 배포 가능한 모든 것 — 조직 카탈로그 가능성(`✓ ready` / `⚙ needs-compile` / `✗ no-capacity`) + 배치 타깃과 스토어 빌드, family로 묶음; `⏎` → Deploy/Compile, 스토어 빌드에서는 **Move**(`V`, 스토어 안에서 이동/이름 변경), **Remove**(`M`, 빌드 삭제), **Rescan**(`r`)도. 각 스토어 빌드에는 **무엇으로 빌드했는지** — 아티팩트 자체에서 읽은 toolchain 버전(`optimum-rbln=0.10.2`, `furiosa-compiler=2026.3.0`) — 를 행과 상세 패널에 표시하고, 컴파일 사전 점검도 빌드하려는 family에 대해 이를 다시 보여줌. 옵션이 같아도 다른 컴파일러가 만든 빌드는 다르게 동작할 수 있기 때문. Move와 Remove는 매니페스트를 먼저 검토하는 Job으로 돌고, 그 경로에서 서빙 중인 Deployment가 있으면 거부함. Rescan은 store-discovery CronJob을 즉시 한 번 돌려, 지운 빌드가 최대 10분 주기 동안 남아 있지 않고 바로 목록에서 빠짐. deploy 폼은 옵션을 먼저 채우고 `⏎`를 누르면 **placement** 선택 화면(후보 노드의 유휴/전체 디바이스, util, mem, 스케줄 가능 여부)이 뜨며, 노드를 고르면 매니페스트가 생성됨. **Activity**(아래): compile Job(진행률 % 바)과 실행 중·시작 중·실패한 deploy rollout을 STARTED 경과(5m/3h/2d), 서빙 노드, 결과와 함께 한 피드로; 끝난 compile Job은 30분 뒤 자동 정리; `⏎` → Logs/Delete. **Zoo**: 벤더(Furiosa/Rebellions) 모델 zoo — [furiosa-ai HF org](https://huggingface.co/furiosa-ai)와 [rbln-model-zoo](https://github.com/RBLN-SW/rbln-model-zoo)에서 공개 HF 모델을 자동 수집(번들 베이스라인은 `scripts/fetch-zoo.sh`, `r`로 Furiosa 목록을 `curl`로 라이브 갱신, `LMD_ZOO`로 대체). 각 행에 컴파일 가능 벤더(NPU 호환 목록)와 상황별 STATUS(`● built` / `◐ compiling` / `⇊ prefetch` / `⇩ gated` / `○ available`) 표시, `o` 정렬. 아래엔 같은 **Activity** 피드(compile + **prefetch** + deploy). `⏎` → **Prefetch**(가중치를 공유 스토어 캐시로 다운로드) / **Compile→벤더**, 산출물은 Library에서 배포 |
| 5 | **Events** | — | Kubernetes + llm-d 이벤트(최신순). `⏎`로 전체 메시지 |
| 6 | **Setup** | — | 새 클러스터 부트스트랩 **Doctor** — llm-d 플랫폼 전제조건(Gateway API·Inference-Extension CRD, `llm-d-gateway`, 공유 EPP Role, `model-store` PVC, `hf-token` secret, 가속기 device plugin, Prometheus)을 `✓`/`!`/`✗`로 점검. 행에서 `⏎`는 위험도에 따라: lmd-top이 정확히 생성하는 오브젝트(namespace, 감지된 `gatewayClassName`의 Gateway)는 미리보기→apply, CRD는 `kubectl apply -f <상류 릴리스 URL>`(admin, 확인), 사이트 특화·Helm 관리 항목(PVC, EPP Role, secret, device plugin)은 직접 실행할 명령만 안내. 확인 전까지 read-only |

리스트 헤더에 보이는 행(전체 또는 필터된 것)의 `Σ` 합계가 표시됩니다. `y`로 선택한 리소스의 live YAML(읽기 전용)을 봅니다.
</details>

<details>
<summary><b>키 바인딩</b></summary>
<br>

| | |
|---|---|
| 이동 | `0-6`/`Tab` 섹션 · `←`/`→` (`[ ]`) 서브탭 · `Ctrl+w` 후 `hjkl`/화살표 패널 포커스 · `↑↓`/`kj` 선택 · `g`/`G` 처음/끝 · `Ctrl+u`/`Ctrl+d` 반 페이지 · `Esc` 뒤로 |
| 액션 | `⏎`/`a` 액션 메뉴(없으면 상세) · `p i r e m` 크로스레이어 pivot(메뉴에 **Go: …**로도) · `/` 필터 · `:` 커맨드 팔레트(뷰 점프 / 표시 액션 실행) · `o`/`O` 정렬 컬럼/방향 · `y` live YAML · `l` 로그 · 메뉴 → Compile/Deploy/Scale/Restart/Stop/Delete/Cordon/Objective (모드 게이팅 `⊘`, 기본 No 확인) · 스토어 행에서: `V` 이동 · `M` 제거 · `r` 재스캔 |
| 표시 | `t` 테마 · `f` 애니메이션 · `z` zoom · `Space` 일시정지 · `A` 알림 · `?` 도움말 · `q` 종료 · `:graf` Grafana · `R` 세션 에너지 리셋 |

**색과 글리프.** 색은 심각도나 정체성을 나타내고, 상태는 별도의 글리프(`●` 정상, `○` 유휴, `◐` 대기, `⚠` throttle,
`⊘` cordon, `✗` 다운)로 표현합니다. 그래서 색맹 테마에서도 읽힙니다. 아직 없는 지표는 `–`로 표시되고, 워크로드가 올라오면
채워집니다.
</details>

<details>
<summary><b>환경 변수</b></summary>
<br>

- `LMD_PROM`, `LMD_NS`(기본 `llm-serving`), `LMD_GRAFANA` — 대상 클러스터 지정.
- `LMD_AIRM_NS` — ai-rm ConfigMap `airm-placement`의 네임스페이스(기본: `LMD_NS`).
- `LMD_THEME` — 시작 테마: `soft`, `default`, `high-contrast`, `colorblind`.
- `LMD_AUDIT` — 감사 로그 경로(기본: `~/.config/lmd-top/audit.log`).
- `LMD_W` / `LMD_H` — `--render` 크기.
- `LMD_COMPILE_IMAGE_RBLN`, `LMD_COMPILE_IMAGE_FURIOSA`, `LMD_SERVING_IMAGE` — 생성되는 compile/deploy 매니페스트의 컨테이너 이미지. 지정 전에는 `TODO-…` placeholder라 앱 내 apply(`a`)가 막히고, `w`로 저장해 직접 편집할 수는 있습니다.
- `LMD_SAVE_DIR` — `w` 저장 위치(기본: 현재 디렉터리).
- `LMD_HISTORY` — compile/serving 결과 기록(기본: `~/.config/lmd-top/history.jsonl`). `lmd-top --history`로 다시 읽습니다.
- `LMD_COMPILE_ENV` — 생성되는 compile Job에 넣을 추가 환경 변수(`K=V,K=V`). 벤더 디버그 스위치용입니다. RBLN은 TVM 위에서 빌드하므로 `TVM_LOG_DEBUG=1` / `TVM_BACKTRACE=1`이 컴파일러까지 전달됩니다. SDK가 거부하는 플래그는 `vendor-flag`로 분류해 옵션 근거에서 제외하므로, 잘못된 스위치 때문에 advisor가 옵션이 실패한다고 학습하지 않습니다.
- `LMD_RBLN_TOOLCHAIN` — RBLN toolchain을 설치할 공유 스토어의 wheel 디렉터리(예: `/mnt/store/rbln-toolchain/0.10.3`). compile Job은 `python:3.10-slim`에서 `pip install --no-index --find-links=<dir>`로 돌아, 모든 버전을 함께 고정하고 노드의 Python 환경을 물려받지 않습니다. hostPath, nodeSelector, 벤더 레지스트리 자격 증명이 필요 없고 wheel만 있으면 됩니다.
- 선택 사항인 `~/.config/lmd-top/lmd-top.yaml`로 컬럼 순서를 바꿀 수 있습니다.

사이트 값: `cluster.example.env`를 `cluster.local.env`(untracked)로 복사해 채우고 `set -a; . ./cluster.local.env; set +a`로
내보낸 뒤 실행합니다. 환경 변수가 없으면 `~/.config/lmd-top/lmd-top.yaml`, 그다음 localhost 기본값을 씁니다.
</details>

<details>
<summary><b>운영 메모: 수집 품질, 워크로드 판별, SLO, 안전장치</b></summary>
<br>

**수집 상태.** `:` → **Data sources**에서 소스별 상태·마지막 성공 시각·오류를 확인할 수 있습니다. 수집에 실패하면 이전 값과
원래 수집 시각을 유지합니다. `–`와 대시 게이지는 0이 아닌 측정값 부재를 뜻합니다. JSON에는 `sources`, `source_states`,
`warnings`가 포함됩니다. 조회창은 스냅샷이므로 다시 열면 갱신됩니다.

**워크로드 판별.** `:` → **Workload classification**에서 모델·Prefill·Decode·게이트웨이·EPP·미확인 배포와 판별 근거를
조회합니다. 이름만으로 모델을 추정하지 않습니다. 인식되지 않는 커스텀 추론 서버는 Deployment에
`lmd-top.io/workload-role: model` 주석을 지정할 수 있습니다(`prefill`, `decode`도 지원). 이 판별과 Ready 수는 실제 추론
성공을 보장하지 않습니다. JSON의 `workloads`에는 모델 외 배포도 포함됩니다.

**성능 목표.** `:` → **Performance assessment**에서 세션에 설정한 SLO의 충족·위반·판단 불가를 조회합니다. 지표가 없거나
오래되었거나 요청 트래픽이 관측되지 않으면 목표 충족으로 판정하지 않습니다. 판단 불가 지표는 Perf 패널에 표시됩니다. 모델
이름이 모호하게 일치하면 더 구체적인 목표가 필요합니다. 단계별 p95는 독립적인 백분위수이므로 합산 지연이나 가속기 용량의
근거로 해석하지 않습니다.

**권한 재검사.** 운영 권한은 실행 함수에서도 다시 검사합니다. 스토어 삭제의 `danger` 요구 권한은 미리보기·매니페스트 편집
후에도 유지됩니다. CLI의 `--plan … --apply`에는 `--mode admin` 또는 `--mode danger`가 필요합니다. 계획 생성과 서버
dry-run은 적용 권한을 요구하지 않습니다. 이 모드는 조작 실수 방지용이며 실제 접근 권한은 Kubernetes RBAC가 결정합니다.

**감사 로그**에는 고정된 컨텍스트, 요청·기본 네임스페이스, 확인한 UID·리소스 버전(있는 경우), 소요 시간이 들어갑니다. CLI
계획 적용도 기록됩니다. 작업 성공 후 로그 기록만 실패한 경우 별도로 표시하므로, 로그를 남기려고 작업을 다시 실행하지 마세요.
기존 로그도 그대로 조회할 수 있습니다.

**Kubernetes 대상.** 최초 클러스터 접근 시 현재 kubeconfig를 세션용으로 고정합니다. 이후 모든 kubectl 조회·변경이 이 설정을
사용하므로 셸에서 현재 컨텍스트를 바꿔도 실행 중인 lmd-top의 대상은 바뀌지 않습니다. 다른 컨텍스트를 선택하거나 초기 설정
실패를 복구하려면 다시 실행하세요. 확인창과 JSON의 `kubernetes_context`에서 선택된 컨텍스트를 확인할 수 있습니다. 세션용
설정 파일은 소유자만 접근할 수 있으며 정상 종료 시 삭제됩니다.

**라우팅 정책 개발**(미릴리스): `lmd-top --policy init --workspace ./my-policy`로 버전을 고정한 Go 프로젝트를 만들고,
`:Routing policy`로 설정과 롤아웃 미리보기 폼을 엽니다. [워크스페이스와 EPP 기능 가이드](docs/EPP-CAPABILITIES.md)를 보세요.
</details>

## 🔌 ai-rm 연동

lmd-top은 [ai-rm](https://github.com/ai-computing/ai-rm)의 **`l5-ops` 계층, kind `tool`** 구성요소이자 관측 콘솔이며,
메타데이터는 [`ai-rm.yaml`](ai-rm.yaml)에 있습니다. 예전 rm_monitor의 역할은 나뉘었습니다. 모니터링 스택(Prometheus,
exporter, Grafana 대시보드)은 [llm-d-setup](https://github.com/ai-computing/llm-d-setup) `monitoring/`(L4)으로, 부하 도구는
rm-bench(L5)로 옮겼습니다. lmd-top은 그 스택이 모은 지표를 읽습니다. rm-core를 import하지 않고 rm-core 형식을 읽습니다.

**ai-rm 패널 (protocol v0.1, 읽기 전용).** 클러스터에 ai-rm 소스가 하나라도 있으면 Infra ▸ Devices 아래에 ai-rm 패널이 나옵니다.

| 소스 | 패널에 보이는 것 |
|---|---|
| `AcceleratorProfile` (`airm.ai-computing.io/v1alpha1`) | 벤더, 모델, 메모리, peak와 그 출처. 선택한 장치의 profile을 표시 |
| Prometheus `airm_*` 지표 | rm-core v0.1의 이름·단위로 표시. airm 지표가 없으면 벤더 값에서 구한 `tok_s` / `req_s` / `power_w` / `accelerators`를 `[vendor]`로 표시. 장치별 PWR 열은 `airm_power_w{scope="card"}`가 있으면 그 값을 씀 |
| ConfigMap `airm-placement` (네임스페이스 `LMD_AIRM_NS`, 기본 `LMD_NS`) | 배치 결정과 이유. 두 형식을 모두 읽음: llm-d-setup이 문서화한 `decisions.json` 문서(`decisions` 목록, `<pool>/<kind>`로 표시)와, 워크로드마다 키 하나에 `{"accelerator_ids": [...], "kv_transport": "...", "reason": "..."}`(JSON 또는 YAML). `decisions`가 비어 있으면 아무것도 표시하지 않음 |

CRD·지표·ConfigMap이 모두 없으면 화면은 그대로이고 경고도 내지 않습니다. 쓰기 액션은 ai-rm 데이터의 영향을 받지 않습니다.

**인터페이스.** 제공: `Metrics`(`--json` agent 상태 `lmd-top/agent-state/v2`, `--doctor` 지표 커버리지), `Placement`
(heterogeneous-capacity EPP 정책, `--policy render/plan/replay`). 사용: `Metrics`(DCGM·RBLN·Furiosa·vLLM·EPP Prometheus
지표), `Accelerator`(가속기 팩 `src/accel/`, 노드 device plugin 리소스), `Workload`(serving Deployment, NPU 컴파일 Job,
npu-lora 학습 Job `--plan train`), `Placement`(InferencePool·EPP·HTTPRoute). KPI: 이종자원수(GPU·RBLN·RNGD를 한 콘솔에서
관측·배포), SLO(TTFT/TPOT/E2E p95, 큐·KV 상태), 전력효율(가속기 전력 지표 표시).

**설치·테스트** (가속기나 클러스터 없이 돈다):

```bash
cargo install --locked --path .                         # lmd-top 바이너리 설치
cargo test --locked                                     # Rust 단위·골든 테스트
python3 -m unittest discover -s tests -p 'test_*.py'    # 설치 스크립트·레시피 회귀 테스트
(cd contrib/epp-score-observer && go test ./...)        # EPP 정책 플러그인 (Go)
```

## 🏗 아키텍처

lmd-top은 클러스터에 상주하지 않습니다. `kubectl`이 되는 호스트(control-plane, 워크스테이션)에서 돌며, Kubernetes는
`kubectl`로, 지표는 Prometheus HTTP API(`LMD_PROM`)로 읽고, 쓰기 액션도 같은 `kubectl` 경로로 적용합니다.

| 계층 | 소스 | 예시 |
|---|---|---|
| 가속기 / 호스트 | Prometheus | `DCGM_FI_DEV_*`, `RBLN_DEVICE_STATUS:*`, `furiosa_npu_*`, `node_*` |
| 모델 서버 | Prometheus | `vllm:*_latency_seconds_bucket`, `vllm:num_requests_*`, `vllm:*kv_cache*` |
| EPP / Pool | Prometheus + ConfigMap | `inference_pool_*`, `inference_extension_*`, `llmd-router-epp` cm |
| 토폴로지 / 상태 / 액션 | `kubectl` | Deployment, Pod, HTTPRoute, Gateway, InferencePool, InferenceObjective |
| 스토어 인벤토리 / 출처 | `model-inventory` cm | 빌드 경로, 크기, 컴파일 타깃, 아티팩트가 기록한 toolchain, 스토어 자체 `df` (`manifests/model-store-discovery.yaml`이 스캔) |
| ai-rm (선택) | `kubectl` + Prometheus | `AcceleratorProfile`, `airm_*`, ConfigMap `airm-placement` |

데이터는 두 계층으로 들어옵니다. 가속기와 노드는 약 1초 주기의 fast tier로, 나머지는 약 3초 주기의 full 스냅샷으로
갱신됩니다. 전부 순수 Rust로, Prometheus는 raw `tokio` HTTP/1.0으로, Kubernetes는 `kubectl`로 조회합니다.

EPP 라우팅 정책 플러그인(`heterogeneous-capacity` filter/scorer, `epp_endpoint_score` 내보내기)은
[`contrib/epp-score-observer/`](contrib/epp-score-observer/README.md)에 있습니다. 이미지 빌드·배포는 아직 하지 않았습니다.

## 📚 문서

| 목적 | 문서 |
|---|---|
| 🚀 **설치·연결** | [Quick start](docs/QUICKSTART.md) · [기여 가이드](CONTRIBUTING.md) |
| 🔀 **EPP와 라우팅** | [EPP 기능 가이드](docs/EPP-CAPABILITIES.md) · [EPP 플러그인](docs/EPP-PLUGINS.md) · [이종 분석](docs/HETEROGENEOUS-ANALYSIS.md) |
| 🧾 **데이터와 출처** | [JSON 인벤토리](docs/JSON-INVENTORY.md) · [JSON 소스 품질](docs/JSON-SOURCE-QUALITY.md) · [빌드 출처](docs/BUILD-PROVENANCE.md) |
| 🛡 **품질과 운영** | [운영 강화](docs/OPERATION-HARDENING.md) · [QA 테스트 계획](docs/QA-TESTPLAN.md) · [1.0 준비](docs/V1-READINESS.md) |
| 📍 **진행 상황** | [변경 이력](CHANGELOG.md) · [로드맵](ROADMAP.md) |

## 🗺 로드맵

**지금 바로 되는 것 (트래픽 불필요).** 12개 뷰 전부, 자동 감지·통합 메모리를 포함한 GPU/RBLN/RNGD 및 노드/디스크
모니터링, Flow 토폴로지와 EPP 우회 진단, EPP ConfigMap 조회, 능동 알림, `scale`·`logs` 액션, Serving 섹션과 Deploy 섹션의
Model List + Activity(카탈로그 가능성, RBLN/Furiosa 컴파일·배포 매니페스트 생성과 권한 모드 게이팅 apply), 헤드리스
`--json`·`--doctor`·`--snapshot`·`--cast`, 테마·애니메이션·zoom·권한 모드, 읽기 전용 ai-rm 패널.

**실제 트래픽이 EPP를 거치고 vLLM이 지표를 내보내면 채워지는 것.** 모델별 p95 지연 분해, tok/s, 파드별 큐 분배,
KV%/TTFT/E2E, EPP 요청 분배. (EPP 가중치 `+`/`-`는 로컬 가중치 시뮬레이션이며 클러스터에 적용되지 않습니다.)

- [x] Deploy 뷰에서 권한 모드로 게이팅되는 NPU 컴파일·배포 자동화
- [x] 권한 모드, 감사 로그, `--json` agent 상태, safe action (v0.35)
- [ ] 현재 apply 흐름을 넘어서는 적용형 컨트롤 플레인 액션(트래픽/정책 가중치 적용, rollout — 각각 dry-run → 확인 → 감사)
- [ ] EPP 엔드포인트별 점수 디버거: 점수 소스는 `contrib/epp-score-observer`에 있고, 그 플러그인을 배포하기 전까지는 `–`로 표시
- [ ] [CHANGELOG.md](CHANGELOG.md)의 미릴리스 변경을 릴리스 (`Cargo.toml`은 0.36.0, 최신 태그는 v0.35.0). 실 클러스터·NPU 검증이 아직 필요

**성숙도.** 실제 이종 클러스터에서 검증했습니다(8개 노드, GB10·RBLN·RNGD 가속기, EPP·라우트·모델 모두 라이브). 아직
실험 단계(0.x)라 인터페이스는 바뀔 수 있습니다. 방향과 우선순위: [ROADMAP.md](ROADMAP.md).

## 🤝 기여

이슈와 PR을 환영합니다. [CONTRIBUTING.md](CONTRIBUTING.md)를 참고하세요.

## 📄 라이선스

[Apache-2.0](LICENSE).
