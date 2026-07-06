//! Serving objectives (SLO targets) — lookup, edit form, submit, and the
//! observed-vs-target performance advisor. Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    /// 모델 목표(SLO) 조회 — 정확 매칭 우선, 없으면 느슨한 부분일치(서빙명 ≠ deploy명 대비).
    pub fn objective_for(&self, model: &str) -> Option<&Objective> {
        if let Some(o) = self.objectives.get(model) {
            return Some(o);
        }
        self.objectives
            .iter()
            .find(|(k, _)| model.contains(k.as_str()) || k.contains(model))
            .map(|(_, o)| o)
    }

    /// Models 액션 메뉴 → Objective: 목표 편집 폼(기존 값 프리필).
    pub fn open_objective_form(&mut self) {
        let Some(m) = self.selected_model() else {
            return;
        };
        let name = m.name.clone();
        let cur = self.objectives.get(&name).cloned().unwrap_or_default();
        let numf =
            |key: &str, label: &str, cur: Option<f64>, choices: &[&str], help: &str| CompileField {
                key: key.into(),
                label: label.into(),
                value: cur
                    .map(|v| format!("{}", v as i64))
                    .unwrap_or_else(|| "none".into()),
                choices: std::iter::once("none")
                    .chain(choices.iter().copied())
                    .map(|s| s.to_string())
                    .collect(),
                numeric: true,
                help: help.into(),
            };
        let fields = vec![
            numf(
                "ttft",
                "TTFT p95 ≤ms",
                cur.ttft_ms,
                &["500", "1000", "2000", "4000"],
                "첫 토큰까지 목표 상한(ms) — 대화형 응답성",
            ),
            numf(
                "tpot",
                "TPOT p95 ≤ms",
                cur.tpot_ms,
                &["20", "50", "100", "200"],
                "토큰당 생성 시간 상한(ms) — 스트리밍 속도",
            ),
            numf(
                "e2e",
                "E2E p95 ≤ms",
                cur.e2e_ms,
                &["1000", "2000", "5000", "10000"],
                "요청 완료까지 상한(ms)",
            ),
            numf(
                "tps",
                "min tok/s ≥",
                cur.min_tps,
                &["10", "50", "100", "500"],
                "최소 처리량(tok/s) — 낮으면 처리량 부족",
            ),
        ];
        self.objective_form = Some(ObjectiveForm {
            model: name,
            fields,
            cursor: 0,
            editing: false,
        });
    }

    /// 목표 폼 제출 → objectives 에 반영(모두 none 이면 삭제).
    pub fn objective_form_submit(&mut self) {
        let Some(form) = self.objective_form.take() else {
            return;
        };
        let p = |s: String| -> Option<f64> {
            if s.is_empty() || s == "none" {
                None
            } else {
                s.parse::<f64>().ok()
            }
        };
        let obj = Objective {
            ttft_ms: p(form.get("ttft")),
            tpot_ms: p(form.get("tpot")),
            e2e_ms: p(form.get("e2e")),
            min_tps: p(form.get("tps")),
        };
        if obj.is_empty() {
            self.objectives.remove(&form.model);
            self.notify(format!("objective cleared for {}", form.model));
        } else {
            self.objectives.insert(form.model.clone(), obj);
            self.notify(format!("objective set for {}", form.model));
        }
    }

    /// 관측(PerfRow) vs 목표 판정 + 병목 기반 조정 제안(값싼 런타임 노브 중심).
    pub fn perf_advice(&self, row: &crate::collect::PerfRow) -> PerfAdvice {
        let Some(o) = self.objective_for(&row.model) else {
            return PerfAdvice {
                has_obj: false,
                checks: Vec::new(),
                tips: Vec::new(),
            };
        };
        let ms = |s: f64| s * 1000.0;
        let mut checks: Vec<(&'static str, bool)> = Vec::new();
        if let Some(t) = o.ttft_ms {
            if !row.ttft_p95.is_nan() {
                checks.push(("TTFT", ms(row.ttft_p95) <= t));
            }
        }
        if let Some(t) = o.tpot_ms {
            if !row.tpot_p95.is_nan() {
                checks.push(("TPOT", ms(row.tpot_p95) <= t));
            }
        }
        if let Some(t) = o.e2e_ms {
            if !row.e2e_p95.is_nan() {
                checks.push(("E2E", ms(row.e2e_p95) <= t));
            }
        }
        if let Some(t) = o.min_tps {
            if !row.tps.is_nan() {
                checks.push(("tok/s", row.tps >= t));
            }
        }
        let mut tips: Vec<String> = Vec::new();
        let violated = checks.iter().any(|(_, ok)| !ok);
        if violated {
            let q = row.queue_p95;
            let pf = row.prefill_p95;
            let dc = row.decode_p95;
            if row.preempt > 0.0 {
                tips.push(
                    "KV/메모리 스래싱(preemption↑) — batch↓ 또는 max-seq-len↓, KV 여유 확보".into(),
                );
            }
            if !q.is_nan() && q > 0.0 && q >= pf.max(dc) {
                tips.push(format!(
                    "스케줄 대기 지배적({:.0}ms) — replicas↑ 또는 배치 버킷↑",
                    ms(q)
                ));
            } else if !pf.is_nan() && pf >= dc && pf > 0.0 {
                tips.push(format!(
                    "prefill 지배적({:.0}ms) — max-seq-len↓ 또는 chunked prefill",
                    ms(pf)
                ));
            } else if !dc.is_nan() && dc > 0.0 {
                tips.push(format!(
                    "decode 지배적({:.0}ms) — TPOT 개선: 동시성 조정, 필요시 TP↑",
                    ms(dc)
                ));
            }
            if let Some(mt) = o.min_tps {
                if !row.tps.is_nan() && row.tps < mt {
                    // 이 모델을 점유한 디바이스 평균 util 로 방향 제시.
                    let ut: Vec<f64> = self
                        .snap
                        .accel
                        .iter()
                        .filter(|a| {
                            !a.busy_model.is_empty() && row.model.contains(&a.busy_model)
                                || a.busy_model == row.model
                        })
                        .map(|a| a.util)
                        .collect();
                    let avg = if ut.is_empty() {
                        f64::NAN
                    } else {
                        ut.iter().sum::<f64>() / ut.len() as f64
                    };
                    if !avg.is_nan() && avg > 70.0 {
                        tips.push(
                            "tok/s 미달 · util 높음 — compute-bound: TP↑ 또는 replica↑".into(),
                        );
                    } else {
                        tips.push("tok/s 미달 · util 여유 — 동시성/배치↑ 로 처리량 확보".into());
                    }
                }
            }
        }
        PerfAdvice {
            has_obj: true,
            checks,
            tips,
        }
    }
}
