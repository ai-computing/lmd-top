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

| 단계 | 내용 | 완료 기준(측정 가능) | 위험 |
|---|---|---|---|
| **0** ✅ | 안전망 확보 — 131개 테스트, 18개 버그 전부 회귀 커버 | 완료 | — |
| **0.5** | **골든 매니페스트 스냅샷** — 현재 생성물(모델×벤더×compile/deploy)을 `tests/golden/` 에 고정 | 골든 파일 ≥ 10개, 의미 단정(이름·이미지·리소스키·replicas) | 낮음 |
| **1** | 매니페스트 → `serde_yaml` 직렬화. `yamlq` 제거 | `\x20` 0개, 골든 통과, 주입 28케이스 통과 | 중 (골든이 방어) |
| **2** | 레시피 → `assets/recipes/*`, Furiosa 를 env 전용으로. `shq` 제거 | Rust 안의 셸/파이썬 조각 0개 | 낮음 |
| **3** | `AcceleratorPack` 도입 — telemetry + 스케줄링. 컬렉터 3개 → 1개. `DEPS` 파생 | 벤더 리터럴이 `vendor/` 밖에 0개, doctor 목록 단일화 | 중 |
| **4** | `Caps` + `CompileSpec`. GPU 는 `compile: None` | 2지 벤더 분기 47 → 0, gpu-compile 이 **표현 불가** | 중 |
| **5** | `Modal` enum + `App` 분리 | 오버레이 `Option` 필드 12 → 1, 입력 라우팅 match 1개 | **높음** (UI 광범위) |
| **6** | (선택) 런타임 벤더 팩 YAML | 재컴파일 없이 새 가속기 1종 추가 시연 | 낮음 |

**권장 순서 근거**: 1·2 는 버그 공장 하나(텍스트 매니페스트)를 없애면서 범위가 좁고 골든으로
검증된다 → 가장 높은 수익/위험비. 3·4 가 "20개 파일 → 1개 파일"을 실현한다.
5 는 이득이 크지만 UI 전반을 건드리므로 1~4 로 테스트 문화가 다져진 뒤에.

**단계 0.5 를 건너뛰지 말 것.** 23k LOC 를 131개 테스트만으로 재구조화하는 건 위험하다.
매니페스트는 이 도구의 출력물이고, 골든 스냅샷은 단계 1~4 전체의 안전망이 된다.

---

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
