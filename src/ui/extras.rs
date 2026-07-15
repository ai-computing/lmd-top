//! Events / Setup 뷰.
//! 공유 렌더 헬퍼는 상위 `ui` 모듈에서 가져온다.
use super::*;

// ── Setup(Doctor) — 새 환경 부트스트랩 전제조건 점검 + 가이드된 조치 ──────────
pub(super) fn view_setup(f: &mut Frame, area: Rect, app: &App) {
    use crate::app::CheckState;
    let checks = app.setup_checks();
    let (mut ok, mut missing) = (0usize, 0usize);
    let mut prev_cat = "";
    let rows: Vec<Row> = checks
        .iter()
        .map(|c| {
            match c.state {
                CheckState::Ok => ok += 1,
                CheckState::Missing => missing += 1,
                _ => {}
            }
            let sc = match c.state.sev() {
                0 => C_OK(),
                2 => C_BAD(),
                _ => C_WARN(),
            };
            // 카테고리는 그룹 첫 행에만 표기(줄 리듬 — 반복 라벨 제거).
            let cat = if c.category == prev_cat {
                String::new()
            } else {
                prev_cat = c.category;
                c.category.to_string()
            };
            let act = c.fix.label();
            let ac = match &c.fix {
                crate::app::SetupFix::None => C_DIM(),
                crate::app::SetupFix::Command(_) => Color::Gray,
                _ => C_ACC(),
            };
            Row::new(vec![
                Cell::from(c.state.glyph()).style(Style::default().fg(sc).add_modifier(Modifier::BOLD)),
                Cell::from(cat).style(Style::default().fg(C_DIM())),
                Cell::from(truncw(&c.name, 20)).style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(truncw(&c.detail, 60)).style(Style::default().fg(Color::Gray)),
                Cell::from(act).style(Style::default().fg(ac)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),  // STATE glyph
        Constraint::Length(12), // CATEGORY
        Constraint::Length(20), // CHECK
        Constraint::Min(24),    // DETAIL
        Constraint::Length(15), // ACTION
    ];
    let header = ["", "CATEGORY", "CHECK", "DETAIL", "ACTION"];
    let conn = if app.snap.setup.probed {
        format!("{} ok · {} missing", ok, missing)
    } else {
        "cluster unreachable".to_string()
    };
    let title = format!(
        "Setup · llm-d platform prerequisites · {} · ⏎ fix/show · read-only checks{}",
        conn,
        count_suffix(app.selected, checks.len())
    );
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(hrow(&header))
            .column_spacing(1)
            .row_highlight_style(hl_style())
            .highlight_symbol("▎")
            .block(block_active(&title)),
        area,
        &mut st,
    );
    list_scrollbar(f, area, checks.len(), app.selected, 1);
}

// ── Events (k8s + llm-d 이벤트) ─────────────────────────
pub(super) fn view_events(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .map(|&i| {
            let e = &app.snap.events[i];
            let tc = if e.typ == "Warning" {
                C_WARN()
            } else {
                C_DIM()
            };
            Row::new(vec![
                Cell::from(Span::styled(e.typ.clone(), Style::default().fg(tc))),
                cellw(e.reason.clone(), 20),
                cellw(e.object.clone(), 28),
                cellw(
                    if e.count > 1 {
                        format!("x{}", e.count)
                    } else {
                        String::new()
                    },
                    5,
                ),
                Cell::from(Span::styled(
                    e.message.clone(),
                    Style::default().fg(Color::White),
                )),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
        Constraint::Length(20),
        Constraint::Length(28),
        Constraint::Length(5),
        Constraint::Min(20),
    ];
    render_list_table(
        f,
        area,
        rows,
        &widths,
        &["TYPE", "REASON", "OBJECT", "CNT", "MESSAGE"],
        "Events (k8s + llm-d, newest first)",
        app.selected,
        order.len(),
        app.sort_header_label(),
        app.sort_arrow(),
    );
}
