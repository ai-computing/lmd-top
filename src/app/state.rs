//! Snapshot application — sparkline history append, edge-triggered alert
//! detection, and history readout. Split out of `app.rs` (see `impl App`).

use super::*;
use crate::collect::Snapshot;

impl App {
    fn push_hist(&mut self, key: &str, val: u64) {
        let buf = self.hist.entry(key.to_string()).or_default();
        buf.push_back(val);
        while buf.len() > HIST {
            buf.pop_front();
        }
    }

    /// 새 스냅샷 반영 + ts 가 바뀌었으면 히스토리 append.
    pub fn apply(&mut self, snap: Snapshot) {
        if snap.ts != self.snap.ts {
            // per-accelerator: util / mem% / temp 타임라인
            for a in &snap.accel {
                let k = format!("acc:{}:{}:{}", a.kind.label(), a.node, a.id);
                self.push_hist(
                    &format!("{}:util", k),
                    a.util.round().clamp(0.0, 100.0) as u64,
                );
                let memp = if a.mem_total_gb > 0.0 {
                    a.mem_used_gb / a.mem_total_gb * 100.0
                } else {
                    0.0
                };
                self.push_hist(&format!("{}:mem", k), memp.round().clamp(0.0, 100.0) as u64);
                self.push_hist(&format!("{}:temp", k), a.temp.round().max(0.0) as u64);
            }
            // per-node: cpu% / mem% / load
            for n in &snap.nodes {
                let k = format!("nod:{}", n.name);
                if !n.cpu_pct.is_nan() {
                    self.push_hist(
                        &format!("{}:cpu", k),
                        n.cpu_pct.round().clamp(0.0, 100.0) as u64,
                    );
                }
                let memp = if n.mem_total_gb > 0.0 {
                    n.mem_used_gb / n.mem_total_gb * 100.0
                } else {
                    0.0
                };
                self.push_hist(&format!("{}:mem", k), memp.round().clamp(0.0, 100.0) as u64);
                if n.disk_total_gb > 0.0 {
                    let dp = n.disk_used_gb / n.disk_total_gb * 100.0;
                    self.push_hist(&format!("{}:disk", k), dp.round().clamp(0.0, 100.0) as u64);
                }
                if !n.load1.is_nan() {
                    self.push_hist(
                        &format!("{}:load", k),
                        (n.load1 * 10.0).round().max(0.0) as u64,
                    );
                }
            }
            // 클러스터 추이 — 실제 존재하는 가속기 종류만 집계(GPU/RBLN/RNGD 각각)
            let mean = |v: &[f64]| {
                if v.is_empty() {
                    f64::NAN
                } else {
                    v.iter().sum::<f64>() / v.len() as f64
                }
            };
            let pct = |u: f64, t: f64| if t > 0.0 { u / t * 100.0 } else { 0.0 };
            let mut byk: std::collections::BTreeMap<&str, (Vec<f64>, f64, f64)> =
                std::collections::BTreeMap::new();
            for a in &snap.accel {
                let e = byk.entry(a.kind.label()).or_default();
                e.0.push(a.util);
                e.1 += a.mem_used_gb;
                e.2 += a.mem_total_gb;
            }
            for (k, (u, mu, mt)) in &byk {
                self.push_hist(
                    &format!("sys:{}_util", k),
                    mean(u).round().clamp(0.0, 100.0) as u64,
                );
                self.push_hist(
                    &format!("sys:{}_mem", k),
                    pct(*mu, *mt).round().clamp(0.0, 100.0) as u64,
                );
            }
            let cpus: Vec<f64> = snap
                .nodes
                .iter()
                .filter(|n| !n.cpu_pct.is_nan())
                .map(|n| n.cpu_pct)
                .collect();
            if !cpus.is_empty() {
                self.push_hist("sys:cpu", mean(&cpus).round().clamp(0.0, 100.0) as u64);
            }
            let (hmu, hmt): (f64, f64) = snap.nodes.iter().fold((0.0, 0.0), |(u, t), n| {
                (u + n.mem_used_gb, t + n.mem_total_gb)
            });
            self.push_hist(
                "sys:host_mem",
                pct(hmu, hmt).round().clamp(0.0, 100.0) as u64,
            );
            let tps = snap.perf.tps;
            if !tps.is_nan() {
                self.push_hist("sys:tps", tps.round().max(0.0) as u64);
            }
            // per-model perf 시계열 — Perf/Model 상세의 지표별 타임라인용.
            // 지연은 ms(정수), 처리량은 rate(반올림)로 저장. 값 없으면(NaN) 스킵.
            for r in &snap.perf_rows {
                let k = format!("mperf:{}", r.model);
                let push_ms = |s: &mut Self, sub: &str, v: f64| {
                    if !v.is_nan() {
                        s.push_hist(
                            &format!("{}:{}", k, sub),
                            (v * 1000.0).round().max(0.0) as u64,
                        );
                    }
                };
                push_ms(self, "ttft", r.ttft_p95);
                push_ms(self, "tpot", r.tpot_p95);
                push_ms(self, "e2e", r.e2e_p95);
                push_ms(self, "queue", r.queue_p95);
                push_ms(self, "prefill", r.prefill_p95);
                push_ms(self, "decode", r.decode_p95);
                if !r.tps.is_nan() {
                    self.push_hist(&format!("{}:tps", k), r.tps.round().max(0.0) as u64);
                }
                if !r.req.is_nan() {
                    self.push_hist(&format!("{}:req", k), r.req.round().max(0.0) as u64);
                }
            }
            self.detect_alerts(&snap);
            // 세션 에너지 기준선(디바이스 최초 관측 시 캡처).
            for a in &snap.accel {
                if !a.energy_mj.is_nan() {
                    self.energy_base
                        .entry(Self::accel_key(a))
                        .or_insert(a.energy_mj);
                }
            }
            if self.energy_since == 0 {
                self.energy_since = snap.ts;
            }
        }
        self.snap = snap;
        let n = self.list_len();
        if n > 0 && self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// 스냅샷에서 임계 조건을 뽑아 신규 발생분만 히스토리에 쌓고 토스트+플래시 트리거.
    /// key 안정성으로 엣지(비활성→활성)만 알림 — 지속 조건은 반복 토스트하지 않음.
    fn detect_alerts(&mut self, snap: &Snapshot) {
        let now = snap.ts;
        // 상태 없는 조건(가속기/노드/pod-Failed) — JSON 출력과 공유.
        let mut current = snapshot_alerts(snap);
        // pod 재시작 증가(델타) — 이전 스냅샷 필요(stateful).
        for p in &snap.pods {
            let prev = self
                .prev_restarts
                .get(&p.name)
                .copied()
                .unwrap_or(p.restarts);
            if p.restarts > prev {
                current.push(Alert {
                    ts: now,
                    sev: Sev::Warn,
                    key: format!("restart:{}:{}", p.name, p.restarts),
                    msg: format!("pod {} restarted (x{})", p.name, p.restarts),
                });
            }
        }
        self.prev_restarts = snap
            .pods
            .iter()
            .map(|p| (p.name.clone(), p.restarts))
            .collect();

        // 엣지 검출: active_alerts 에 없던 key = 신규.
        let mut new_alerts: Vec<Alert> = Vec::new();
        for a in &current {
            if !self.active_alerts.contains(&a.key) {
                new_alerts.push(a.clone());
            }
        }
        self.active_alerts = current.iter().map(|a| a.key.clone()).collect();
        if new_alerts.is_empty() {
            return;
        }
        // 히스토리 적재(최신 앞, cap 50)
        for a in &new_alerts {
            self.alerts.push_front(a.clone());
        }
        while self.alerts.len() > 50 {
            self.alerts.pop_back();
        }
        // 토스트: 1건이면 메시지, 여러건이면 요약. 하나라도 Bad 면 빨강.
        let any_bad = new_alerts.iter().any(|a| a.sev == Sev::Bad);
        let msg = if new_alerts.len() == 1 {
            let a = &new_alerts[0];
            format!("{} {}", if a.sev == Sev::Bad { "✗" } else { "⚠" }, a.msg)
        } else {
            format!("⚠ {} new alerts — press A", new_alerts.len())
        };
        self.toast = Some(msg);
        self.toast_until = now + 5;
        self.toast_bad = any_bad;
        self.flash_until = now + 3; // 3초 플래시
    }

    pub fn hist_for(&self, key: &str) -> Vec<u64> {
        self.hist
            .get(key)
            .map(|d| d.iter().copied().collect())
            .unwrap_or_default()
    }
}
