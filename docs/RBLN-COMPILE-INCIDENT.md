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

첫 번째 compile unit(`Compile(#0)`) 에서 발생하며, **그래프 생성·최적화 진행바는 100% 까지
완주한 뒤** 실패한다(아래 "좁힌 범위" 참조 — 실제 실패 지점은 codegen 이 아니라 결과 조립으로
보인다). 그 앞 단계는 모두 성공한다:

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

## 벤더 소스를 읽어 좁힌 범위

`rebel` 패키지는 frozen 모듈(`core`)과 **동일 구현의 평문 소스(`core_ori`)를 함께 배포**한다.
이를 읽어 두 가지가 좁혀졌다.

**1. 실패 지점은 codegen 이 아니라 결과 조립이다.** frozen 트레이스백의 `_impl.py:974` 는
`core_ori` 에서 `_build(...)`(959행)가 아니라 **`model_builder.add_module(...)`(974행)** 에
대응한다. 그리고 로그의 "Computation graph generation/optimization" 진행바가 **100% 까지
도달**한다. 즉 컴파일 자체는 끝나고 `PyRblnModelBuilder`(네이티브 `rebel._C`)에 결과를 넣는
단계에서 죽는다. (line 이 정확히 대응한다는 보장은 없으나, 진행바가 완주하는 사실이 이를 뒷받침한다.)

**2. `<frozen core.utility>:66 in wrapper` 는 범인이 아니다.** `core_ori/utility.py` 의 해당
데코레이터는 `warn_deprecated_kwargs` — 통과용이며 예외를 삼키지 않는다. 삼키는 코드는 frozen
`_impl` 내부이고, 따라서 `core_ori` 는 frozen 과 바이트 동일하지 않다.

**3. `librbln.so` 는 TVM 빌드다.** `core_ori` 를 억지로 import 해보다 얻은 부산물:

```
InternalError: Check failed: (p.second != plevel) is false:
Attribute target.rebel_descriptor of abs is already registered with same plevel=10
  /home/gspark/.local/lib/python3.10/site-packages/tvm/librbln.so
  tvm::OpRegEntry::UpdateAttr(...), tvm::runtime::detail::RebelException(...)
```

이 오류 자체는 **이중 import 가 만든 것**(frozen core 와 core_ori 가 같은 TVM 연산자 속성을
등록)이라 원래 버그와 무관하다. 하지만 두 가지를 알려준다 — `core_ori` 는 낡은 스냅샷이 아니라
살아있는 구현이고, 컴파일러가 **TVM 기반**이므로 TVM 자신의 `TVM_LOG_DEBUG` / `TVM_BACKTRACE`
가 적용된다. (그래서 `core_ori` 우회는 불가능하다 — 재시도하지 말 것.)

## 네이티브 라이브러리가 노출하는 진단 플래그

`librbln.so` 는 `site-packages/**tvm**/` 아래 있다 (`site-packages/rebel/` 가 아니다 —
초기 문자열 검색이 이것을 놓쳤다). 405MB 바이너리를 뒤져 **컴파일러 자신의** 환경변수를 찾았다:

```
RBLN_COMPILER_LOG_LEVEL   ← 컴파일러 로그 레벨 (파이썬 레벨 RBLN_VERBOSE 와 별개)
RBLN_DUMP_LOG  RBLN_DUMP_LOG_TVM  RBLN_DUMP_PATH  RBLN_DUMP_TVM_IRMODULE
RBLN_DUMP_RELOC_BIN  RBLN_DUMP_TRANSFORM  RBLN_DUMP_OPTRACE  RBLN_DEBUG_RELAY
RBLN_COMPILE_ONLY  RBLN_COMP_DTYPE  RBLN_COMP_DTYPE_MODE  RBLN_CHIPLET_SIZE
RBLN_DUMMY_DEVICE  RBLN_APPLY_TIMER  RBLN_BATCH_ATTN_OPT  ...
```

`RBLN_VERBOSE`(파이썬 로깅)를 올려도 효과가 없었던 이유가 이것으로 설명된다 — 컴파일러 진단은
별도 레벨을 쓴다. 그러나 **그 레벨도 deploy 빌드에서 막혀 있다**:

```
$ RBLN_COMPILER_LOG_LEVEL=debug
[flag] invalid value for RBLN_COMPILER_LOG_LEVEL: expected int, got "debug"

$ RBLN_COMPILER_LOG_LEVEL=4
[flag] environment variable RBLN_COMPILER_LOG_LEVEL is dev-only and cannot be used
       in a deploy build (raw: "4"). Unset it or use a development build.
```

## 이 사건의 교훈은 이제 도구에 들어가 있다

이 조사가 오래 걸린 이유는 정보가 없어서가 아니다. **동작하는 산출물 안에 답이 적혀 있었고,
lmd-top 이 그 파일을 읽지 않았다.** 같은 실수가 반복되지 않도록 출처를 인벤토리에 태웠다:

- `manifests/model-store-discovery.yaml` 스캔이 산출물에서 툴체인을 읽어 7번째 열
  `built_with` 로 발행한다. RBLN 은 `rbln_config.json` 의 `optimum_rbln_version`,
  Furiosa 는 `model.fxb`(zip, 매니페스트가 파일 끝에 비압축) 에서 꺼낸다.
- lmd-top 은 이것을 스토어 행·상세 패널·`--json` 에 표시한다.
- **컴파일 프리플라이트가 같은 계열의 기존 빌드 툴체인을 알려준다** — "이 계열의 기존 빌드는
  `optimum-rbln=0.10.2` 로 만들어졌음". 차단하지는 않는다(Job 이 어떤 버전을 쓸지는 실행
  전에 알 수 없다). 정보이지 판정이 아니다.

즉 다음 사람은 컴파일을 누르기 **전에** "여기서 되던 버전은 0.10.2" 를 화면에서 본다.

## 검증된 스크립트를 찾았다 — 그리고 파라미터는 원인이 아니다

노드 `etri-001` 에 **성공한 컴파일의 스크립트와 로그가 그대로 남아 있었다.** 처음에 봐야 했던 것이다.

```python
# ~/compile-koni-tp4.py  →  ~/rbln-KONI-Llama3.1-8B-Instruct-tp4-bs1-s8192 (동작 확인됨)
model = RBLNAutoModelForCausalLM.from_pretrained(
    model_id="KISTI-KONI/KONI-Llama3.1-8B-Instruct-20241024", export=True,
    rbln_batch_size=1, rbln_max_seq_len=8192, rbln_tensor_parallel_size=4,
    rbln_create_runtimes=False,      # NPU 실물 없이 컴파일만
    rbln_npu="RBLN-CA22")
```

```
# ~/compile-koni.log
2026-06-01 01:20:19 INFO [rebel-compiler] RBLN SDK compiler version: 0.10.2
2026-06-01 01:20:19 WARNING [rebel-compiler] Could not determine the current machine's NPU. Skipping NPU mismatch check.
2026-06-01 01:20:19 INFO [rebel-compiler] -- Tensor parallel size: 4
...  [+] DONE → /home/gspark/rbln-KONI-Llama3.1-8B-Instruct-tp4-bs1-s8192
```

**NPU 가 안 보이는 상태로도 컴파일이 끝났다.** 디바이스 가시성도, 이 클러스터도 원인이 아니다.

### 사다리 테스트 — 내 레시피에서 검증된 스크립트까지 한 번에 한 변수씩

스토어 0.10.3 툴체인, `Qwen/Qwen2.5-0.5B-Instruct`, 컨테이너:

| 단 | 바꾼 것 | 결과 |
|---|---|---|
| 1 | 내 레시피 그대로 (`attn_impl=eager`, tp1, s2048) | `ValueError: Device 0 is not a valid NPU device` |
| 2 | `attn_impl` 제거 | 동일한 디바이스 오류 |
| 3 | `+ rbln_create_runtimes=False` | `_impl:946` |
| 4 | `+ tp=4` | `_impl:946` |
| 5 | **검증된 스크립트와 동일한 형태** (tp4, s8192, bs1, attn_impl 없음) | `_impl:946` |

**파라미터는 원인이 아니다.** 이 하드웨어에서 증명된 그 설정이 0.10.3 에서 똑같이 실패한다.
(1·2단의 디바이스 오류는 내 사다리 스크립트가 `create_runtimes=False` 를 빼서 난 것으로,
lmd-top 레시피는 이미 `rbln_create_runtimes=False` 를 넣고 있다 — 제품 버그가 아니다.
다만 이 오류 메시지는 "NPU 없는 곳에서 컴파일할 때 무엇을 놓치면 무슨 말이 나오는지"의 좋은 예다.)

남은 변수는 **둘뿐이다: 툴체인 버전(0.10.2 vs 0.10.3), 그리고 모델.** 성공 사례는 전부
Llama-3.1 계열 8B 였고, 내 실패는 전부 Qwen 등 다른 계열이었다.

## 원인 확정: rebel-compiler 0.10.3 회귀 (0.10.2 에서는 됨)

**모델 축도 닫혔다.** 검증된 파라미터를 고정하고 모델만 바꿔서:

| 모델 | 결과 (0.10.3) |
|---|---|
| `Qwen/Qwen2.5-0.5B-Instruct` | `_impl:946` |
| `unsloth/Llama-3.2-1B-Instruct` | `_impl:946` |
| `KISTI-KONI/KONI-Llama3.1-8B-Instruct-20241024` — **0.10.2 로 성공한 그 모델** | `_impl:946` |

마지막 줄이 결정적이다. 같은 호스트, 같은 모델, 같은 파라미터, 그리고 **같은 그래프 해시**:

```
2026-06-01, SDK 0.10.2:  |Compile(#0), mod_name=1a4e47, input_info_index=0|  → DONE
2026-09-08, SDK 0.10.3:  |Compile(#0), mod_name=1a4e47, input_info_index=0|  → RuntimeError @ _impl:946
```

`mod_name` 이 일치한다는 것은 컴파일러가 받는 그래프가 **성공했던 것과 동일**하다는 뜻이다.
바뀐 것은 컴파일러 버전뿐이다. 두 진행 단계(graph generation / optimization)는 양쪽 다 100% 에
도달하고, 실패는 그 뒤 결과 조립 단계에서 난다.

**따라서 원인은 컴파일 옵션도, 모델도, 환경도, 의존성 버전도 아니다 — 0.10.3 회귀다.**

### 신뢰할 수 없는 실행으로 기록해두는 것

KONI 대조군은 세 번 돌렸고 앞의 두 번은 아무것도 증명하지 않는다. 증거로 오인되지 않도록 남긴다:

1. **64Gi 제한에서 OOMKilled** (exit 137) — 컴파일 실패가 아니라 리소스 부족. 8B/TP4 는
   컨테이너 limit 320Gi 가 필요했다.
2. **잘못된 스크립트 실행** (exit 2) — 부트스트랩 파일에 `sed` 를 잘못 걸어 이전 사다리
   스크립트 경로가 그대로 남았다. 컴파일이 시작조차 안 됐다.
3. 세 번째가 유효한 실행이다 (exit 0, 컴파일만 실패).

### 해결 경로

`docs/RBLN-SUPPORT-ENQUIRY.md` — 발송 가능 상태. 요청 순서는 **0.10.2 번들 우선**,
development 빌드는 차선이다. 우리가 원하는 것은 안 되는 구성을 더 잘 보는 것이 아니라
되던 구성을 되돌리는 것이다.

0.10.2 번들을 받으면 코드 변경 없이 끝난다:

```bash
LMD_RBLN_TOOLCHAIN=/mnt/store/rbln-toolchain/0.10.2 lmd-top   # 그게 전부다
```

0.10.2 는 이 클러스터 안에서는 구할 수 없음을 확인했다 — 벤더 인덱스 설정 없음, 노드 pip 캐시
없음, 스토어 `pipcache`(616MB, 132파일)에도 `rebel_compiler-0.10.2` 없음. 스토어에는 0.10.3
번들(50 wheel)만 있다.

## 결정적 사실 — 동작하는 산출물이 스스로 버전을 기록하고 있다

이 클러스터에서 **실제로 동작하는** RBLN 산출물(수동 컴파일, 노드 `/home/gspark/` 아래)의
`rbln_config.json` 은 자신을 만든 버전을 담고 있다:

```json
{ "cls_name": "RBLNLlamaForCausalLMConfig", "dtype": "float32",
  "optimum_rbln_version": "0.10.2", ... }
```

**0.10.2 다.** 내가 시험한 두 버전 중 어느 것도 아니다:

| 버전 | 출처 | 결과 |
|---|---|---|
| **0.10.2** | 동작하는 산출물이 기록 (2026-06-01 수동 빌드) | **성공한 유일한 버전** |
| 0.10.3 | 스토어 번들 `rbln-toolchain/0.10.3/` | 실패 (`_impl:946`) |
| 0.11.0.post1 | 노드 호스트 설치본 | 실패 (`_impl:974`) |

즉 **버전 자체가 원인은 아니지만**(0.10.3 과 0.11.0 이 똑같이 실패), **알려진 성공 버전은
0.10.2 하나뿐이다.** 다음에 시험할 것이 명확해졌다.

### 다음 행동 (가장 유력)

Rebellions 에서 **0.10.2 번들**을 받아 스토어에 스테이징하고 그것으로 컴파일한다:

```bash
# 0.10.3 번들과 같은 형태로 받아 두고
/mnt/store/rbln-toolchain/0.10.2/{rebel_compiler,optimum_rbln,torch,transformers,...}.whl

LMD_RBLN_TOOLCHAIN=/mnt/store/rbln-toolchain/0.10.2   lmd-top --plan compile --model <id> --vendor rbln --apply
```

0.10.3 번들이 이미 있으므로 벤더 인덱스 접근권은 확보돼 있다 — 0.10.2 를 같은 방식으로 받으면 된다.

## 컨테이너화는 이제 동작한다 (원인과 별개로)

`LMD_RBLN_TOOLCHAIN` 은 스토어의 wheel 번들로 `python:3.10-slim` 안에 툴체인을 설치한다.
**hostPath 없음, nodeSelector 없음, 레지스트리 접근권 없음** — 라이선스된 `rebel_compiler`
wheel(332MB)이 이미 스토어에 있기 때문이다(앞서 "공개 PyPI 에 없어 불가능"이라고 적었던 것을
정정한다). 검증됨: 50개 wheel 설치 → `optimum-rbln=0.10.3 rebel-compiler=0.10.3
transformers=4.57.6 torch=2.10.0+cpu` 로 실행 → 컴파일 단계까지 정상 진입.

이것으로 **노드 환경 드리프트는 변수에서 제거된다.** 버전을 바꿔 시험하는 일이 환경 오염 없이
반복 가능해졌다 — 0.10.2 시험이 바로 이 경로로 가능하다.

## 결론: deploy 빌드로는 더 알아낼 수 없다

진단 경로를 **전부** 시도했고, 모두 무효이거나 dev 빌드 전용으로 명시적으로 차단된다.

| 경로 | 결과 |
|---|---|
| `RBLN_VERBOSE=debug` (파이썬 로깅) | 효과 없음 — 컴파일러는 별도 레벨 사용 |
| `RBLN_COMPILER_LOG_LEVEL` (컴파일러 로깅) | **dev-only, 거부** |
| `RBLN_DEBUG_LEVEL` | **dev-only, 거부** |
| `TVM_LOG_DEBUG`, `TVM_BACKTRACE` | 효과 없음. `Check failed:` 줄도 없음 |
| Python 예외 `args` + cause chain 5단 | 비어 있음 |
| `rebel.core_ori` 로 우회 | TVM 연산자 이중 등록으로 import 불가 |
| 오류 문자열 위치 | `librbln.so` 내부(2회) — Rebellions 네이티브 코드가 직접 던짐 |

**따라서 오류의 *내용*을 보려면 Rebellions 의 development 빌드가 필요하다.** SDK 가 설계상
deploy 빌드에서 진단을 닫아두었다.

다만 **해결에 그것이 필요하다는 뜻은 아니다** — 위의 0.10.2 시험이 먼저다. 그것으로 되면
원인은 "0.10.3 이후 회귀"로 확정되고, 안 되면 그때 dev 빌드를 요청하면서 이 문서를 보내면 된다.

같은 바이너리에서 나온, 눈여겨볼 제약 문자열:

```
is not supported since its first dimension is not divisible by 2
                       and second dimension is not divisible by 64
is not supported since its second dimension is not divisible by 32
```

텐서 차원 정합 제약이다. 다만 서로 형상이 다른 모델 4종이 모두 실패했으므로 **단일 형상 제약이
원인일 가능성은 낮다** — 기록만 해둔다(추정을 결론으로 올리지 않는다).

## 다음 단계

1. **벤더 지원 문의 — 이 문서 그대로.** 질문은 두 개로 좁혀졌다:
   (a) SDK 0.11.0 / 드라이버 3.0.0 에서 `PyRblnModelBuilder` 결과 조립이 왜 실패하는가.
   (b) deploy 빌드에서 진단을 보려면 무엇이 필요한가 — development 빌드 배포를 받을 수 있는가.
2. **드라이버↔컴파일러 버전 조합 확인** — 드라이버 3.0.0 과 컴파일러 0.11.0 이 벤더가
   의도한 짝인지. (미확인 영역. 추측하지 않았다.)
3. ~~`LMD_COMPILE_IMAGE_RBLN` 에 핀된 이미지 설정 — 레지스트리 접근권 필요~~ →
   **`LMD_RBLN_TOOLCHAIN` 으로 이미 해결됨.** 라이선스된 wheel 이 스토어에 있으므로 레지스트리
   접근권 없이 컨테이너에서 툴체인을 고정할 수 있다. `rebel-compiler` 가 공개 PyPI 에 없다는
   사실은 여전히 맞지만(`pip download` → `No matching distribution found`), 이 클러스터에서는
   무관하다.

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
