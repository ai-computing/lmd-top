# lmd-top 리팩터링 방향 — 다중 가속기 · 플러그인 구조 (2026-09-08)

> 대상: v0.36.0 (23,344 LOC). 근거는 모두 현 코드 실측값이며, 2026-09-08 QA 에서 찾은
> 18개 버그(`docs/BUGS-2026-09-08.md`)의 **구조적 원인**과 연결해 우선순위를 정했다.

---

## 1. 현재 상태 — 측정값

| 지표 | 값 | 의미 |
|---|---|---|
| 새 가속기 1종 추가 시 **수정해야 하는 파일** | **20개** | 벤더 정체성이 코드 전역에 흩어져 있다 |
| 벤더 문자열 리터럴(`"rbln"`/`"furiosa"`/`"gpu"`) | 228곳 | 타입이 아니라 문자열로 분기 |
| 2지 분기(`if rbln {} else {}` / `match vendor`) | 47곳 | **3번째 벤더가 양쪽에 동시에 떨어진다** |
| k8s 리소스 키 하드코딩 | 30곳 | `rebellions.ai/ATOM` 등이 로직에 박혀 있음 |
| 매니페스트 텍스트가 차지하는 비율 | deploy.rs 17% · manifest.rs 16% | YAML 이 Rust `format!` 안에 문자열로 |
| YAML 들여쓰기 이스케이프 `\x20` | 181개 | 텍스트/코드 혼재의 직접 증거 |
| Rust 문자열에 박힌 셸/파이썬 조각 | 12곳 | 컴파일 레시피가 코드 안에 |
| `App` 구조체의 `pub` 필드 | **58개** | 단일 가변 god state |
| 최대 파일 | ui/mod.rs 4,717 · app.rs 2,885 · collect.rs 2,425 · main.rs 2,220 | 응집도 낮음 |

새 가속기를 추가하려면 손대야 하는 20개 파일:
```
app.rs  app/action.rs  app/compile/{fields,fit,manifest,mod,plan,preflight}.rs
app/deploy.rs  app/library.rs  app/zoo.rs  catalog.rs  collect.rs  compat.rs
doctor.rs  main.rs  metrics.rs  ops.rs  ui/mod.rs  ui/theme.rs
```

## 2. 버그는 우연이 아니었다 — 구조 → 결함 매핑

오늘 고친 18건 중 9건은 위 구조에서 **필연적으로** 나온다. 리팩터링 우선순위는 이 표가 정한다.

| 버그 | 구조적 원인 |
|---|---|
| BUG-06 매니페스트 YAML·셸 주입 | 매니페스트를 `format!` **텍스트**로 조립 → 이스케이프가 사람 책임 |
| BUG-07 gpu compile 이 RBLN 스크립트+Furiosa 이미지 혼합 | 2지 분기. `gpu` 가 분기마다 다른 쪽으로 떨어짐 |
| BUG-04 util 1% → 100% | 메트릭의 **단위**가 소스별로 선언돼 있지 않아 공용 휴리스틱이 추측 |
| BUG-05 형제 변형 메트릭 오귀속 | 조인 키가 선언이 아니라 이름 매칭 휴리스틱 |
| BUG-10 doctor 가 배선된 메트릭을 "미사용"으로 오보 | 메트릭 목록이 **두 곳**(collect / DEPS)에 중복 |
| BUG-17 `--plan deploy` 가 다른 모델 배포 | 인자를 `App.selected` **전역 상태로 전달** |
| BUG-02 Scale 이 엉뚱한 replica 수로 실행 | 액션이 자기 subject 대신 라이브 `App` 상태를 읽음 |
| BUG-18 커서가 배경 패널에서 움직임 | 오버레이가 12개 병렬 `Option<T>` + **손으로 정렬한** 입력 디스패치 |
| BUG-12 CJK 열 밀림 | 열 폭 프리미티브 없이 호출처마다 `format!("{:<N}")` |

핵심: **god state 로 인자를 넘기는 패턴**(BUG-02·17·18)과 **텍스트로 조립하는 매니페스트**
(BUG-06·07)가 두 개의 버그 공장이다.

---

## 3. 목표 구조

### 3.1 `AcceleratorPack` — 가속기 1종 = 레코드 1개

벤더 정체성을 한 곳에 모은다. **데이터로 표현 가능한 것은 전부 데이터로.**

```rust
pub struct AcceleratorPack {
    // 정체성
    pub id: &'static str,                     // "rbln"
    pub aliases: &'static [&'static str],     // ["atom", "rebellions"]   ← plan_vendor() 소멸
    pub label: &'static str,                  // "RBLN"
    pub kind: AccelKind,
    pub color: ThemeSlot,                     // ← ui/theme.rs 의 AccelKind match 소멸

    // 스케줄링
    pub resource_key: &'static str,           // "rebellions.ai/ATOM"     ← 30곳 하드코딩 소멸
    pub node_selector: Option<(&'static str, &'static str)>,

    pub telemetry: TelemetrySpec,
    pub caps: Caps,
    pub compile: Option<CompileSpec>,         // None = AOT 컴파일 개념 없음(GPU)
    pub serve: ServeSpec,
}
```

### 3.2 `Caps` — "각 NPU 마다 피처가 다른 점"을 타입으로

지금은 없는 기능을 **0.0 으로 채운다**. RBLN 컬렉터는 `throttle: 0.0` 을 쓰는데, 이건
"스로틀링 없음"과 "이 하드웨어는 스로틀을 보고하지 않음"을 구분할 수 없다.

```rust
pub struct Caps {
    pub compiles_ahead_of_time: bool,     // GPU=false → BUG-07 이 표현 불가능해진다
    pub health: Option<HealthPolarity>,   // furiosa: alive>0 / rbln: health==0
    pub energy: bool,
    pub throttle: bool,
    pub unified_memory: bool,
    pub tensor_parallel: Option<TpLimits>,// CA22 max 4, RNGD 8PE …
}
```
→ 없는 기능은 `None`. UI 는 `0` 과 `–` 를 구분해 그린다. 새 벤더는 **필드를 안 쓰면 끝**이고,
match arm 을 빠뜨려 컴파일이 깨지거나(좋은 경우) 조용히 0 을 보고하는(나쁜 경우) 일이 없다.

### 3.3 `TelemetrySpec` — 단위·조인키를 선언

BUG-04(단위 추측)와 BUG-05(조인 휴리스틱)의 일반해.

```rust
pub struct TelemetrySpec {
    pub id_labels: IdLabels,          // uuid+device+hostname vs gpu+Hostname
    pub series: &'static [SeriesSpec],
}
pub struct SeriesSpec {
    pub field: AccelField,            // Util|Temp|Power|MemUsed|MemTotal|Health|Throttle|Energy
    pub metric: &'static str,
    pub unit: Unit,                   // Percent|Ratio|Bytes|Mib|Celsius|Watt|Millijoule
    pub agg: Agg,                     // Max|Avg|Sum  (furiosa util 은 device 별 avg 필요)
}
```
- `unit` 이 선언되므로 스케일 추측이 사라진다 → **BUG-04 재발 불가**.
- `collect_furiosa` / `collect_rbln` / `collect_gpu` 세 함수(각 ~45줄, 거의 동일)가
  **스펙을 순회하는 하나의 제너릭 컬렉터**로 합쳐진다.
- `metrics::DEPS`(doctor 용 목록)를 **팩에서 파생**한다 → 두 목록이 어긋날 수 없다.
  오늘 BUG-10 은 테스트로 막았지만, 이 구조에선 목록이 애초에 하나다.

### 3.4 매니페스트: 포맷팅이 아니라 **직렬화**

```rust
// AS-IS — 181개 \x20, 이스케이프는 사람 책임(BUG-06)
format!("\x20         env:\n\x20           - {{ name: MODEL_ID, value: \"{}\" }}\n", model_id)

// TO-BE — 타입 → serde_yaml (이미 의존성에 있음, 새 크레이트 불필요)
ManifestDoc { comments, body: serde_yaml::to_value(&job)? }
```
얻는 것:
- **주입이 구조적으로 불가능**해진다. `serde_yaml` 이 인용을 처리하므로 `quote::yamlq` 가 필요 없다.
- 들여쓰기 이스케이프 181개 소멸.
- 테스트가 문자열 `contains` 가 아니라 데이터 단정이 된다:
  `assert_eq!(doc["spec"]["replicas"], 2)`.
- 운영자용 주석은 `comments` 프리앰블로 분리 보존(주석은 이 도구의 가치 중 하나라 버리지 않는다).

### 3.5 컴파일 레시피: 정적 에셋 + env 전용 인터페이스

```
assets/recipes/rbln-compile.py      # include_str!
assets/recipes/furiosa-compile.sh
```
RBLN 레시피는 **이미** 전부 `os.environ` 으로 읽는다. Furiosa 만 `fxb build {model_id}` 로
보간하는데, 이것도 `$MODEL_ID` 를 읽게 바꾸면 **보간이 0개**가 된다
→ `quote::shq` 도 필요 없어지고, 레시피는 문법 하이라이팅·린트·단독 실행이 가능한 실제 파일이 된다.
이것이 "텍스트와 코드 분리"의 가장 깨끗한 형태다.

### 3.6 플러그인 — 2단계

**Level 1 (지금 하자): 컴파일타임 레지스트리**
```rust
// vendor/mod.rs — 가속기 추가 = 파일 1개 + 이 줄 1개
pub static PACKS: &[&AcceleratorPack] = &[&rbln::PACK, &furiosa::PACK, &nvidia::PACK];
```
데이터로 표현 안 되는 부분만 트레잇 훅으로:
```rust
pub trait VendorHooks {
    fn param_issue(&self, f: &CompileForm) -> Option<String> { None }  // rbln kvpart 배수 검증
    fn memory_model(&self) -> &dyn MemoryModel;                        // bytes/param, KV 레이아웃
    fn preflight(&self, ctx: &PreflightCtx) -> Vec<Check> { vec![] }
}
```
→ 20개 파일 → **1개 파일 + 1줄**. 단일 정적 바이너리(install.sh·krew 배포)를 유지한다.

**Level 2 (나중): 런타임 벤더 팩**
`~/.config/lmd-top/vendors/*.yaml` 로 팩의 **데이터 절반**을 로드. 기존 벤더와 "모양이 같은"
NPU 는 재컴파일 없이 추가된다. 선례가 이미 있다 — `npu-compat.json`, `catalog/{models,zoo}.yaml`.

**하지 않을 것**: 동적 라이브러리(dlopen)·WASM 플러그인. 이 도구의 핵심 속성은
"C 컴파일러 없이 빌드되는 단일 정적 바이너리"이고, 그걸 깨면서 얻을 게 없다.

### 3.7 god state 해체

```rust
// AS-IS — 오버레이 12개가 병렬 Option<T>. "동시에 2개 열림"이 표현 가능하고,
//         입력 디스패치 순서를 사람이 손으로 PRECEDENCE 와 맞춰야 한다(BUG-18).
pub compile_form: Option<CompileForm>, pub deploy_form: Option<DeployForm>,
pub place_picker: Option<PlacePick>,   pub preview: Option<(String,String)>, ...

// TO-BE — 모달은 하나뿐임이 타입으로 보장되고, 입력 라우팅이 match 하나가 된다.
pub enum Modal {
    Help, Confirm(Pending), Palette(Palette), Preview(Preview), Logs(Logs),
    Form(FormKind), Picker(PickerKind), ActionMenu(ActionMenu),
}
pub modal: Option<Modal>,
```
- 입력·휠·렌더가 **같은 하나의 소스**를 보므로 오늘 같은 순서 불일치가 재발 불가.
- 액션은 자기 `subject` 를 들고 다니고(BUG-02), 플래닝 함수는 인자를 받는다(BUG-17).
  즉 두 버그의 수정이 *가드*가 아니라 *구조*가 된다.
- `App` 58 필드 → `Data`(스냅샷) / `Nav`(뷰·선택·필터) / `Modal` / `Ui`(테마·이펙트) 로 분리.

파일 분해: `ui/mod.rs` 4,717 → 뷰별 모듈, `collect.rs` 2,425 → `collect/{accel,nodes,kube,perf,epp}.rs`,
`main.rs` 2,220 → `main.rs`(부팅) + `input/`(키·마우스 라우팅).

---

## 4. 이행 계획 — 단계별로 독립 머지 가능

각 단계는 **끝에서 테스트가 초록**이고 단독으로 배포 가능해야 한다.

| 단계 | 내용 | 결과 |
|---|---|---|
| **0** | 안전망 — 18버그 회귀 커버 | ✅ 완료 (커밋 `3f0952f`) |
| **0.5** | 골든 매니페스트 스냅샷 | ✅ 9케이스 바이트 고정 + 의미 단정 (`c9a616e`) |
| **1** | 매니페스트 → `serde_yaml` 직렬화 | ✅ `\x20` **181 → 0**, `yamlq` 제거 (`b05a8c1`, `519ab90`) |
| **2** | 레시피 → `assets/recipes/*`, env 전용 | ✅ 에셋 5개, Rust 안 셸/파이썬 **0**, `shq` 제거 |
| **3** | `AcceleratorPack` — telemetry + 스케줄링 | ✅ 컬렉터 3→1, doctor 목록 단일화 (`0047526`) |
| **4** | `Caps`, 능력 기반 분기 | ✅ 2지 분기 **47 → 32**, gpu-compile 표현 불가 |
| **5** | 입력 라우팅 단일화 | ⚠️ **설계 수정** — 아래 참조 (`2408f28`) |
| **6** | 런타임 벤더 팩 YAML | ✅ 재컴파일 없이 추가 시연 (`1796ec6`) — 단, 범위 축소. 아래 참조 |

**권장 순서 근거**: 1·2 는 버그 공장 하나(텍스트 매니페스트)를 없애면서 범위가 좁고 골든으로
검증된다 → 가장 높은 수익/위험비. 3·4 가 "20개 파일 → 1개 파일"을 실현한다.
5 는 이득이 크지만 UI 전반을 건드리므로 1~4 로 테스트 문화가 다져진 뒤에.

**단계 0.5 를 건너뛰지 말 것.** 23k LOC 를 131개 테스트만으로 재구조화하는 건 위험하다.
매니페스트는 이 도구의 출력물이고, 골든 스냅샷은 단계 1~4 전체의 안전망이 된다.

---

## 4.1 실행 결과 — 계획이 틀렸던 두 곳

구현하면서 이 문서의 전제 두 개가 실제로는 성립하지 않는 걸 확인했다. 기록해 둔다.

### 단계 5: `Modal` enum 단일 슬롯은 **틀린 모양**이었다

"모달은 하나뿐"이라는 전제로 12개 `Option` 필드를 enum 하나로 접으려 했다. 그런데 이 앱의
**2단계 picker 는 부모 폼을 의도적으로 열어 둔다** — 목적지 picker 에서 Esc 를 누르면 고른
옵션을 버리지 않고 옵션 폼으로 돌아간다. 단일 슬롯은 이 흐름을 깨고, 올바른 모양은
"top 이 입력을 소유하는 **스택**"이다. 그건 `Overlay::top` 이 이미 계산하고 있다.

그래서 실제로 한 것: **입력 디스패치를 선언된 z-order 에서 유도**. 각 블록이 "내 게 열렸나"가
아니라 "내가 top 인가"를 단정하므로 순서가 어긋날 수 없다(BUG-18 의 원인). 테스트가
PRECEDENCE 의 모든 오버레이에 대응 arm 이 있는지, 필드 존재로 분기하는 코드가 없는지 고정한다.
필드를 접는 건 이제 **정확성 문제가 아니라 가독성 문제**로 남았다.

### 단계 6: 선언만으로 "가속기 1종 추가"는 절반만 사실이다

두 가지가 걸렸다.

1. **`AccelKind` 는 닫힌 enum 이고 `by_kind` 는 그것과 1:1 을 가정한다.** 런타임 팩이
   `kind: gpu` 를 재사용하면 모든 `by_kind` 조회가 모호해져 NVIDIA 의 색·라벨·능력을 조용히
   물려받는다. 계획에 이 고려가 없었다. → `AccelKind::Other(slot)` 추가로 선언된 가속기가
   **자기 디바이스 클래스**를 갖게 하고, 내장 클래스 이름을 쓰면 이유와 함께 거부한다.
2. **컴파일 레시피와 메모리 모델은 코드다.** 선언은 그걸 담을 수 없다. → 런타임 팩은
   `compiles_ahead_of_time` 을 절대 주장하지 않고, `--plan compile` 이 거절한다(다른 벤더
   스크립트를 물려주는 BUG-07 의 실패 모양을 피한다).

즉 런타임 팩이 할 수 있는 건 **서빙·관측 전용 가속기 추가**다. 완전한 신규 가속기는 여전히
내장 팩(파일 1개 + 등록 1줄)이 필요하다.

---

## 4.2 부수 수확 — BUG-19

단계 3 에서 조인 키를 **명시적으로 선언**하게 만든 순간 드러난 실데이터 버그.

DCGM 의 `gpu` 라벨은 **노드별 인덱스**라 노드가 둘이면 둘 다 `gpu="0"` 이다. 그걸 조인 키로
쓰면 두 디바이스가 하나로 합쳐지고, 한 노드의 값이 다른 노드 것으로 보고된다.
실측: dgx-spark0 이 93% 부하로 **67°C / 37W** 인데 lmd-top 은 **44°C / 6W**(gx10-4744 의 유휴값)를
보여줬다. 온도·전력 알림이 엉뚱한 디바이스 기준으로 평가되고 있었다.

→ nvidia 팩은 `UUID` 로 조인. 테스트가 **모든 팩의 조인 키가 표시 인덱스가 아닌 고유 식별자**임을
요구하고, 런타임 팩 검증도 같은 규칙을 적용한다.

## 5. 이 리팩터링이 막는 것 (요약)

| 없어지는 것 | 남는 결과 |
|---|---|
| 벤더 문자열 228개 · 2지 분기 47개 | 팩 1개 = 가속기 1종. 추가 시 파일 1개 + 1줄 |
| `format!` YAML 181개 이스케이프 | 직렬화 — 주입·들여쓰기 오류가 표현 불가 |
| Rust 안의 셸/파이썬 12조각 | 실행·린트 가능한 정적 에셋 |
| 메트릭 목록 2중화 | 팩에서 파생 — 어긋날 수 없음 |
| 단위 추측 휴리스틱 | `Unit` 선언 |
| 없는 기능을 `0.0` 으로 위장 | `Option` — `0` 과 `–` 구분 |
| 오버레이 12 × 병렬 `Option` | `Modal` 하나 — 입력/렌더 단일 소스 |
| 전역 상태로 인자 전달 | 명시적 인자 |

## 6. 최종 측정 (2026-09-08)

| 지표 | 전 | 후 |
|---|---|---|
| YAML 들여쓰기 이스케이프 `\x20` | 181 | **0** |
| Rust 문자열에 박힌 셸/파이썬 | 12 | **0** (에셋 5개) |
| 벤더 문자열 리터럴(Rust, `src/accel/` 제외) | 186 | 131 (잔여 대부분 테스트 픽스처·벤더 라벨 파싱) |
| 2지 벤더 분기 | 47 | 32 |
| 가속기 메트릭 목록 | 2곳(collect / DEPS) | **1곳**(팩에서 파생) |
| 새 가속기 추가 비용 | 20개 파일 | **파일 1개 + 등록 1줄** (서빙 전용은 재컴파일 0) |
| 테스트 | 108 | **146** |
| 빌드 경고 | 6 | **0** |
| 검증 | — | 골든 9케이스 + 실 API 서버 dry-run 60객체 + pty TUI 구동 |
