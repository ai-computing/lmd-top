//! Deploy/Compile/Objective/Action 기능의 값 타입 — App 상태(app.rs)에서 분리.
//! 메서드(compile_*/deploy_*/objective_*/fit 등)는 app.rs 의 impl App 에 있고 여기 타입만 소유.

/// 컴파일 옵션 편집 폼의 필드 하나. NPU 컴파일 파라미터(TP/PP/seq/batch/dtype/quant/npu).
#[derive(Clone)]
pub struct CompileField {
    pub key: String,          // 매니페스트/스크립트 옵션 키 (tp/pp/max-len/batch/dtype/quant/npu)
    pub label: String,        // 표시 라벨
    pub value: String,        // 현재 값
    pub choices: Vec<String>, // ←→ 로 순환할 프리셋
    pub numeric: bool,        // true 면 숫자 직접 입력(digit/backspace) 허용
    pub help: String,         // 하단 도움말
}

/// NPU 컴파일 옵션 편집 폼(오버레이). `c` → 편집 → Enter → 매니페스트 미리보기.
#[derive(Clone)]
pub struct CompileForm {
    pub model: String,        // 표시용 모델명
    pub model_id: String,     // HF id (org/name)
    pub vendor: &'static str, // "rbln" | "furiosa"
    pub engine: String,       // 원본 엔진 라벨
    pub fields: Vec<CompileField>,
    pub cursor: usize,
    pub editing: bool,        // 활성 필드 자유 입력(커스텀 값) 모드 — `e` 토글
}

// ── 필드 편집 폼 공용 로직(CompileForm·DeployForm 공유) ──
fn ff_get(fields: &[CompileField], key: &str) -> String {
    fields.iter().find(|f| f.key == key).map(|f| f.value.clone()).unwrap_or_default()
}
fn ff_move(cursor: &mut usize, len: usize, dir: i32) {
    if len == 0 {
        return;
    }
    let n = len as i32;
    *cursor = (((*cursor as i32 + dir) % n + n) % n) as usize;
}
fn ff_cycle(fields: &mut [CompileField], cursor: usize, dir: i32) {
    let f = &mut fields[cursor];
    if f.choices.is_empty() {
        return;
    }
    let cur = f.choices.iter().position(|c| *c == f.value);
    let len = f.choices.len() as i32;
    let next = match cur {
        Some(i) => (((i as i32 + dir) % len + len) % len) as usize,
        None => 0,
    };
    f.value = f.choices[next].clone();
}
fn ff_type_digit(fields: &mut [CompileField], cursor: usize, ch: char) {
    let f = &mut fields[cursor];
    if f.numeric && ch.is_ascii_digit() && f.value.len() < 6 {
        // "0" 또는 비숫자(예: "none") 값 위에 타이핑하면 새로 시작.
        if f.value == "0" || f.value.parse::<f64>().is_err() {
            f.value.clear();
        }
        f.value.push(ch);
    }
}
fn ff_type_char(fields: &mut [CompileField], cursor: usize, ch: char) {
    if ch.is_control() {
        return;
    }
    let f = &mut fields[cursor];
    if f.numeric && !(ch.is_ascii_digit() || ch == '.') {
        return; // 숫자 필드엔 숫자/소수점만
    }
    // 숫자 필드에 비숫자 값(예: "none")이 있으면 첫 입력 시 비움.
    if f.numeric && f.value.parse::<f64>().is_err() {
        f.value.clear();
    }
    if f.value.len() >= 24 {
        return;
    }
    f.value.push(ch);
}
fn ff_backspace(fields: &mut [CompileField], cursor: usize, editing: bool) {
    let f = &mut fields[cursor];
    if editing || f.numeric {
        f.value.pop();
    }
}

impl CompileForm {
    pub fn get(&self, key: &str) -> String {
        ff_get(&self.fields, key)
    }
    pub fn move_cursor(&mut self, dir: i32) {
        ff_move(&mut self.cursor, self.fields.len(), dir);
    }
    pub fn cycle(&mut self, dir: i32) {
        ff_cycle(&mut self.fields, self.cursor, dir);
    }
    pub fn type_digit(&mut self, ch: char) {
        ff_type_digit(&mut self.fields, self.cursor, ch);
    }
    pub fn type_char(&mut self, ch: char) {
        ff_type_char(&mut self.fields, self.cursor, ch);
    }
    pub fn backspace(&mut self) {
        ff_backspace(&mut self.fields, self.cursor, self.editing);
    }
    /// 컴파일 타깃 문자열 — npu 칩·TP·seq 로부터 산출(디스커버리 레이아웃과 일치).
    pub fn target(&self) -> String {
        let tp = self.get("tp");
        let seq = self.get("max-len");
        if self.vendor == "rbln" {
            let chip = self.get("npu").to_lowercase().replace("rbln-", "");
            format!("rbln-{}-tp{}-s{}", if chip.is_empty() { "ca22".into() } else { chip }, tp, seq)
        } else {
            let pp = self.get("pp");
            format!("rngd-tp{}-pp{}-s{}", tp, if pp.is_empty() { "1".into() } else { pp }, seq)
        }
    }
}

/// 배포(서빙) 옵션 편집 폼 — `d` → 편집 → Enter → Deployment 매니페스트 미리보기.
/// replicas·replica당 디바이스·노드 배치를 고른다(컴파일 폼과 대칭).
#[derive(Clone)]
pub struct DeployForm {
    pub model: String,
    pub model_id: String,
    pub engine: String,
    pub vendor: &'static str, // "rbln" | "furiosa" | "gpu"
    pub mount: String,        // 서빙할 스토어 아티팩트 경로
    pub fields: Vec<CompileField>,
    pub cursor: usize,
    pub editing: bool,
}
impl DeployForm {
    pub fn get(&self, key: &str) -> String {
        ff_get(&self.fields, key)
    }
    pub fn move_cursor(&mut self, dir: i32) {
        ff_move(&mut self.cursor, self.fields.len(), dir);
    }
    pub fn cycle(&mut self, dir: i32) {
        ff_cycle(&mut self.fields, self.cursor, dir);
    }
    pub fn type_digit(&mut self, ch: char) {
        ff_type_digit(&mut self.fields, self.cursor, ch);
    }
    pub fn type_char(&mut self, ch: char) {
        ff_type_char(&mut self.fields, self.cursor, ch);
    }
    pub fn backspace(&mut self) {
        ff_backspace(&mut self.fields, self.cursor, self.editing);
    }
}

/// Enter 시 뜨는 컨텍스트 액션(하드코딩된 단축키를 몰라도 되게 — 발견 가능한 메뉴).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Info,                    // 상세 보기(drill)
    Compile(&'static str),   // NPU 컴파일 옵션 폼(대상 벤더 rbln/furiosa)
    Deploy,                  // 배포 옵션 폼
    Stop,    // 서빙 중지(replicas 0)
    Logs,     // 로그 tail
    Scale,    // replicas 0/1 토글
    Restart,  // 롤아웃 재시작
    Cordon,   // 노드 스케줄 차단
    Uncordon, // 노드 스케줄 해제
    Yaml,      // live YAML 보기(읽기전용)
    Delete,    // 파드 삭제(재스케줄)
    Objective, // 서빙 목표(SLO) 설정
    RouteRename,   // 라우트 경로 변경(HTTPRoute path)
    RouteRetarget, // 라우트 백엔드 변경
    RouteDelete,   // 라우트 규칙 삭제
}

/// 라우트 편집 폼 — rename(경로 텍스트) 또는 retarget(백엔드 선택).
#[derive(Clone)]
pub struct RouteForm {
    pub route: String,   // 소속 HTTPRoute 이름
    pub path: String,    // 현재 경로(대상)
    pub rename: bool,    // true=rename(텍스트 편집) · false=retarget(선택)
    pub value: String,   // 새 경로 또는 선택된 "kind:name"
    pub choices: Vec<String>, // retarget 후보(kind:name)
    pub cursor: usize,   // retarget 선택 인덱스
}
#[derive(Clone)]
pub struct ActionItem {
    pub key: char,       // 단축키(가속기) — 메뉴 안에서도 직접 누르면 실행
    pub label: &'static str,
    pub desc: &'static str,
    pub action: Action,
}
/// Enter 액션 메뉴 오버레이 — 선택 항목에 대해 가능한 동작 목록.
#[derive(Clone)]
pub struct ActionMenu {
    pub title: String,
    pub subject: String, // 대상 이름(모델/빌드) — 액션 실행 시 참조
    pub items: Vec<ActionItem>,
    pub cursor: usize,
}
impl ActionMenu {
    pub fn move_cursor(&mut self, dir: i32) {
        let n = self.items.len() as i32;
        if n == 0 {
            return;
        }
        self.cursor = (((self.cursor as i32 + dir) % n + n) % n) as usize;
    }
    pub fn current(&self) -> Option<Action> {
        self.items.get(self.cursor).map(|i| i.action)
    }
    pub fn by_key(&self, c: char) -> Option<Action> {
        self.items.iter().find(|i| i.key == c).map(|i| i.action)
    }
}

/// 서빙 목표(SLO) — 모델별. None 인 항목은 목표 없음. 사용자가 입력.
#[derive(Clone, Default)]
pub struct Objective {
    pub ttft_ms: Option<f64>, // TTFT p95 상한
    pub tpot_ms: Option<f64>, // TPOT p95 상한
    pub e2e_ms: Option<f64>,  // E2E p95 상한
    pub min_tps: Option<f64>, // 최소 tok/s
}
impl Objective {
    pub fn is_empty(&self) -> bool {
        self.ttft_ms.is_none() && self.tpot_ms.is_none() && self.e2e_ms.is_none() && self.min_tps.is_none()
    }
}

/// 목표 편집 폼(그리드) — Models 액션 메뉴 → Objective.
#[derive(Clone)]
pub struct ObjectiveForm {
    pub model: String,
    pub fields: Vec<CompileField>,
    pub cursor: usize,
    pub editing: bool,
}
impl ObjectiveForm {
    pub fn get(&self, key: &str) -> String {
        ff_get(&self.fields, key)
    }
    pub fn move_cursor(&mut self, dir: i32) {
        ff_move(&mut self.cursor, self.fields.len(), dir);
    }
    pub fn cycle(&mut self, dir: i32) {
        ff_cycle(&mut self.fields, self.cursor, dir);
    }
    pub fn type_digit(&mut self, ch: char) {
        ff_type_digit(&mut self.fields, self.cursor, ch);
    }
    pub fn type_char(&mut self, ch: char) {
        ff_type_char(&mut self.fields, self.cursor, ch);
    }
    pub fn backspace(&mut self) {
        ff_backspace(&mut self.fields, self.cursor, self.editing);
    }
}

/// 목표 대비 관측 판정 + 데이터기반 조정 제안(값싼 런타임 노브 중심).
pub struct PerfAdvice {
    pub has_obj: bool,
    pub checks: Vec<(&'static str, bool)>, // (지표, 충족?)
    pub tips: Vec<String>,
}
impl PerfAdvice {
    pub fn all_met(&self) -> bool {
        self.has_obj && self.checks.iter().all(|(_, ok)| *ok)
    }
}

/// 배포 용량 판정 — replica×디바이스 총 수요 대비 클러스터 동종 가속기 총량.
pub struct DeployFit {
    pub demand: i64,        // replicas × replica당 디바이스
    pub total: i64,         // 클러스터 동종 디바이스 총 수
    pub free: i64,          // 유휴(metric busy_model 비어있음) 추정
    pub resource_free: i64, // k8s 리소스 유휴 = allocatable - requested(스케줄러 관점)
    pub nodes: i64,         // 동종 디바이스 보유 노드 수
    pub verdict: FitVerdict,
    pub tips: Vec<String>,
}

/// 컴파일 옵션 적합성 판정 — 선택 인프라(NPU 메모리) 대비 대략 추정.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FitVerdict {
    Fits,    // 여유
    Tight,   // 빠듯(≥85%)
    Oom,     // 초과 위험(>100%)
    Unknown, // 모델 크기 추정 불가
}
impl FitVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            FitVerdict::Fits => "fits",
            FitVerdict::Tight => "tight",
            FitVerdict::Oom => "OOM risk",
            FitVerdict::Unknown => "unknown size",
        }
    }
}

/// 컴파일 메모리 적합성 추정 결과(대략치 — 모델 config 없이 이름·표준 heuristic).
pub struct FitEstimate {
    pub params_b: Option<f64>, // 추정 파라미터 수(B)
    pub weight_gb: f64,        // 가중치 총 메모리
    pub kv_gb: f64,            // KV 캐시(batch·seq 기준)
    pub overhead_gb: f64,      // 런타임/활성 오버헤드
    pub chips: f64,            // 메모리 분산 칩 수
    pub per_chip_gb: f64,      // 칩당 요구 메모리
    pub avail_gb: f64,         // 칩당 가용 메모리
    pub verdict: FitVerdict,
    pub tips: Vec<String>, // 구체적 조정 제안
}
