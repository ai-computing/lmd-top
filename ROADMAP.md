# lmd-top — ROADMAP

> **명제**: lmd-top 은 "Kubernetes 리소스 뷰어"가 아니라, **LLM inference 운영 콘솔**이다 —
> TTFT/TPOT/SLO · KV-cache/prefix routing · Prefill/Decode disaggregation · EPP routing decision 을
> 한 화면에서 보고 조정한다. (k9s/btop/nvtop/kubectl 과의 차별점은 전부 여기서 나온다.)

llm-d = Kubernetes-native distributed inference stack (vLLM/SGLang 를 클러스터 production serving 으로 확장).
핵심 요소: **Inference Gateway + EPP scheduler · ModelService · KV-cache aware routing · disaggregated serving**.

---

## 차별화 5선 (killer features)

1. **PD-aware dashboard** — Prefill / Decode 분리: pod 역할, queue, latency(TTFT vs TPOT), GPU util, **P/D ratio**, KV transfer latency, imbalance detector, replica 권장.
2. **EPP decision debugger** — "라우터가 왜 이 pod를 골랐는가": endpoint 별 score(cache/queue/load/health) 테이블 + 설명 + dry-run/what-if(scorer weight 조정 시뮬).
3. **KV / prefix cache locality map** — pod별 KV usage, prefix hit ratio, hot prefix, eviction/fragmentation, "어느 decode pod가 어떤 prefix를 갖는가".
4. **SLO-aware diagnosis** — "TTFT 병목 vs TPOT 병목", "prefill 부족 vs decode 부족" 자동 판정 + evidence + suggested action(explain→suggest→confirm→apply).
5. **Safe management actions** — scale(model/prefill/decode), **endpoint drain**(즉시 kill 아님: 신규 라우팅 제외 후 stream 종료 시 제거), traffic weight, policy weight, rollout/rollback — dry-run + confirmation 기반.

---

## LLM-native 지표 카탈로그 (일반 k8s 지표 아님)

| 범주 | 지표 |
|---|---|
| 요청 성능 | QPS, active/queued requests, admission rejected |
| 지연 | TTFT, TPOT, E2E — P50/P90/P99 |
| SLO | violation rate, **goodput**, deadline miss |
| 토큰 | input/output tokens·s, generated tokens/req |
| GPU/NPU | util, mem, **KV cache mem**, OOM risk |
| 라우팅 | selected backend, cache-aware hit ratio, **EPP decision reason** |
| PD 분리 | prefill/decode queue, P/D ratio, KV transfer latency |
| 장애 | 5xx, timeout, failed pods, unhealthy endpoints |

> ⚠ **데이터 가용성**: 대부분은 (1) 트래픽이 **EPP(InferencePool) 경유** + (2) 모델서버가 **vLLM/SGLang 메트릭 노출**(표준 서버) + (3) **disaggregation 배포** + (4) **EPP tracing on** 이어야 채워진다. lmd-top 은 부재 시 `–`로 표기하고, 조건 충족 시 자동 표시.

---

## 목표 뷰 구조 & 현재 상태 매핑

```
lmd-top
├── overview   # 모델별 serving 상태(QPS/TTFT/TPOT/SLO/health)   [현재: Overview — 확장 필요]
├── model      # ModelService 상세(replicas/preset/values/artifact)[신규: Models는 Deploy 기반 → ModelService화]
├── pd         # prefill/decode 분리 상태                          [신규 ★차별]
├── epp        # routing decision/debug + score table              [현재: EPP scorers+설명 → decision trace 확장]
├── cache      # KV/prefix cache locality                          [신규 ★차별]
├── requests   # active/slow/failed requests, tenant               [신규]
├── gpu        # node/pod/GPU 매핑, placement                      [현재: Accel — placement 확장]
├── events     # k8s + llm-d events (join by req/model/pod)        [신규]
└── actions    # scale/drain/rollout/dry-run (권한모드)            [부분: scale · Launch(read-only)]
```

**현재 구현됨(v0.1)**: Overview · Accel · Models(deploy기반) · **EPP(scorer+설명)** · Topo(Gateway→route→pool+SLO+autoscale+EPP우회진단) · Pods · **Perf(구간별 지연 p50/95/99·토큰·per-model·timeline chart)** · **Launch(카탈로그×재고 read-only)**. scale 액션, 필터, 정렬, 테마, 마우스.

---

## MVP 티어 (우선순위)

**MVP1 — 관측** (일부 완료): ModelService 목록 · 모델별 QPS/TTFT/TPOT/error/queue · **PD pod 상태** · KV cache hit · EPP health · pod별 GPU mem/util · **logs/events 통합**.
→ *현재*: 모델/EPP/Perf/Accel 있음. **미비: ModelService 단위, PD 뷰, events 통합, cache hit.**

**MVP2 — 디버깅** (연구/데모성): **request trace · EPP decision trace · endpoint score table** · cache hit/miss 분석 · TTFT/TPOT 병목 진단 · long-context/top-tenant.

**MVP3 — 관리** (운영 도구): prefill/decode scale · **endpoint drain** · traffic weight · policy weight · rollout/rollback · recommendation + confirm-apply.

---

## 권한 모드 (운영 사고 방지)

| 모드 | 동작 |
|---|---|
| **Observe** | 보기만 |
| **Debug** | logs, traces, port-forward, **dry-run** |
| **Admin** | scale, rollout restart, policy change |
| **Danger** | delete pod, force rollout, traffic cutover |

→ 기동 시 모드 선택(`--mode observe|admin`), 헤더에 표시. 변경 작업은 모드 + confirmation 필요.

---

## Agent-friendly (human-in-the-loop)

Human UI(terminal) 외에 **machine-readable state** 를 병행 노출 → AI agent 가 화면 파싱 없이 상태·가능 액션 이해.
```
Human UI: terminal layout   Agent UI: JSON state tree   Control API: typed actions   Audit log: who/what/why/when
```
- `lmd-top snapshot --json` (이미 --snapshot 텍스트 존재 → JSON 스키마화): `{screen, model, symptom, metrics{...}, actions[{id,label,risk,requires_confirmation}]}`.
- 논문 확장: "TUI for human-in-the-loop LLM serving management".

---

## 진단 규칙(예시, SLO-aware)

| 관측 | 가능 원인 |
|---|---|
| TTFT만↑ | prefill queue 포화, 긴 prompt↑, prefill GPU 부족 |
| TPOT만↑ | decode GPU 부족, KV bandwidth 병목 |
| 둘 다↑ | overload, routing 실패, network |
| cache hit↓ | prefix locality 깨짐, policy 문제, eviction↑ |
| GPU 낮은데 latency↑ | queueing, KV transfer, scheduler 병목 |
| 특정 pod만 느림 | hot shard, bad placement, thermal cap, noisy neighbor |
| 5xx↑ | crash, OOM, readiness 실패 |

출력: `Primary symptom → Likely cause → Evidence(3) → Suggested action [s]scale [p]inspect [r]reduce`.

---

## 다음 스텝 제안 (이 로드맵 기준)
1. **PD 뷰** + **EPP decision trace/score table** (차별화 1·2, MVP2 핵심) — 단 트래픽/tracing 인프라 선행.
2. **ModelService 단위** 전환 (Models → ModelService CRD/Helm 인지).
3. **events 통합** + **cache 뷰** (MVP1 완성).
4. **권한 모드** + **agent JSON** 스키마.

> 대부분의 LLM-native 지표는 인프라 조건(EPP-in-path·vLLM 메트릭·disaggregation·tracing) 충족 시 채워지므로,
> 뷰/스켈레톤을 먼저 만들고 데이터가 흐르면 자동 표시되는 방식으로 진행(현재 Perf/EPP 와 동일 패턴).

---

## 참고 TUI 분석 → UI/기능 패턴 (awesome-tuis / talos-pilot / k8s-tui / btop / all-smi / AdGuardian)

| 레퍼런스(스택) | 차용할 패턴 | lmd-top 상태 |
|---|---|---|
| **talos-pilot** (Rust/ratatui/tokio — 동일 스택) | 니모닉 단일키 화면(`c/s/l/p/n`…), `Esc`=뒤로, `a` auto-refresh 토글, 멀티서비스 로그(Stern식), **진단+실행가능한 fix**, PDB-aware drain | Esc back ✅ · 진단 부분 ✅ · 로그/drain/`a` ⬜ |
| **k8s-tui** (Go/bubbletea) | tab→list→detail→**edit(외부 $EDITOR)** cascade, **Lua 플러그인**, JSON 테마/config, **멀티클러스터 전환(Ctrl+←→)** | list/detail ✅ · edit/멀티클러스터/플러그인 ⬜ |
| **btop** | 그라디언트 그래프, 적응형 레이아웃, per-core 미터 그리드 | 그라디언트바 ✅ · 반응형 two_panes ✅ · 종류별 지표 그리드 ✅ |
| **all-smi / nvtop** | per-device 스택 그래프, 축+현재값, 프로세스/pod per-device | 드릴다운 라인차트 ✅ · Accel per-device ✅ |
| **AdGuardian** | 명확한 테두리 + 균형 비율 그리드 | 둥근테두리·two_panes·nice_ceil ✅ |
| **grafterm** | singlestat 대형 숫자, 게이지 | ⬜ |

### UI 제작 원칙(정립)
- **collectors(IN, 단방향) → Snapshot bus → panels(OUT)**. fast tier(가속기+노드 1s, join! 병렬) + full(3s).
- **뷰별 order()/list_len()** 로 선택·정렬·필터 일원화. detail_panel 은 선택 엔티티의 다중지표 라인차트.
- **반응형**: two_panes(폭<100 세로 스택), nice_ceil(축 상한), zoom(포커스). **semantic colors**(색=심각도/vendor, 상태=글리프).
- **지표 분리**: host cpu/mem · GPU util/mem · NPU util/mem 을 종류별 + per-device(드릴다운).

### 향후 기능 백로그(우선순위)
1. **Logs 뷰(`l`)** — 선택 pod의 kubectl logs + `/`검색(talos/k8s-tui 공통 핵심). 실데이터 지금 가능.
2. **외부 편집(`e`)** — EPP ConfigMap/deploy를 `$EDITOR`로(k8s-tui). safe-action.
3. **auto-refresh 토글(`a`) / 간격 조절** (talos). (space 일시정지는 구현됨)
4. **멀티클러스터 컨텍스트 전환**(Ctrl+←→, k8s-tui).
5. **singlestat 대형 숫자 패널**(grafterm) — QPS/TTFT/goodput.
6. **니모닉 단일키 네비**(talos) — 숫자키 외 첫글자.
7. **권한 모드**(observe/admin/danger) + **agent JSON**(DESIGN §11 방향).
8. 인프라 켜지면: PD 뷰 · EPP decision trace · cache locality.
