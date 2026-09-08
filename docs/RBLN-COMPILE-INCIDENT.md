# RBLN 컴파일 전면 실패 — 조사 기록 (2026-09-08)

> 이 클러스터의 **모든** RBLN 컴파일이 실패한다. 가설 6개를 실험으로 제거했고, 남은 원인은
> `rebel-compiler` 가 이유를 숨기고 있어 벤더 지원이 필요하다. 이 문서는 그 지원 요청에
> 그대로 쓸 수 있도록 쓰였다.
>
> Furiosa 컴파일은 같은 기간에 **정상 동작**했다(`furiosa-ai/Qwen3-4B-FP8`, 76분, 성공).

## 증상

```
RuntimeError: Error occurred while compiling the model
  File "<frozen core.compilation._impl>", line 974, in compile
```

첫 번째 compile unit(`Compile(#0)`)에서 즉시 발생. 그 앞 단계는 모두 성공한다:

```
INFO [rebel-compiler] Export done. Elapsed time: 0:00:01
INFO [rebel-compiler] Exported model conversion done. Elapsed time: 0:00:07
INFO [rebel-compiler] RBLN SDK compiler version: 0.11.0
INFO [rebel-compiler] -- Target NPU: RBLN-CA22
INFO [rebel-compiler] +------------------------------------------------+
INFO [rebel-compiler] |Compile(#0), mod_name=da81ed, input_info_index=0|
INFO [rebel-compiler] +------------------------------------------------+
Traceback ...
```

즉 **그래프 변환은 성공하고 코드 생성에서 죽는다.**

## 환경

| 항목 | 값 |
|---|---|
| 노드 | `etri-001` (RBLN-CA22 ×4, 드라이버 3.0.0) |
| optimum-rbln | 0.11.0.post1 |
| rebel-compiler | 0.11.0 |
| transformers | 5.8.1 |
| torch | 2.11.0+cpu |
| 실행 형태 | `ubuntu:22.04` 컨테이너 + 노드 site-packages hostPath 마운트 |
| (대조) Furiosa | furiosa-llm 2026.3.0, torch 2.10.0+cpu, transformers 5.1.0 — 컨테이너 내부 |

의존성은 **optimum-rbln 0.11.0.post1 이 스스로 선언한 핀과 정확히 일치**한다
(`importlib.metadata.requires` 조회): `transformers==5.8.1`, `torch==2.11.0+cpu`,
`torchvision==0.26.0+cpu`, `diffusers==0.38.0`.

## 제거한 가설 6개

| # | 가설 | 검증 방법 | 결과 |
|---|---|---|---|
| 1 | 컴파일 옵션 조합 | tp∈{1,4} × max-len∈{2048,4096,8192} × attn∈{flash_attn,eager} 5조합 | **전부 동일 실패** |
| 2 | 모델 미지원 | Qwen3-4B, Qwen2.5-0.5B, Llama-3.1-8B-Instruct | **전부 동일 실패**. `RBLNQwen3ForCausalLM` 등 클래스는 모두 존재 |
| 3 | 의존성 버전 skew | 호스트 패키지 메타데이터 조회 | **불일치 없음** — 벤더 자신의 핀과 일치 |
| 4 | 디바이스 가시성 | `rebellions.ai/ATOM: 1` 을 컴파일 Job 에 부착해 재실행 | `Could not determine the current machine's NPU` 경고는 **사라짐**. 그래도 73초에 동일 실패 |
| 5 | Python 예외에 상세가 있음 | `exc.args` + `__cause__`/`__context__` 체인 5단까지 출력 | **비어 있음** — `RuntimeError('Error occurred while compiling the model',)` 뿐 |
| 6 | 벤더 verbose 로깅 | `RBLN_VERBOSE=debug` (출처: `rebel/logging.py`, `rebel/flags.py`) | **추가 정보 없음** |

`RBLN_DEBUG_LEVEL` 은 시도했으나 거부된다:

```
[flag] environment variable RBLN_DEBUG_LEVEL is dev-only and cannot be used in a
       deploy build (raw: "1"). Unset it or use a development build.
```

## 대조군: Furiosa 는 정상이고, 오류 메시지도 쓸 만하다

같은 모델(`Qwen/Qwen2.5-0.5B-Instruct`)을 Furiosa 로 컴파일해봤다. 실패했지만 **정확한 이유를
말한다**:

```
error: no model registry entry for architecture=Qwen2ForCausalLM, hidden_size=896,
       intermediate_size=4864, num_hidden_layers=Some(24), quant_method=None
```

`quant_method=None` — furiosa-llm 은 furiosa-ai 사전양자화 체크포인트를 빌드하는 도구이고,
이 모델은 양자화되지 않았다. **의도된 동작이며 테스트 선택이 잘못된 것이다**(lmd-top 의
`furiosa-build` 분류와 힌트가 이 경우를 정확히 맞혔다). Furiosa 컴파일 경로는 건강하다 —
`furiosa-ai/Qwen3-4B-FP8` 성공이 그 증거다.

대비가 이 문제의 핵심을 보여준다: 같은 상황에서 한 벤더는 아키텍처·차원·양자화 여부까지
말해주고, 다른 벤더는 `Error occurred while compiling the model` 한 줄을 준다.

## 왜 RBLN 만 호스트 환경에 노출되는가 (구조적 차이)

- **Furiosa**: `furiosaai/furiosa-llm` **컨테이너 이미지 안에서** 컴파일 → 호스트 환경과 무관
- **RBLN**: `LMD_COMPILE_IMAGE_RBLN` 미설정 시 노드의 rebel-compiler 를 **hostPath 로 빌려 씀**
  → 그 노드 python 환경을 그대로 물려받음

이 비대칭이 "한쪽만 깨진" 이유를 설명한다. 다만 **가설 3 이 제거됐으므로, 이번 장애의 원인이
환경 드리프트라는 뜻은 아니다.** 컨테이너화는 *재발 방지*로는 옳지만 현재 장애의 해결책이 아니다.

## 다음 단계

1. **벤더 지원 문의** — 이 문서 그대로. 핵심 질문: SDK 0.11.0 / 드라이버 3.0.0 조합에서
   `core.compilation._impl:974` 가 삼키는 오류를 어떻게 볼 수 있는가. dev 빌드가 필요한가.
2. **드라이버↔컴파일러 버전 조합 확인** — 드라이버 3.0.0 과 컴파일러 0.11.0 이 벤더가
   의도한 짝인지. (미확인 영역. 추측하지 않았다.)
3. **`LMD_COMPILE_IMAGE_RBLN` 에 핀된 이미지 설정** — 원인 규명 **후**. 재발 방지책이며,
   Rebellions 레지스트리 접근권이 필요하다. `rebel-compiler` 는 공개 PyPI 에 없다
   (`pip download rebel-compiler==0.11.0` → `No matching distribution found`).

## 재현

```bash
# 이력·분류·추천을 포함해 그대로 재현된다
LMD_COMPILE_ENV="RBLN_VERBOSE=debug" \
  lmd-top --plan compile --model Qwen/Qwen2.5-0.5B-Instruct --vendor rbln \
          --set tp=1 --set max-len=2048 --set attn=eager --apply

lmd-top --history     # 기록된 시도·분류·패턴
```

## lmd-top 쪽에 반영한 것

- 실패 원인 분류(`src/diagnose.rs`) — `rbln-codegen` 힌트에서 **반증된 조언 제거**.
  "max-len 낮춰라 / eager 써라"는 실험으로 반증됐으므로, 그대로 두면 다음 사람이 같은 90분을
  쓴다. 이제 이 문서를 가리킨다.
- 다음 실험 제안(`src/advisor.rs`) — `rbln-codegen` 은 **아무것도 제안하지 않는다**.
  파라미터가 원인이 아님이 확인됐으므로 제안은 빌드 한 번을 낭비시킬 뿐이다.
- `LMD_COMPILE_ENV` — 벤더 플래그 전달. 이 조사에 필요했고, 생성 매니페스트를 손으로 고치는
  방식은 다음 장애에 남지 않는다.
- Job TTL 1h → 24h — 원래 이 조사가 어려웠던 이유. 실패가 1시간 뒤 증거 없이 사라졌다.
