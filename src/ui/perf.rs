//! Perf 뷰 — 구간별 p95(QUEUE→PREFILL→DECODE→TPOT→E2E) + tok/s + SLO advisor,
//! 그리고 선택 모델의 상세(percentile 테이블 + 타임라인 그리드 + E2E 히스토그램).
//! 공유 렌더 헬퍼(ms/rate/bar_timeline/two_panes/block/…)는 상위 `ui` 모듈에서 가져온다.
use super::*;

pub(super) fn view_perf(f: &mut Frame, area: Rect, app: &App) {
    // 드릴: 선택 모델 지연 분포(Enter). perf_detail 이 채워져 있으면 그것부터.
    if app.detail {
        if let Some(d) = &app.perf_detail {
            perf_detail_view(f, area, app, d);
            return;
        }
    }
    let p = &app.snap.perf;
    let any = [p.e2e_p95, p.ttft_p95, p.tps, p.req_rate]
        .iter()
        .any(|x| !x.is_nan());

    // 디바이스 패널 높이는 대수에 맞춰 가변(작은 클러스터는 컴팩트, 큰 건 상한). 상한 초과분은 "+N more".
    let ndev = app.snap.accel.len().max(1) as u16;
    let dev_h = (ndev + 2).clamp(6, 18);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(dev_h),
            Constraint::Length(3),
            Constraint::Min(5),
        ])
        .split(area);

    // 상단: 디바이스별 util/VRAM 시계열을 컴팩트 한 줄 스파크라인으로(바로 보이는 개요).
    let inner_w = (rows[0].width as usize).saturating_sub(2);
    // 라벨/값 고정폭(≈37) 제외한 나머지를 util·VRAM 스파크 두 개로 균등 분배.
    let spark_w = (inner_w.saturating_sub(38) / 2).clamp(6, 30);
    let mut dlines: Vec<Line> = Vec::new();
    if app.snap.accel.is_empty() {
        dlines.push(Line::from(Span::styled(
            "(no accelerators)",
            Style::default().fg(C_DIM()),
        )));
    }
    for a in &app.snap.accel {
        let k = format!("acc:{}:{}:{}", a.kind.label(), a.node, a.id);
        let uh = app.hist_for(&format!("{}:util", k));
        let mh = app.hist_for(&format!("{}:mem", k));
        let memp = if a.mem_total_gb > 0.0 {
            a.mem_used_gb / a.mem_total_gb * 100.0
        } else {
            0.0
        };
        let (hg, hc) = if !a.alive {
            ("✗", C_BAD())
        } else if a.throttle > 0.0 {
            ("⚠", C_WARN())
        } else {
            ("●", C_OK())
        };
        let mut sp = vec![
            Span::styled(format!("{} ", hg), Style::default().fg(hc)),
            Span::styled(
                format!("{:<5}", a.disp()),
                Style::default()
                    .fg(kind_color(a.kind))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<6} ", truncw(&a.id, 6)),
                Style::default().fg(C_DIM()),
            ),
            Span::styled("util ", Style::default().fg(C_DIM())),
        ];
        sp.extend(spark_colored(&uh, spark_w, 100));
        sp.push(Span::styled(
            format!(" {:>3.0}%", a.util),
            Style::default().fg(util_color(a.util)),
        ));
        sp.push(Span::styled(
            if a.unified_mem { "  m∪ " } else { "  vram " },
            Style::default().fg(C_DIM()),
        ));
        sp.extend(spark_colored(&mh, spark_w, 100));
        sp.push(Span::styled(
            format!(" {:>3.0}%", memp),
            Style::default().fg(mem_color(memp)),
        ));
        dlines.push(Line::from(sp));
    }
    // 무언의 잘림 방지: 패널에 안 들어가면 마지막 줄을 "+N more" 로(전체는 Accel 탭).
    let cap = (rows[0].height as usize).saturating_sub(2);
    if dlines.len() > cap && cap > 0 {
        let hidden = dlines.len() - (cap - 1);
        dlines.truncate(cap - 1);
        dlines.push(Line::from(Span::styled(
            format!("  … +{} more (see Accel tab)", hidden),
            Style::default().fg(C_DIM()),
        )));
    }
    f.render_widget(
        Paragraph::new(dlines).block(block("Devices · util / VRAM over time (now on right)")),
        rows[0],
    );

    // throughput 숫자 + 데이터 없음 안내
    let tl = Line::from(vec![
        Span::styled("req/s ", Style::default().fg(C_DIM())),
        Span::styled(
            format!("{}  ", rate(p.req_rate)),
            Style::default().fg(C_OK()),
        ),
        Span::styled("err/s ", Style::default().fg(C_DIM())),
        Span::styled(
            format!("{}  ", rate(p.err_rate)),
            Style::default().fg(if p.err_rate > 0.0 { C_BAD() } else { C_DIM() }),
        ),
        Span::styled("tok/s ", Style::default().fg(C_DIM())),
        Span::styled(format!("{}  ", rate(p.tps)), Style::default().fg(C_OK())),
        Span::styled("prefix-hit ", Style::default().fg(C_DIM())),
        Span::styled(
            if p.prefix_hit.is_nan() {
                "–  ".into()
            } else {
                format!("{:.0}%  ", p.prefix_hit * 100.0)
            },
            Style::default().fg(C_ACC()),
        ),
        Span::styled(
            if any {
                ""
            } else {
                "· no data: needs EPP-path traffic + vLLM metrics"
            },
            Style::default().fg(C_WARN()),
        ),
    ]);
    f.render_widget(Paragraph::new(tl).block(block("Throughput")), rows[1]);

    // per-model 성능(모델=하드웨어 배치별) + per-pod 큐
    let (bodyc_l, bodyc_r) = two_panes(rows[2], 72);

    let mfocus = app.panel_focus == 0; // per-model 패널 포커스
    let order = app.perf_rows_order(); // per-model: active(서빙 중) 만 + 정렬(포커스 무관)
    if app.snap.perf_rows.is_empty() || order.is_empty() {
        let msg = if app.snap.perf_rows.is_empty() {
            "shows per model once EPP-path traffic + vLLM metrics are present."
        } else {
            "no active models right now — rows appear while a model is serving."
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "no per-model perf data",
                    Style::default().fg(C_DIM()),
                )),
                Line::from(Span::styled(msg, Style::default().fg(C_DIM()))),
            ])
            .block(block(
                "Per-model perf (p95) · latency / tokens / throughput",
            )),
            bodyc_l,
        );
    } else {
        let mrows: Vec<Row> = order
            .iter()
            .map(|&i| {
                let r = &app.snap.perf_rows[i];
                let preempt_cell = if r.preempt.is_nan() || r.preempt <= 0.0 {
                    Cell::from(Span::styled("·", Style::default().fg(C_DIM())))
                } else {
                    Cell::from(Span::styled(
                        format!("{:.2}", r.preempt),
                        Style::default().fg(C_BAD()),
                    ))
                };
                // SLO 상태 글리프(목표 대비): ●충족 ◐부분 ✗위반 ·목표없음.
                let adv = app.perf_advice(r);
                let (sg, sc) = if !adv.has_obj {
                    ("·", C_DIM())
                } else if adv.all_met() {
                    ("●", C_OK())
                } else if adv.checks.iter().any(|(_, ok)| !*ok) {
                    ("✗", C_BAD())
                } else {
                    ("◐", C_WARN())
                };
                let model_cell = Cell::from(Line::from(vec![
                    Span::styled(format!("{} ", sg), Style::default().fg(sc)),
                    Span::styled(truncw(&r.model, 14), Style::default().fg(Color::White)),
                ]));
                Row::new(vec![
                    model_cell,
                    Cell::from(Span::styled(rate(r.req), Style::default().fg(C_OK()))),
                    Cell::from(Span::styled(rate(r.tps), Style::default().fg(C_OK()))),
                    cellw(ms(r.ttft_p95), 7),
                    Cell::from(Span::styled(ms(r.queue_p95), Style::default().fg(C_WARN()))), // 대기
                    Cell::from(Span::styled(
                        ms(r.prefill_p95),
                        Style::default().fg(C_PREFILL()),
                    )), // P
                    Cell::from(Span::styled(
                        ms(r.decode_p95),
                        Style::default().fg(C_DECODE()),
                    )), // D
                    cellw(ms(r.tpot_p95), 7),
                    Cell::from(Span::styled(ms(r.e2e_p95), Style::default().fg(C_WARN()))),
                    preempt_cell,
                ])
            })
            .collect();
        let mt = Table::new(
            mrows,
            [
                Constraint::Min(12),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(6),
            ],
        )
        .header(hrow_sorted(
            &[
                "MODEL", "req/s", "tok/s", "TTFT", "QUEUE", "PFILL", "DECODE", "TPOT", "E2E",
                "premt",
            ],
            app.sort_header_label(),
            app.sort_arrow(),
        ));
        let title = match app.agg_summary() {
            Some(a) => format!("Per-model perf · ⏎ drill  —  {}", a),
            None => format!(
                "Per-model perf · active · o sort:{} · ⏎ drill{}",
                app.sort_label(),
                if mfocus {
                    count_suffix(app.selected, order.len())
                } else {
                    String::new()
                }
            ),
        };
        // per-model 표 + 하단 SLO 어드바이저(선택 모델).
        let lc = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4), Constraint::Length(5)])
            .split(bodyc_l);
        let (tbl_area, adv_area) = (lc[0], lc[1]);
        let mut mt = mt.column_spacing(1).block(if mfocus {
            block_active(&title)
        } else {
            block(&title)
        });
        if mfocus {
            mt = mt.row_highlight_style(hl_style()).highlight_symbol("▎");
        }
        let mut st = TableState::default();
        st.select(if mfocus { Some(app.selected) } else { None });
        f.render_stateful_widget(mt, tbl_area, &mut st);
        if mfocus {
            list_scrollbar(f, tbl_area, order.len(), app.selected, 1);
        }
        // 선택 모델 SLO 판정 + 조정 제안.
        let mut al: Vec<Line> = Vec::new();
        if let Some(&si) = order.get(app.selected) {
            let r = &app.snap.perf_rows[si];
            let adv = app.perf_advice(r);
            if !adv.has_obj {
                al.push(Line::from(Span::styled(
                    "no objective — set via Models → ⏎ Objective (target TTFT/TPOT/E2E/tok·s)",
                    Style::default().fg(C_DIM()),
                )));
            } else {
                let mut sp = vec![Span::styled("SLO  ", Style::default().fg(C_DIM()))];
                for (m, ok) in &adv.checks {
                    sp.push(Span::styled(
                        format!("{}{}  ", if *ok { "✓" } else { "✗" }, m),
                        Style::default().fg(if *ok { C_OK() } else { C_BAD() }),
                    ));
                }
                if adv.checks.is_empty() {
                    sp.push(Span::styled(
                        "(no observed metrics yet)",
                        Style::default().fg(C_DIM()),
                    ));
                }
                al.push(Line::from(sp));
                for t in adv.tips.iter().take(2) {
                    al.push(Line::from(Span::styled(
                        format!("→ {}", t),
                        Style::default().fg(C_WARN()),
                    )));
                }
                if adv.all_met() {
                    al.push(Line::from(Span::styled(
                        "→ meets objective ✓",
                        Style::default().fg(C_OK()),
                    )));
                }
            }
        }
        f.render_widget(
            Paragraph::new(al).block(block("SLO advisor (data-driven)")),
            adv_area,
        );
    }

    // per-pod queue (요청 분배 — 절대 큐 깊이). focus 1 이면 선택 강조.
    let qfocus = app.panel_focus == 1;
    let mut ql: Vec<Line> = Vec::new();
    let maxq = app
        .snap
        .pod_queues
        .iter()
        .map(|(_, q)| *q)
        .fold(1.0, f64::max);
    if app.snap.pod_queues.is_empty() {
        ql.push(Line::from(Span::styled(
            "no per-pod queue data",
            Style::default().fg(C_DIM()),
        )));
    } else {
        for (j, (pod, q)) in app.snap.pod_queues.iter().enumerate().take(12) {
            let mut sp = vec![Span::styled(
                format!("{:<20} ", truncw(pod, 20)),
                Style::default().fg(Color::White),
            )];
            sp.extend(bar_line(q / maxq * 100.0, 8, C_ACC()).spans);
            sp.push(Span::styled(
                format!(" {:.0}", q),
                Style::default().fg(C_DIM()),
            ));
            let mut line = Line::from(sp);
            if qfocus && app.selected == j {
                line.style = Style::default().bg(C_HL()).add_modifier(Modifier::BOLD);
            }
            ql.push(line);
        }
    }
    let qtitle = format!(
        "request distribution (per-pod queue){}",
        if qfocus {
            count_suffix(app.selected, app.snap.pod_queues.len())
        } else {
            String::new()
        }
    );
    f.render_widget(
        Paragraph::new(ql).block(if qfocus {
            block_active(&qtitle)
        } else {
            block(&qtitle)
        }),
        bodyc_r,
    );
}

/// Perf 드릴다운 — 선택 모델 구간별 p50/p95/p99 + 지표별 시계열 타임라인 + E2E 버킷 히스토그램.
pub(super) fn perf_detail_view(f: &mut Frame, area: Rect, app: &App, d: &PerfDetail) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(4)])
        .split(area);
    // 구간별 percentile 테이블
    let qrow = |label: &str, a: &[f64; 3], col: Color| {
        Row::new(vec![
            Cell::from(Span::styled(
                label.to_string(),
                Style::default().fg(C_DIM()),
            )),
            Cell::from(Span::styled(ms(a[0]), Style::default().fg(col))),
            Cell::from(Span::styled(ms(a[1]), Style::default().fg(col))),
            Cell::from(Span::styled(
                ms(a[2]),
                Style::default().fg(col).add_modifier(Modifier::BOLD),
            )),
        ])
    };
    let qt = Table::new(
        vec![
            qrow("TTFT", &d.ttft, C_ACC()),
            qrow("TPOT", &d.tpot, C_DECODE()),
            qrow("E2E", &d.e2e, C_WARN()),
        ],
        [
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(hrow(&["METRIC", "p50", "p95", "p99"]))
    .column_spacing(2)
    .block(block(&format!(
        "latency percentiles · {} · esc back",
        truncw(&d.model, 30)
    )));
    f.render_widget(qt, rows[0]);

    // 하단: 좌 = 지표별 시계열 타임라인 그리드, 우 = E2E 버킷 히스토그램.
    let (grid_area, hist_area) = two_panes(rows[1], 58);

    // per-model 지표 타임라인 — 컬럼 값들을 시간축으로. 데이터 있는 것만.
    let mk = format!("mperf:{}", d.model);
    let series: [(&str, &str, &str); 6] = [
        ("tps", "tok/s", ""),
        ("ttft", "TTFT", "ms"),
        ("queue", "QUEUE", "ms"),
        ("prefill", "PREFILL", "ms"),
        ("decode", "DECODE", "ms"),
        ("e2e", "E2E", "ms"),
    ];
    let present: Vec<&(&str, &str, &str)> = series
        .iter()
        .filter(|(s, _, _)| !app.hist_for(&format!("{}:{}", mk, s)).is_empty())
        .collect();
    if present.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no per-model time series yet — populates under traffic",
                Style::default().fg(C_DIM()),
            )))
            .block(block("metrics over time")),
            grid_area,
        );
    } else {
        let mut dash = Dashboard::new().min_width(30);
        for (s, label, unit) in present {
            let key = format!("{}:{}", mk, s);
            dash = dash.cell(move |f, rect| bar_timeline(f, rect, app, &key, label, unit, None));
        }
        dash.render(f, grid_area);
    }

    // E2E 지연 버킷 분포(히스토그램) — 누적차 rate, 바 길이 = 상대 빈도.
    let maxc = d
        .buckets
        .iter()
        .map(|(_, c)| *c)
        .fold(0.0f64, f64::max)
        .max(1e-9);
    let mut hl: Vec<Line> = Vec::new();
    if d.buckets.iter().all(|(_, c)| *c <= 0.0) {
        hl.push(Line::from(Span::styled(
            "idle — E2E buckets populate under traffic",
            Style::default().fg(C_DIM()),
        )));
    } else {
        let bw = (hist_area.width as usize).saturating_sub(20).clamp(8, 34);
        for (le, c) in &d.buckets {
            if *c <= 0.0 {
                continue;
            }
            let lbl = if le.is_infinite() {
                "  ∞".to_string()
            } else {
                format!("≤{}", ms(*le))
            };
            let mut sp = vec![Span::styled(
                format!("{:>8} ", lbl),
                Style::default().fg(C_DIM()),
            )];
            sp.extend(bar_line(c / maxc * 100.0, bw, C_ACC()).spans);
            sp.push(Span::styled(
                format!(" {:.2}/s", c),
                Style::default().fg(C_DIM()),
            ));
            hl.push(Line::from(sp));
        }
    }
    f.render_widget(
        Paragraph::new(hl).block(block("E2E distribution · rate/bucket")),
        hist_area,
    );
}
