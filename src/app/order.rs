//! Row ordering — per-view sort/group into original-index order, filter
//! application, selection→original mapping, and the aggregate summary line.
//! Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    pub fn order(&self) -> Vec<usize> {
        use crate::collect::{Accel, ModelRow, PodRow};
        use crate::measurement::compare;
        let mut idx = match self.view {
            // ── 컬럼 기반 정렬(o=컬럼 순환 · O=방향 토글). 비교는 오름차순으로 쓰고 desc 면 reverse. ──
            View::Accel => {
                let v = &self.snap.accel;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&Accel, &Accel) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        0 => compare(Some(x.util), Some(y.util), desc),
                        1 => compare(Some(x.temp), Some(y.temp), desc),
                        2 => compare(Some(x.mem_used_gb), Some(y.mem_used_gb), desc),
                        3 => compare(Some(x.power), Some(y.power), desc),
                        _ => (x.kind.rank(), x.node.as_str(), x.id.as_str()).cmp(&(
                            y.kind.rank(),
                            y.node.as_str(),
                            y.id.as_str(),
                        )),
                    };
                    let asc = if desc && self.sort > 3 {
                        asc.reverse()
                    } else {
                        asc
                    };
                    asc.then_with(|| {
                        (x.node.as_str(), x.id.as_str()).cmp(&(y.node.as_str(), y.id.as_str()))
                    })
                });
                idx
            }
            View::Overview => {
                let v = &self.snap.models;
                let desc = self.sort_desc;
                let oc = |a, b| compare(a, b, desc);
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&ModelRow, &ModelRow) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.status.cmp(&y.status),
                        2 => x.ready.cmp(&y.ready),
                        3 => oc(x.tps, y.tps),
                        4 => oc(x.kv, y.kv),
                        5 => oc(x.waiting, y.waiting),
                        6 => x.accel.cmp(&y.accel),
                        _ => x.name.cmp(&y.name),
                    };
                    let asc = if desc && !matches!(self.sort, 3..=5) {
                        asc.reverse()
                    } else {
                        asc
                    };
                    asc.then_with(|| x.name.cmp(&y.name)) // 동점은 이름 오름차순(안정)
                });
                idx
            }
            View::Pods => {
                let v = &self.snap.pods;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&PodRow, &PodRow) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.phase.cmp(&y.phase),
                        2 => x.restarts.cmp(&y.restarts),
                        3 => x.node.cmp(&y.node),
                        4 => x.ready.cmp(&y.ready),
                        _ => x.name.cmp(&y.name),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
                    asc.then_with(|| x.name.cmp(&y.name))
                });
                idx
            }
            View::Nodes => {
                let v = &self.snap.nodes;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => compare(Some(x.cpu_pct), Some(y.cpu_pct), desc),
                        2 => compare(Some(x.mem_used_gb), Some(y.mem_used_gb), desc),
                        3 => compare(Some(x.disk_used_gb), Some(y.disk_used_gb), desc),
                        4 => compare(Some(x.load1), Some(y.load1), desc),
                        _ => x.name.cmp(&y.name),
                    };
                    let asc = if desc && !matches!(self.sort, 1..=4) {
                        asc.reverse()
                    } else {
                        asc
                    };
                    asc.then_with(|| x.name.cmp(&y.name))
                });
                idx
            }
            View::Events => {
                let v = &self.snap.events;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.typ.cmp(&y.typ),
                        2 => x.reason.cmp(&y.reason),
                        3 => x.count.cmp(&y.count),
                        _ => a.cmp(&b), // recent: 수집 순서(최신 먼저) = 인덱스 오름차순
                    };
                    if desc {
                        asc.reverse()
                    } else {
                        asc
                    }
                });
                idx
            }
            // Serving: 배포 아티팩트를 컬럼 기준 정렬(o=컬럼 · O=방향). 균일 정렬표 내비.
            View::Serving => {
                let v = &self.snap.artifacts;
                let desc = self.sort_desc;
                let dr = |a: &crate::collect::ModelArtifact| {
                    self.snap
                        .models
                        .iter()
                        .find(|m| m.name == a.model)
                        .map(|m| (m.desired, m.ready, m.tps))
                        .unwrap_or((0, 0, None))
                };
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&x, &y| {
                    let (ax, ay) = (&v[x], &v[y]);
                    let (dx, rx, tx) = dr(ax);
                    let (dy, ry, ty) = dr(ay);
                    let asc = match self.sort {
                        0 => self
                            .deploy_phase(&ax.model, dx, rx)
                            .rank()
                            .cmp(&self.deploy_phase(&ay.model, dy, ry).rank()),
                        2 => rx.cmp(&ry),
                        3 => compare(tx, ty, desc),
                        4 => ax.node.cmp(&ay.node),
                        _ => ax.model.cmp(&ay.model),
                    };
                    let asc = if desc && self.sort != 3 {
                        asc.reverse()
                    } else {
                        asc
                    };
                    asc.then_with(|| ax.model.cmp(&ay.model))
                });
                idx
            }
            // Deploy: 위=Model List(패널0) · 아래=Activity 피드(패널1).
            View::Library if self.panel_focus == 1 => (0..self.activity_rows().len()).collect(),
            View::Library => (0..self.library_items().len()).collect(),
            View::Epp if self.panel_focus == 1 => (0..self.snap.pools.len()).collect(),
            View::Epp => (0..self.selected_epp().map(|e| e.scorers.len()).unwrap_or(0)).collect(),
            View::Perf if self.panel_focus == 1 => (0..self.snap.pod_queues.len()).collect(),
            // Perf 는 다지표 전용 정렬(perf_rows_order, 기본 best-first=내림). O 로 역순.
            View::Perf => self.perf_rows_order(),
            View::Routing if self.panel_focus == 1 => (0..self.snap.pools.len()).collect(),
            View::Routing => (0..self.snap.routes.len()).collect(),
            View::Topo => Vec::new(), // 맵 뷰 — 리스트 선택 없음
            View::Zoo if self.panel_focus == 1 => (0..self.activity_rows().len()).collect(),
            View::Zoo => {
                let v = &self.zoo;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        1 => x.role.cmp(&y.role),
                        2 => self.zoo_state(x).1.cmp(&self.zoo_state(y).1),
                        _ => x.display.to_lowercase().cmp(&y.display.to_lowercase()),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
                    asc.then_with(|| x.display.to_lowercase().cmp(&y.display.to_lowercase()))
                });
                idx
            }
            View::Setup => (0..self.setup_checks().len()).collect(),
        };
        if !self.filter.is_empty() {
            let fl = self.filter.to_lowercase();
            idx.retain(|&i| self.search_text(i).to_lowercase().contains(&fl));
        }
        idx
    }

    /// 표시 순서상 selected 위치 → 원본 인덱스.
    pub fn sel_orig(&self) -> Option<usize> {
        self.order().get(self.selected).copied()
    }

    /// 현재 뷰의 (필터된/전체) 행 집계 요약 — Overview 처럼 통합 값을 함께 보이려는 용도.
    /// 필터가 있으면 보이는 행만, 없으면 전체. 없으면 None.
    pub fn agg_summary(&self) -> Option<String> {
        use crate::collect::{Accel, ModelRow, NodeInfo, PerfRow};
        use crate::measurement::{complete_sum, Metric};
        let order = self.order();
        if order.is_empty() {
            return None;
        }
        let scope = if self.filter.is_empty() {
            "all"
        } else {
            "filt"
        };
        let n = order.len();
        match self.view {
            View::Accel => {
                let d: Vec<&Accel> = order
                    .iter()
                    .filter_map(|&i| self.snap.accel.get(i))
                    .collect();
                if d.is_empty() {
                    return None;
                }
                let util = d.iter().map(|x| x.util).sum::<f64>() / d.len() as f64;
                let mu: f64 = d.iter().map(|x| x.mem_used_gb).sum();
                let mt: f64 = d.iter().map(|x| x.mem_total_gb).sum();
                let pw: f64 = d.iter().map(|x| x.power).sum();
                let busy = d.iter().filter(|x| !x.busy_model.is_empty()).count();
                Some(format!(
                    "Σ{} {}dev · {}busy · util {:.0}% · VRAM {:.0}/{:.0}G · {:.0}W",
                    scope,
                    n,
                    busy,
                    Metric(util),
                    Metric(mu),
                    Metric(mt),
                    Metric(pw)
                ))
            }
            View::Overview => {
                let m: Vec<&ModelRow> = order
                    .iter()
                    .filter_map(|&i| self.snap.models.get(i))
                    .collect();
                if m.is_empty() {
                    return None;
                }
                let ready: i64 = m.iter().map(|x| x.ready).sum();
                let desired: i64 = m.iter().map(|x| x.desired).sum();
                let run = complete_sum(m.iter().map(|x| x.running));
                let wait = complete_sum(m.iter().map(|x| x.waiting));
                let tps = complete_sum(m.iter().map(|x| x.tps));
                Some(format!(
                    "Σ{} {}mdl · {}/{}ready · run {:.0} wait {:.0} · {:.0}tok/s",
                    scope,
                    n,
                    ready,
                    desired,
                    Metric(run),
                    Metric(wait),
                    Metric(tps)
                ))
            }
            View::Nodes => {
                let nn: Vec<&NodeInfo> = order
                    .iter()
                    .filter_map(|&i| self.snap.nodes.get(i))
                    .collect();
                if nn.is_empty() {
                    return None;
                }
                let cpu = nn.iter().map(|x| x.cpu_pct).sum::<f64>() / nn.len() as f64;
                let mu: f64 = nn.iter().map(|x| x.mem_used_gb).sum();
                let mt: f64 = nn.iter().map(|x| x.mem_total_gb).sum();
                let du: f64 = nn.iter().map(|x| x.disk_used_gb).sum();
                let dt: f64 = nn.iter().map(|x| x.disk_total_gb).sum();
                let ready = nn.iter().filter(|x| x.ready).count();
                Some(format!(
                    "Σ{} {}node · {}ready · CPU {:.0}% · mem {:.0}/{:.0}G · disk {:.1}/{:.1}T",
                    scope,
                    n,
                    ready,
                    Metric(cpu),
                    Metric(mu),
                    Metric(mt),
                    Metric(du / 1024.0),
                    Metric(dt / 1024.0)
                ))
            }
            View::Perf => {
                let p: Vec<&PerfRow> = order
                    .iter()
                    .filter_map(|&i| self.snap.perf_rows.get(i))
                    .collect();
                if p.is_empty() {
                    return None;
                }
                let tps = complete_sum(p.iter().map(|x| Some(x.tps)));
                let e2e: Vec<f64> = p
                    .iter()
                    .map(|x| x.e2e_p95)
                    .filter(|v| !v.is_nan())
                    .collect();
                let e2e_avg = if e2e.is_empty() {
                    f64::NAN
                } else {
                    e2e.iter().sum::<f64>() / e2e.len() as f64
                };
                let e2e_s = if e2e_avg.is_nan() {
                    "–".to_string()
                } else {
                    format!("{:.0}ms", e2e_avg * 1000.0)
                };
                Some(format!(
                    "Σ{} {}active · mean endpoint p95 {} · {:.0}tok/s",
                    scope,
                    n,
                    e2e_s,
                    Metric(tps)
                ))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn performance_visible_order_matches_selection_and_filter_in_both_directions() {
        let mut app = App::new();
        app.view = View::Perf;
        app.panel_focus = 0;
        for (model, value) in [("missing", f64::NAN), ("high", 9.0), ("low", 1.0)] {
            app.snap.perf_rows.push(crate::collect::PerfRow {
                model: model.into(),
                req: 1.0,
                tps: value,
                ttft_p95: value,
                tpot_p95: value,
                e2e_p95: value,
                in_tok_p95: value,
                out_tok_p95: value,
                queue_p95: value,
                prefill_p95: value,
                decode_p95: value,
                preempt: value,
            });
        }
        for column in 0..4 {
            app.sort = column;
            for (desc, expected) in [(true, vec![1, 2, 0]), (false, vec![2, 1, 0])] {
                app.sort_desc = desc;
                assert_eq!(app.perf_rows_order(), expected);
                assert_eq!(app.order(), expected);
            }
        }
        app.filter = "HIGH".into();
        assert_eq!(app.perf_rows_order(), [1]);
        assert_eq!(app.order(), [1]);
        app.selected = 0;
        assert_eq!(app.selected_perf_model().as_deref(), Some("high"));
    }

    #[test]
    fn node_metric_sort_keeps_missing_rows_last_in_both_directions() {
        let mut app = App::new();
        app.view = View::Nodes;
        for (name, value) in [
            ("a", f64::NAN),
            ("b", 9.0),
            ("c", 1.0),
            ("d", f64::INFINITY),
        ] {
            app.snap.nodes.push(crate::collect::NodeInfo {
                name: name.into(),
                cpu_pct: value,
                mem_used_gb: value,
                disk_used_gb: value,
                load1: value,
                ..Default::default()
            });
        }
        for column in 1..=4 {
            app.sort = column;
            app.sort_desc = false;
            assert_eq!(app.order(), [2, 1, 0, 3]);
            app.sort_desc = true;
            assert_eq!(app.order(), [1, 2, 0, 3]);
        }
    }
}
