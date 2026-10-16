# EPP 기능과 lmd-top 정책 개발 환경

기준일: 2026-09-18. 실제 플러그인 개발 기준은 `contrib/epp-score-observer/go.mod`에
고정된 llm-d-router `e194457dc507`이다. 공식 웹 문서의 `dev` 기능과 이름이 다를 수 있다.
이 문서는 클러스터의 실행 이미지가 모든 기능을 지원하거나 활성화했다는 뜻이 아니다.

## EPP가 제공하는 기능 영역

| 영역 | 제공 기능 | 전제·경계 |
|---|---|---|
| 요청 해석 | OpenAI 계열, Anthropic, vLLM HTTP/gRPC, Vertex AI, passthrough parser | API/버전에 따라 지원 범위가 다름. passthrough는 본문 기반 정책 정보가 제한됨 |
| 요청별 상태 준비 | 토큰화, approximate/precise prefix 상태, session ID, in-flight 요청·토큰, 예상 latency, multimodal 상태 | tokenizer/모델 서버 이벤트/예측 sidecar 등 각 producer의 의존성 필요 |
| 입장 제어 | latency SLO에 따른 admission, 확률 기반 admission | 응답을 생성하는 엔진 자체가 아님. 목표·정책·feature gate 설정 필요 |
| 후보 필터 | label/role, session affinity, prefix affinity, SLO headroom | 선택 가능한 기존 endpoint 집합을 줄임 |
| 점수 계산 | queue, running/active requests, token load, KV 사용량, prefix hit, session, LoRA, context 길이, 예상 latency, multimodal cache | scorer 여러 개의 가중 합. util만으로 이종 장치 성능을 자동 비교하지 않음 |
| 최종 선택 | max score, random, weighted random | 가중치는 scorer 기여도이며 곧바로 GPU:NPU 트래픽 비율을 뜻하지 않음 |
| 프로파일 조합 | single profile, P/D 분리 프로파일, prefill/decode/encode 역할, disaggregation 결정 | 실제 KV 전송·엔진/connector 호환성은 별도 조건 |
| 흐름 제어 | 우선순위 band, fairness ID별 큐, global/round-robin/program-aware fairness, FCFS/EDF/SLO deadline ordering, 사용량 제한 | 고정 소스와 dev 버전의 feature gate·설정 차이를 확인. gateway와 연결된 큐 제어 |
| 과부하 판단 | utilization·concurrency saturation detector, 큐 용량/TTL, admission·대기·거절 | 처리 능력을 새로 만드는 기능은 아님 |
| 데이터 계층 | Prometheus polling, 엔진별 metric mapping/custom extractor, Kubernetes notification, endpoint lifecycle, 모델 정보, file discovery | 필요한 계측이 없으면 해당 정책의 근거도 없음 |
| 요청·응답 hook | PreRequest, response header/body 처리, 사용량/응답 속성 기록 | streaming/usage 응답 설정이 필요할 수 있음 |
| 관측·운영 | pool 상태, pick 분포, plugin 수행시간, 요청 오류/지연/토큰, 로그·trace, 상태 dump 인터페이스 | 전체 trace/메트릭 가용성은 이미지·설정에 따라 달라짐 |

공식 구조 설명: [EPP](https://llm-d.ai/docs/dev/architecture/core/router/epp),
[Request Handler](https://llm-d.ai/docs/dev/architecture/core/router/epp/request-handling),
[Scheduler](https://llm-d.ai/docs/dev/architecture/core/router/epp/scheduling),
[Flow Control](https://llm-d.ai/docs/dev/architecture/core/router/epp/flow-control),
[Data Layer](https://llm-d.ai/docs/dev/architecture/core/router/epp/datalayer).

`EndpointPickerConfig`는 Kubernetes CRD가 아니라 EPP가 시작할 때 읽는 설정이다.
변경 후에는 EPP 재시작이 필요하다. [공식 설정 문서](https://llm-d.ai/docs/dev/architecture/core/router/epp/configuration)

## 정확한 플러그인 목록

[고정 버전 등록 타입 72개](EPP-PLUGINS.md)는 runner의 실제 등록 코드를 추출한 목록이다.
호환 별칭·deprecated 타입·conformance 테스트용 타입도 포함하므로 “독립 기능 72개”가 아니다.
등록된 것과 현재 프로파일에 활성화된 것을 구분해야 한다. 이 목록 외에 새로 구현한 타입은:

- `heterogeneous-capacity`: 클래스/모델/선택적 입력 길이·데이터 freshness 필터 + 용량 대비 부하 scorer.
- `heterogeneous-load-extractor`: 완전한 waiting/running gauge 표본만 독립 timestamp와 함께 저장.
- 기존 `endpoint-score-observer`: 최종 composite score 관측, max-score picker에 선택 위임.

## lmd-top에서 제공하는 개발 흐름 (이번 구현)

현재 소스 빌드에 포함되며 아직 릴리스하지 않았다.

```bash
lmd-top --policy init --workspace ./my-policy
lmd-top --policy test --workspace ./my-policy
lmd-top --policy replay --workspace ./my-policy
lmd-top --policy build --workspace ./my-policy
```

Go 버전·upstream 의존성을 고정한 프로젝트, 순수 정책 함수, 실제 EPP adapter, 회귀 테스트,
JSON 시나리오, Dockerfile, Makefile과 개발 가이드가 생성된다. 기존 폴더는 덮어쓰지 않는다.
첫 Go 실행은 의존성을 다운로드할 수 있다. `test/replay/build`는 명시한 작업공간의 개발 코드를 실행한다.

```bash
lmd-top --policy validate --file ./my-policy/policy.yaml
lmd-top --policy render --file ./my-policy/policy.yaml \
  --base ./my-policy/examples/base-epp.yaml > epp.yaml
lmd-top --policy plan --file ./my-policy/policy.yaml > rollout.yaml
```

- `validate`: 로컬 policy 스키마·값 검사. 실행 이미지의 플러그인 존재 증명은 아님.
- `render`: 기존 EPP 설정에 정책을 병합. 별도 프로파일과 벤더 metric 설정 보존.
- `replay`: 실제 플러그인과 같은 Evaluate 함수로 후보별 score/제외 이유/동점 후보 출력.
  전체 EPP 프로파일 재생·트래픽 부하 시험·TTFT 예측은 아님.
- `plan`: 지정 Deployment/ConfigMap을 읽고 mount와 `--config-file` 연결을 확인한 뒤 새 immutable
  ConfigMap + EPP image/config 전환 Deployment를 생성. 기존 ConfigMap은 보존.

TUI에서는 `:` → **Routing policy** → 파일 경로 → Enter로 불러오기 →
이미지·프로파일·weight·metric age·클래스별 capacity 수정 → Enter로 preview → `v` 검증 →
`a` 적용(admin + 기존 confirmation/audit). 이 폼의 변경은 배포안에 반영되며 원본 policy.yaml은
자동 저장하지 않는다. 정책 파일은 Git으로 관리하고 수정값을 반영해 두는 것을 권장한다.
`EPP capabilities` 팔레트 항목은 이 설명을 연다.

기존 이미지와 ConfigMap을 같은 Deployment revision에 남기므로 `rollout undo`가 두 항목을 함께
되돌린다. 단, 오래된 리비전·이미지·ConfigMap을 삭제하면 복구할 수 없다. 여러 오브젝트의 적용은
트랜잭션이 아니며, Deployment 충돌 시 사용되지 않는 새 ConfigMap이 남을 수 있다.
다른 운영자가 바꾼 Deployment는 resourceVersion 충돌로 실패하도록 원본 버전을 보존한다.
Helm/GitOps가 관리하는 환경은 생성안을 해당 원본에도 반영해야 한다.

## 정책 의미와 남은 범위

`score = 1 / (1 + (waiting + running) / measured_capacity)`이며 capacity는 동일 워크로드에서
측정한 req/s이다. 예제의 8/4는 합성 값이다. 모델 revision·정밀도 호환성은 관리자가 pod selector에
반영한다. maxInputTokens는 parser가 정확한 tokenized prompt를 제공할 때만 검사 가능하고,
전체 context/출력 길이/컴파일 ABI를 자동 검증하지 않는다.

실측 용량, 이미지 registry 주소, endpoint labels, 벤더 metric 이름을 채워야 실제 적용할 수 있다.
이번 검증은 로컬 시나리오·고정 프레임워크 로딩·빌드까지다. 실클러스터 정책 적용과 canary 성능
검증은 아직 수행하지 않았다. NPU 우선·전력 최적화·SLO 예측 정책은 별도 확장이다.

다음 개발 편의 확장으로는 TUI 안의 코드 편집/테스트 결과 패널, 실제 요청 metadata를 익명화한
fixture 수집, 기존 정책과 후보 정책의 shadow 비교, staged rollout·자동 실패 감지/복구를 권장한다.
이들은 이번 구현에 포함되지 않았다. 새로운 Go 알고리즘은 재빌드가 필요하며 동적 스크립트
플러그인이나 hot reload를 제공한다고 표시하지 않는다.
