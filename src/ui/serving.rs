//! Serving 뷰 — 실행 중 배포 정렬표와 llm-serving Pods.
//! 공유 렌더 헬퍼는 상위 `ui` 모듈에서 가져온다.
use super::*;

// ── Pods ───────────────────────────────────────────────
pub(super) fn view_pods(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let p = &app.snap.pods[i];
            let color = match p.phase.as_str() {
                "Running" => C_OK(),
                "Pending" => C_WARN(),
                "Failed" => C_BAD(),
                _ => C_DIM(),
            };
            let glyph = match p.phase.as_str() {
                "Running" => "●",
                "Pending" => "◐",
                "Failed" => "✗",
                "Succeeded" => "✓",
                _ => "○",
            };
            let name = if pos == app.selected {
                marquee(&p.name, 40, app.tick)
            } else {
                truncw(&p.name, 40)
            };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(format!("{} ", glyph), Style::default().fg(color)),
                    Span::raw(name),
                ])),
                cellw(p.ready.clone(), 6),
                Cell::from(Span::styled(p.phase.clone(), Style::default().fg(color))),
                cellw(p.node.clone(), 18),
                Cell::from(Span::styled(
                    p.restarts.to_string(),
                    Style::default().fg(if p.restarts > 0 { C_WARN() } else { C_DIM() }),
                )),
            ])
        })
        .collect();
    let widths = [
        Constraint::Min(20),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(18),
        Constraint::Length(8),
    ];
    render_list_table(
        f,
        area,
        rows,
        &widths,
        &["POD", "READY", "PHASE", "NODE", "RESTARTS"],
        "Pods (llm-serving) · ⏎ detail",
        app.selected,
        order.len(),
        app.sort_header_label(),
        app.sort_arrow(),
    );
}

// ── Serving — 현재 서빙 중인 배포(라이브 아티팩트)를 균일 정렬표로. ──
// 컬럼: ● 상태(좌) · MODEL · ENGINE · TARGET(opts) · REP · NODE · t/s. o/O 정렬 · x/s/r/y/l/⏎ 액션.
pub(super) fn view_serving(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order(); // 컬럼 정렬 반영
    let engine_short = |e: &str| -> &'static str {
        if e.contains("RBLN") {
            "vLLM-RBLN"
        } else if e.contains("Furiosa") {
            "Furiosa-LLM"
        } else if e.contains("vLLM") {
            "vLLM"
        } else {
            "custom"
        }
    };
    let rows: Vec<Row> = order
        .iter()
        .map(|&i| {
            let a = &app.snap.artifacts[i];
            let m = app.snap.models.iter().find(|m| m.name == a.model);
            let (desired, ready, tps) =
                m.map(|m| (m.desired, m.ready, m.tps)).unwrap_or((0, 0, None));
            let phase = app.deploy_phase(&a.model, desired, ready);
            let gc = match phase.label() {
                "Serving" => C_OK(),
                "Failed" => C_BAD(),
                "Scaled-0" => C_DIM(),
                _ => C_WARN(), // Starting / Degraded
            };
            let opts = opts_summary(a);
            let target = if opts.is_empty() { "–".to_string() } else { opts };
            let npu = a.engine.contains("RBLN") || a.engine.contains("Furiosa");
            let tps_s = tps.map(|t| format!("{:.0}", t)).unwrap_or_else(|| "–".into());
            Row::new(vec![
                Cell::from(format!("{} {}", phase.glyph(), phase.label()))
                    .style(Style::default().fg(gc)),
                Cell::from(truncw(&a.model, 22)).style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(engine_short(&a.engine)).style(Style::default().fg(C_DIM())),
                Cell::from(truncw(&target, 20))
                    .style(Style::default().fg(if npu { C_WARN() } else { Color::Gray })),
                Cell::from(format!("{}/{}", ready, desired)).style(Style::default().fg(gc)),
                Cell::from(truncw(if a.node.is_empty() { "?" } else { &a.node }, 14))
                    .style(Style::default().fg(C_DIM())),
                Cell::from(tps_s).style(Style::default().fg(C_OK())),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(11), // STATUS
        Constraint::Min(16),    // MODEL
        Constraint::Length(12), // ENGINE
        Constraint::Length(20), // TARGET
        Constraint::Length(6),  // REP
        Constraint::Length(14), // NODE
        Constraint::Length(6),  // t/s
    ];
    let header = ["STATUS", "MODEL", "ENGINE", "TARGET", "REP", "NODE", "t/s"];
    let title = format!(
        "Serving · running deployments · x stop · s scale · r restart · o sort · ⏎ actions{}",
        count_suffix(app.selected, order.len())
    );
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(hrow_sorted(&header, app.sort_header_label(), app.sort_arrow()))
            .column_spacing(1)
            .row_highlight_style(hl_style())
            .highlight_symbol("▎")
            .block(block_active(&title)),
        area,
        &mut st,
    );
    list_scrollbar(f, area, order.len(), app.selected, 1);
}
