# lmd-top — ROADMAP

> **명제**: lmd-top 은 "Kubernetes 리소스 뷰어"가 아니라 **LLM inference 운영 콘솔**이다 —
> TTFT/TPOT/SLO · KV-cache/prefix routing · Prefill/Decode disaggregation · EPP routing decision 을
> 한 화면에서 보고 **조정**한다. (k9s/btop/nvtop/kubectl 과의 차별점은 전부 여기서 나온다.)

llm-d = Kubernetes-native distributed inference stack (vLLM/SGLang 를 클러스터 production serving 으로 확장).
핵심 요소: **Inference Gateway + EPP scheduler · ModelService · KV-cache aware routing · disaggregated serving**.

> **문서 역할 분리**: 공개 소개는 `README.md`(영어), 릴리스 이력은 `CHANGELOG.md`,
> **이 문서는 방향·우선순위(내부 기획)** 만 다룬다.

---

## 현재 상태 — Phase 1 (Monitor) ✅ 완료 (v0.5.1)

10개 상관 뷰 + 능동 알림 + 액션까지 갖춘 **성숙한 관측 도구**. 더는 "폴리시"가 병목이 아니며,
다음 노력은 **차별화 기능(아래)과 컨트롤 플레인**으로 옮겨간다.

**출시된 뷰(0–9)**: Overview(Σ·LED 그리드·VRAM 구성 바·EPP 경로·진단) · Accel(per-device + area-fill 타임라인) ·
Models(deploy 기반, `service` 조인) · EPP(scorer/weight/picker + 요청분배) · Topo(Gateway→route→pool + SLO/autoscale + **EPP 우회 진단**) ·
Pods · Perf(구간별 p95 **QUEUE→PREFILL→DECODE→TPOT→E2E** + preempt, **런칭 모델 전부 표시**) · Launch(카탈로그×재고, read-only) · Events(k8s+llm-d) · Nodes(health/placement).

**출시된 교차 기능**: 능동 알림(`A` — 임계/헬스 감지 → 요약바 플래시 + 토스트 + 히스토리) ·
로그 오버레이(`l`) · scale 액션(`s`) · Grafana 열기(`g`) · 필터(`/`) · 정렬(`o`) · 드릴다운(`⏎`) ·
스크롤바+위치 카운터 · **데이터 freshness 시계** · 반응형 탭 · 활성 패널 포커스 · 3테마(default/고대비/**색맹**) · zoom(`z`) · pause(space) · 마우스.

---

## 차별화 5선 (killer features) — 아직 이것들이 진짜 차별점

각 항목에 **데이터 의존성** 표기(⚡=지금 구현 가능 / 🔌=인프라 조건 충족 시 채워짐).

1. 🔌 **PD-aware dashboard** — Prefill/Decode 분리: pod 역할, queue, latency(TTFT vs TPOT), **P/D ratio**, KV transfer latency, imbalance detector, replica 권장. *(부분: Perf 에 P/D p95 구간 이미 있음)*
2. 🔌 **EPP decision debugger** — "라우터가 왜 이 pod를?": endpoint 별 score(cache/queue/load/health) 테이블 + 설명 + what-if(scorer weight 조정 시뮬).
3. 🔌 **KV / prefix cache locality map** — pod별 KV usage, prefix hit ratio, hot prefix, eviction/fragmentation.
4. 🔌 **SLO-aware diagnosis** — TTFT vs TPOT 병목, prefill vs decode 부족 자동 판정 + evidence + suggested action. *(부분: Overview 1줄 진단 있음 → 규칙 확장 필요)*
5. ⚡ **Safe management actions** — scale(model/prefill/decode), **endpoint drain**(신규 라우팅 제외 후 stream 종료 시 제거), traffic/policy weight, rollout/rollback — dry-run + confirmation. *(부분: scale 만 구현)*

> 대부분(1–4)은 (1) 트래픽 **EPP(InferencePool) 경유** + (2) 모델서버 **vLLM/SGLang 메트릭 노출** +
> (3) **disaggregation 배포** + (4) **EPP tracing on** 이 선행돼야 채워진다. 현 클러스터는 EPP 우회가 잦아
> Track B 는 **"스켈레톤 먼저, 데이터 오면 자동 표시"**(현 Perf/EPP 와 동일 패턴)로 진행한다.

---

## 두 트랙으로 재편

| | Track A — Control plane | Track B — LLM-native depth |
|---|---|---|
| 성격 | **운영 콘솔화**(보기→조정) | **차별화 지표 심화** |
| 데이터 의존 | ⚡ 없음(kubectl/자체) — **지금 가능** | 🔌 EPP-in-path·vLLM·disagg·tracing 필요 |
| 내용 | 권한 모드 · agent JSON · safe actions · ModelService-native · 외부편집 · 멀티클러스터 | PD 뷰 · EPP decision trace/score table · cache locality · SLO/goodput 진단 |
| 리스크 | 변경 작업 → 안전장치 필수 | 인프라 대기 → UI 스켈레톤 선행 |

---

## ▶ 다음 앵커 마일스톤 — Track A: Control plane

Phase 1 이 "보는 것"을 끝냈으니, 다음은 **안전하게 조정하는 콘솔 + 기계가독 상태**다.
인프라 조건에 안 걸려 **지금 바로** 진행 가능하고, "운영 콘솔" 명제와 human-in-the-loop/agent 방향을 직접 전진시킨다.

### M1 — 권한 모드 (안전장치 먼저)
기동 시 모드 선택(`--mode observe|debug|admin|danger`), 헤더에 상시 표시. 모든 변경 작업은 모드 게이트 + confirmation.

| 모드 | 허용 |
|---|---|
| **Observe** | 보기만(기본) |
| **Debug** | logs · traces · port-forward · **dry-run** |
| **Admin** | scale · rollout restart · policy(weight) change |
| **Danger** | delete pod · force rollout · traffic cutover |

- 현재 `s`(scale)/`g`(grafana) 를 모드 게이트 뒤로. 위험 작업은 2단계 확인(diff 미리보기 → 확정).

### M2 — Agent JSON (기계가독 상태)
`lmd-top --snapshot --json` — 현 텍스트 스냅샷을 **스키마화**해 AI agent 가 화면 파싱 없이 상태·가능 액션을 이해.
```json
{ "screen": "...", "cluster": {...}, "models": [...], "accelerators": [...],
  "alerts": [{ "sev": "...", "key": "...", "msg": "..." }],
  "symptoms": [...],
  "actions": [{ "id": "scale:ds4:1", "label": "...", "risk": "admin", "requires_confirmation": true }] }
```
- 이미 `--snapshot`(텍스트)/`--render` 존재 → JSON 출력 경로 추가가 최소 변경.
- 논문 확장 각: *"TUI for human-in-the-loop LLM serving management"* — Human UI(terminal) ∥ Agent UI(JSON) ∥ Control API(typed actions) ∥ Audit log(who/what/why/when).

### M3 — Safe actions 확장 (killer #5)
scale 를 넘어 **endpoint drain**(즉시 kill 아님: 신규 라우팅 제외 → 진행 stream 종료 후 제거) · traffic/policy weight · rollout/rollback.
- 전부 dry-run(diff) → confirm → apply, Audit log 기록. 권한 모드(M1)·JSON 액션(M2)과 한 몸.

> **M1→M2→M3 순서 이유**: 안전장치(모드) 없이 액션(M3)을 늘리면 사고 위험. M2(JSON)는 M1의 권한/액션 모델을 그대로 직렬화하므로 중간에 둔다.

---

## Track B 백로그 (인프라 충족 시, 스켈레톤 선행)

1. **PD 뷰** — prefill/decode pod 역할·queue·P/D ratio·KV transfer latency·imbalance → replica 권장. (Perf 의 P/D p95 를 뷰로 승격)
2. **EPP decision trace + score table** — endpoint별 cache/queue/load/health score + pick 이유 + what-if(weight 시뮬).
3. **Cache locality 뷰** — pod별 KV/prefix hit·hot prefix·eviction.
4. **SLO/goodput 진단 확장** — 아래 규칙표를 Overview 1줄 진단에서 전용 뷰로.

### 진단 규칙 (SLO-aware, 출력: Primary symptom → Likely cause → Evidence(3) → Suggested action)
| 관측 | 가능 원인 |
|---|---|
| TTFT만↑ | prefill queue 포화, 긴 prompt↑, prefill GPU 부족 |
| TPOT만↑ | decode GPU 부족, KV bandwidth 병목 |
| 둘 다↑ | overload, routing 실패, network |
| cache hit↓ | prefix locality 깨짐, policy 문제, eviction↑ |
| GPU 낮은데 latency↑ | queueing, KV transfer, scheduler 병목 |
| 특정 pod만 느림 | hot shard, bad placement, thermal cap, noisy neighbor |
| 5xx↑ | crash, OOM, readiness 실패 |

---

## ModelService-native (Track A/B 공통 기반, 별도 트랙)

현재 Models 는 **raw Deployment** 기반. llm-d 는 **ModelService CRD + preset/values/artifact** 로 모델을 정의 →
Models/Launch 를 ModelService 인지로 전환하면 (a) 배포 스펙·프리셋 가시화, (b) Launch write(실제 배포), (c) PD 구조 인지가 자연스러워진다. kubectl 기반이라 ⚡ 지금 가능하나 llm-d CRD 스키마 조사 선행.

---

## UI 레퍼런스 패턴 (awesome-tuis / talos-pilot / k8s-tui / btop / all-smi)

| 레퍼런스 | 차용 패턴 | lmd-top 상태 |
|---|---|---|
| **talos-pilot** (Rust/ratatui — 동일 스택) | 니모닉 단일키, `Esc`=뒤로, auto-refresh 토글, 멀티서비스 로그, **진단+실행가능 fix**, PDB-aware drain | Esc/로그/진단/pause ✅ · drain/`a` 토글 ⬜(M3) |
| **k8s-tui** (Go/bubbletea) | list→detail cascade, **외부 $EDITOR edit**, 멀티클러스터(Ctrl+←→), Lua 플러그인 | list/detail ✅ · edit/멀티클러스터 ⬜ |
| **btop / all-smi / nvtop** | 그라디언트 그래프, per-device 스택, area-fill, LED 그리드 | 그라디언트바·area-fill·LED·per-device·드릴다운 ✅ |
| **grafterm** | singlestat 대형 숫자, 게이지 | ⬜ (QPS/TTFT/goodput 대형 패널 후보) |

### 확정된 UI 제작 원칙
- **collectors(IN, 단방향) → Snapshot bus → panels(OUT)**. fast tier(가속기+노드 1s) + full(3s).
- 뷰별 `order()/list_len()` 로 선택·정렬·필터 일원화. detail_panel = 선택 엔티티 다중지표 타임라인.
- 반응형(two_panes 폭<100 세로 스택·nice_ceil·zoom·반응형 탭). **semantic colors**(색=심각도/vendor, 상태=글리프).
- **스켈레톤 먼저, 데이터 오면 자동 표시** — 부재 시 `–`.

---

## 요약: 다음 3스텝
1. **M1 권한 모드** — 변경 작업 안전장치(observe/debug/admin/danger + confirm). ⚡
2. **M2 agent JSON** — `--snapshot --json` 스키마(상태+가능 액션). ⚡
3. **M3 safe actions** — drain/weight/rollout(dry-run→confirm→apply + audit). ⚡

> 이후 인프라(EPP-in-path·tracing) 켜지면 Track B(PD·decision trace·cache·SLO) 를 스켈레톤부터 채운다.
