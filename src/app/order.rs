//! Row ordering — per-view sort/group into original-index order, filter
//! application, selection→original mapping, and the aggregate summary line.
//! Split out of `app.rs` (see `impl App`).

use super::*;

impl App {
    pub fn order(&self) -> Vec<usize> {
        use crate::collect::{Accel, ModelRow, PodRow};
        use std::cmp::Ordering::Equal;
        let mut idx = match self.view {
            // ── 컬럼 기반 정렬(o=컬럼 순환 · O=방향 토글). 비교는 오름차순으로 쓰고 desc 면 reverse. ──
            View::Accel => {
                let v = &self.snap.accel;
                let desc = self.sort_desc;
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|&a, &b| {
                    let (x, y): (&Accel, &Accel) = (&v[a], &v[b]);
                    let asc = match self.sort {
                        0 => x.util.partial_cmp(&y.util).unwrap_or(Equal),
                        1 => x.temp.partial_cmp(&y.temp).unwrap_or(Equal),
                        2 => x.mem_used_gb.partial_cmp(&y.mem_used_gb).unwrap_or(Equal),
                        3 => x.power.partial_cmp(&y.power).unwrap_or(Equal),
                        _ => (x.kind as u8, x.node.as_str(), x.id.as_str()).cmp(&(
                            y.kind as u8,
                            y.node.as_str(),
                            y.id.as_str(),
                        )),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
                    asc.then_with(|| {
                        (x.node.as_str(), x.id.as_str()).cmp(&(y.node.as_str(), y.id.as_str()))
                    })
                });
                idx
            }
            View::Overview => {
                let v = &self.snap.models;
                let desc = self.sort_desc;
                let oc = |a: Option<f64>, b: Option<f64>| {
                    a.unwrap_or(f64::NEG_INFINITY)
                        .partial_cmp(&b.unwrap_or(f64::NEG_INFINITY))
                        .unwrap_or(Equal)
                };
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
                    let asc = if desc { asc.reverse() } else { asc };
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
                        1 => x.cpu_pct.partial_cmp(&y.cpu_pct).unwrap_or(Equal),
                        2 => x.mem_used_gb.partial_cmp(&y.mem_used_gb).unwrap_or(Equal),
                        3 => x.disk_used_gb.partial_cmp(&y.disk_used_gb).unwrap_or(Equal),
                        4 => x.load1.partial_cmp(&y.load1).unwrap_or(Equal),
                        _ => x.name.cmp(&y.name),
                    };
                    let asc = if desc { asc.reverse() } else { asc };
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
            // Serving: 배포된 아티팩트를 family›version 그룹 순서로(트리 내비게이션이 그룹을 따라가게).
            View::Serving => self.serving_order(),
            // Deploy▸Model List: 배포 가능한 것 통합 트리(카탈로그+스토어), 단일 패널.
            View::Library => (0..self.library_items().len()).collect(),
            // Deploy▸Activity: compile Job + deploy rollout 통합 피드.
            View::Activity => (0..self.activity_rows().len()).collect(),
            View::Epp if self.panel_focus == 1 => (0..self.snap.pools.len()).collect(),
            View::Epp => {
                (0..self.snap.epp.as_ref().map(|e| e.scorers.len()).unwrap_or(0)).collect()
            }
            View::Perf if self.panel_focus == 1 => (0..self.snap.pod_queues.len()).collect(),
            // Perf 는 다지표 전용 정렬(perf_rows_order, 기본 best-first=내림). O 로 역순.
            View::Perf => {
                let mut o = self.perf_rows_order();
                if !self.sort_desc {
                    o.reverse();
                }
                o
            }
            View::Routing if self.panel_focus == 1 => (0..self.snap.pools.len()).collect(),
            View::Routing => (0..self.snap.routes.len()).collect(),
            View::Topo => Vec::new(), // 맵 뷰 — 리스트 선택 없음
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
                    scope, n, busy, util, mu, mt, pw
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
                let run: f64 = m.iter().filter_map(|x| x.running).sum();
                let wait: f64 = m.iter().filter_map(|x| x.waiting).sum();
                let tps: f64 = m.iter().filter_map(|x| x.tps).sum();
                Some(format!(
                    "Σ{} {}mdl · {}/{}ready · run {:.0} wait {:.0} · {:.0}tok/s",
                    scope, n, ready, desired, run, wait, tps
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
                    cpu,
                    mu,
                    mt,
                    du / 1024.0,
                    dt / 1024.0
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
                let tps: f64 = p
                    .iter()
                    .filter_map(|x| if x.tps.is_nan() { None } else { Some(x.tps) })
                    .sum();
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
                    "Σ{} {}active · E2E p95 {} · {:.0}tok/s",
                    scope, n, e2e_s, tps
                ))
            }
            _ => None,
        }
    }
}
