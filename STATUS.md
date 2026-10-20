# STATUS — lmd top 클러스터 모니터링 TUI

<!-- research-hub:header:begin (maintained by .research/report.py; keep the "- **Key**: value" format) -->
- **State**: idle
- **Updated**: 2026-10-16 13:24+09:00
- **Agent**: claude@control-node
- **Task**: Furiosa SDK 2026.4 대응: 이미지 고정 + serve -tp 버전 분기 (done)
- **Last result**: 기본 이미지 2026.3.0 고정, 2026.4+ 이미지는 --tensor-parallel-size 생략; 라이브 k-exaone/qwen3-4b 배포(replicas 0)도 2026.3.0 고정; 테스트 425+11, clippy, golden 12/12
- **Next**: 2026.4 공식 이미지 나오면 FXB 재빌드 후 전환
- **Blockers**: none
<!-- research-hub:header:end -->

## Goal

llm-d 클러스터용 터미널 관측·운영 콘솔을 만든다. 'Kubernetes 리소스 뷰어'가 아니라 TTFT/TPOT/SLO, KV/prefix 라우팅, PD 분리, EPP 라우팅 결정을 한 화면에서 보고 조정하는 LLM inference 운영 콘솔이 목표다 (`lmd-top/ROADMAP.md`). 코드는 독립 git 레포 `lmd-top/`(github gspark-etri/lmd-top, 부모 레포에서 추적 제외)에 있다. `lmd-top-autoloop/`은 같은 레포의 git worktree(브랜치 `auto-loop`)다. 1.0 준비 기준은 `lmd-top/docs/V1-READINESS.md`에 있다.

## Findings

최신순. 출처를 괄호에 적었다.

- **2026-10-16 · npu-lora 학습 연동이 작업 트리에 있다(미커밋).** `lmd-top --plan train`이 npu-lora 학습 Job을 만든다. 소스는 `assets/npu-lora/`에 벤더링했고 연동 코드는 `src/app/train.rs`다(둘 다 `??` 미추적). 2026-10-15 RNGD Llama-3.1-8B 학습 PoC가 이 경로(`--plan train … --apply`)로 돌았다 (`npu-lora/README.md`, `npu-lora/STATUS.md`).
- **2026-10-14 · Scaling/EPP 증빙은 시연이다.** 공식 릴리스 UI를 시험 데이터로 재현했고, 운영 클러스터 캡처가 아니다. 복제본 전환은 0↔1이고, EPP 확장은 플러그인 구현·설정 변경·재기동 방식이다 (자원관리 과제 `Y2/04_성과및증빙/02_SW개발/lmd-top/20261014_Scaling_EPP증빙/`. 이 노드에는 없는 경로라 원본은 확인하지 못했다).
- **2026-10-14 · EPP 설정 그룹을 바꿨다.** EPP `:main` 새 빌드는 EndpointPickerConfig 그룹으로 `llm-d.ai/v1alpha1`만 받는다. lmd-top 라우팅 매니페스트도 이에 맞췄다 (`src/app/deploy/routing.rs:71`).
- **2026-09-30 · 성숙도를 잠정 자체 평가했다.** 이종 자원 검색·관측(A1·A2)과 지표 수집(E1)은 4단계 중 3단계다. 배치·배포·운영 기능은 대부분 2단계다(코드와 회귀 테스트는 있고 실증은 없음). SLO 폐루프 확장, 동일 조건 성능 개선, 장시간 운영, 대규모 처리량은 자료가 없어 NA다 (`docs/analysis/resource-management-assessment-20260930.csv`, 부모 레포).
- **2026-09-28 · 미릴리스 변경이 쌓여 있다(작업 트리, 133개 파일 변경/추가).** `CHANGELOG.md` [Unreleased]에 정리돼 있다.
  - 운영 정확성: compile history 키에 Job UID를 넣었다. HTTPRoute 편집이 형제 match와 weight를 보존한다. endpoint 제외는 실제 selector로 계산하고, 검증에 실패하면 롤백을 시도한다. subprocess에 상한을 뒀다. 빌드 산출물은 staging 디렉터리를 거쳐 rename으로 게시한다.
  - 라우팅 정책: `heterogeneous-capacity` EPP filter/scorer, `--policy init/validate/render/plan/test/replay/build`, TUI `:Routing policy`를 넣었다. 업스트림 등록 플러그인 72종을 인벤토리로 만들었다. 라이브 롤아웃과 벤치마크는 하지 않았다.
  - 셋업: `--init`, `--check-setup`, `--version`을 추가했다. 다음 릴리스에 포함될 예정이다 (`README.ko.md`).
  - 2026-09-28 기준선은 Cargo 0.36.0, Rust 단위 테스트 247개 + CLI 테스트 8개 통과다 (`docs/V1-READINESS.md`).
- **2026-09-10 · 파괴적 액션 ACT-01..08을 처음 실행했고 모두 통과했다.** 스크래치 네임스페이스 `lmd-qa`에서 실제 TUI를 pty로 돌렸다. 이 과정에서 제품 버그 2개를 찾아 고쳤다: replica 토글이 스냅샷 지연으로 뒤집히지 않던 문제, confirm 팝업 위에서 토스트가 안 보이던 문제 (`docs/QA-ACT-2026-09-10.md`, `5c00e66`).
- **2026-09-10 · 공유 모델 스토어 관리 기능을 넣었다.** 빌드 삭제·이동, 인벤토리 재스캔, 남은 용량, 빌드 toolchain 표시, 노드 로컬 아티팩트 provenance 프로브(Serving▸`P`)를 추가했다 (`888c11d`~`9c9fa86`).
- **2026-09-08~10 · 이 클러스터의 RBLN decoder-only 컴파일이 전부 실패한다.** 원인은 lmd-top이 아니다. 정리한 결론은 이렇다: decoder-only(Llama·Qwen) 경로는 optimum-rbln 0.10.2에서 성공하고 0.10.3과 0.11.0.post1에서 실패한다. Gemma4(conditional-generation)는 0.11.0.post1에서 성공한다. 옵션·모델·실행 환경은 원인에서 배제했다. 벤더 dev 빌드나 지원이 필요하다. lmd-top은 이제 실패 종류와 toolchain을 history에 남기고 다음 시도를 제안한다 (`docs/RBLN-COMPILE-INCIDENT.md`, `docs/RBLN-SUPPORT-ENQUIRY.md`, `34cc39f`, `9c9fa86`).
- **2026-09-08 · 라이브 클러스터 QA에서 18개 버그를 고쳤고, 리팩터링 6단계를 마쳤다.** 버그는 P0 데이터 정확도, injection, 시작 시 멈춤 등이다 (`3f0952f`, `docs/QA-RUN-2026-09-08.md`, `docs/BUGS-2026-09-08.md`). 리팩터링으로 가속기는 선언형 팩(`src/accel/`, 런타임 `~/.config/lmd-top/accelerators/*.yaml`)이 됐다. 매니페스트는 `serde_yaml` 직렬화로 바꾸고 골든 테스트로 고정했다. 레시피는 에셋으로 옮겼다. 계획이 틀렸던 두 곳은 `docs/REFACTOR-PLAN.md` §4.1에 적었다 (`c9a616e`~`06c9fd5`).
- **2026-07-08 · v0.36.0에서 EPP decision debugger의 score 소스를 확보했다.** `contrib/epp-score-observer` 커스텀 EPP picker 플러그인이 `epp_endpoint_score{pod}`를 내보내고, lmd-top이 PromQL로 조인한다. 플러그인을 배포하지 않으면 '–'로 표시한다. endpoint drain과 rollback도 이때 들어갔다 (`ROADMAP.md`, `3a8cce2`, `0c312d1`). Cargo 버전은 0.36.0이다. 다만 git 태그는 v0.35.0까지만 있고 v0.36.0 태그는 없다.
- **v0.35 · Track A(Control plane) 핵심을 출시했다.** 권한 모드 `--mode observe|debug|admin|danger`와 audit 로그(`src/audit.rs`), `--json` agent 상태(스키마 `lmd-top/agent-state/v2`, `src/agent.rs`), safe action(scale, stop, restart/undo, endpoint drain, cordon, route 편집 등)이 들어갔다 (`ROADMAP.md`).
- **v0.31 · Phase 1(Monitor)을 마쳤다.** 10개 상관 뷰, 능동 알림, 액션, 배포 라이프사이클 관측이 들어갔다 (`ROADMAP.md`).
- **`lmd-top-autoloop/`은 낡은 worktree다.** lmd-top 레포의 브랜치 `auto-loop` worktree로, 마지막 커밋은 `b4940a4`(2026-07-03, "WIP baseline snapshot before auto-improve loop"), Cargo는 0.34.0이다. `auto-loop`과 `auto-improve`는 둘 다 main에 이미 병합됐다 (`git worktree list`, `git merge-base`).

## Open questions

- **epp-score-observer를 배포해야 한다.** `contrib/epp-score-observer` 플러그인 이미지를 빌드·배포하는 일이 남았다. scorer별(queue/kv/prefix) 분해에는 scorer 레벨 계측이 필요하다 (`ROADMAP.md`).
- **Track B는 인프라가 먼저다.** PD 뷰, cache locality, SLO 진단 지표는 EPP 경유 트래픽, vLLM 메트릭, disaggregation 배포, EPP tracing이 있어야 채워진다. 현재 클러스터는 EPP 우회가 잦다 (`ROADMAP.md`).
- **미릴리스 변경을 정리해야 한다.** 작업 트리에 133개 변경이 있고 v0.36.0 태그도 없다. 라우팅 정책·운영 정확성 변경은 실 클러스터/NPU 검증 전이다 (`CHANGELOG.md` [Unreleased]).
- **Furiosa SDK 2026.4에서 serving이 깨질 수 있다.** 2026.4 `furiosa-llm serve`는 `--tensor-parallel-size`를 거부한다(2026-10-15 세션 실측, 레포에 원자료 없음). lmd-top은 Furiosa serving 매니페스트에 이 플래그를 여전히 넣는다 (`src/app/deploy/serving.rs:165,225,357`). 이미지가 2026.4로 바뀌기 전에 고쳐야 한다.
- **RBLN decoder-only 컴파일 회귀는 벤더 지원이 필요하다.** 문의서(`docs/RBLN-SUPPORT-ENQUIRY.md`)는 "Ready to send" 상태다. 실제로 보냈는지는 레포에 기록이 없다.
- **SW 등록 증빙이 없다.** 등록 완료는 연구자 진술뿐이고, 등록증(등록일, 2026 귀속)이 레포에 없다 (자원관리 과제 `STATUS.md`, 이 노드에서는 확인 불가).
- **krew 중앙 인덱스 등록 전이다.** 지금은 자체 매니페스트로 설치한다 (`README.ko.md`).
- **`lmd-top-autoloop/` worktree를 지울지 정해야 한다.** 내용이 main에 병합돼 남길 이유가 확인되지 않는다.

## Log

<!-- research-hub:log (report.py inserts new entries below this line, newest first) -->

### 2026-10-16 13:24+09:00 — Furiosa SDK 2026.4 대응: 이미지 고정 + serve -tp 버전 분기 ✅

- Result: 기본 이미지 2026.3.0 고정, 2026.4+ 이미지는 --tensor-parallel-size 생략; 라이브 k-exaone/qwen3-4b 배포(replicas 0)도 2026.3.0 고정; 테스트 425+11, clippy, golden 12/12
- Next: 2026.4 공식 이미지 나오면 FXB 재빌드 후 전환
- By: claude@control-node · main@1eb3665-dirty

### 2026-10-16 11:57+09:00 — 가속기 커버리지: 수집 실패를 0이 아닌 unknown으로 ✅

- Result: Prometheus 장애 시 Overview/snapshot/JSON이 'accel unknown'+원인 표시(0% 대신 –), 노드 라벨 대비 exporter 미보고 장치를 원인과 함께 표시(DiskPressure 등), 반복 에러 ×N 축약. 424+11 테스트·clippy 통과, 라이브 정상상태 오탐 0
- Next: Furiosa 2026.4 serve --tensor-parallel-size 대응(serving.rs), CrashLoop 원인 로그 한 줄, 이미지 떠다니는 태그/arch 불일치 감지
- By: claude@control-node · main@1eb3665-dirty

### 2026-10-16 11:47+09:00 — STATUS.md 초기 작성 ✅

- Result: 초안 검증·보완: v0.36.0 태그 없음(v0.35.0까지), 09-08 QA 18버그·리팩터링, 09-10 ACT-01..08, RBLN decoder-only 회귀(0.10.2만 성공), 미릴리스 133개 변경(routing policy 등), autoloop=main에 병합된 낡은 worktree로 정정
- Next: Furiosa 2026.4 serve의 --tensor-parallel-size 거부 대응(serving.rs), 미릴리스 변경 정리·v0.36.0 태그, epp-score-observer 배포
- By: claude@control-node · main@1eb3665-dirty

### 2026-10-16 11:43+09:00 — STATUS.md 초기 작성 ✅

- Result: STATUS.md 초안 작성(hub에서 저장소 스냅샷 기반, 노드 agent 검토 필요)
- Next: epp-score-observer 플러그인을 클러스터에 배포해 EPP decision debugger의 score 열을 실데이터로 채우고, Track B의 PD 뷰를 스켈레톤부터 구현한다.
- By: research-hub@control-node · main@1eb3665-dirty
