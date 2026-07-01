//! ratatui 렌더링 — 헤더/탭/본문(뷰별, 정렬·상세 포함)/footer.
//! 모든 문자열은 표시 폭(unicode-width) 기준으로 절단해 CJK/와이드 글자 깨짐을 방지.
//! 선택 하이라이트는 REVERSED 대신 은은한 배경색(htop/all-smi 스타일).

use crate::app::{App, View};
use crate::collect::{AccelKind, Snapshot};
use ratatui::prelude::*;
use ratatui::symbols::Marker;
use ratatui::widgets::{
    Axis, Block, BorderType, Borders, Cell, Chart, Clear, Dataset, GraphType, Paragraph, Row,
    Sparkline, Table, TableState, Wrap,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ── 팔레트 (테마별) ─────────────────────────────────────
// 색=심각도/정체성. 테마: 0 default · 1 고대비 · 2 색맹친화(파랑/주황 계열)
fn th() -> usize {
    crate::app::theme()
}
#[allow(non_snake_case)]
fn C_OK() -> Color {
    match th() { 1 => Color::LightGreen, 2 => Color::Rgb(0, 114, 178), _ => Color::Green }
}
#[allow(non_snake_case)]
fn C_WARN() -> Color {
    match th() { 1 => Color::LightYellow, 2 => Color::Rgb(230, 159, 0), _ => Color::Yellow }
}
#[allow(non_snake_case)]
fn C_BAD() -> Color {
    match th() { 1 => Color::LightRed, 2 => Color::Rgb(213, 94, 0), _ => Color::Red }
}
#[allow(non_snake_case)]
fn C_DIM() -> Color {
    match th() { 1 => Color::Gray, _ => Color::DarkGray }
}
#[allow(non_snake_case)]
fn C_TRACK() -> Color {
    Color::Indexed(236)
}
#[allow(non_snake_case)]
fn C_HEAD() -> Color {
    match th() { 1 => Color::White, _ => Color::Indexed(244) }
}
#[allow(non_snake_case)]
fn C_ACC() -> Color {
    match th() { 1 => Color::LightCyan, 2 => Color::Rgb(86, 180, 233), _ => Color::Cyan }
}
#[allow(non_snake_case)]
fn C_HL() -> Color {
    Color::Indexed(238)
}

const FRAC: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    title_bar(f, chunks[0], &app.snap, app.tick);
    summary_bar(f, chunks[1], &app.snap);
    tabs(f, chunks[2], app);

    let body = chunks[3];
    if app.detail && matches!(app.view, View::Accel | View::Models | View::Overview | View::Pods | View::Nodes) {
        detail_panel(f, body, app);
    } else {
        match app.view {
            View::Overview => view_overview(f, body, app),
            View::Accel => view_accel(f, body, app),
            View::Models => view_models(f, body, app),
            View::Epp => view_epp(f, body, app),
            View::Routing => view_routing(f, body, app),
            View::Pods => view_pods(f, body, app),
            View::Perf => view_perf(f, body, app),
            View::Launch => view_launch(f, body, app),
            View::Events => view_events(f, body, app),
            View::Nodes => view_nodes(f, body, app),
        }
    }
    footer(f, chunks[4], app);
    if app.help {
        help_overlay(f);
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w.min(area.width), height: h.min(area.height) }
}

fn help_overlay(f: &mut Frame) {
    let area = centered(f.area(), 62, 20);
    f.render_widget(Clear, area);
    let g = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(format!("  {:<10}", k), Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)),
            Span::styled(d.to_string(), Style::default().fg(Color::White)),
        ])
    };
    let sec = |t: &str| Line::from(Span::styled(format!(" {}", t), Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD)));
    let lines = vec![
        sec("navigation"),
        g("0-8 / Tab", "view (Overview/Accel/Models/EPP/Topo/Pods/Perf/Launch/Events)"),
        g("up/dn j k", "select row (mouse scroll works too)"),
        g("Enter", "detail (drill-down)"),
        g("o", "cycle sort"),
        g("/", "filter (substring)"),
        g("s", "scale selected model up/down"),
        g("t", "cycle theme (default/high-contrast/colorblind)"),
        g("g", "open Grafana dashboard"),
        g("? / Esc", "help / close-back   q quit"),
        Line::from(""),
        sec("color / glyph"),
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(C_OK())), Span::styled("up  ", Style::default().fg(C_DIM())),
            Span::styled("○ ", Style::default().fg(C_DIM())), Span::styled("idle  ", Style::default().fg(C_DIM())),
            Span::styled("◐ ", Style::default().fg(C_WARN())), Span::styled("pending  ", Style::default().fg(C_DIM())),
            Span::styled("⚠ ", Style::default().fg(C_WARN())), Span::styled("throttle  ", Style::default().fg(C_DIM())),
            Span::styled("✗ ", Style::default().fg(C_BAD())), Span::styled("down", Style::default().fg(C_DIM())),
        ]),
        Line::from(vec![
            Span::styled("  util/mem/temp: ", Style::default().fg(C_DIM())),
            Span::styled("low", Style::default().fg(C_OK())), Span::raw(" "),
            Span::styled("mid", Style::default().fg(C_WARN())), Span::raw(" "),
            Span::styled("high", Style::default().fg(C_BAD())),
        ]),
        Line::from(vec![
            Span::styled("  vendor: ", Style::default().fg(C_DIM())),
            Span::styled("GPU ", Style::default().fg(Color::Green)),
            Span::styled("RBLN ", Style::default().fg(Color::Magenta)),
            Span::styled("RNGD", Style::default().fg(Color::Cyan)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(C_ACC()))
                .title(Span::styled(" lmd-top · help (press any key to close) ", Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD))),
        ),
        area,
    );
}

// ── 헤더 ───────────────────────────────────────────────
fn title_bar(f: &mut Frame, area: Rect, s: &Snapshot, tick: u64) {
    let spin = SPINNER[(tick as usize / 2) % SPINNER.len()];
    let gw = if s.gw_addr.is_empty() {
        Span::styled("⌂ gw —", Style::default().fg(C_DIM()))
    } else if s.gw_ok {
        Span::styled(format!("⌂ gw {} ●", s.gw_addr), Style::default().fg(C_OK()))
    } else {
        Span::styled(format!("⌂ gw {} ○", s.gw_addr), Style::default().fg(C_WARN()))
    };
    let line = Line::from(vec![
        Span::styled(format!("{} ", spin), Style::default().fg(C_ACC())),
        Span::styled("lmd-top", Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  llm-d · {} nodes  ", s.nodes.len()), Style::default().fg(C_DIM())),
        gw,
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn summary_bar(f: &mut Frame, area: Rect, s: &Snapshot) {
    let (mut gpu, mut rbln, mut rngd, mut busy) = (0, 0, 0, 0);
    let (mut mu, mut mt) = (0.0, 0.0);
    for a in &s.accel {
        match a.kind {
            AccelKind::Gpu => gpu += 1,
            AccelKind::Rbln => rbln += 1,
            AccelKind::Rngd => rngd += 1,
        }
        if a.util > 5.0 {
            busy += 1;
        }
        mu += a.mem_used_gb;
        mt += a.mem_total_gb;
    }
    let serving = s.models.iter().filter(|m| m.ready > 0).count();
    let power: f64 = s.accel.iter().map(|a| a.power).sum();
    let mempct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
    let mut spans = vec![
        Span::styled(format!("GPU {} ", gpu), Style::default().fg(if gpu == 0 { C_DIM() } else { kind_color(AccelKind::Gpu) })),
        Span::styled(format!("RBLN {} ", rbln), Style::default().fg(kind_color(AccelKind::Rbln))),
        Span::styled(format!("RNGD {} ", rngd), Style::default().fg(kind_color(AccelKind::Rngd))),
        Span::styled(format!("· {} busy ", busy), Style::default().fg(C_DIM())),
        Span::raw("│ "),
        Span::styled(format!("vram {:.0}/{:.0}G ", mu, mt), Style::default().fg(mem_color(mempct))),
        Span::raw("│ "),
        Span::styled(
            format!("models {}/{} ", serving, s.models.len()),
            Style::default().fg(if serving == 0 { C_WARN() } else { C_OK() }),
        ),
        Span::styled(format!("│ ⚡{:.0}W ", power), Style::default().fg(C_DIM())),
    ];
    let warns = s.events.iter().filter(|e| e.typ == "Warning").count();
    if warns > 0 {
        spans.push(Span::styled(format!("│ ⚠{} warn", warns), Style::default().fg(C_WARN())));
    }
    let line = Line::from(spans);
    f.render_widget(Paragraph::new(line), area);
}

fn tabs(f: &mut Frame, area: Rect, app: &App) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, v) in View::ALL.iter().enumerate() {
        let sel = *v == app.view;
        let st = if sel {
            Style::default().fg(Color::Black).bg(C_ACC()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(C_DIM())
        };
        spans.push(Span::styled(format!(" {}:{} ", i, v.title()), st));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn footer(f: &mut Frame, area: Rect, app: &App) {
    // 필터 입력 모드
    if app.filtering {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" / ", Style::default().fg(Color::Black).bg(C_ACC()).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {}", app.filter), Style::default().fg(Color::White)),
                Span::styled("▏", Style::default().fg(C_ACC())),
                Span::styled("   Enter/Esc to apply", Style::default().fg(C_DIM())),
            ])),
            area,
        );
        return;
    }
    if let Some(t) = &app.toast {
        let msg = truncw(t, area.width.saturating_sub(1) as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(format!(" {} ", msg), Style::default().fg(Color::Black).bg(C_WARN())))),
            area,
        );
        return;
    }
    let mut spans: Vec<Span> = Vec::new();
    if !app.filter.is_empty() {
        spans.push(Span::styled(format!("[filter: {}] ", app.filter), Style::default().fg(Color::Black).bg(C_ACC())));
        spans.push(Span::raw(" "));
    }
    let sortable = app.sort_modes() > 1;
    let mut hint = String::from("↑↓ sel  ⏎ detail  / filter  ");
    if sortable {
        hint.push_str(&format!("o sort:{}  ", app.sort_label()));
    }
    hint.push_str("s scale  t theme  g grafana↗  ? help  q quit");
    spans.push(Span::styled(hint, Style::default().fg(C_DIM())));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── helpers ────────────────────────────────────────────
fn dwidth(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}
fn truncw(s: &str, max: usize) -> String {
    if dwidth(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut w = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// 프랙셔널 블록 바 (filled=colored, track=dim).
fn bar_line(pct: f64, width: usize, color: Color) -> Line<'static> {
    let p = pct.clamp(0.0, 100.0) / 100.0;
    let frac = p * width as f64;
    let full = (frac.floor() as usize).min(width);
    let mut filled = "█".repeat(full);
    let mut used = full;
    if used < width {
        let rem = ((frac - full as f64) * 8.0).round() as usize;
        if rem > 0 {
            filled.push(FRAC[rem - 1]);
            used += 1;
        }
    }
    let track = "░".repeat(width.saturating_sub(used));
    Line::from(vec![
        Span::styled(filled, Style::default().fg(color)),
        Span::styled(track, Style::default().fg(C_TRACK())),
    ])
}

fn util_color(p: f64) -> Color {
    if p > 85.0 {
        C_BAD()
    } else if p > 60.0 {
        C_WARN()
    } else if p > 10.0 {
        C_OK()
    } else {
        C_DIM()
    }
}
fn mem_color(p: f64) -> Color {
    if p > 90.0 {
        C_BAD()
    } else if p > 70.0 {
        C_WARN()
    } else if p > 1.0 {
        C_OK()
    } else {
        C_DIM()
    }
}
fn temp_color(t: f64) -> Color {
    if t > 75.0 {
        C_BAD()
    } else if t > 60.0 {
        C_WARN()
    } else if t > 0.0 {
        Color::Gray
    } else {
        C_DIM()
    }
}
fn kind_color(k: AccelKind) -> Color {
    match k {
        AccelKind::Gpu => Color::Green,
        AccelKind::Rbln => Color::Magenta,
        AccelKind::Rngd => Color::Cyan,
    }
}

fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(x) if !x.is_nan() => format!("{:.0}", x),
        _ => "–".into(),
    }
}
fn fmt_nan(v: f64, dec: usize) -> String {
    if v.is_nan() {
        "–".into()
    } else {
        format!("{:.*}", dec, v)
    }
}

fn block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_TRACK()))
        .title(Span::styled(format!(" {} ", title), Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)))
}

/// 애니메이션 마퀴 — 폭 초과 시 tick 에 따라 가로 스크롤(선택 행 강조용). 이름은 대개 ASCII.
fn marquee(s: &str, width: usize, tick: u64) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width {
        return s.to_string();
    }
    let mut ring = chars.clone();
    ring.extend("   ◂ ".chars()); // 구분자
    let period = ring.len();
    let off = ((tick / 3) as usize) % period; // 3틱마다 한 칸
    (0..width).map(|i| ring[(off + i) % period]).collect()
}

/// 상태 아이콘(폭1 BMP — 이모지 회피).
fn dot(up: bool) -> Span<'static> {
    if up {
        Span::styled("● ", Style::default().fg(C_OK()))
    } else {
        Span::styled("○ ", Style::default().fg(C_DIM()))
    }
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn hrow(cols: &[&str]) -> Row<'static> {
    Row::new(
        cols.iter()
            .map(|c| Cell::from(Span::styled(c.to_string(), Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD))))
            .collect::<Vec<_>>(),
    )
}

fn hl_style() -> Style {
    Style::default().bg(C_HL()).add_modifier(Modifier::BOLD)
}

fn cellw(text: String, w: usize) -> Cell<'static> {
    Cell::from(truncw(&text, w))
}

// ── Accel ──────────────────────────────────────────────
fn view_accel(f: &mut Frame, area: Rect, app: &App) {
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
            let mempct = if a.mem_total_gb > 0.0 { a.mem_used_gb / a.mem_total_gb * 100.0 } else { 0.0 };
            let mut util = bar_line(a.util, 9, util_color(a.util)).spans;
            util.push(Span::styled(format!(" {:>3.0}%", a.util), Style::default().fg(util_color(a.util))));
            let mut mem = bar_line(mempct, 7, mem_color(mempct)).spans;
            mem.push(Span::styled(
                format!(" {:.0}/{:.0}G", a.mem_used_gb, a.mem_total_gb),
                Style::default().fg(C_DIM()),
            ));
            let (hg, hc) = if !a.alive {
                ("✗", C_BAD())
            } else if a.throttle > 0.0 {
                ("⚠", C_WARN())
            } else {
                ("●", C_OK())
            };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(hg, Style::default().fg(hc)),                                  // 상태=글리프
                    Span::raw(" "),
                    Span::styled(a.kind.label(), Style::default().fg(kind_color(a.kind)).add_modifier(Modifier::BOLD)), // 정체성=vendor색
                ])),
                cellw(a.id.clone(), 6),
                cellw(a.node.clone(), 16),
                Cell::from(Line::from(util)),
                Cell::from(Line::from(mem)),
                Cell::from(Span::styled(format!("{:.0}°C", a.temp), Style::default().fg(temp_color(a.temp)))),
                cellw(format!("{:.0}W", a.power), 5),
                Cell::from(Span::styled(model_cell, Style::default().fg(C_DIM()))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(16),
        Constraint::Length(15),
        Constraint::Length(17),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(hrow(&["KIND", "ID", "NODE", "UTIL", "MEM", "TEMP", "PWR", "MODEL/POD"]))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block("Accelerators · UTIL=compute% MEM=VRAM · ⏎ full timeline"));

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(table, split[0], &mut st);

    if let Some(a) = app.selected_accel() {
        let key = format!("acc:{}:{}:{}:util", a.kind.label(), a.node, a.id);
        let data = app.hist_for(&key);
        let spark = Sparkline::default()
            .block(block(&format!("compute util % · {} {} (⏎ for full timeline)", a.kind.label(), a.id)))
            .data(&data)
            .max(100)
            .style(Style::default().fg(util_color(a.util)));
        f.render_widget(spark, split[1]);
    }
}

// ── Models ─────────────────────────────────────────────
fn view_models(f: &mut Frame, area: Rect, app: &App) {
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(models_table(app, "Models · ⏎ detail"), area, &mut st);
}

const MODEL_COLS: [&str; 10] = ["name", "engine", "accel", "ready", "run", "wait", "kv", "tps", "path", "status"];

fn model_col_header(k: &str) -> &'static str {
    match k {
        "name" => "MODEL", "engine" => "ENGINE", "accel" => "ACCEL", "ready" => "READY", "run" => "RUN",
        "wait" => "WAIT", "kv" => "KV", "tps" => "t/s", "path" => "PATH", "status" => "STATUS", _ => "?",
    }
}
fn model_col_width(k: &str) -> Constraint {
    match k {
        "name" => Constraint::Min(14),
        "accel" => Constraint::Length(13),
        "engine" => Constraint::Length(12),
        "path" => Constraint::Length(11),
        "status" => Constraint::Length(11),
        "ready" => Constraint::Length(6),
        "kv" | "tps" => Constraint::Length(5),
        _ => Constraint::Length(4),
    }
}
fn model_cell(k: &str, m: &crate::collect::ModelRow, selected: bool, tick: u64) -> Cell<'static> {
    match k {
        "name" => Cell::from(if selected { marquee(&m.name, 20, tick) } else { truncw(&m.name, 20) }),
        "engine" => Cell::from(Span::styled(truncw(&m.engine, 12), Style::default().fg(C_ACC()))),
        "accel" => Cell::from(Span::styled(truncw(&m.accel, 13), Style::default().fg(C_DIM()))),
        "ready" => cellw(format!("{}/{}", m.ready, m.desired), 6),
        "run" => cellw(fmt_opt(m.running), 4),
        "wait" => cellw(fmt_opt(m.waiting), 4),
        "kv" => cellw(m.kv.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or("–".into()), 5),
        "tps" => cellw(m.tps.map(|x| format!("{:.0}", x)).unwrap_or("–".into()), 5),
        "path" => cellw(if m.route.is_empty() { "–".into() } else { m.route.clone() }, 11),
        "status" => {
            let color = if m.status.contains("Running") { C_OK() } else if m.status.contains("Pending") { C_WARN() } else { C_DIM() };
            Cell::from(Span::styled(m.status.clone(), Style::default().fg(color)))
        }
        _ => Cell::from(""),
    }
}

fn models_table<'a>(app: &'a App, title: &'a str) -> Table<'static> {
    let _ = title;
    let cols = app.columns("models", &MODEL_COLS); // 설정파일 columns.models 순서/표시
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let m = &app.snap.models[i];
            Row::new(cols.iter().map(|c| model_cell(c, m, pos == app.selected, app.tick)).collect::<Vec<_>>())
        })
        .collect();
    let widths: Vec<Constraint> = cols.iter().map(|c| model_col_width(c)).collect();
    let header: Vec<&str> = cols.iter().map(|c| model_col_header(c)).collect();
    Table::new(rows, widths)
        .header(hrow(&header))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block("Models · ⏎ detail"))
}

// ── Pods ───────────────────────────────────────────────
fn view_pods(f: &mut Frame, area: Rect, app: &App) {
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
            let name = if pos == app.selected { marquee(&p.name, 40, app.tick) } else { truncw(&p.name, 40) };
            Row::new(vec![
                Cell::from(name),
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
    let table = Table::new(rows, widths)
        .header(hrow(&["POD", "READY", "PHASE", "NODE", "RESTARTS"]))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block("Pods (llm-serving) · ⏎ detail"));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(table, area, &mut st);
}

// ── EPP ────────────────────────────────────────────────
fn scorer_desc(name: &str) -> &'static str {
    match name {
        "queue-scorer" => "Prefers endpoints with shorter waiting queues — balances load across pods.",
        "kv-cache-utilization-scorer" => "Prefers endpoints with lower KV-cache usage — avoids memory pressure and preemption.",
        "prefix-cache-scorer" => "Prefers endpoints that already hold the prompt's prefix in KV cache — reuse means faster TTFT.",
        "no-hit-lru-scorer" => "On a prefix-cache miss, biases toward the LRU eviction target so future prefixes hit more often.",
        "load-aware-scorer" => "Factors each pod's utilization and capacity into the score.",
        "active-request-scorer" => "Prefers endpoints with fewer in-flight requests.",
        "session-affinity-scorer" => "Keeps a session's requests pinned to the same endpoint.",
        "latency-prediction-scorer" => "Uses a predicted-latency model to pick the fastest endpoint.",
        _ => "EPP scoring plugin. Weighted scores are summed; the max-score endpoint is picked.",
    }
}

fn view_epp(f: &mut Frame, area: Rect, app: &App) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(6)])
        .split(area);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(split[0]);

    match &app.snap.epp {
        Some(cfg) => {
            let order = app.order();
            let maxw = cfg.scorers.iter().map(|(_, w)| *w).fold(1.0, f64::max);
            let srows: Vec<Row> = order
                .iter()
                .map(|&i| {
                    let (name, w) = &cfg.scorers[i];
                    let bw = ((w / maxw) * 10.0).round() as usize;
                    Row::new(vec![
                        cellw(name.clone(), 28),
                        Cell::from(Span::styled(format!("{:.0}", w), Style::default().fg(C_WARN()))),
                        Cell::from(Span::styled("█".repeat(bw), Style::default().fg(C_ACC()))),
                    ])
                })
                .collect();
            let t = Table::new(srows, [Constraint::Min(16), Constraint::Length(3), Constraint::Length(11)])
                .header(hrow(&["SCORER", "WT", "WEIGHT"]))
                .column_spacing(1)
                .row_highlight_style(hl_style())
                .highlight_symbol("▎")
                .block(block("EPP scorers · select for description"));
            let mut st = TableState::default();
            st.select(Some(app.selected));
            f.render_stateful_widget(t, top[0], &mut st);

            let sel = order.get(app.selected).and_then(|&i| cfg.scorers.get(i));
            let mut dl: Vec<Line> = vec![
                Line::from(vec![
                    Span::styled("profile: ", Style::default().fg(C_DIM())),
                    Span::styled(cfg.profile.clone(), Style::default().fg(Color::White)),
                    Span::styled("   picker: ", Style::default().fg(C_DIM())),
                    Span::styled(cfg.picker.clone(), Style::default().fg(Color::White)),
                ]),
                Line::from(""),
            ];
            if let Some((name, w)) = sel {
                dl.push(Line::from(vec![
                    Span::styled(name.clone(), Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  (weight {:.0})", w), Style::default().fg(C_DIM())),
                ]));
                dl.push(Line::from(""));
                dl.push(Line::from(Span::styled(scorer_desc(name), Style::default().fg(Color::White))));
            }
            f.render_widget(Paragraph::new(dl).wrap(Wrap { trim: true }).block(block("what this scorer does")), top[1]);
        }
        None => f.render_widget(
            Paragraph::new(Line::from(Span::styled("EPP ConfigMap not found (llmd-router-epp)", Style::default().fg(C_DIM())))).block(block("EPP scorers")),
            top[0],
        ),
    }

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(split[1]);

    let rows: Vec<Row> = app
        .snap
        .pools
        .iter()
        .map(|p| {
            Row::new(vec![
                cellw(p.name.clone(), 14),
                cellw(format!("{}/{}", p.ep_ready, p.ep_total), 7),
                cellw(fmt_nan(p.queue, 1), 8),
                Cell::from(Span::styled(
                    fmt_nan(p.sat, 2),
                    Style::default().fg(if p.sat > 0.8 { C_BAD() } else if p.sat > 0.5 { C_WARN() } else { C_DIM() }),
                )),
            ])
        })
        .collect();
    let t = Table::new(rows, [Constraint::Min(12), Constraint::Length(7), Constraint::Length(8), Constraint::Length(6)])
        .header(hrow(&["POOL", "EP r/t", "QUEUE", "SAT"]))
        .block(block("InferencePool"));
    f.render_widget(t, bottom[0]);

    // request distribution
    let mut dl: Vec<Line> = vec![Line::from(vec![
        Span::styled("EPP in path: ", Style::default().fg(C_DIM())),
        Span::styled(
            if app.snap.epp_in_path { "yes" } else { "no (bypassed)" },
            Style::default().fg(if app.snap.epp_in_path { C_OK() } else { C_WARN() }),
        ),
        Span::styled(
            format!("   prefix idx: {}", if app.snap.prefix_idx.is_nan() { "-".into() } else { format!("{:.0}", app.snap.prefix_idx) }),
            Style::default().fg(C_DIM()),
        ),
    ])];
    let total: f64 = app.snap.decisions.iter().map(|(_, c)| c).sum();
    if app.snap.decisions.is_empty() || total <= 0.0 {
        dl.push(Line::from(Span::styled(
            if app.snap.epp_in_path { "no distribution data (waiting for traffic)" } else { "no distribution data (EPP bypassed - see Topo)" },
            Style::default().fg(C_DIM()),
        )));
    } else {
        for (pod, cnt) in app.snap.decisions.iter().take(5) {
            let share = cnt / total * 100.0;
            let mut sp = vec![Span::styled(format!("{:<20} ", truncw(pod, 20)), Style::default().fg(Color::White))];
            sp.extend(bar_line(share, 8, C_ACC()).spans);
            sp.push(Span::styled(format!(" {:>3.0}%", share), Style::default().fg(C_DIM())));
            dl.push(Line::from(sp));
        }
    }
    f.render_widget(Paragraph::new(dl).block(block("request distribution (routing decisions)")), bottom[1]);
}

// ── Topology (구성/라우팅/분배 한눈에) ──────────────────
fn view_routing(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let mut lines: Vec<Line> = Vec::new();

    // Gateway → HTTPRoute → backend (모델 상태/가속기/노드 주석)
    let gw = if s.gw_addr.is_empty() {
        "llm-d-gateway (—)".to_string()
    } else {
        format!("llm-d-gateway  {}  {}", s.gw_addr, if s.gw_ok { "●Programmed" } else { "○" })
    };
    lines.push(Line::from(Span::styled(gw, Style::default().fg(C_OK()).add_modifier(Modifier::BOLD))));
    for (i, r) in s.routes.iter().enumerate() {
        let branch = if i + 1 == s.routes.len() { "└─" } else { "├─" };
        let m = s.models.iter().find(|m| m.name == r.backend);
        let up = m.map(|m| m.ready > 0).unwrap_or(false);
        let annot = match m {
            Some(m) => format!("{}/{}  {}", m.ready, m.desired, m.accel),
            None => "?".into(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", branch), Style::default().fg(C_DIM())),
            dot(up),
            Span::styled(format!("{:<10}", truncw(&r.path, 10)), Style::default().fg(Color::White)),
            Span::styled("→ ", Style::default().fg(C_DIM())),
            Span::styled(format!("{}:", r.kind), Style::default().fg(C_DIM())),
            Span::styled(format!("{:<24}", truncw(&r.backend, 24)), Style::default().fg(if up { C_OK() } else { C_DIM() })),
            Span::styled(annot, Style::default().fg(C_DIM())),
        ]));
    }
    // EPP 경유 여부 진단
    if !s.routes.is_empty() {
        if s.epp_in_path {
            lines.push(Line::from(Span::styled("  ✓ routes go through InferencePool (EPP)", Style::default().fg(C_OK()))));
        } else {
            lines.push(Line::from(Span::styled(
                "  ⚠ HTTPRoute points to Service directly → InferencePool/EPP bypassed (EPP metrics empty)",
                Style::default().fg(C_WARN()),
            )));
        }
    }

    let top = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(10)])
        .split(area);
    f.render_widget(Paragraph::new(lines).block(block("Topology · Gateway → HTTPRoute → backend")), top[0]);

    // InferencePool + EPP + SLO
    let mut pl: Vec<Line> = Vec::new();
    if s.pools.is_empty() {
        pl.push(Line::from(Span::styled("(no InferencePool)", Style::default().fg(C_DIM()))));
    }
    for p in &s.pools {
        pl.push(Line::from(vec![
            Span::styled(format!("{:<18}", truncw(&p.name, 18)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("ep {}/{} ", p.ep_ready, p.ep_total),
                Style::default().fg(if p.ep_total == 0 { C_WARN() } else { C_OK() }),
            ),
            Span::styled(format!("EPP:{} ", if p.epp.is_empty() { "–" } else { &p.epp }), Style::default().fg(C_ACC())),
            Span::styled(format!("sel={}", if p.selector.is_empty() { "–" } else { &p.selector }), Style::default().fg(C_DIM())),
        ]));
    }
    if !s.objectives.is_empty() {
        let so: Vec<String> = s.objectives.iter().map(|o| format!("{}(p{}→{})", o.name, o.priority, o.pool)).collect();
        pl.push(Line::from(vec![
            Span::styled("SLO  ", Style::default().fg(C_DIM())),
            Span::styled(so.join("  "), Style::default().fg(Color::White)),
        ]));
    }
    for a in &s.autoscalers {
        pl.push(Line::from(vec![
            Span::styled("autoscale ", Style::default().fg(C_DIM())),
            Span::styled(truncw(&a.target, 26), Style::default().fg(Color::White)),
            Span::styled(format!("  {}↔{} rep={} ", a.min, a.max, a.replicas), Style::default().fg(C_DIM())),
            Span::styled(if a.active { "active" } else { "idle" }, Style::default().fg(if a.active { C_OK() } else { C_DIM() })),
            Span::styled(if a.ready { " ✓" } else { " ⚠notready" }, Style::default().fg(if a.ready { C_OK() } else { C_WARN() })),
            Span::styled(format!(" [{}]", a.triggers), Style::default().fg(C_DIM())),
        ]));
    }
    f.render_widget(Paragraph::new(pl).block(block("InferencePool / EPP / SLO / Autoscale")), top[1]);
}

// ── Overview ───────────────────────────────────────────
fn view_overview(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Length(5), Constraint::Min(4), Constraint::Length(3)])
        .split(area);

    // 가속기: (종류,노드)별 집계 — 한눈에 + 절대 메모리(GB) + health 아이콘
    let mut groups: Vec<(AccelKind, String, usize, f64, f64, f64, bool, bool)> = Vec::new();
    for a in &s.accel {
        if let Some(g) = groups.iter_mut().find(|g| g.0 == a.kind && g.1 == a.node) {
            g.2 += 1; g.3 += a.util; g.4 += a.mem_used_gb; g.5 += a.mem_total_gb;
            g.6 = g.6 && a.alive; g.7 = g.7 || a.throttle > 0.0;
        } else {
            groups.push((a.kind, a.node.clone(), 1, a.util, a.mem_used_gb, a.mem_total_gb, a.alive, a.throttle > 0.0));
        }
    }
    let mut al: Vec<Line> = Vec::new();
    for (kind, node, cnt, us, mu, mt, alive, thr) in &groups {
        let util = us / (*cnt as f64);
        let mempct = if *mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
        let (hi, hc) = if !*alive { ("✗", C_BAD()) } else if *thr { ("⚠", C_WARN()) } else { ("●", C_OK()) };
        let mut sp = vec![
            Span::styled(format!("{} ", hi), Style::default().fg(hc)),
            Span::styled(format!("{:<4}×{} ", kind.label(), cnt), Style::default().fg(kind_color(*kind)).add_modifier(Modifier::BOLD)),
            Span::styled(format!("@{:<16} ", truncw(node, 16)), Style::default().fg(C_DIM())),
        ];
        sp.extend(bar_line(util, 10, util_color(util)).spans);
        sp.push(Span::styled(format!(" {:>3.0}%  ", util), Style::default().fg(util_color(util))));
        sp.push(Span::styled(format!("mem {:.0}/{:.0} GB", mu, mt), Style::default().fg(mem_color(mempct)))); // 절대값
        al.push(Line::from(sp));
    }
    if al.is_empty() {
        al.push(Line::from(Span::styled("  (no accelerator metrics)", Style::default().fg(C_DIM()))));
    }
    f.render_widget(Paragraph::new(al).block(block("Accelerators (by kind / node)")), rows[0]);

    // Inference: EPP 경로 + 풀 endpoints + scorers + autoscale
    let mut pl: Vec<Line> = Vec::new();
    pl.push(Line::from(vec![
        Span::styled("EPP path ", Style::default().fg(C_DIM())),
        Span::styled(
            if s.epp_in_path { "via InferencePool ●" } else { "bypassed (HTTPRoute→Service) ⚠" },
            Style::default().fg(if s.epp_in_path { C_OK() } else { C_WARN() }),
        ),
    ]));
    for p in s.pools.iter().take(2) {
        pl.push(Line::from(vec![
            dot(p.ep_ready > 0),
            Span::styled(format!("{:<16} ", truncw(&p.name, 16)), Style::default().fg(Color::White)),
            Span::styled(format!("endpoints {}/{}  sat {}", p.ep_ready, p.ep_total, fmt_nan(p.sat, 2)), Style::default().fg(C_DIM())),
        ]));
    }
    if let Some(cfg) = &s.epp {
        let names: Vec<String> = cfg.scorers.iter().map(|(n, w)| format!("{}·{:.0}", n.replace("-scorer", ""), w)).collect();
        pl.push(Line::from(Span::styled(format!("scorers: {}", names.join("  ")), Style::default().fg(C_DIM()))));
    }
    f.render_widget(Paragraph::new(pl).block(block("Inference (EPP / InferencePool)")), rows[1]);

    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(models_table(app, "Models"), rows[2], &mut st);

    let (txt, col) = diagnose(s);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(truncw(&txt, rows[3].width.saturating_sub(2) as usize), Style::default().fg(col))))
            .block(block("Diagnosis")),
        rows[3],
    );
}

// ── Detail (drill-down) ────────────────────────────────
fn detail_panel(f: &mut Frame, area: Rect, app: &App) {
    // Accelerator: info + util/mem/temp timeline
    if let Some(a) = app.selected_accel() {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(9), Constraint::Min(3)]).split(area);
        let mempct = if a.mem_total_gb > 0.0 { a.mem_used_gb / a.mem_total_gb * 100.0 } else { 0.0 };
        let health = if !a.alive {
            ("✗ not alive".to_string(), C_BAD())
        } else if a.throttle > 0.0 {
            (format!("⚠ throttling {:.0}", a.throttle), C_WARN())
        } else {
            ("● healthy".to_string(), C_OK())
        };
        let lines = vec![
            kv("kind", &format!("{} (accelerator)", a.kind.label()), kind_color(a.kind)),
            kv("id / node", &format!("{} / {}", a.id, a.node), Color::White),
            kv("compute util", &format!("{:.0} %", a.util), util_color(a.util)),
            kv("memory (VRAM)", &format!("{:.1} / {:.1} GB ({:.0}%)", a.mem_used_gb, a.mem_total_gb, mempct), mem_color(mempct)),
            kv("temp", &format!("{:.0} °C", a.temp), temp_color(a.temp)),
            kv("power", &format!("{:.0} W", a.power), Color::White),
            kv("health", &health.0, health.1),
            kv("model/pod", if a.busy_model.is_empty() { "(idle)" } else { a.busy_model.as_str() }, C_ACC()),
        ];
        f.render_widget(Paragraph::new(lines).block(block("Accelerator detail · esc to close")), rows[0]);
        let k = format!("acc:{}:{}:{}", a.kind.label(), a.node, a.id);
        let sp = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Ratio(1, 3); 3]).split(rows[1]);
        line_chart(f, sp[0], app, &format!("{}:util", k), "compute util", "%", Some(100.0), util_color(a.util));
        line_chart(f, sp[1], app, &format!("{}:mem", k), "memory", "%", Some(100.0), C_ACC());
        line_chart(f, sp[2], app, &format!("{}:temp", k), "temp", "C", None, temp_color(a.temp));
        return;
    }
    // Node: info + cpu/mem/load timeline
    if let Some(n) = app.selected_node() {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(8), Constraint::Min(3)]).split(area);
        let (hg, hc) = if n.cordoned {
            ("⊘ cordoned", C_WARN())
        } else if !n.ready {
            ("✗ not ready", C_BAD())
        } else if n.pressure {
            ("⚠ pressure", C_WARN())
        } else {
            ("● ready", C_OK())
        };
        let lines = vec![
            kv("node", &n.name, Color::White),
            kv("status", hg, hc),
            kv("kubelet", &n.version, Color::White),
            kv("host CPU", &if n.cpu_pct.is_nan() { "-".into() } else { format!("{:.0} %", n.cpu_pct) }, Color::White),
            kv("host memory", &if n.mem_total_gb <= 0.0 { "-".into() } else { format!("{:.0} / {:.0} GB", n.mem_used_gb, n.mem_total_gb) }, Color::White),
            kv("load1", &if n.load1.is_nan() { "-".into() } else { format!("{:.2}", n.load1) }, Color::White),
        ];
        f.render_widget(Paragraph::new(lines).block(block("Node detail · esc to close")), rows[0]);
        let k = format!("nod:{}", n.name);
        let sp = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Ratio(1, 3); 3]).split(rows[1]);
        line_chart(f, sp[0], app, &format!("{}:cpu", k), "host cpu", "%", Some(100.0), C_OK());
        line_chart(f, sp[1], app, &format!("{}:mem", k), "host mem", "%", Some(100.0), C_ACC());
        line_chart(f, sp[2], app, &format!("{}:load", k), "load1x10", "", None, C_WARN());
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut title = "Detail";
    if let Some(m) = app.selected_model() {
        title = "Model detail · esc to close";
        lines.push(kv("model", &m.name, Color::White));
        lines.push(kv("status", &m.status, if m.ready > 0 { C_OK() } else { C_DIM() }));
        lines.push(kv("replicas", &format!("{}/{} (ready/desired)", m.ready, m.desired), Color::White));
        lines.push(kv("engine", &m.engine, C_ACC()));
        lines.push(kv("accelerator", &m.accel, C_ACC()));
        lines.push(kv("route", if m.route.is_empty() { "–" } else { m.route.as_str() }, Color::White));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("inference (vLLM)", Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD))));
        lines.push(kv("  running / waiting", &format!("{} / {}", fmt_opt(m.running), fmt_opt(m.waiting)), Color::White));
        lines.push(kv("  KV cache", &m.kv.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or("- (no vLLM metrics)".into()), Color::White));
        lines.push(kv("  tokens/s", &m.tps.map(|x| format!("{:.1}", x)).unwrap_or("–".into()), Color::White));
        lines.push(kv("  TTFT p95", &m.ttft.map(|x| format!("{:.0} ms", x * 1000.0)).unwrap_or("–".into()), Color::White));
        lines.push(Line::from(""));
        let pods: Vec<&str> = app.snap.pods.iter().filter(|p| p.name.starts_with(&m.name)).map(|p| p.name.as_str()).collect();
        lines.push(kv("pods", &if pods.is_empty() { "(none)".to_string() } else { pods.join(", ") }, C_DIM()));
        lines.push(Line::from(Span::styled("  s = scale up/down", Style::default().fg(C_DIM()))));
    } else if let Some(p) = app.selected_pod() {
        title = "Pod detail · esc to close";
        lines.push(kv("pod", &p.name, Color::White));
        lines.push(kv("phase", &p.phase, if p.phase == "Running" { C_OK() } else { C_DIM() }));
        lines.push(kv("ready", &p.ready, Color::White));
        lines.push(kv("node", &p.node, Color::White));
        lines.push(kv("restarts", &p.restarts.to_string(), if p.restarts > 0 { C_WARN() } else { Color::White }));
    } else {
        lines.push(Line::from(Span::styled("no item selected", Style::default().fg(C_DIM()))));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block(title)), area);
}

fn kv(k: &str, v: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<18} ", k), Style::default().fg(C_DIM())),
        Span::styled(v.to_string(), Style::default().fg(color)),
    ])
}

// ── Perf (EPP 정책용 성능/분배) ─────────────────────────
fn ms(v: f64) -> String {
    if v.is_nan() { "–".into() } else if v >= 1.0 { format!("{:.2}s", v) } else { format!("{:.0}ms", v * 1000.0) }
}
fn tok(v: f64) -> String {
    if v.is_nan() { "–".into() } else { format!("{:.0}", v) }
}
fn rate(v: f64) -> String {
    if v.is_nan() { "–".into() } else { format!("{:.2}", v) }
}
/// all-smi 식 라인 차트: X축=시간(최근 N초), Y축=값+단위, 제목에 현재/최대값.
/// ymax_opt=Some(100) 이면 0~100 고정(%), None 이면 데이터 최대×1.25 자동.
fn line_chart(f: &mut Frame, area: Rect, app: &App, key: &str, label: &str, unit: &str, ymax_opt: Option<f64>, color: Color) {
    let raw = app.hist_for(key);
    let cur = raw.last().copied().unwrap_or(0);
    let dmax = raw.iter().copied().max().unwrap_or(0);
    let data: Vec<(f64, f64)> = raw.iter().enumerate().map(|(i, v)| (i as f64, *v as f64)).collect();
    let ymax = ymax_opt.unwrap_or(((dmax as f64) * 1.25).max(1.0));
    let n = crate::app::HIST as f64;
    // 제목: 현재값 크게 + 최대
    let title = format!("{} ▏ now {}{} ▏ max {}{}", label, cur, unit, dmax, unit);
    let ds = vec![Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(color))
        .data(&data)];
    let chart = Chart::new(ds)
        .block(block(&title))
        .x_axis(
            Axis::default()
                .bounds([0.0, n])
                .labels([format!("-{}s", crate::app::HIST), "now".into()])
                .style(Style::default().fg(C_TRACK())),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, ymax])
                .labels([format!("0{}", unit), format!("{:.0}{}", ymax, unit)])
                .style(Style::default().fg(C_TRACK())),
        );
    f.render_widget(chart, area);
}


fn view_perf(f: &mut Frame, area: Rect, app: &App) {
    let p = &app.snap.perf;
    let any = [p.e2e_p95, p.ttft_p95, p.tps, p.req_rate].iter().any(|x| !x.is_nan());

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Length(3), Constraint::Min(5)])
        .split(area);

    // timeline: 라인 차트(util%/vram%) + tok/s 라인차트
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(rows[0]);
    let util_d: Vec<(f64, f64)> = app.hist_for("sys:util").iter().enumerate().map(|(i, v)| (i as f64, *v as f64)).collect();
    let vram_d: Vec<(f64, f64)> = app.hist_for("sys:vram").iter().enumerate().map(|(i, v)| (i as f64, *v as f64)).collect();
    let ds = vec![
        Dataset::default().name("util%").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(C_OK())).data(&util_d),
        Dataset::default().name("vram%").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(C_ACC())).data(&vram_d),
    ];
    let cur_u = app.hist_for("sys:util").last().copied().unwrap_or(0);
    let cur_v = app.hist_for("sys:vram").last().copied().unwrap_or(0);
    let chart = Chart::new(ds)
        .block(block(&format!("Timeline cluster avg ▏ util {}% ▏ vram {}%", cur_u, cur_v)))
        .x_axis(
            Axis::default()
                .bounds([0.0, crate::app::HIST as f64])
                .labels([format!("-{}s", crate::app::HIST), "now".into()])
                .style(Style::default().fg(C_TRACK())),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, 100.0])
                .labels(["0%", "50%", "100%"])
                .style(Style::default().fg(C_TRACK())),
        );
    f.render_widget(chart, top[0]);
    line_chart(f, top[1], app, "sys:tps", "tokens/s", "", None, C_OK());

    // throughput 숫자 + 데이터 없음 안내
    let tl = Line::from(vec![
        Span::styled("req/s ", Style::default().fg(C_DIM())),
        Span::styled(format!("{}  ", rate(p.req_rate)), Style::default().fg(C_OK())),
        Span::styled("err/s ", Style::default().fg(C_DIM())),
        Span::styled(format!("{}  ", rate(p.err_rate)), Style::default().fg(if p.err_rate > 0.0 { C_BAD() } else { C_DIM() })),
        Span::styled("tok/s ", Style::default().fg(C_DIM())),
        Span::styled(format!("{}  ", rate(p.tps)), Style::default().fg(C_OK())),
        Span::styled("prefix-hit ", Style::default().fg(C_DIM())),
        Span::styled(
            if p.prefix_hit.is_nan() { "–  ".into() } else { format!("{:.0}%  ", p.prefix_hit * 100.0) },
            Style::default().fg(C_ACC()),
        ),
        Span::styled(
            if any { "" } else { "· no data: needs EPP-path traffic + vLLM metrics" },
            Style::default().fg(C_WARN()),
        ),
    ]);
    f.render_widget(Paragraph::new(tl).block(block("Throughput")), rows[1]);

    // per-model 성능(모델=하드웨어 배치별) + per-pod 큐
    let bodyc = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(rows[2]);

    if app.snap.perf_rows.is_empty() {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("no per-model perf data", Style::default().fg(C_DIM()))),
                Line::from(Span::styled(
                    "shows per model once EPP-path traffic + vLLM metrics are present.",
                    Style::default().fg(C_DIM()),
                )),
            ])
            .block(block("Per-model perf (p95) · latency / tokens / throughput")),
            bodyc[0],
        );
    } else {
        let mrows: Vec<Row> = app
            .snap
            .perf_rows
            .iter()
            .map(|r| {
                Row::new(vec![
                    cellw(r.model.clone(), 16),
                    Cell::from(Span::styled(rate(r.req), Style::default().fg(C_OK()))),
                    Cell::from(Span::styled(rate(r.tps), Style::default().fg(C_OK()))),
                    cellw(ms(r.ttft_p95), 7),
                    cellw(ms(r.tpot_p95), 7),
                    Cell::from(Span::styled(ms(r.e2e_p95), Style::default().fg(C_WARN()))),
                    cellw(tok(r.in_tok_p95), 5),
                    cellw(tok(r.out_tok_p95), 5),
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
                Constraint::Length(5),
                Constraint::Length(5),
            ],
        )
        .header(hrow(&["MODEL", "req/s", "tok/s", "TTFT", "TPOT", "E2E", "inTk", "outTk"]))
        .column_spacing(1)
        .block(block("Per-model perf (p95) · latency in ms/s"));
        f.render_widget(mt, bodyc[0]);
    }

    // per-pod queue (요청 분배 — 절대 큐 깊이)
    let mut ql: Vec<Line> = Vec::new();
    let maxq = app.snap.pod_queues.iter().map(|(_, q)| *q).fold(1.0, f64::max);
    if app.snap.pod_queues.is_empty() {
        ql.push(Line::from(Span::styled("no per-pod queue data", Style::default().fg(C_DIM()))));
    } else {
        for (pod, q) in app.snap.pod_queues.iter().take(8) {
            let mut sp = vec![Span::styled(format!("{:<20} ", truncw(pod, 20)), Style::default().fg(Color::White))];
            sp.extend(bar_line(q / maxq * 100.0, 8, C_ACC()).spans);
            sp.push(Span::styled(format!(" {:.0}", q), Style::default().fg(C_DIM())));
            ql.push(Line::from(sp));
        }
    }
    f.render_widget(Paragraph::new(ql).block(block("request distribution (per-pod queue, absolute)")), bodyc[1]);
}

// ── Launch (모델 카탈로그 + 배치 솔버, 읽기전용) ────────
fn view_launch(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(4)])
        .split(area);

    // 가속기 재고
    let mut inv: Vec<Span> = vec![Span::styled("accelerators free/total: ", Style::default().fg(C_DIM()))];
    for (res, total, used) in &app.snap.inventory {
        let free = (total - used).max(0);
        let short = res.split('/').last().unwrap_or(res);
        inv.push(Span::styled(
            format!("{} {}/{}   ", short, free, total),
            Style::default().fg(if free > 0 { C_OK() } else { C_DIM() }),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(inv)).block(block("Launch · deployable models (read-only)")), rows[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(rows[1]);

    // 카탈로그 목록
    let order = app.order();
    let lrows: Vec<Row> = order
        .iter()
        .map(|&i| {
            let m = &app.catalog[i];
            Row::new(vec![
                cellw(m.id.clone(), 22),
                Cell::from(Span::styled(m.role.clone(), Style::default().fg(C_DIM()))),
            ])
        })
        .collect();
    let lt = Table::new(lrows, [Constraint::Min(14), Constraint::Length(10)])
        .header(hrow(&["MODEL", "ROLE"]))
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block("Catalog · up/dn to select"));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(lt, body[0], &mut st);

    // 선택 모델의 배치 후보 × 라이브 재고
    let mut pl: Vec<Line> = Vec::new();
    if let Some(m) = app.selected_cat() {
        pl.push(Line::from(Span::styled(
            if m.display.is_empty() { m.id.clone() } else { m.display.clone() },
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )));
        pl.push(Line::from(""));
        for p in &m.placements {
            let (state, free, need) = crate::catalog::solve(p, &app.snap.inventory);
            let col = match state {
                crate::catalog::Ready::Ready => C_OK(),
                crate::catalog::Ready::NeedsArtifact => C_WARN(),
                crate::catalog::Ready::NoCapacity => C_BAD(),
            };
            pl.push(Line::from(vec![
                Span::styled(format!("{:<16} ", state.glyph()), Style::default().fg(col)),
                Span::styled(format!("{} @{} {}×{}rep ", p.engine, p.accel, p.count, p.replicas), Style::default().fg(Color::White)),
                Span::styled(format!("need {} / free {}", need, free), Style::default().fg(C_DIM())),
            ]));
            pl.push(Line::from(Span::styled(format!("      {}", truncw(&p.uri, 44)), Style::default().fg(C_DIM()))));
        }
        pl.push(Line::from(""));
        pl.push(Line::from(Span::styled(
            "read-only — actual deploy (ModelService) is the next step",
            Style::default().fg(C_DIM()),
        )));
    } else {
        pl.push(Line::from(Span::styled("← select a model", Style::default().fg(C_DIM()))));
    }
    f.render_widget(Paragraph::new(pl).block(block("placements × live inventory")), body[1]);
}

// ── Nodes (node health / placement) ────────────────────
fn view_nodes(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .map(|&i| {
            let n = &app.snap.nodes[i];
            let (g, gc) = if n.cordoned {
                ("⊘ cordon", C_WARN())
            } else if !n.ready {
                ("✗ notready", C_BAD())
            } else if n.pressure {
                ("⚠ pressure", C_WARN())
            } else {
                ("● ready", C_OK())
            };
            // accelerators on this node
            let (mut gpu, mut rbln, mut rngd) = (0, 0, 0);
            for a in &app.snap.accel {
                if a.node == n.name {
                    match a.kind {
                        AccelKind::Gpu => gpu += 1,
                        AccelKind::Rbln => rbln += 1,
                        AccelKind::Rngd => rngd += 1,
                    }
                }
            }
            let accel = {
                let mut v = Vec::new();
                if gpu > 0 { v.push(format!("GPU{}", gpu)); }
                if rbln > 0 { v.push(format!("RBLN{}", rbln)); }
                if rngd > 0 { v.push(format!("RNGD{}", rngd)); }
                if v.is_empty() { "-".into() } else { v.join(" ") }
            };
            let mut cpu = bar_line(n.cpu_pct, 8, util_color(n.cpu_pct)).spans;
            cpu.push(Span::styled(
                if n.cpu_pct.is_nan() { " -".into() } else { format!(" {:.0}%", n.cpu_pct) },
                Style::default().fg(C_DIM()),
            ));
            Row::new(vec![
                cellw(n.name.clone(), 22),
                Cell::from(Span::styled(g, Style::default().fg(gc))),
                cellw(n.version.clone(), 11),
                Cell::from(Line::from(cpu)),
                cellw(if n.load1.is_nan() { "-".into() } else { format!("{:.1}", n.load1) }, 5),
                cellw(if n.mem_total_gb <= 0.0 { "-".into() } else { format!("{:.0}/{:.0}G", n.mem_used_gb, n.mem_total_gb) }, 10),
                Cell::from(Span::styled(accel, Style::default().fg(C_ACC()))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Min(16),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(13),
        Constraint::Length(5),
        Constraint::Length(10),
        Constraint::Min(12),
    ];
    let t = Table::new(rows, widths)
        .header(hrow(&["NODE", "STATUS", "VERSION", "CPU", "LOAD", "MEM", "ACCEL"]))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block("Nodes (health / placement)"));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(t, area, &mut st);
}

// ── Events (k8s + llm-d 이벤트) ─────────────────────────
fn view_events(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .map(|&i| {
            let e = &app.snap.events[i];
            let tc = if e.typ == "Warning" { C_WARN() } else { C_DIM() };
            Row::new(vec![
                Cell::from(Span::styled(e.typ.clone(), Style::default().fg(tc))),
                cellw(e.reason.clone(), 20),
                cellw(e.object.clone(), 28),
                cellw(if e.count > 1 { format!("x{}", e.count) } else { String::new() }, 5),
                Cell::from(Span::styled(e.message.clone(), Style::default().fg(Color::White))),
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
    let t = Table::new(rows, widths)
        .header(hrow(&["TYPE", "REASON", "OBJECT", "CNT", "MESSAGE"]))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block("Events (k8s + llm-d, newest first)"));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(t, area, &mut st);
}

// ── 진단 ───────────────────────────────────────────────
fn diagnose(s: &Snapshot) -> (String, Color) {
    let serving = s.models.iter().filter(|m| m.ready > 0).count();
    if s.accel.is_empty() && serving == 0 {
        return ("⚠ no accelerator metrics + no serving models — check Prometheus / model state".into(), C_BAD());
    }
    if serving == 0 {
        return ("⚠ 0 models serving — press 's' in Models to start one (no backend)".into(), C_WARN());
    }
    let warns = s.events.iter().filter(|e| e.typ == "Warning").count();
    if warns > 0 {
        let top = s.events.iter().find(|e| e.typ == "Warning").map(|e| e.reason.clone()).unwrap_or_default();
        return (
            format!("● {} model(s) serving · ⚠ {} warning event(s) (top: {}) — see Events", serving, warns, top),
            C_WARN(),
        );
    }
    let busy = s.accel.iter().filter(|a| a.util > 80.0).count();
    if busy > 0 {
        return (format!("● {} model(s) serving, {} accelerator(s) hot (>80%)", serving, busy), C_OK());
    }
    (format!("● {} model(s) serving, accelerators have headroom", serving), C_OK())
}
