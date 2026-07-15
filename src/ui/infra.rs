//! Infra 뷰 — Accel(per-device 타임라인)·Nodes(health/placement)·Topology(캔버스 흐름 + 히트맵).
//! 공유 렌더 헬퍼는 상위 `ui` 모듈에서 가져온다.
use super::*;

// ── Accel ──────────────────────────────────────────────
pub(super) fn view_accel(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let a = &app.snap.accel[i];
            let model_cell = if pos == app.selected {
                marquee(&a.busy_model, 22, app.tick)
            } else {
                truncw(&a.busy_model, 22)
            };
            let mempct = if a.mem_total_gb > 0.0 {
                a.mem_used_gb / a.mem_total_gb * 100.0
            } else {
                0.0
            };
            let mut util = dot_bar(a.util, 9, util_color(a.util)).spans;
            util.push(Span::styled(
                format!(" {:>3.0}%", a.util),
                Style::default().fg(util_color(a.util)),
            ));
            let mut mem = dot_bar(mempct, 7, mem_color(mempct)).spans;
            mem.push(Span::styled(
                format!(
                    " {:.0}/{:.0}GB{}",
                    a.mem_used_gb,
                    a.mem_total_gb,
                    if a.unified_mem { "∪" } else { "" }
                ),
                Style::default().fg(C_DIM()),
            ));
            let (hg, hc) = if !a.alive {
                ("✗", C_BAD())
            } else if a.throttle > 0.0 {
                ("⚠", C_WARN())
            } else {
                ("●", C_OK())
            };
            let trend = sparkstr(
                &app.hist_for(&format!("acc:{}:{}:{}:util", a.kind.label(), a.node, a.id)),
                12,
                100,
            );
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(hg, Style::default().fg(hc)), // 상태=글리프
                    Span::raw(" "),
                    Span::styled(
                        a.disp().to_string(),
                        Style::default()
                            .fg(kind_color(a.kind))
                            .add_modifier(Modifier::BOLD),
                    ), // 모델(감지)·vendor색
                ])),
                cellw(a.id.clone(), 6),
                cellw(a.node.clone(), 14),
                Cell::from(Line::from(util)),
                Cell::from(Line::from(mem)),
                Cell::from(Span::styled(
                    format!("{:.0}°C", a.temp),
                    Style::default().fg(temp_color(a.temp)),
                )),
                cellw(format!("{:.0}W", a.power), 5),
                Cell::from(Span::styled(trend, Style::default().fg(util_color(a.util)))), // 인라인 트렌드(all-smi식)
                Cell::from(Span::styled(model_cell, Style::default().fg(C_DIM()))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(14),
        Constraint::Length(15),
        Constraint::Length(17),
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(13),
        Constraint::Min(8),
    ];
    let title = match app.agg_summary() {
        Some(a) => format!("Accelerators · ⏎ timeline  —  {}", a),
        None => "Accelerators · UTIL=compute% MEM=VRAM · ⏎ timeline".to_string(),
    };
    render_list_table(
        f,
        area,
        rows,
        &widths,
        &[
            "KIND",
            "ID",
            "NODE",
            "UTIL",
            "MEM",
            "TEMP",
            "PWR",
            "TREND(util)",
            "MODEL/POD",
        ],
        &title,
        app.selected,
        order.len(),
        app.sort_header_label(),
        app.sort_arrow(),
    );
}

pub(super) fn view_nodes(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order();
    let sel = app.selected;
    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line = 0usize;
    for (pos, &i) in order.iter().enumerate() {
        let n = &app.snap.nodes[i];
        let selected = pos == sel;
        if selected {
            sel_line = lines.len();
        }
        let (glyph, gc) = node_status(n);
        let memp = if n.mem_total_gb > 0.0 {
            n.mem_used_gb / n.mem_total_gb * 100.0
        } else {
            0.0
        };
        let mut h = vec![
            Span::styled(
                if selected { "▎" } else { " " },
                Style::default().fg(C_ACC()),
            ),
            Span::styled(format!("{} ", glyph), Style::default().fg(gc)),
            Span::styled(
                format!("{:<20} ", truncw(&n.name, 20)),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        h.extend(
            dot_bar(
                if n.cpu_pct.is_nan() { 0.0 } else { n.cpu_pct },
                8,
                util_color(n.cpu_pct.max(0.0)),
            )
            .spans,
        );
        h.push(Span::styled(
            if n.cpu_pct.is_nan() {
                " cpu   –".into()
            } else {
                format!(" cpu{:>3.0}%", n.cpu_pct)
            },
            Style::default().fg(C_DIM()),
        ));
        h.push(Span::styled(
            if n.mem_total_gb <= 0.0 {
                "  mem       –   ".into()
            } else {
                format!("  mem {:>4.0}/{:>4.0}GB", n.mem_used_gb, n.mem_total_gb)
            },
            Style::default().fg(mem_color(memp)),
        ));
        let diskp = if n.disk_total_gb > 0.0 {
            n.disk_used_gb / n.disk_total_gb * 100.0
        } else {
            0.0
        };
        h.push(Span::styled(
            if n.disk_total_gb <= 0.0 {
                "  disk      –  ".into()
            } else {
                format!("  disk {:>3.0}%", diskp)
            },
            Style::default().fg(mem_color(diskp)),
        ));
        h.push(Span::styled(
            if n.load1.is_nan() {
                "  load –".into()
            } else {
                format!("  load {:.1}", n.load1)
            },
            Style::default().fg(C_DIM()),
        ));
        let mut hline = Line::from(h);
        if selected {
            hline = hline.style(Style::default().bg(C_HL()).add_modifier(Modifier::BOLD));
        }
        lines.push(hline);
        // 이 노드의 디바이스들(트리 자식)
        let devs: Vec<&crate::collect::Accel> =
            app.snap.accel.iter().filter(|a| a.node == n.name).collect();
        if devs.is_empty() {
            lines.push(Line::from(Span::styled(
                "   └─ (no accelerators)",
                Style::default().fg(C_TRACK()),
            )));
        } else {
            let last = devs.len();
            for (j, a) in devs.iter().enumerate() {
                lines.push(accel_brief(
                    a,
                    if j + 1 == last { "└─" } else { "├─" },
                    false,
                ));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no nodes)",
            Style::default().fg(C_DIM()),
        )));
    }
    // 선택 노드가 화면에 보이도록 세로 스크롤.
    let vis = (area.height as usize).saturating_sub(2);
    let scroll = if sel_line + 2 > vis {
        (sel_line + 3).saturating_sub(vis) as u16
    } else {
        0
    };
    f.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .block(block(&match app.agg_summary() {
                Some(a) => format!("Nodes · ⏎ detail  —  {}", a),
                None => format!(
                    "Nodes · node → devices · ⏎ detail{}",
                    count_suffix(sel, order.len())
                ),
            })),
        area,
    );
}

// ── Topology / device pressure 맵 (Canvas) ─────────────────────────
// 서빙 요청 경로(Gateway→EPP→Pool)와 노드별 가속기 pressure(util 히트, ●=점유 모델)를 2D 로.
pub(super) fn view_topo(f: &mut Frame, area: Rect, app: &App) {
    use ratatui::widgets::canvas::{Canvas, Line as CLine, Rectangle};
    // 노드(가속기 보유) 목록.
    let mut node_names: Vec<String> = app
        .snap
        .accel
        .iter()
        .map(|a| a.node.clone())
        .filter(|n| !n.is_empty())
        .collect();
    node_names.sort();
    node_names.dedup();
    let epp_present = app.snap.epp.is_some() || !app.snap.pools.is_empty();
    let gw = !app.snap.gw_addr.is_empty();
    // 노드 수에 맞춰 열/박스 크기 적응 — 많아도 캔버스(0..100) 안에 다 들어가게(겹침 방지).
    let ncount = node_names.len().max(1);
    let cols = if ncount <= 2 {
        1
    } else if ncount <= 6 {
        2
    } else {
        3
    };
    let x0 = 16.0; // 노드 그리드 시작 x(왼쪽 flow 컬럼 뒤)
    let bw = (100.0 - x0) / cols as f64; // 열 폭(간격 포함)
    let nrows = ncount.div_ceil(cols);
    let y_top = 92.0;
    let row_h = (86.0 / nrows as f64).min(24.0); // 행 높이(행 많으면 축소)
    let bh = (row_h - 3.0).max(6.0); // 박스 높이(행 간격 3)
    let canvas = Canvas::default()
        .marker(ratatui::symbols::Marker::HalfBlock)
        .x_bounds([0.0, 100.0])
        .y_bounds([0.0, 100.0])
        .block(block(
            "Topology · request flow → device pressure · util heat ●busy ·free · w hub",
        ))
        .paint(move |ctx| {
            // ── 서빙 경로 체인(왼쪽 세로) ──
            let chain = [
                ("Gateway", gw, C_ACC()),
                ("EPP", epp_present, C_OK()),
                ("Pool", epp_present, C_OK()),
            ];
            let mut cy = 80.0;
            let mut prev: Option<f64> = None;
            for (label, on, col) in chain {
                let c = if on { col } else { C_DIM() };
                let g = if on { "●" } else { "○" };
                ctx.print(
                    2.0,
                    cy,
                    Line::from(Span::styled(
                        format!("{} {}", g, label),
                        Style::default().fg(c).add_modifier(Modifier::BOLD),
                    )),
                );
                if let Some(py) = prev {
                    ctx.draw(&CLine {
                        x1: 3.0,
                        y1: py - 2.0,
                        x2: 3.0,
                        y2: cy + 1.0,
                        color: C_TRACK(),
                    });
                }
                prev = Some(cy);
                cy -= 14.0;
            }
            // Pool 앵커(서빙 흐름선 시작점).
            let pool_x = 4.0;
            let pool_y = 80.0 - 2.0 * 14.0; // 체인 3번째(Pool) y
                                            // ── 서빙 흐름선(Pool → 실제 서빙 중인 노드), 모델색 — 박스보다 먼저 그려 아래 깔림 ──
            for (i, node) in node_names.iter().enumerate() {
                let busy: Vec<&crate::collect::Accel> = app
                    .snap
                    .accel
                    .iter()
                    .filter(|a| &a.node == node && !a.busy_model.is_empty())
                    .collect();
                if busy.is_empty() {
                    continue;
                }
                let bx = x0 + (i % cols) as f64 * bw;
                let by = y_top - (i / cols) as f64 * row_h;
                let col = model_color(&busy[0].busy_model);
                if epp_present {
                    ctx.draw(&CLine {
                        x1: pool_x,
                        y1: pool_y,
                        x2: bx,
                        y2: by - bh / 2.0,
                        color: col,
                    });
                }
            }
            // ── 노드 박스 + 디바이스 pressure ──
            for (i, node) in node_names.iter().enumerate() {
                let bx = x0 + (i % cols) as f64 * bw;
                let by = y_top - (i / cols) as f64 * row_h;
                ctx.draw(&Rectangle {
                    x: bx,
                    y: by - bh,
                    width: bw - 3.0,
                    height: bh,
                    color: C_TRACK(),
                });
                // 노드명.
                ctx.print(
                    bx + 1.0,
                    by - 2.0,
                    Line::from(Span::styled(
                        truncw(node, 18),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )),
                );
                // 디바이스들 — kind 별 줄, 각 디바이스는 util 히트 블록.
                let devs: Vec<&crate::collect::Accel> =
                    app.snap.accel.iter().filter(|a| &a.node == node).collect();
                let mut by_kind: std::collections::BTreeMap<&str, Vec<&crate::collect::Accel>> =
                    std::collections::BTreeMap::new();
                for d in &devs {
                    by_kind.entry(d.kind.label()).or_default().push(d);
                }
                // 내부는 위→아래로 일관 스택: 이름(by-2) → 서빙모델(by-5) → device 행(by-9↓).
                // 서빙 모델명이 박스 바닥에 고정돼 device 행과 겹치던 문제 해소.
                let mut ry = by - 9.0;
                for (k, ds) in &by_kind {
                    if ry < by - bh + 1.5 {
                        break; // 박스 바닥 넘어가면 중단(오버플로 방지)
                    }
                    let mut sp = vec![Span::styled(
                        format!("{:<5} ", k),
                        Style::default().fg(C_DIM()),
                    )];
                    for d in ds {
                        let (g, c) = if !d.alive {
                            ("✗", C_BAD())
                        } else if d.busy_model.is_empty() {
                            ("·", C_TRACK())
                        } else {
                            ("█", util_color(d.util))
                        };
                        sp.push(Span::styled(g, Style::default().fg(c)));
                    }
                    let live: Vec<&&crate::collect::Accel> =
                        ds.iter().filter(|d| d.alive).collect();
                    let avg = if live.is_empty() {
                        0.0
                    } else {
                        live.iter().map(|d| d.util).sum::<f64>() / live.len() as f64
                    };
                    let free = ds
                        .iter()
                        .filter(|d| d.alive && d.busy_model.is_empty())
                        .count();
                    sp.push(Span::styled(
                        format!("  {:>3.0}%  {}free", avg, free),
                        Style::default().fg(C_DIM()),
                    ));
                    ctx.print(bx + 1.0, ry, Line::from(sp));
                    ry -= 3.5;
                }
                // 서빙 중인 모델명(어느 pod/모델이 이 노드를 점유하는지) — 이름 바로 아래(by-5).
                let mut serving: Vec<&str> = devs
                    .iter()
                    .filter(|d| !d.busy_model.is_empty())
                    .map(|d| d.busy_model.as_str())
                    .collect();
                serving.sort();
                serving.dedup();
                if !serving.is_empty() {
                    let mut msp: Vec<Span> = vec![Span::styled("⇢ ", Style::default().fg(C_DIM()))];
                    for m in serving.iter().take(2) {
                        msp.push(Span::styled(
                            format!("{} ", truncw(m, 14)),
                            Style::default().fg(model_color(m)),
                        ));
                    }
                    ctx.print(bx + 1.0, by - 5.0, Line::from(msp));
                }
            }
            if node_names.is_empty() {
                ctx.print(
                    20.0,
                    50.0,
                    Line::from(Span::styled(
                        "(no accelerators detected)",
                        Style::default().fg(C_DIM()),
                    )),
                );
            }
        });
    f.render_widget(canvas, area);
}
