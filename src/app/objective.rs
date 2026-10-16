//! Serving objectives (SLO targets) — lookup, edit form, submit, and the
//! observed-vs-target performance advisor. Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    /// 모델 목표(SLO) 조회 — 정확 매칭 우선, 없으면 느슨한 부분일치(서빙명 ≠ deploy명 대비).
    pub fn objective_for(&self, model: &str) -> Option<&Objective> {
        if let Some(o) = self.objectives.get(model) {
            return Some(o);
        }
        let mut candidates = self.objectives.iter().filter(|(k, _)| {
            !k.is_empty() && !model.is_empty() && (model.contains(k.as_str()) || k.contains(model))
        });
        let first = candidates.next()?;
        candidates.next().is_none().then_some(first.1)
    }

    /// Models 액션 메뉴 → Objective: 목표 편집 폼(기존 값 프리필).
    pub fn open_objective_form(&mut self) {
        // Overview 는 flat 모델 선택, Serving 은 배포 아티팩트 선택 — 둘 다 배포 이름을 준다.
        let Some(name) = self
            .selected_model()
            .map(|m| m.name.clone())
            .or_else(|| self.selected_artifact().map(|a| a.model.clone()))
        else {
            return;
        };
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
        let obj = match parse_objective(&form) {
            Ok(obj) => obj,
            Err(error) => {
                self.objective_form = Some(form);
                self.notify(error);
                return;
            }
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
        let fresh = self.snap.sources.get("performance").is_none_or(|status| {
            matches!(
                status.state(
                    crate::collect::now_secs(),
                    self.snap.stale_after_secs.max(10)
                ),
                crate::quality::Quality::Fresh
            )
        });
        crate::performance::evaluate(row, self.objective_for(&row.model), fresh)
    }
}

fn parse_objective(form: &ObjectiveForm) -> Result<Objective, String> {
    let parse = |key: &str| -> Result<Option<f64>, String> {
        let value = form.get(key);
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        value
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(Some)
            .ok_or_else(|| format!("{key}: use a positive finite target or none"))
    };
    Ok(Objective {
        ttft_ms: parse("ttft")?,
        tpot_ms: parse("tpot")?,
        e2e_ms: parse("e2e")?,
        min_tps: parse("tps")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ambiguous_objective_names_never_select_a_random_target() {
        let mut app = App::new();
        app.objectives.insert(
            "qwen".into(),
            Objective {
                ttft_ms: Some(100.0),
                ..Default::default()
            },
        );
        app.objectives.insert(
            "qwen-large".into(),
            Objective {
                ttft_ms: Some(500.0),
                ..Default::default()
            },
        );
        assert!(app.objective_for("serve-qwen-large").is_none());
        assert_eq!(
            app.objective_for("qwen-large").unwrap().ttft_ms,
            Some(500.0)
        );
    }
    #[test]
    fn invalid_target_preserves_form_and_existing_objective() {
        let mut app = App::new();
        app.snap.models.push(crate::collect::ModelRow {
            name: "m".into(),
            ready: 1,
            desired: 1,
            status: String::new(),
            route: String::new(),
            engine: String::new(),
            accel: String::new(),
            running: None,
            waiting: None,
            tps: None,
            kv: None,
            ttft: None,
        });
        app.objectives.insert(
            "m".into(),
            Objective {
                ttft_ms: Some(100.0),
                ..Default::default()
            },
        );
        for invalid in ["NaN", "inf", "-1", "0", "invalid"] {
            app.open_objective_form();
            app.objective_form
                .as_mut()
                .unwrap()
                .fields
                .iter_mut()
                .find(|f| f.key == "ttft")
                .unwrap()
                .value = invalid.into();
            app.objective_form_submit();
            assert!(app.objective_form.is_some());
            assert_eq!(app.objectives["m"].ttft_ms, Some(100.0));
        }
    }
}
