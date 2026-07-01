//! ratatui 렌더링 — 헤더/탭/본문(뷰별, 정렬·상세 포함)/footer.
//! 모든 문자열은 표시 폭(unicode-width) 기준으로 절단해 CJK/와이드 글자 깨짐을 방지.
//! 선택 하이라이트는 REVERSED 대신 은은한 배경색(htop/all-smi 스타일).

use crate::app::{App, View};
use crate::collect::{AccelKind, Snapshot};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Sparkline, Table, TableState, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ── 팔레트 ─────────────────────────────────────────────
const C_OK: Color = Color::Green;
const C_WARN: Color = Color::Yellow;
const C_BAD: Color = Color::Red;
const C_DIM: Color = Color::DarkGray;
const C_TRACK: Color = Color::Indexed(236); // 바 빈 트랙
const C_HEAD: Color = Color::Indexed(244); // 헤더 글자
const C_ACC: Color = Color::Cyan;
const C_HL: Color = Color::Indexed(238); // 선택 배경

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
    if app.detail && matches!(app.view, View::Accel | View::Models | View::Overview | View::Pods) {
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
        }
    }
    footer(f, chunks[4], app);
}

// ── 헤더 ───────────────────────────────────────────────
fn title_bar(f: &mut Frame, area: Rect, s: &Snapshot, tick: u64) {
    let spin = SPINNER[(tick as usize / 2) % SPINNER.len()];
    let gw = if s.gw_addr.is_empty() {
        Span::styled("⌂ gw —", Style::default().fg(C_DIM))
    } else if s.gw_ok {
        Span::styled(format!("⌂ gw {} ●", s.gw_addr), Style::default().fg(C_OK))
    } else {
        Span::styled(format!("⌂ gw {} ○", s.gw_addr), Style::default().fg(C_WARN))
    };
    let line = Line::from(vec![
        Span::styled(format!("{} ", spin), Style::default().fg(C_ACC)),
        Span::styled("lmd-top", Style::default().fg(C_ACC).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  llm-d · {} nodes  ", s.nodes.len()), Style::default().fg(C_DIM)),
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
    let line = Line::from(vec![
        Span::styled(format!("GPU {} ", gpu), Style::default().fg(if gpu == 0 { C_DIM } else { kind_color(AccelKind::Gpu) })),
        Span::styled(format!("RBLN {} ", rbln), Style::default().fg(kind_color(AccelKind::Rbln))),
        Span::styled(format!("RNGD {} ", rngd), Style::default().fg(kind_color(AccelKind::Rngd))),
        Span::styled(format!("· {} busy ", busy), Style::default().fg(C_DIM)),
        Span::raw("│ "),
        Span::styled(format!("vram {:.0}/{:.0}G ", mu, mt), Style::default().fg(mem_color(mempct))),
        Span::raw("│ "),
        Span::styled(
            format!("models {}/{} ", serving, s.models.len()),
            Style::default().fg(if serving == 0 { C_WARN } else { C_OK }),
        ),
        Span::styled(format!("│ ⚡{:.0}W", power), Style::default().fg(C_DIM)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn tabs(f: &mut Frame, area: Rect, app: &App) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, v) in View::ALL.iter().enumerate() {
        let sel = *v == app.view;
        let st = if sel {
            Style::default().fg(Color::Black).bg(C_ACC).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(C_DIM)
        };
        spans.push(Span::styled(format!(" {}:{} ", i, v.title()), st));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some(t) = &app.toast {
        let msg = truncw(t, area.width.saturating_sub(1) as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(format!(" {} ", msg), Style::default().fg(Color::Black).bg(C_WARN)))),
            area,
        );
        return;
    }
    let sortable = app.sort_modes() > 1;
    let mut hint = String::from("↑↓ sel  ⏎ detail  ");
    if sortable {
        hint.push_str(&format!("o sort:{}  ", app.sort_label()));
    }
    hint.push_str("s scale  Tab/0-6 view  q quit");
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(truncw(&hint, area.width as usize), Style::default().fg(C_DIM)))),
        area,
    );
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
        Span::styled(track, Style::default().fg(C_TRACK)),
    ])
}

fn util_color(p: f64) -> Color {
    if p > 85.0 {
        C_BAD
    } else if p > 60.0 {
        C_WARN
    } else if p > 10.0 {
        C_OK
    } else {
        C_DIM
    }
}
fn mem_color(p: f64) -> Color {
    if p > 90.0 {
        C_BAD
    } else if p > 70.0 {
        C_WARN
    } else if p > 1.0 {
        C_OK
    } else {
        C_DIM
    }
}
fn temp_color(t: f64) -> Color {
    if t > 75.0 {
        C_BAD
    } else if t > 60.0 {
        C_WARN
    } else if t > 0.0 {
        Color::Gray
    } else {
        C_DIM
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
        .border_style(Style::default().fg(C_TRACK))
        .title(Span::styled(format!(" {} ", title), Style::default().fg(C_ACC).add_modifier(Modifier::BOLD)))
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
        Span::styled("● ", Style::default().fg(C_OK))
    } else {
        Span::styled("○ ", Style::default().fg(C_DIM))
    }
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn hrow(cols: &[&str]) -> Row<'static> {
    Row::new(
        cols.iter()
            .map(|c| Cell::from(Span::styled(c.to_string(), Style::default().fg(C_HEAD).add_modifier(Modifier::BOLD))))
            .collect::<Vec<_>>(),
    )
}

fn hl_style() -> Style {
    Style::default().bg(C_HL).add_modifier(Modifier::BOLD)
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
                Style::default().fg(C_DIM),
            ));
            let kc = if !a.alive {
                C_BAD
            } else if a.throttle > 0.0 {
                C_WARN
            } else {
                kind_color(a.kind)
            };
            Row::new(vec![
                Cell::from(Span::styled(a.kind.label(), Style::default().fg(kc).add_modifier(Modifier::BOLD))),
                cellw(a.id.clone(), 6),
                cellw(a.node.clone(), 16),
                Cell::from(Line::from(util)),
                Cell::from(Line::from(mem)),
                Cell::from(Span::styled(format!("{:.0}°C", a.temp), Style::default().fg(temp_color(a.temp)))),
                cellw(format!("{:.0}W", a.power), 5),
                Cell::from(Span::styled(model_cell, Style::default().fg(C_DIM))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(5),
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
        .block(block("Accelerators (GPU / RBLN / Furiosa) · ⏎ detail"));

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(table, split[0], &mut st);

    if let Some(a) = app.selected_accel() {
        let key = format!("{}:{}:{}", a.kind.label(), a.node, a.id);
        let data = app.hist_for(&key);
        let spark = Sparkline::default()
            .block(block(&format!("util history · {} {}", a.kind.label(), a.id)))
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

fn models_table<'a>(app: &'a App, title: &'a str) -> Table<'static> {
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let m = &app.snap.models[i];
            let color = if m.status.contains("Running") {
                C_OK
            } else if m.status.contains("Pending") {
                C_WARN
            } else {
                C_DIM
            };
            let kv = m.kv.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or("–".into());
            let name = if pos == app.selected { marquee(&m.name, 20, app.tick) } else { truncw(&m.name, 20) };
            Row::new(vec![
                Cell::from(name),
                Cell::from(Span::styled(truncw(&m.accel, 13), Style::default().fg(C_DIM))),
                cellw(format!("{}/{}", m.ready, m.desired), 6),
                cellw(fmt_opt(m.running), 4),
                cellw(fmt_opt(m.waiting), 4),
                cellw(kv, 5),
                cellw(m.tps.map(|x| format!("{:.0}", x)).unwrap_or("–".into()), 5),
                cellw(if m.route.is_empty() { "–".into() } else { m.route.clone() }, 11),
                Cell::from(Span::styled(m.status.clone(), Style::default().fg(color))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Min(14),
        Constraint::Length(13),
        Constraint::Length(6),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(11),
        Constraint::Length(11),
    ];
    let _ = title;
    Table::new(rows, widths)
        .header(hrow(&["MODEL", "ACCEL", "READY", "RUN", "WAIT", "KV", "t/s", "PATH", "STATUS"]))
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
                "Running" => C_OK,
                "Pending" => C_WARN,
                "Failed" => C_BAD,
                _ => C_DIM,
            };
            let name = if pos == app.selected { marquee(&p.name, 40, app.tick) } else { truncw(&p.name, 40) };
            Row::new(vec![
                Cell::from(name),
                cellw(p.ready.clone(), 6),
                Cell::from(Span::styled(p.phase.clone(), Style::default().fg(color))),
                cellw(p.node.clone(), 18),
                Cell::from(Span::styled(
                    p.restarts.to_string(),
                    Style::default().fg(if p.restarts > 0 { C_WARN } else { C_DIM }),
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
fn view_epp(f: &mut Frame, area: Rect, app: &App) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(6)])
        .split(area);

    let mut lines: Vec<Line> = Vec::new();
    match &app.snap.epp {
        Some(cfg) => {
            lines.push(Line::from(vec![
                Span::styled("profile: ", Style::default().fg(C_DIM)),
                Span::styled(cfg.profile.clone(), Style::default().fg(Color::White)),
                Span::styled("    picker: ", Style::default().fg(C_DIM)),
                Span::styled(cfg.picker.clone(), Style::default().fg(Color::White)),
            ]));
            lines.push(Line::from(""));
            let maxw = cfg.scorers.iter().map(|(_, w)| *w).fold(1.0, f64::max);
            for (name, w) in &cfg.scorers {
                let bw = ((w / maxw) * 16.0).round() as usize;
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<32}", truncw(name, 32)), Style::default().fg(Color::White)),
                    Span::styled(format!("w{:>2.0} ", w), Style::default().fg(C_WARN)),
                    Span::styled("█".repeat(bw), Style::default().fg(C_ACC)),
                ]));
            }
        }
        None => lines.push(Line::from(Span::styled("EPP ConfigMap 미발견 (llmd-router-epp)", Style::default().fg(C_DIM)))),
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("prefix indexer size: ", Style::default().fg(C_DIM)),
        Span::styled(
            if app.snap.prefix_idx.is_nan() { "–".to_string() } else { format!("{:.0}", app.snap.prefix_idx) },
            Style::default().fg(Color::White),
        ),
        Span::styled("    EPP in request path: ", Style::default().fg(C_DIM)),
        Span::styled(
            if app.snap.epp_in_path { "yes" } else { "no (HTTPRoute가 Service 직접 → 우회)" },
            Style::default().fg(if app.snap.epp_in_path { C_OK } else { C_WARN }),
        ),
    ]));
    f.render_widget(Paragraph::new(lines).block(block("EPP Inspector · active scorers (ConfigMap)")), split[0]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(split[1]);

    // 왼쪽: InferencePool 상태(endpoints/queue/sat)
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
                    Style::default().fg(if p.sat > 0.8 { C_BAD } else if p.sat > 0.5 { C_WARN } else { C_DIM }),
                )),
            ])
        })
        .collect();
    let t = Table::new(
        rows,
        [Constraint::Min(12), Constraint::Length(7), Constraint::Length(8), Constraint::Length(6)],
    )
    .header(hrow(&["POOL", "EP r/t", "QUEUE", "SAT"]))
    .block(block("InferencePool"));
    f.render_widget(t, bottom[0]);

    // 오른쪽: 요청 분배 (scheduler_attempts_total)
    let mut dl: Vec<Line> = Vec::new();
    let total: f64 = app.snap.decisions.iter().map(|(_, c)| c).sum();
    if app.snap.decisions.is_empty() || total <= 0.0 {
        dl.push(Line::from(Span::styled("요청 분배 데이터 없음", Style::default().fg(C_DIM))));
        dl.push(Line::from(Span::styled(
            if app.snap.epp_in_path { "(트래픽 대기 중)" } else { "(EPP 우회 — Topo 뷰 참조)" },
            Style::default().fg(C_DIM),
        )));
    } else {
        for (pod, cnt) in app.snap.decisions.iter().take(6) {
            let share = cnt / total * 100.0;
            let mut sp = vec![Span::styled(format!("{:<22} ", truncw(pod, 22)), Style::default().fg(Color::White))];
            sp.extend(bar_line(share, 8, C_ACC).spans);
            sp.push(Span::styled(format!(" {:>3.0}%", share), Style::default().fg(C_DIM)));
            dl.push(Line::from(sp));
        }
    }
    f.render_widget(Paragraph::new(dl).block(block("요청 분배 (EPP 라우팅 결정)")), bottom[1]);
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
    lines.push(Line::from(Span::styled(gw, Style::default().fg(C_OK).add_modifier(Modifier::BOLD))));
    for (i, r) in s.routes.iter().enumerate() {
        let branch = if i + 1 == s.routes.len() { "└─" } else { "├─" };
        let m = s.models.iter().find(|m| m.name == r.backend);
        let up = m.map(|m| m.ready > 0).unwrap_or(false);
        let annot = match m {
            Some(m) => format!("{}/{}  {}", m.ready, m.desired, m.accel),
            None => "?".into(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", branch), Style::default().fg(C_DIM)),
            dot(up),
            Span::styled(format!("{:<10}", truncw(&r.path, 10)), Style::default().fg(Color::White)),
            Span::styled("→ ", Style::default().fg(C_DIM)),
            Span::styled(format!("{}:", r.kind), Style::default().fg(C_DIM)),
            Span::styled(format!("{:<24}", truncw(&r.backend, 24)), Style::default().fg(if up { C_OK } else { C_DIM })),
            Span::styled(annot, Style::default().fg(C_DIM)),
        ]));
    }
    // EPP 경유 여부 진단
    if !s.routes.is_empty() {
        if s.epp_in_path {
            lines.push(Line::from(Span::styled("  ✓ 라우팅이 InferencePool(EPP) 경유", Style::default().fg(C_OK))));
        } else {
            lines.push(Line::from(Span::styled(
                "  ⚠ HTTPRoute 가 Service 로 직접 라우팅 → InferencePool/EPP 우회 (EPP 메트릭 비어있음)",
                Style::default().fg(C_WARN),
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
        pl.push(Line::from(Span::styled("(InferencePool 없음)", Style::default().fg(C_DIM))));
    }
    for p in &s.pools {
        pl.push(Line::from(vec![
            Span::styled(format!("{:<18}", truncw(&p.name, 18)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("ep {}/{} ", p.ep_ready, p.ep_total),
                Style::default().fg(if p.ep_total == 0 { C_WARN } else { C_OK }),
            ),
            Span::styled(format!("EPP:{} ", if p.epp.is_empty() { "–" } else { &p.epp }), Style::default().fg(C_ACC)),
            Span::styled(format!("sel={}", if p.selector.is_empty() { "–" } else { &p.selector }), Style::default().fg(C_DIM)),
        ]));
    }
    if !s.objectives.is_empty() {
        let so: Vec<String> = s.objectives.iter().map(|o| format!("{}(p{}→{})", o.name, o.priority, o.pool)).collect();
        pl.push(Line::from(vec![
            Span::styled("SLO  ", Style::default().fg(C_DIM)),
            Span::styled(so.join("  "), Style::default().fg(Color::White)),
        ]));
    }
    for a in &s.autoscalers {
        pl.push(Line::from(vec![
            Span::styled("autoscale ", Style::default().fg(C_DIM)),
            Span::styled(truncw(&a.target, 26), Style::default().fg(Color::White)),
            Span::styled(format!("  {}↔{} rep={} ", a.min, a.max, a.replicas), Style::default().fg(C_DIM)),
            Span::styled(if a.active { "active" } else { "idle" }, Style::default().fg(if a.active { C_OK } else { C_DIM })),
            Span::styled(if a.ready { " ✓" } else { " ⚠notready" }, Style::default().fg(if a.ready { C_OK } else { C_WARN })),
            Span::styled(format!(" [{}]", a.triggers), Style::default().fg(C_DIM)),
        ]));
    }
    f.render_widget(Paragraph::new(pl).block(block("InferencePool / EPP / SLO / Autoscale")), top[1]);
}

// ── Overview ───────────────────────────────────────────
fn view_overview(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Length(5), Constraint::Min(4), Constraint::Length(3)])
        .split(area);

    let mut al: Vec<Line> = Vec::new();
    for a in app.snap.accel.iter().take(6) {
        let mut spans = vec![
            Span::styled(format!("{:<5}", a.kind.label()), Style::default().fg(kind_color(a.kind))),
            Span::styled(format!("{:<6}", truncw(&a.id, 6)), Style::default().fg(C_DIM)),
        ];
        spans.extend(bar_line(a.util, 10, util_color(a.util)).spans);
        spans.push(Span::styled(format!(" {:>3.0}% ", a.util), Style::default().fg(util_color(a.util))));
        spans.push(Span::styled(format!("{:.0}/{:.0}G {:.0}°C ", a.mem_used_gb, a.mem_total_gb, a.temp), Style::default().fg(C_DIM)));
        spans.push(Span::styled(truncw(&a.busy_model, 18), Style::default().fg(C_DIM)));
        al.push(Line::from(spans));
    }
    if al.is_empty() {
        al.push(Line::from(Span::styled("  (no accelerator metrics)", Style::default().fg(C_DIM))));
    }
    f.render_widget(Paragraph::new(al).block(block("Accelerators")), rows[0]);

    let mut pl: Vec<Line> = Vec::new();
    for p in &app.snap.pools {
        pl.push(Line::from(vec![
            dot(p.ep_ready > 0),
            Span::styled(format!("{:<16} ", truncw(&p.name, 16)), Style::default().fg(Color::White)),
            Span::styled(
                format!("ep {}/{}  queue {}  sat {}", p.ep_ready, p.ep_total, fmt_nan(p.queue, 1), fmt_nan(p.sat, 2)),
                Style::default().fg(C_DIM),
            ),
        ]));
    }
    if let Some(cfg) = &app.snap.epp {
        let names: Vec<String> = cfg.scorers.iter().map(|(n, w)| format!("{}·{:.0}", n.replace("-scorer", ""), w)).collect();
        pl.push(Line::from(Span::styled(format!(" scorers: {}", names.join("  ")), Style::default().fg(C_DIM))));
    }
    if pl.is_empty() {
        pl.push(Line::from(Span::styled("  (no InferencePool)", Style::default().fg(C_DIM))));
    }
    f.render_widget(Paragraph::new(pl).block(block("Inference (EPP pools)")), rows[1]);

    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(models_table(app, "Models"), rows[2], &mut st);

    let (txt, col) = diagnose(&app.snap);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(truncw(&txt, rows[3].width.saturating_sub(2) as usize), Style::default().fg(col))))
            .block(block("Diagnosis")),
        rows[3],
    );
}

// ── Detail (drill-down) ────────────────────────────────
fn detail_panel(f: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    let mut title = "Detail";

    if let Some(a) = app.selected_accel() {
        title = "Accelerator detail · esc 닫기";
        let mempct = if a.mem_total_gb > 0.0 { a.mem_used_gb / a.mem_total_gb * 100.0 } else { 0.0 };
        lines.push(kv("kind", a.kind.label(), kind_color(a.kind)));
        lines.push(kv("id / node", &format!("{} / {}", a.id, a.node), Color::White));
        let mut u = vec![Span::styled("util       ", Style::default().fg(C_DIM))];
        u.extend(bar_line(a.util, 24, util_color(a.util)).spans);
        u.push(Span::styled(format!(" {:.0}%", a.util), Style::default().fg(util_color(a.util))));
        lines.push(Line::from(u));
        let mut m = vec![Span::styled("memory     ", Style::default().fg(C_DIM))];
        m.extend(bar_line(mempct, 24, mem_color(mempct)).spans);
        m.push(Span::styled(format!(" {:.1}/{:.1} GB ({:.0}%)", a.mem_used_gb, a.mem_total_gb, mempct), Style::default().fg(C_DIM)));
        lines.push(Line::from(m));
        lines.push(kv("temp", &format!("{:.0} °C", a.temp), temp_color(a.temp)));
        lines.push(kv("power", &format!("{:.0} W", a.power), Color::White));
        let health = if !a.alive {
            ("✗ not alive".to_string(), C_BAD)
        } else if a.throttle > 0.0 {
            (format!("⚠ alive · throttling {:.0}", a.throttle), C_WARN)
        } else {
            ("● healthy".to_string(), C_OK)
        };
        lines.push(kv("health", &health.0, health.1));
        lines.push(kv("model/pod", if a.busy_model.is_empty() { "(idle)" } else { a.busy_model.as_str() }, C_ACC));
    } else if let Some(m) = app.selected_model() {
        title = "Model detail · esc 닫기";
        lines.push(kv("model", &m.name, Color::White));
        lines.push(kv("status", &m.status, if m.ready > 0 { C_OK } else { C_DIM }));
        lines.push(kv("replicas", &format!("{}/{} (ready/desired)", m.ready, m.desired), Color::White));
        lines.push(kv("accelerator", &m.accel, C_ACC));
        lines.push(kv("route", if m.route.is_empty() { "–" } else { m.route.as_str() }, Color::White));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("inference (vLLM)", Style::default().fg(C_HEAD).add_modifier(Modifier::BOLD))));
        lines.push(kv("  running / waiting", &format!("{} / {}", fmt_opt(m.running), fmt_opt(m.waiting)), Color::White));
        lines.push(kv("  KV cache", &m.kv.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or("– (vLLM 메트릭 미수집)".into()), Color::White));
        lines.push(kv("  tokens/s", &m.tps.map(|x| format!("{:.1}", x)).unwrap_or("–".into()), Color::White));
        lines.push(kv("  TTFT p95", &m.ttft.map(|x| format!("{:.0} ms", x * 1000.0)).unwrap_or("–".into()), Color::White));
        lines.push(Line::from(""));
        let pods: Vec<&str> = app.snap.pods.iter().filter(|p| p.name.starts_with(&m.name)).map(|p| p.name.as_str()).collect();
        lines.push(kv("pods", &if pods.is_empty() { "(none)".to_string() } else { pods.join(", ") }, C_DIM));
        lines.push(Line::from(Span::styled("  s = scale up/down", Style::default().fg(C_DIM))));
    } else if let Some(p) = app.selected_pod() {
        title = "Pod detail · esc 닫기";
        lines.push(kv("pod", &p.name, Color::White));
        lines.push(kv("phase", &p.phase, if p.phase == "Running" { C_OK } else { C_DIM }));
        lines.push(kv("ready", &p.ready, Color::White));
        lines.push(kv("node", &p.node, Color::White));
        lines.push(kv("restarts", &p.restarts.to_string(), if p.restarts > 0 { C_WARN } else { Color::White }));
    } else {
        lines.push(Line::from(Span::styled("선택된 항목 없음", Style::default().fg(C_DIM))));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block(title)), area);
}

fn kv(k: &str, v: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<18} ", k), Style::default().fg(C_DIM)),
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
fn hist_last(app: &App, key: &str) -> f64 {
    app.hist_for(key).last().copied().unwrap_or(0) as f64
}
fn spark(f: &mut Frame, area: Rect, app: &App, key: &str, title: &str, max: u64, color: Color) {
    let data = app.hist_for(key);
    let cur = data.last().copied().unwrap_or(0);
    let mut s = Sparkline::default()
        .block(block(&format!("{} · {}", title, cur)))
        .data(&data)
        .style(Style::default().fg(color));
    if max > 0 {
        s = s.max(max);
    }
    f.render_widget(s, area);
}

fn view_perf(f: &mut Frame, area: Rect, app: &App) {
    let p = &app.snap.perf;
    let any = [p.e2e_p95, p.ttft_p95, p.tps, p.req_rate].iter().any(|x| !x.is_nan());

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Length(3), Constraint::Length(6), Constraint::Min(3)])
        .split(area);

    // timeline 스파크라인 (부하/자원/처리량/지연 추이)
    let sp = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 4); 4])
        .split(rows[0]);
    spark(f, sp[0], app, "sys:util", "util% 추이", 100, util_color(hist_last(app, "sys:util")));
    spark(f, sp[1], app, "sys:vram", "vram% 추이", 100, C_ACC);
    spark(f, sp[2], app, "sys:tps", "tok/s 추이", 0, C_OK);
    spark(f, sp[3], app, "sys:lat", "e2e p95(ms) 추이", 0, C_WARN);

    // throughput 숫자 + 데이터 없음 안내
    let tl = Line::from(vec![
        Span::styled("req/s ", Style::default().fg(C_DIM)),
        Span::styled(format!("{}  ", rate(p.req_rate)), Style::default().fg(C_OK)),
        Span::styled("err/s ", Style::default().fg(C_DIM)),
        Span::styled(format!("{}  ", rate(p.err_rate)), Style::default().fg(if p.err_rate > 0.0 { C_BAD } else { C_DIM })),
        Span::styled("tok/s ", Style::default().fg(C_DIM)),
        Span::styled(format!("{}  ", rate(p.tps)), Style::default().fg(C_OK)),
        Span::styled("prefix-hit ", Style::default().fg(C_DIM)),
        Span::styled(
            if p.prefix_hit.is_nan() { "–  ".into() } else { format!("{:.0}%  ", p.prefix_hit * 100.0) },
            Style::default().fg(C_ACC),
        ),
        Span::styled(
            if any { "" } else { "· 값 없음: EPP 경유 트래픽+vLLM 메트릭 필요" },
            Style::default().fg(C_WARN),
        ),
    ]);
    f.render_widget(Paragraph::new(tl).block(block("Throughput")), rows[1]);

    // latency percentiles per stage
    let lrows = vec![
        Row::new(vec![Cell::from("queue (EPP)"), Cell::from("–"), cellw(ms(p.queue_p95), 8), Cell::from("–")]),
        Row::new(vec![Cell::from("TTFT (prefill)"), Cell::from("–"), cellw(ms(p.ttft_p95), 8), cellw(ms(p.ttft_p99), 8)]),
        Row::new(vec![Cell::from("TPOT (decode/tok)"), Cell::from("–"), cellw(ms(p.tpot_p95), 8), Cell::from("–")]),
        Row::new(vec![
            Cell::from("E2E (전체)"),
            cellw(ms(p.e2e_p50), 8),
            cellw(ms(p.e2e_p95), 8),
            cellw(ms(p.e2e_p99), 8),
        ]),
    ];
    let lt = Table::new(
        lrows,
        [Constraint::Length(20), Constraint::Length(10), Constraint::Length(10), Constraint::Length(10)],
    )
    .header(hrow(&["STAGE", "p50", "p95", "p99"]))
    .block(block("Latency percentiles (구간별) · EPP 정책 튜닝용"));
    f.render_widget(lt, rows[2]);

    // token distribution + per-pod queue
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(rows[3]);

    let toklines = vec![
        Line::from(vec![
            Span::styled("input  ", Style::default().fg(C_DIM)),
            Span::styled(format!("p50 {:<7} p95 {}", tok(p.in_tok_p50), tok(p.in_tok_p95)), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("output ", Style::default().fg(C_DIM)),
            Span::styled(format!("p50 {:<7} p95 {}", tok(p.out_tok_p50), tok(p.out_tok_p95)), Style::default().fg(Color::White)),
        ]),
    ];
    f.render_widget(Paragraph::new(toklines).block(block("Token length 분포")), split[0]);

    // per-pod queue (요청 분배)
    let mut ql: Vec<Line> = Vec::new();
    let maxq = app.snap.pod_queues.iter().map(|(_, q)| *q).fold(1.0, f64::max);
    if app.snap.pod_queues.is_empty() {
        ql.push(Line::from(Span::styled("per-pod 큐 데이터 없음", Style::default().fg(C_DIM))));
    } else {
        for (pod, q) in app.snap.pod_queues.iter().take(6) {
            let mut sp = vec![Span::styled(format!("{:<24} ", truncw(pod, 24)), Style::default().fg(Color::White))];
            sp.extend(bar_line(q / maxq * 100.0, 10, C_ACC).spans);
            sp.push(Span::styled(format!(" {:.0}", q), Style::default().fg(C_DIM)));
            ql.push(Line::from(sp));
        }
    }
    f.render_widget(Paragraph::new(ql).block(block("요청 분배 (per-pod queue)")), split[1]);
}

// ── 진단 ───────────────────────────────────────────────
fn diagnose(s: &Snapshot) -> (String, Color) {
    let serving = s.models.iter().filter(|m| m.ready > 0).count();
    if s.accel.is_empty() && serving == 0 {
        return ("⚠ 가속기 메트릭 없음 + 서빙 모델 없음 — Prometheus/모델 상태 점검".into(), C_BAD);
    }
    if serving == 0 {
        return ("⚠ 서빙 중인 모델 0 — Models 뷰에서 's'로 기동 (백엔드 없음)".into(), C_WARN);
    }
    let busy = s.accel.iter().filter(|a| a.util > 80.0).count();
    if busy > 0 {
        return (format!("● {} 모델 서빙 중, 가속기 {}개 고부하(>80%)", serving, busy), C_OK);
    }
    (format!("● {} 모델 서빙 중, 가속기 여유", serving), C_OK)
}
