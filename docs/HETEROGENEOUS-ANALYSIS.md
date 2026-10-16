# 이종 GPU/NPU 분석 기능 제안

상태: 설계 제안. 아래 신규 분석은 아직 구현되지 않았다. 기존 Deploy의 배치 가능성,
아티팩트 provenance, Perf/SLO, EPP score, compile history 위에 확장한다.

## 우선순위

| 순서 | 기능 | 운영자가 답을 얻는 질문 | 필요한 데이터 / 완료 기준 |
|---|---|---|---|
| P1 | 컴파일본 호환성·준비 상태 | 이 모델을 이 NPU에 지금 올릴 수 있나? | 모델 revision·양자화·target chip·TP·context/batch/bucket·compiler/runtime/driver. ready / needs compile / incompatible / unknown과 근거를 표시. 알려진 불일치만 배포 차단, 결측을 호환으로 취급하지 않음 |
| P1 | 동일 조건 성능 비교 | 같은 요구사항에서 어느 GPU/NPU가 적합한가? | 모델 revision·tokenizer·정밀도·prompt/output 길이 구간·동시성·시간창·워밍업·cache 조건을 명시. TTFT p95, ITL p95, 완료 req/s, output tok/s, 오류율과 표본 수 비교. 조건이 다른 실측은 별도 그룹 |
| P2 | 배포 준비 시간 | 여유 NPU가 있는데 왜 트래픽을 못 받나? | download→compile→queue/schedule→load→ready→첫 성공 추론의 타임스탬프. 현재 phase와 병목, 동일 조건 과거 분포에 근거한 예상 시간/표본 수. 단순 Ready를 추론 성공으로 간주하지 않음 |
| P2 | SLO·전력 효율 | 지연 목표를 만족하면서 어디가 효율적인가? | 같은 시간창의 output tokens / 적분 energy(J), SLO 만족 완료 requests / sec. 장치/노드 측정 범위 구분. 공유 디바이스의 모델별 전력 귀속은 불명확하면 unknown |
| P3 | 이종 라우팅 적합성 | 긴 prompt/짧은 응답 등 요청 유형별로 어디에 보내야 하나? | 요청 길이별 latency/queue/cache 분포와 artifact의 지원 범위. 먼저 근거 있는 읽기 전용 추천, 이후 검증된 정책만 what-if |
| P3 | PD 분리 배치 분석 | GPU prefill + NPU decode 조합이 가능한가, 이득인가? | KV 형식/정밀도/레이아웃·connector·엔진 호환성 확인을 먼저 수행. 검증된 조합만 transfer latency/bytes·네트워크 병목·P/D 불균형 분석 |

## 수치가 정직해야 한다

- GPU util 50%와 NPU util 50%를 같은 성능 여유로 해석하지 않는다. util은 해당 장치의
  보조 신호이고, 실제 latency·처리량·queue로 병목을 판단한다.
- 결측, stale, 미지원, 측정된 0을 구분한다. 모든 비교에 출처·단위·시간창·측정 범위를 붙인다.
- goodput은 **동일 요청에서** TTFT/TPOT 등 목표를 모두 만족한 완료 요청 수/시간이다.
  개별 p95나 독립 histogram만으로 joint goodput을 계산할 수 없다. 요청별 계측이 없으면
  “goodput unknown”, 현재 관측된 SLO 상태만 표시한다.
- ITL(스트림 출력 이벤트 사이 간격)과 요청별 평균 TPOT를 구분한다. 엔진/버전별 메트릭을
  명시적으로 매핑하고 이름만 비슷한 지표를 섞지 않는다.
- tok/J = 시간창의 output token 증가량 / 같은 장치 집합의 ∫power(W)dt.
  메트릭 공백·리셋·장치 공유로 분모가 불확실하면 값을 감추고 사유를 표시한다.
- 컴파일러 버전이 다르다는 것만으로 incompatibility를 단정하지 않는다. 벤더 호환 규칙과
  실기 성공 기록이 필요하다. 장치 메모리 합계만으로 TP 배치 가능성을 판단하지 않는다.
- GPU↔NPU KV 전송은 가능한 것으로 가정하지 않는다. connector/형식 호환성을 증명해야 한다.

## 구현 순서와 기존 코드 연결

1. `store.rs`/`collect.rs` provenance를 확장한 artifact compatibility 레코드와 `agent.rs` JSON 계약.
   버전 정보가 없는 기존 빌드, target 불일치, context/batch 초과를 회귀 테스트한다.
2. `advisor.rs`/Perf에 비교 그룹과 data-quality 표시. 초기에는 관측 데이터만 사용한다.
   재현 가능한 비교를 위한 별도 벤치마크는 트래픽·자원을 사용하므로 명시적으로 실행한다.
3. `history.rs`/Activity에 준비 시간 분해. 캐시 hit 빌드와 cold compile 기록은 구분한다.
4. 전력/요청별 계측 커버리지가 확보된 환경부터 efficiency와 goodput을 활성화한다.
5. 마지막으로 EPP 추천과 PD 분석. 기능 지원 여부를 숨기지 않고 unknown 상태도 완성된 UX로 취급한다.

## 근거 문서 (2026-09-18 확인)

- [vLLM metrics design](https://docs.vllm.ai/en/latest/design/metrics/) — ITL과 요청별 TPOT 정의.
- [vLLM production metrics](https://docs.vllm.ai/en/latest/usage/metrics/) — TTFT/ITL, cache 계측 기반.
- [RBLN common configuration](https://docs.rbln.ai/v0.10.4/software/optimum/model_api/transformers/common/common.html)
  — target NPU, TP, batch, max sequence length를 포함하는 컴파일 설정.
- [Furiosa artifact builder](https://developer.furiosa.ai/v2026.1.0/en/_modules/furiosa_llm/artifact/builder.html)
  — bucket 설정 및 컴파일러/엔진 provenance. 설치된 SDK 버전의 스키마를 기준으로 adapter를 구현한다.
