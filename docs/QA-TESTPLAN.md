# lmd-top QA 테스트 시나리오 (v0.34.x 기준)

> 목적: 릴리스 전 수동/자동 검증 체크리스트. 코드 리뷰(2026-07-02)에서 발견된 결함은
> **REG-**(회귀) 섹션에 분리 — 수정 전에는 "실패 예상"이 정상이며, 수정 후 회귀 기준으로 승격한다.
>
> 실행 기록은 이 파일을 복사해 릴리스별로 채운다(✅/❌/⏭ + 비고).

## 0. 테스트 환경 매트릭스

| 축 | 값 |
|---|---|
| 터미널 | truecolor(kitty/alacritty/wezterm) · 256색(xterm) · tmux 경유 |
| 크기 | 표준(≥120×40) · 좁음(80×24) · 극단(20×5, 리사이즈 도중) |
| 클러스터 | 라이브(정상) · Prometheus 차단 · kubectl RBAC 제한 · 완전 오프라인 |
| 모드 | observe(기본) · debug · admin · danger |
| 테마 | soft(기본) · default · high-contrast · colorblind |

우선순위: **P0** = 릴리스 차단(안전/데이터 정확성) · **P1** = 주요 기능 · **P2** = 미관/편의.

---

## A. CLI / 헤드리스 (자동화 가능 — CI 후보)

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| CLI-01 | 도움말 | `lmd-top --help` / `-h` | 사용법 출력, exit 0 | P1 |
| CLI-02 | 미지 플래그 | `lmd-top --nope` | 오류 + 도움말, **exit 2** | P1 |
| CLI-03 | 모드 검증 | `--mode admin` / `--mode xyz` | 유효값 기동 / 무효값 exit 2 | P0 |
| CLI-04 | 스냅샷 | `--snapshot` | 1회 수집 후 텍스트 요약, exit 0 | P1 |
| CLI-05 | Agent JSON | `--json \| jq .` | 유효 JSON, `schema: lmd-top/agent-state/v2` | P0 |
| CLI-06 | 렌더 스모크 | `LMD_W/LMD_H` ∈ {1,3,20,80}×{1,3,10,40}로 `--render` | 전 뷰(허브 서브뷰 포함) 패닉 없음 | P0 |
| CLI-07 | 닥터 | `--doctor` | exporter/메트릭 커버리지 표, 결측 항목 명시 | P1 |
| CLI-08 | 캐스트 | `--cast /tmp/t.cast` | asciicast v2 유효 파일 생성 | P2 |

## B. 기동·환경 견고성

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| ENV-01 | Prometheus 다운 | `LMD_PROM=1.2.3.4:9` 로 기동 | TUI는 뜨고, 경고 표시. 프리즈 없음. kube 데이터는 정상 | P0 |
| ENV-02 | Prometheus 지연 | tc/프록시로 5s+ 지연 주입 | tick이 밀려도 UI 반응(키 입력) 유지, freshness 시계로 낙후 표시 | P0 |
| ENV-03 | 잘못된 NS | `LMD_NS=no-such-ns` | 빈 뷰 + 원인 식별 가능한 경고(무언의 공백 금지) | P1 |
| ENV-04 | kubectl 없음/kubeconfig 오류 | PATH에서 제거 후 기동 | 명확한 오류 메시지, 패닉·행 없음 | P1 |
| ENV-05 | RBAC 제한 | get pods 권한 없는 SA로 | 해당 뷰 경고 표시(빈 뷰≠권한오류 구분 — 현재 일부 컬렉터 무경고, §REG-08 참조) | P1 |
| ENV-06 | 설정 파일 오타 | `lmd-top.yaml`에 잘못된 YAML | 무시하더라도 경고 1줄(현재는 조용히 기본값 — 개선 대상) | P2 |
| ENV-07 | 터미널 복원 | TUI 중 SIGINT / 강제 패닉 | 터미널 raw mode 해제·커서 복원(panic hook) | P0 |

## C. TUI 탐색·뷰 (라이브 클러스터)

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| NAV-01 | 탭 순환 | Tab/Shift+Tab, 숫자키 0–7 | 8개 뷰 왕복, 순서 일관 | P1 |
| NAV-02 | 허브 전환 | Nodes 뷰에서 `w` 반복 | Nodes↔Accel↔Perf↔Topology 순환, 선택 상태 유지 | P1 |
| NAV-03 | 드릴다운 | 각 뷰에서 `⏎` → Esc | 상세 진입/복귀, 선택 인덱스 보존 | P1 |
| NAV-04 | 필터 | `/` + 문자열, CJK 포함 | 실시간 필터, Esc 해제, 카운터 갱신 | P1 |
| NAV-05 | 정렬 | `o` 반복 | 정렬 키 순환, 헤더에 표시 | P2 |
| NAV-06 | 오버플로 | 행 수 > 화면(창 축소로 유도) | 스크롤바/`+N more`, 무언의 잘림 없음 | P1 |
| NAV-07 | 일시정지/줌 | `p`(pause) 중 값 고정, `z` | pause 표시 명확, zoom 왕복 무손실 | P2 |
| NAV-08 | 마우스 | 리스트 위 스크롤/클릭 | 선택 이동. **오버레이 열림 중엔 무시**(§REG-01) | P0 |
| NAV-09 | 리사이즈 폭풍 | TUI 중 창 크기 연속 변경(극소 포함) | 패닉·잔상 없음, 반응형 탭 전환 | P0 |
| NAV-10 | 애니메이션 | `f` 토글, 테마 변경 중 전환 | 이펙트 on/off 즉시 반영, off 시 성능 영향 0 | P2 |

## D. 권한 모드 게이팅 — 전 모드 × 전 액션 매트릭스 (P0)

각 모드로 기동해 아래 표의 **차단**이 실제로 차단되는지(토스트로 사유 표시), 허용이 동작하는지 확인.

| 액션 | observe | debug | admin | danger |
|---|---|---|---|---|
| 보기/필터/드릴다운 | ✅ | ✅ | ✅ | ✅ |
| 로그 `l` | ❌ | ✅ | ✅ | ✅ |
| dry-run 검증 `v` | ❌ | ✅ | ✅ | ✅ |
| scale `s` / stop `x` / rollout `S` | ❌ | ❌ | ✅ | ✅ |
| manifest apply `a` | ❌ | ❌ | ✅ | ✅ |
| compile/deploy 폼 `c`/`d` | ❌ | ❌ | ✅ | ✅ |
| pod delete / cordon | ❌ | ❌ | **❌(의도)** | ✅ |

- PERM-01: 표의 모든 ❌ 셀에서 키 입력 → Pending 미생성 + 거부 사유 토스트. **P0**
- PERM-02: 헤더에 현재 모드 상시 표시, 모드는 런타임 변경 불가. **P1**
- PERM-03: 액션 메뉴(Enter)에도 동일 게이트 적용 — 단축키 경로와 메뉴 경로의 게이트 불일치 없어야 함. **P0**
- ⚠ 현재 `Danger` 게이트는 코드에 없음(delete가 admin에서 열림) — §REG-02. 표는 **의도 기준**.

## E. 액션 메뉴·폼·확인 다이얼로그

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| FORM-01 | 메뉴 발견성 | Deploy/Models/Pods에서 `⏎` | 컨텍스트에 맞는 액션만 노출(예: 노드엔 Cordon, pod엔 Logs) | P1 |
| FORM-02 | 메뉴 대상 고정 | 메뉴 연 뒤 (키보드로) 선택 이동 시도 | 메뉴의 subject 불변 — 실행 대상 = 메뉴 연 시점의 항목 | P0 |
| FORM-03 | compile 폼 순환 | 필드 이동/값 순환, fit 판정 표시 | OOM/tight 추정이 옵션 변경에 즉시 반응 | P1 |
| FORM-04 | 커스텀 입력 | `e` 로 각 필드에 특수문자(`"` `:` 공백·CJK) 입력 | 매니페스트가 여전히 유효 YAML(§REG-04) + `v` 서버 dry-run 통과 | P0 |
| FORM-05 | 편집 취소 규약 | 편집 중 Esc | 타이틀 문구("Enter/Esc confirm")와 실동작 일치 | P2 |
| FORM-06 | 확인 다이얼로그 | Pending 생성 → `y`/`n`/무관한 키 | y만 실행, 그 외 전부 취소. 프롬프트에 **대상 이름+값** 명시 | P0 |
| FORM-07 | deploy 폼 용량 판정 | 후보 노드 여유보다 큰 TP 선택 | 배치 불가 판정 표시(노드 패킹 기반) — 리소스 requests 기반 여부는 §REG-09 | P1 |
| FORM-08 | 매니페스트 저장 | preview에서 `w` | 파일 생성, 내용=화면 preview, NS는 `LMD_NS` 반영 | P1 |

## F. 변경 액션 E2E (라이브, admin, 실기 검증 이력 있음)

> 2026-07 실기 검증 완료 이력: scale/stop/restart(GPU·RBLN·RNGD), cordon/uncordon,
> gemma4-rbln scale→0 시 ATOM 4장 리소스 해제 확인. 릴리스마다 아래 최소셋 재실행.

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| ACT-01 | scale 왕복 | 테스트 모델 1→2→1 | replicas 반영, 뷰에 45s 내 수렴, 실패 시 토스트에 kubectl 오류 | P0 |
| ACT-02 | stop/재개 | `x` 로 0, 다시 1 | 가속기 리소스 해제/재점유가 Nodes·Deploy targets에 반영 | P0 |
| ACT-03 | rollout restart | `S` | 새 pod 교체 관찰, Events 뷰에 기록 | P1 |
| ACT-04 | dry-run→apply | compile 폼 → preview → `v` → `a` | `v` 는 서버 검증만(무변경), `a` 후 Job 생성 확인 | P0 |
| ACT-05 | cordon 왕복 | 노드 cordon→uncordon | Nodes 뷰 상태 반영, 배치 타깃에서 제외/복귀 | P1 |
| ACT-06 | delete pod | (danger 의도) 테스트 pod 삭제 | 삭제 + ReplicaSet 재생성 관찰 | P1 |
| ACT-07 | 실패 경로 | 존재하지 않는 대상에 액션(경합 유도: 외부에서 먼저 삭제) | 명확한 오류 토스트, 상태 불일치 없음 | P1 |
| ACT-08 | 액션 중 UI | logs/apply 등 동기 kubectl 동안 | 프리즈 허용 상한 ~8s, 이후 반드시 복귀 | P1 |

## G. 데이터 정확성 (Prometheus/kubectl 대조) — 관측 도구의 존재 이유

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| DATA-01 | util 대조 | 화면 util vs `DCGM_FI_DEV_GPU_UTIL` 직접 쿼리 | 오차 ≤ 반올림. **저사용률(1~5%) 구간 필수 포함**(§REG-05) | P0 |
| DATA-02 | VRAM/디스크 | 화면 GB vs 메트릭 원값 | 단위 일관(GB vs GiB 혼용 여부 확인 — 알려진 ~7% 편차) | P1 |
| DATA-03 | 형제 변형 분리 | `<모델>`과 `<모델>-fp8` 동시 서빙 | 각자의 tps/kv/ttft가 올바른 행에(§REG-06) | P0 |
| DATA-04 | 모델 수 | Overview Σ vs `--json` vs kubectl 수동 집계 | 삼자 일치 | P1 |
| DATA-05 | 알림 발화 | 디스크 90%+ 노드 존재 시(또는 임계 임시 하향 빌드) | 알림 1회 발화·해소 시 제거, 중복 발화 없음 | P1 |
| DATA-06 | SLO 판정 | objective 설정 후 부하 주입(간단 벤치) | 충족/위반 판정과 실측 p95 일치, 병목 제안이 관측과 부합 | P1 |
| DATA-07 | 타임라인 | 1시간 관찰 후 sparkline | 값 변화 시점 일치(3배 오버샘플로 인한 평탄화 확인 — 알려진 이슈) | P2 |
| DATA-08 | EPP 경로 | `epp_in_path` 판정 vs 실제 라우팅(요청 주입) | 우회/경유 판정 정확 | P1 |

## H. 장애 주입·복원력

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| FAULT-01 | Prometheus 순단 | 서빙 중 30s 차단 후 복구 | 기존 테이블 유지(빈 결과로 안 덮음), 복구 후 자동 재개 | P0 |
| FAULT-02 | kubectl 행 | kubeconfig에 응답 없는 서버 추가 | full tick 정체가 **유한**해야 함(§REG-07: 현재 무한 대기 가능) | P0 |
| FAULT-03 | 대량 이벤트 | pod 100개 스케일 등으로 이벤트 폭주 | Events 뷰 반응 유지, 수집 tick 폭주 없음 | P2 |
| FAULT-04 | 비ASCII 라벨 | 한국어 포함 라벨의 메트릭 노출 | 값 정상 파싱(§REG-03: chunked 디코딩), 표 폭 정렬 유지 | P1 |
| FAULT-05 | 장기 구동 | 24h 방치 | RSS 증가 상한 확인(히스토리 버퍼 바운드), CPU 정상 | P1 |

## I. 테마·렌더링 — 실기 터미널 필수 (--render 는 색 검증 불가)

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| UI-01 | 4테마 가독성 | `t` 순환하며 전 뷰 + **오버레이(메뉴/폼/확인)** 육안 확인 | 선택 항목 텍스트 판독 가능(§REG-10: fg=bg 의심), 심각도 색 구분 | P0 |
| UI-02 | 256색 터미널 | xterm-256color에서 soft 테마 | 근사색 표시, 판독 가능 | P1 |
| UI-03 | 색맹 테마 | colorblind 테마에서 상태 구분 | 색 외 글리프(●/⚠/✗)로도 상태 구분 가능 | P1 |
| UI-04 | CJK 폭 | 한국어 모델명/필터 입력 | 열 정렬 유지(truncw+패딩 조합 — §REG-11) | P1 |
| UI-05 | tmux | tmux 안에서 전 뷰 | 글리프·마우스·색 정상 | P2 |
| UI-06 | 스크롤 상한 | YAML/logs/상세에서 끝까지 `j` 연타 | 콘텐츠 끝에서 멈춤(§REG-12: 현재 무한 스크롤) | P2 |

## J. Agent JSON 계약 (자동화 가능)

| ID | 시나리오 | 절차 | 기대 결과 | P |
|---|---|---|---|---|
| JSON-01 | 스키마 안정성 | `--json` 을 스키마 파일과 대조(jq 기반 필수 키 체크) | v2 필수 키 전부 존재, 타입 일치 | P0 |
| JSON-02 | NaN/결측 | 트래픽 0 상태에서 | NaN 없이 `null`, 숫자 필드에 문자열 없음 | P0 |
| JSON-03 | 액션 목록 | 모드별 `--json` 의 actions | 모드 게이트와 일치(observe엔 변경 액션 없음) | P1 |
| JSON-04 | 화면 일치 | `--json` vs `--snapshot` vs TUI | 모델 수·알림·노드 상태 동일 시점 기준 일치 | P1 |

## K. 성능

| ID | 시나리오 | 기준 | P |
|---|---|---|---|
| PERF-01 | full tick 소요 | 정상 클러스터에서 < 3s(tick 주기 내) | P1 |
| PERF-02 | 유휴 CPU | 애니메이션 off 시 코어의 < 5% | P2 |
| PERF-03 | 기동 시간 | 첫 화면까지 < 5s | P2 |

---

## REG. 알려진 결함 회귀 시트 (2026-07-02 코드 리뷰 출처)

> 수정 전: ❌가 정상(재현 확인용). 수정 후: 해당 시나리오를 위 본문 섹션의 회귀 기준으로 사용.

| ID | 결함 | 재현 절차 | 올바른 기대 동작 | 심각도 |
|---|---|---|---|---|
| REG-01 | 마우스 스크롤이 오버레이 게이트 우회 (main.rs:413) | 모델 A에 액션 메뉴 → 마우스 스크롤로 B 선택 → Scale | 실행 대상이 **A로 고정**되거나 오버레이 중 마우스 무시 | P0 |
| REG-02 | `Mode::Danger` 게이트 부재 — delete가 admin에서 허용 (main.rs:242) | `--mode admin` 으로 pod delete 시도 | 거부(danger 전용) 또는 문서/help 수정으로 정합 | P0 |
| REG-03 | chunked 디코딩이 lossy 문자열 위에서 동작 (prom.rs:93-138) | 비ASCII 라벨 메트릭 노출 후 해당 쿼리 | JSON 정상 파싱(바이트 기준 디코딩) | P0 |
| REG-04 | 매니페스트 YAML 인터폴레이션 무이스케이프 (app.rs:1292-) | 폼 커스텀 입력에 `"` 포함 | 유효 YAML 생성 또는 입력 거부 | P1 |
| REG-05 | `norm_pct`: 1% util → 100% (collect.rs:1114) | GPU 정확히 1% 사용 상태 관찰 | 1%로 표시 | P0 |
| REG-06 | 형제 변형 메트릭 오귀속 (collect.rs:1476) | `x`·`x-fp8` 동시 서빙 | 각 행에 자기 메트릭(최장/정확 매치 우선) | P0 |
| REG-07 | kubectl 외곽 타임아웃 부재 (kube.rs:9) | 응답 없는 API 서버 | full tick 유한 시간 내 오류·경고로 복귀 | P0 |
| REG-08 | 컬렉터 절반이 무경고 실패 (inferencepool/gateway 등) | RBAC 제한 계정 | 빈 뷰 대신 warnings 표기 | P1 |
| REG-09 | deploy_fit이 k8s 리소스 requests 미반영 (실기 발견) | metric 유휴 + requests 만점 노드에 배치 시도 | 배치 불가로 판정(allocatable-requested 기준 병용) | P1 |
| REG-10 | 오버레이 선택 항목 fg=`C_HL()`(배경색) — 비가시 의심 (overlays.rs:163,204) | 실기 터미널에서 액션 메뉴/폼 선택행 확인 | 선택 항목 텍스트 뚜렷하게 판독 | P1 |
| REG-11 | truncw 후 char 기준 `{:<N}` 패딩 — CJK 열 밀림 (mod.rs 다수) | CJK 이름 항목이 있는 표 | 열 정렬 유지 | P2 |
| REG-12 | preview/logs/detail 스크롤 무한 (main.rs:602,646) | `j` 연타 | 콘텐츠 끝에서 클램프 | P2 |
| REG-13 | retry가 4xx/5xx·타임아웃도 재시도 (prom.rs:70) | Prometheus 400 응답 유도 | 상태 오류는 즉시 전파(재시도 없음) | P1 |
| REG-14 | 뮤텍스 poison 시 "빈 클러스터"로 위장 (main.rs:403) | (코드 검토로 갈음) | 오류 상태를 명시 표시 | P2 |

---

## 릴리스 최소셋 (스모크, ~30분)

CLI-02·05·06 → ENV-01·07 → NAV-01·08·09 → PERM-01(observe에서 s/x/a 차단) → FORM-02·06 → ACT-01 → DATA-01·03 → UI-01 → JSON-01·02
