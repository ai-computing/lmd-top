//! Column sorting — per-view sortable columns, cycling, direction toggle, and
//! header/arrow labels. Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    /// 현재 뷰의 정렬 모드 수(순환용).
    /// 정렬 가능한 컬럼 — 뷰가 실제로 보여주는 컬럼 기준. `o` 로 순환, `O` 로 방향 토글.
    /// desc=true 면 그 컬럼 선택 시 기본이 내림차순(수치 컬럼은 큰 값 먼저가 유용).
    pub fn sort_cols(&self) -> &'static [SortCol] {
        use View::*;
        match self.view {
            Accel => &[
                SortCol {
                    label: "util",
                    desc: true,
                },
                SortCol {
                    label: "temp",
                    desc: true,
                },
                SortCol {
                    label: "mem",
                    desc: true,
                },
                SortCol {
                    label: "power",
                    desc: true,
                },
                SortCol {
                    label: "name",
                    desc: false,
                },
            ],
            Models | Overview => &[
                SortCol {
                    label: "name",
                    desc: false,
                },
                SortCol {
                    label: "status",
                    desc: false,
                },
                SortCol {
                    label: "ready",
                    desc: true,
                },
                SortCol {
                    label: "tok/s",
                    desc: true,
                },
                SortCol {
                    label: "kv%",
                    desc: true,
                },
                SortCol {
                    label: "waiting",
                    desc: true,
                },
                SortCol {
                    label: "node",
                    desc: false,
                },
            ],
            Pods => &[
                SortCol {
                    label: "name",
                    desc: false,
                },
                SortCol {
                    label: "phase",
                    desc: false,
                },
                SortCol {
                    label: "restarts",
                    desc: true,
                },
                SortCol {
                    label: "node",
                    desc: false,
                },
                SortCol {
                    label: "ready",
                    desc: false,
                },
            ],
            Nodes => &[
                SortCol {
                    label: "name",
                    desc: false,
                },
                SortCol {
                    label: "cpu",
                    desc: true,
                },
                SortCol {
                    label: "mem",
                    desc: true,
                },
                SortCol {
                    label: "disk",
                    desc: true,
                },
                SortCol {
                    label: "load",
                    desc: true,
                },
            ],
            Events => &[
                SortCol {
                    label: "recent",
                    desc: false,
                },
                SortCol {
                    label: "type",
                    desc: false,
                },
                SortCol {
                    label: "reason",
                    desc: false,
                },
                SortCol {
                    label: "count",
                    desc: true,
                },
            ],
            // Perf 는 기존 다지표 정렬(perf_rows_order) 유지 — 전부 desc 기본이라 진입 시 자연순서, O 로 역순.
            Perf => &[
                SortCol {
                    label: "tok/s",
                    desc: true,
                },
                SortCol {
                    label: "E2E",
                    desc: true,
                },
                SortCol {
                    label: "TTFT",
                    desc: true,
                },
                SortCol {
                    label: "queue",
                    desc: true,
                },
                SortCol {
                    label: "name",
                    desc: true,
                },
            ],
            _ => &[],
        }
    }
    pub fn sort_modes(&self) -> usize {
        self.sort_cols().len().max(1)
    }
    /// 뷰 진입/전환 시 정렬 초기화 — 첫 컬럼 + 그 컬럼의 기본 방향.
    pub fn reset_sort(&mut self) {
        self.sort = 0;
        self.sort_desc = self.sort_cols().first().map(|c| c.desc).unwrap_or(true);
    }
    pub fn cycle_sort(&mut self) {
        let cols = self.sort_cols();
        if cols.len() <= 1 {
            return;
        }
        self.sort = (self.sort + 1) % cols.len();
        self.sort_desc = self.sort_cols()[self.sort].desc; // 새 컬럼의 기본 방향
    }
    /// 정렬 방향 토글(`O`) — 정렬 가능한 뷰에서만.
    pub fn toggle_sort_dir(&mut self) {
        if !self.sort_cols().is_empty() {
            self.sort_desc = !self.sort_desc;
        }
    }
    pub fn sort_label(&self) -> &'static str {
        self.sort_cols()
            .get(self.sort)
            .map(|c| c.label)
            .unwrap_or("—")
    }
    /// 현재 정렬 컬럼에 대응하는 **헤더 텍스트**(테이블 헤더에 화살표를 붙일 대상 매칭용).
    /// 헤더 라벨과 sort 컬럼 라벨이 달라(예: util→"UTIL", name→"MODEL") 뷰별로 명시 매핑.
    /// 대응 헤더가 없으면(예: Events recent, Nodes) 빈 문자열 → 마킹 안 함.
    pub fn sort_header_label(&self) -> &'static str {
        use View::*;
        match (self.view, self.sort) {
            (Accel, 0) => "UTIL",
            (Accel, 1) => "TEMP",
            (Accel, 2) => "MEM",
            (Accel, 3) => "PWR",
            (Accel, _) => "KIND",
            (Models, 0) => "MODEL",
            (Models, 1) => "STATUS",
            (Models, 2) => "READY",
            (Models, 3) => "t/s",
            (Models, 4) => "KV",
            (Models, 5) => "WAIT",
            (Models, _) => "ACCEL",
            (Pods, 0) => "POD",
            (Pods, 1) => "PHASE",
            (Pods, 2) => "RESTARTS",
            (Pods, 3) => "NODE",
            (Pods, _) => "READY",
            (Events, 1) => "TYPE",
            (Events, 2) => "REASON",
            (Events, 3) => "CNT",
            (Perf, 0) => "tok/s",
            (Perf, 1) => "E2E",
            (Perf, 2) => "TTFT",
            (Perf, 3) => "QUEUE",
            (Perf, 4) => "MODEL",
            _ => "", // Events recent, Nodes(헤더 없음), 그 외 → 마킹 안 함
        }
    }

    /// 정렬 방향 표시 글리프(내림 ▼ / 오름 ▲). 정렬 불가 뷰는 공백.
    pub fn sort_arrow(&self) -> &'static str {
        if self.sort_cols().is_empty() {
            ""
        } else if self.sort_desc {
            "▼"
        } else {
            "▲"
        }
    }
}
