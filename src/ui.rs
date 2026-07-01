//! ratatui 렌더링 — 헤더/탭/본문(뷰별, 정렬·상세 포함)/footer.
//! 모든 문자열은 표시 폭(unicode-width) 기준으로 절단해 CJK/와이드 글자 깨짐을 방지.
//! 선택 하이라이트는 REVERSED 대신 은은한 배경색(htop/all-smi 스타일).

use crate::app::{App, Mode, Sev, View};
use crate::collect::{AccelKind, PerfDetail, Snapshot};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState, Wrap,
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

// ── 심각도 임계치 (단일 출처) ───────────────────────────
// 개념 "warn/bad" 를 지표별로 한 곳에서 정의 → 바 색과 값 색이 어긋나지 않음.
const UTIL_WARN: f64 = 60.0;
const UTIL_BAD: f64 = 85.0;
const MEM_WARN: f64 = 70.0;
const MEM_BAD: f64 = 90.0;
const TEMP_WARN: f64 = 60.0;
const TEMP_BAD: f64 = 80.0;
const IDLE_UTIL: f64 = 10.0; // 이하는 dim(유휴)

pub fn draw(f: &mut Frame, app: &App) {
    let (body, footer_area) = if app.zoom {
        // 포커스 모드: 헤더/탭 숨기고 본문 최대화
        let c = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
            .split(f.area());
        title_bar(f, c[0], app);
        (c[1], c[2])
    } else {
        let c = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(f.area());
        title_bar(f, c[0], app);
        summary_bar(f, c[1], app);
        tabs(f, c[2], app);
        (c[3], c[4])
    };
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
    footer(f, footer_area, app);
    if app.logs_mode {
        logs_overlay(f, app);
    }
    if app.alerts_panel {
        alerts_overlay(f, app);
    }
    if app.help {
        help_overlay(f);
    }
}

/// 알림 히스토리 오버레이(A) — 최신 앞, 상대시각 + 심각도색.
fn alerts_overlay(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 78, 22);
    f.render_widget(Clear, area);
    let now = crate::collect::now_secs();
    let lines: Vec<Line> = if app.alerts.is_empty() {
        vec![Line::from(Span::styled("  no alerts — all clear ●", Style::default().fg(C_OK())))]
    } else {
        app.alerts
            .iter()
            .map(|al| {
                let age = now.saturating_sub(al.ts);
                let (g, c) = if al.sev == Sev::Bad { ("✗", C_BAD()) } else { ("⚠", C_WARN()) };
                Line::from(vec![
                    Span::styled(format!("  {} ", g), Style::default().fg(c).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{:>4}s ago  ", age), Style::default().fg(C_DIM())),
                    Span::styled(truncw(&al.msg, area.width.saturating_sub(18) as usize), Style::default().fg(Color::White)),
                ])
            })
            .collect()
    };
    let title = format!(" alerts · {} recent · esc/A close ", app.alerts.len());
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(C_BAD()))
                .title(Span::styled(title, Style::default().fg(C_BAD()).add_modifier(Modifier::BOLD))),
        ),
        area,
    );
}

/// 2-패널: 넓으면 좌우, 좁으면(<100) 위아래로 — 반응형.
fn two_panes(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let dir = if area.width >= 100 { Direction::Horizontal } else { Direction::Vertical };
    let c = Layout::default()
        .direction(dir)
        .constraints([Constraint::Percentage(left_pct), Constraint::Percentage(100 - left_pct)])
        .split(area);
    (c[0], c[1])
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w.min(area.width), height: h.min(area.height) }
}

fn help_overlay(f: &mut Frame) {
    let area = centered(f.area(), 66, 24);
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
        g("p i r e m", "cross-layer pivot (model↔pods↔infra↔route↔epp), esc retraces"),
        g("o", "cycle sort"),
        g("/", "filter (substring)"),
        g("l", "logs (selected pod/model, scroll+refresh)"),
        g("s", "scale selected model (needs --mode admin+, confirms y/n)"),
        g("A", "alert history (threshold/health events)"),
        g("t", "cycle theme (default/high-contrast/colorblind)"),
        g("g", "open Grafana dashboard"),
        g("z", "zoom/focus (hide header+tabs)"),
        g("Esc", "back: close detail/filter/zoom (does NOT quit)"),
        g("? / q", "help / quit"),
        Line::from(""),
        sec("color / glyph"),
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(C_OK())), Span::styled("up  ", Style::default().fg(C_DIM())),
            Span::styled("○ ", Style::default().fg(C_DIM())), Span::styled("idle  ", Style::default().fg(C_DIM())),
            Span::styled("◐ ", Style::default().fg(C_WARN())), Span::styled("pending  ", Style::default().fg(C_DIM())),
            Span::styled("⚠ ", Style::default().fg(C_WARN())), Span::styled("throttle  ", Style::default().fg(C_DIM())),
            Span::styled("⊘ ", Style::default().fg(C_WARN())), Span::styled("cordoned  ", Style::default().fg(C_DIM())),
            Span::styled("✗ ", Style::default().fg(C_BAD())), Span::styled("down", Style::default().fg(C_DIM())),
        ]),
        Line::from(vec![
            Span::styled("  util/mem/temp: ", Style::default().fg(C_DIM())),
            Span::styled("low", Style::default().fg(C_OK())), Span::raw(" "),
            Span::styled("mid", Style::default().fg(C_WARN())), Span::raw(" "),
            Span::styled("high", Style::default().fg(C_BAD())),
            Span::styled("   ∪ = unified mem (GB10 등, CPU·GPU 공유)", Style::default().fg(C_DIM())),
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

fn logs_overlay(f: &mut Frame, app: &App) {
    let full = f.area();
    let area = Rect {
        x: full.x + 1,
        y: full.y + 1,
        width: full.width.saturating_sub(2),
        height: full.height.saturating_sub(2),
    };
    f.render_widget(Clear, area);
    let lines: Vec<Line> = app
        .logs
        .iter()
        .map(|l| {
            let low = l.to_ascii_lowercase();
            let col = if low.contains("error") || low.contains("traceback") || low.contains("fatal") || low.contains("exception") {
                C_BAD()
            } else if low.contains("warn") {
                C_WARN()
            } else if low.contains("info") {
                C_OK()
            } else {
                Color::Gray
            };
            Line::from(Span::styled(l.clone(), Style::default().fg(col)))
        })
        .collect();
    let title = format!(
        " logs · {} · {} lines · ↑↓ scroll · r refresh · esc/q close ",
        app.logs_target,
        app.logs.len()
    );
    let total = app.logs.len();
    f.render_widget(
        Paragraph::new(lines).scroll((app.logs_scroll, 0)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(C_ACC()))
                .title(Span::styled(title, Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD))),
        ),
        area,
    );
    list_scrollbar(f, area, total, (app.logs_scroll as usize).min(total.saturating_sub(1)), 0);
}

// ── 헤더 ───────────────────────────────────────────────
fn title_bar(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let (tick, paused) = (app.tick, app.paused);
    let spin = if paused { "⏸" } else { SPINNER[(tick as usize) % SPINNER.len()] };
    let gw = if s.gw_addr.is_empty() {
        Span::styled("⌂ gw —", Style::default().fg(C_DIM()))
    } else if s.gw_ok {
        Span::styled(format!("⌂ gw {} ●", s.gw_addr), Style::default().fg(C_OK()))
    } else {
        Span::styled(format!("⌂ gw {} ○", s.gw_addr), Style::default().fg(C_WARN()))
    };
    // 데이터 신선도: 마지막 스냅샷이 몇 초 전인지(수집 주기 3s). stale 판단용.
    let fresh = if s.ts == 0 {
        Span::styled("  · connecting…", Style::default().fg(C_DIM()))
    } else {
        let age = crate::collect::now_secs().saturating_sub(s.ts);
        let col = if age > 10 { C_WARN() } else { C_DIM() };
        Span::styled(format!("  · updated {}s ago", age), Style::default().fg(col))
    };
    // 권한 모드 배지 — observe 는 은은하게, 상승 권한은 색+굵게(사고 방지 인지).
    let (mcol, mmod) = match app.mode {
        Mode::Observe => (C_DIM(), Modifier::empty()),
        Mode::Debug => (C_ACC(), Modifier::BOLD),
        Mode::Admin => (C_WARN(), Modifier::BOLD),
        Mode::Danger => (C_BAD(), Modifier::BOLD),
    };
    let line = Line::from(vec![
        Span::styled(format!("{} ", spin), Style::default().fg(if paused { C_WARN() } else { C_ACC() })),
        Span::styled("lmd-top", Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" [{}]", app.mode.name()), Style::default().fg(mcol).add_modifier(mmod)),
        Span::styled(format!("  llm-d · {} nodes  ", s.nodes.len()), Style::default().fg(C_DIM())),
        gw,
        fresh,
        Span::styled(if paused { "  ⏸ PAUSED (space)" } else { "" }, Style::default().fg(C_WARN())),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn summary_bar(f: &mut Frame, area: Rect, app: &App) {
    // 서빙/SLO 우선(왼쪽), 인프라는 뒤(오른쪽). "지금 서빙 건강한가?"가 항상 보이는 자리로.
    let s = &app.snap;
    let p = &s.perf;
    let (mut busy, mut mu, mut mt, mut pw) = (0usize, 0.0f64, 0.0f64, 0.0f64);
    for a in &s.accel {
        if a.util > 5.0 {
            busy += 1;
        }
        mu += a.mem_used_gb;
        mt += a.mem_total_gb;
        pw += a.power;
    }
    let nacc = s.accel.len();
    let serving = s.models.iter().filter(|m| m.ready > 0).count();
    let total = s.models.len();
    let mempct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
    let err = p.err_rate;
    // 서빙 건강 글리프: 0서빙=✗, 에러>0=⚠, 아니면 ●.
    let (sg, sc) = if total == 0 {
        ("○", C_DIM())
    } else if serving == 0 {
        ("✗", C_BAD())
    } else if !err.is_nan() && err > 0.0 {
        ("⚠", C_WARN())
    } else {
        ("●", C_OK())
    };
    let mut spans = vec![
        Span::styled(format!("{} SERVING {}/{}  ", sg, serving, total), Style::default().fg(sc).add_modifier(Modifier::BOLD)),
        Span::styled(format!("req/s {}  ", rate(p.req_rate)), Style::default().fg(C_DIM())),
        Span::styled(format!("err {}  ", rate(err)), Style::default().fg(if !err.is_nan() && err > 0.0 { C_BAD() } else { C_DIM() })),
        Span::styled(format!("TTFT {}  ", ms(p.ttft_p95)), Style::default().fg(C_DIM())),
        Span::styled(format!("E2E {}  ", ms(p.e2e_p95)), Style::default().fg(C_DIM())),
        Span::raw("│ "),
        Span::styled(format!("accel {}/{} busy  ", busy, nacc), Style::default().fg(C_DIM())),
        Span::styled(format!("VRAM {:.0}%  ", mempct), Style::default().fg(mem_color(mempct))),
        Span::styled(format!("⚡{:.0}W", pw), Style::default().fg(C_DIM())),
    ];
    // 활성 알림 카운트(A 로 히스토리)
    let nalert = app.active_alerts.len();
    if nalert > 0 {
        spans.push(Span::styled(format!("  ⚠{} alert (A)", nalert), Style::default().fg(C_BAD()).add_modifier(Modifier::BOLD)));
    }
    let mut para = Paragraph::new(Line::from(spans));
    // 신규 알림 플래시: flash_until 이전이면 ~0.6s 주기로 요약바 전체를 반전.
    let now = crate::collect::now_secs();
    if now < app.flash_until && (app.tick / 3) % 2 == 0 {
        para = para.style(Style::default().bg(C_BAD()).fg(Color::Black).add_modifier(Modifier::BOLD));
    }
    f.render_widget(para, area);
}

fn tabs(f: &mut Frame, area: Rect, app: &App) {
    // 전체 라벨 폭이 화면을 넘으면 비활성 탭은 번호만 표시(활성 탭은 라벨 유지) — 반응형.
    let full_w: usize = View::ALL
        .iter()
        .enumerate()
        .map(|(i, v)| format!(" {}:{} ", i, v.title()).len() + 1)
        .sum();
    let compact = full_w > area.width as usize;
    let mut spans: Vec<Span> = Vec::new();
    for (i, v) in View::ALL.iter().enumerate() {
        let sel = *v == app.view;
        let st = if sel {
            Style::default().fg(Color::Black).bg(C_ACC()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(C_DIM())
        };
        let label = if sel || !compact {
            format!(" {}:{} ", i, v.title())
        } else {
            format!(" {} ", i)
        };
        spans.push(Span::styled(label, st));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn footer(f: &mut Frame, area: Rect, app: &App) {
    // 변경 작업 확인(y/n) 프롬프트
    if let Some(pending) = &app.confirm {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" confirm ", Style::default().fg(Color::Black).bg(C_WARN()).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {} ", pending.prompt()), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled("  y confirm · n/esc cancel", Style::default().fg(C_DIM())),
            ])),
            area,
        );
        return;
    }
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
        if crate::collect::now_secs() < app.toast_until {
            let msg = truncw(t, area.width.saturating_sub(1) as usize);
            let bg = if app.toast_bad { C_BAD() } else { C_WARN() };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(format!(" {} ", msg), Style::default().fg(Color::Black).bg(bg)))),
                area,
            );
            return;
        }
    }
    let mut spans: Vec<Span> = Vec::new();
    if !app.filter.is_empty() {
        spans.push(Span::styled(format!("[filter: {}] ", app.filter), Style::default().fg(Color::Black).bg(C_ACC())));
        spans.push(Span::raw(" "));
    }
    // 컨텍스트 푸터: 현재 뷰가 실제 할 수 있는 액션만(no-op 숨김).
    use View::*;
    let v = app.view;
    let mut parts: Vec<String> = Vec::new();
    parts.push("↑↓ sel".into());
    match v {
        Accel | Models | Overview | Pods | Nodes => parts.push("⏎ detail".into()),
        Perf => parts.push("⏎ p50/95/99".into()),
        _ => {}
    }
    if matches!(v, Accel | Models | Overview | Pods | Launch | Epp | Events | Nodes) {
        parts.push("/ filter".into());
    }
    if app.sort_modes() > 1 {
        parts.push(format!("o sort:{}", app.sort_label()));
    }
    match v {
        Models | Overview => parts.push("p/i/r/e pivot".into()),
        Accel => parts.push("p/m/n pivot".into()),
        Pods => parts.push("i/m pivot".into()),
        Nodes => parts.push("i pivot".into()),
        _ => {}
    }
    if matches!(v, Pods | Models | Overview | Accel) {
        parts.push("l logs".into());
    }
    if matches!(v, Models | Overview) {
        parts.push("s scale".into());
    }
    // 전역
    parts.push("A alerts".into());
    parts.push("t theme".into());
    parts.push("z zoom".into());
    parts.push("g grafana↗".into());
    parts.push("? help".into());
    parts.push("q quit".into());
    spans.push(Span::styled(parts.join("  "), Style::default().fg(C_DIM())));
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

/// 셀 위치별 색(초록→노랑→빨강). util 임계치와 동일 기준.
fn grad_color(pct: f64) -> Color {
    if pct > UTIL_BAD {
        C_BAD()
    } else if pct > UTIL_WARN {
        C_WARN()
    } else {
        C_OK()
    }
}

/// btop 식 그라디언트 바 — 채워진 길이만큼 초록→노랑→빨강. 사용률(util/mem/cpu)용.
fn grad_bar(pct: f64, width: usize) -> Line<'static> {
    let p = pct.clamp(0.0, 100.0) / 100.0;
    let frac = p * width as f64;
    let full = (frac.floor() as usize).min(width);
    let mut spans: Vec<Span> = Vec::new();
    let mut i = 0;
    while i < full {
        let c = grad_color((i as f64 / width as f64) * 100.0);
        let mut run = 0;
        while i + run < full && grad_color(((i + run) as f64 / width as f64) * 100.0) == c {
            run += 1;
        }
        spans.push(Span::styled("█".repeat(run), Style::default().fg(c)));
        i += run;
    }
    let mut used = full;
    if used < width {
        let rem = ((frac - full as f64) * 8.0).round() as usize;
        if rem > 0 {
            let c = grad_color((full as f64 / width as f64) * 100.0);
            spans.push(Span::styled(FRAC[rem - 1].to_string(), Style::default().fg(c)));
            used += 1;
        }
    }
    spans.push(Span::styled("░".repeat(width.saturating_sub(used)), Style::default().fg(C_TRACK())));
    Line::from(spans)
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

/// all-smi 식 스택형 바: 세그먼트(값,색)를 비율대로 이어 붙이고 나머지는 track.
/// 이종 가속기 VRAM 구성(GPU|RBLN|RNGD|free) 등 "무엇이 얼마나 차지하나" 표시용.
fn stacked_bar(segments: &[(f64, Color)], total: f64, width: usize) -> Vec<Span<'static>> {
    if total <= 0.0 || width == 0 {
        return vec![Span::styled("░".repeat(width), Style::default().fg(C_TRACK()))];
    }
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0usize;
    for (val, col) in segments {
        if used >= width {
            break;
        }
        let w = (((val / total) * width as f64).round() as usize).min(width - used);
        if w > 0 {
            spans.push(Span::styled("█".repeat(w), Style::default().fg(*col)));
            used += w;
        }
    }
    if used < width {
        spans.push(Span::styled("░".repeat(width - used), Style::default().fg(C_TRACK())));
    }
    spans
}

/// all-smi 식 게이지 행: `label  ██████░░░░  value`.
/// pct=바 채움(0~100), value=우측 현재값 텍스트, color=값 색.
fn gauge_row(label: &str, pct: f64, value: &str, color: Color, barw: usize) -> Line<'static> {
    let mut sp = vec![Span::styled(format!("{:<8} ", label), Style::default().fg(C_DIM()))];
    sp.extend(grad_bar(pct, barw).spans);
    sp.push(Span::styled(format!("  {}", value), Style::default().fg(color).add_modifier(Modifier::BOLD)));
    Line::from(sp)
}

/// all-smi 식 인라인 텍스트 스파크라인(▁▂▃▄▅▆▇█). 최근 width개, max 기준 정규화.
fn sparkstr(data: &[u64], width: usize, max: u64) -> String {
    const B: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let slice: &[u64] = if data.len() > width { &data[data.len() - width..] } else { data };
    let mx = if max > 0 { max } else { (*slice.iter().max().unwrap_or(&1)).max(1) };
    let mut s = String::new();
    for _ in 0..width.saturating_sub(slice.len()) {
        s.push(' ');
    }
    for &v in slice {
        let idx = ((v as f64 / mx as f64) * 7.0).round().clamp(0.0, 7.0) as usize;
        s.push(B[idx]);
    }
    s
}

fn util_color(p: f64) -> Color {
    if p > UTIL_BAD {
        C_BAD()
    } else if p > UTIL_WARN {
        C_WARN()
    } else if p > IDLE_UTIL {
        C_OK()
    } else {
        C_DIM()
    }
}
fn mem_color(p: f64) -> Color {
    if p > MEM_BAD {
        C_BAD()
    } else if p > MEM_WARN {
        C_WARN()
    } else if p > 1.0 {
        C_OK()
    } else {
        C_DIM()
    }
}
fn temp_color(t: f64) -> Color {
    if t > TEMP_BAD {
        C_BAD()
    } else if t > TEMP_WARN {
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
/// prefill/decode 구간색 — 테마·색맹 대응(Okabe-Ito 계열). Perf 테이블 phase 구분용.
#[allow(non_snake_case)]
fn C_PREFILL() -> Color {
    match th() { 1 => Color::LightCyan, 2 => Color::Rgb(86, 180, 233), _ => Color::Cyan }
}
#[allow(non_snake_case)]
fn C_DECODE() -> Color {
    match th() { 1 => Color::LightMagenta, 2 => Color::Rgb(204, 121, 167), _ => Color::Magenta }
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

/// 활성(선택 대상) 패널용 블록 — 멀티패널 뷰에서 ↑↓가 움직이는 패널을 밝은 테두리로 강조.
fn block_active(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD))
        .title(Span::styled(format!(" ▸ {} ", title), Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)))
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

/// 리스트/테이블 오른쪽에 스크롤바(오버플로 표시). 블록 테두리 안쪽 세로로 렌더.
/// total=전체 항목 수, pos=현재 선택 인덱스, header=본문 위 고정 행(테이블 헤더=1, 없으면 0).
/// 화면보다 짧으면 그리지 않음.
fn list_scrollbar(f: &mut Frame, area: Rect, total: usize, pos: usize, header: usize) {
    let viewport = (area.height as usize).saturating_sub(2 + header); // 테두리(2) + 헤더 행 제외
    if total == 0 || total <= viewport {
        return;
    }
    let mut st = ScrollbarState::new(total).position(pos);
    let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .thumb_symbol("█")
        .track_symbol(Some("│"))
        .thumb_style(Style::default().fg(C_ACC()))
        .track_style(Style::default().fg(C_TRACK()));
    let inner = area.inner(Margin { vertical: 1, horizontal: 0 });
    f.render_stateful_widget(sb, inner, &mut st);
}

/// 블록 타이틀용 위치 카운터 접미사 " · sel/total". total<=0 이면 빈 문자열.
fn count_suffix(sel: usize, total: usize) -> String {
    if total == 0 {
        " · 0".to_string()
    } else {
        format!(" · {}/{}", sel + 1, total)
    }
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
            let mut util = grad_bar(a.util, 9).spans;
            util.push(Span::styled(format!(" {:>3.0}%", a.util), Style::default().fg(util_color(a.util))));
            let mut mem = grad_bar(mempct, 7).spans;
            mem.push(Span::styled(
                format!(" {:.0}/{:.0}GB{}", a.mem_used_gb, a.mem_total_gb, if a.unified_mem { "∪" } else { "" }),
                Style::default().fg(C_DIM()),
            ));
            let (hg, hc) = if !a.alive {
                ("✗", C_BAD())
            } else if a.throttle > 0.0 {
                ("⚠", C_WARN())
            } else {
                ("●", C_OK())
            };
            let trend = sparkstr(&app.hist_for(&format!("acc:{}:{}:{}:util", a.kind.label(), a.node, a.id)), 12, 100);
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(hg, Style::default().fg(hc)),                                  // 상태=글리프
                    Span::raw(" "),
                    Span::styled(a.disp(), Style::default().fg(kind_color(a.kind)).add_modifier(Modifier::BOLD)), // 모델(감지)·vendor색
                ])),
                cellw(a.id.clone(), 6),
                cellw(a.node.clone(), 14),
                Cell::from(Line::from(util)),
                Cell::from(Line::from(mem)),
                Cell::from(Span::styled(format!("{:.0}°C", a.temp), Style::default().fg(temp_color(a.temp)))),
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
    let total = order.len();
    let table = Table::new(rows, widths)
        .header(hrow(&["KIND", "ID", "NODE", "UTIL", "MEM", "TEMP", "PWR", "TREND(util)", "MODEL/POD"]))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block(&format!("Accelerators · UTIL=compute% MEM=VRAM · ⏎ timeline{}", count_suffix(app.selected, total))));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(table, area, &mut st);
    list_scrollbar(f, area, total, app.selected, 1);
}

// ── Models ─────────────────────────────────────────────
fn view_models(f: &mut Frame, area: Rect, app: &App) {
    let mut st = TableState::default();
    st.select(Some(app.selected));
    let total = app.order().len();
    f.render_stateful_widget(models_table(app, "Models · ⏎ detail"), area, &mut st);
    list_scrollbar(f, area, total, app.selected, 1);
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
        .block(block(&format!("Pods (llm-serving) · ⏎ detail{}", count_suffix(app.selected, order.len()))));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(table, area, &mut st);
    list_scrollbar(f, area, order.len(), app.selected, 1);
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
    let (top_l, top_r) = two_panes(split[0], 52);

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
                .block(block_active(&format!("EPP scorers · select for description{}", count_suffix(app.selected, order.len()))));
            let mut st = TableState::default();
            st.select(Some(app.selected));
            f.render_stateful_widget(t, top_l, &mut st);
            list_scrollbar(f, top_l, order.len(), app.selected, 1);

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
            f.render_widget(Paragraph::new(dl).wrap(Wrap { trim: true }).block(block("what this scorer does")), top_r);
        }
        None => f.render_widget(
            Paragraph::new(Line::from(Span::styled("EPP ConfigMap not found (llmd-router-epp)", Style::default().fg(C_DIM())))).block(block("EPP scorers")),
            top_l,
        ),
    }

    let (bottom_l, bottom_r) = two_panes(split[1], 52);

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
    f.render_widget(t, bottom_l);

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
    f.render_widget(Paragraph::new(dl).block(block("request distribution (routing decisions)")), bottom_r);
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
    lines.push(Line::from(vec![
        Span::styled("└─ ", Style::default().fg(C_DIM())),
        Span::styled("HTTPRoute ", Style::default().fg(C_DIM())),
        Span::styled("openai-route", Style::default().fg(Color::White)),
    ]));
    let n = s.routes.len();
    for (i, r) in s.routes.iter().enumerate() {
        let last = i + 1 == n;
        let rbr = if last { "   └─ " } else { "   ├─ " };
        let m = s.models.iter().find(|m| m.name == r.backend);
        let up = m.map(|m| m.ready > 0).unwrap_or(false);
        let annot = match m {
            Some(m) => format!("{}/{} {} [{}]", m.ready, m.desired, m.accel, m.engine),
            None => "?".into(),
        };
        lines.push(Line::from(vec![
            Span::styled(rbr, Style::default().fg(C_DIM())),
            dot(up),
            Span::styled(format!("{:<9}", truncw(&r.path, 9)), Style::default().fg(Color::White)),
            Span::styled("→ ", Style::default().fg(C_DIM())),
            Span::styled(format!("{}:{}  ", r.kind, truncw(&r.backend, 22)), Style::default().fg(if up { C_OK() } else { C_DIM() })),
            Span::styled(annot, Style::default().fg(C_DIM())),
        ]));
        // 하위: 이 backend 의 파드들(트리 자식)
        let cont = if last { "      " } else { "   │  " };
        let pods: Vec<&crate::collect::PodRow> = s.pods.iter().filter(|p| p.name.starts_with(&r.backend)).collect();
        for (j, p) in pods.iter().enumerate() {
            let pbr = if j + 1 == pods.len() { "└─ " } else { "├─ " };
            let pc = if p.phase == "Running" { C_OK() } else { C_DIM() };
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}", cont, pbr), Style::default().fg(C_TRACK())),
                Span::styled(format!("{} ", truncw(&p.name, 32)), Style::default().fg(Color::Gray)),
                Span::styled(format!("{} @{}", p.phase, p.node), Style::default().fg(pc)),
            ]));
        }
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

    // ── 클러스터 요약 카드(all-smi 식 aggregate) ──────────
    // Σ 요약 1줄 + LED 그리드(폭에 맞춰 줄바꿈). 카드 높이는 LED 줄 수에 맞춰 가변.
    let mut cluster_lines: Vec<Line> = Vec::new();
    {
        // 벤더별 (수, 색, 사용메모리GB) — 스택 바용.
        let mut kinds: std::collections::BTreeMap<&str, (usize, Color, f64)> = std::collections::BTreeMap::new();
        let (mut usum, mut mu, mut mt, mut pw) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for a in &s.accel {
            let e = kinds.entry(a.disp()).or_insert((0, kind_color(a.kind), 0.0));
            e.0 += 1;
            e.2 += a.mem_used_gb;
            usum += a.util; mu += a.mem_used_gb; mt += a.mem_total_gb; pw += a.power;
        }
        let ncnt = s.accel.len().max(1);
        let avg = usum / ncnt as f64;
        let mempct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
        let ready = s.models.iter().filter(|m| m.ready > 0).count();
        let mut sp = vec![Span::styled("Σ ", Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD))];
        for (k, (c, col, _)) in &kinds {
            sp.push(Span::styled(format!("{}×{} ", k, c), Style::default().fg(*col).add_modifier(Modifier::BOLD)));
        }
        sp.push(Span::styled(format!("· util {:.0}% ", avg), Style::default().fg(util_color(avg))));
        sp.push(Span::styled(format!("· VRAM {:.0}/{:.0}GB ({:.0}%) ", mu, mt, mempct), Style::default().fg(mem_color(mempct))));
        sp.push(Span::styled(format!("· {:.0}W ", pw), Style::default().fg(C_DIM())));
        sp.push(Span::styled(format!("· models {}/{} ", ready, s.models.len()), Style::default().fg(if ready > 0 { C_OK() } else { C_DIM() })));
        sp.push(Span::styled(format!("· req/s {} ", rate(s.perf.req_rate)), Style::default().fg(C_DIM())));
        sp.push(Span::styled(format!("· TTFT {}", ms(s.perf.ttft_p95)), Style::default().fg(C_DIM())));
        cluster_lines.push(Line::from(sp));

        // VRAM 구성(벤더별 스택 바 + free) — 이종 가속기 메모리 점유를 한눈에.
        if mt > 0.0 {
            let barw = ((area.width as usize).saturating_sub(24)).clamp(10, 48);
            let segs: Vec<(f64, Color)> = kinds.values().map(|(_, col, m)| (*m, *col)).collect();
            let mut vsp = vec![Span::styled(format!("{:<6}", "VRAM"), Style::default().fg(C_DIM()))];
            vsp.extend(stacked_bar(&segs, mt, barw));
            vsp.push(Span::styled(format!(" {:.0}/{:.0}GB used", mu, mt), Style::default().fg(C_DIM())));
            cluster_lines.push(Line::from(vsp));
        }

        // all-smi 식 LED 그리드: 디바이스 1개=글리프 1개. vendor=색, util=●채움/○유휴, dead=✗, throttle=⚠.
        // 폭 초과 시 다음 줄로 감싸고(라벨 폭만큼 들여쓰기), 큰 fleet 대비 최대 줄 수 제한.
        const MAX_LED_LINES: usize = 8;
        const LABEL_W: usize = 5; // "{:<4} "
        let iw = (area.width as usize).saturating_sub(2); // 카드 내부 폭(테두리 제외)
        let per_line = iw.saturating_sub(LABEL_W) / 2; // 글리프 "● " = 2칸씩
        let per_line = per_line.max(1);
        let mut bykind: std::collections::BTreeMap<&str, Vec<&crate::collect::Accel>> = std::collections::BTreeMap::new();
        for a in &s.accel {
            bykind.entry(a.disp()).or_default().push(a);
        }
        let mut led_lines: Vec<Line> = Vec::new();
        'kinds: for (k, list) in &bykind {
            let kc = kind_color(list[0].kind);
            let mut cur: Vec<Span> = vec![Span::styled(format!("{:<4} ", k), Style::default().fg(kc).add_modifier(Modifier::BOLD))];
            let mut n = 0usize;
            for a in list {
                if n == per_line {
                    led_lines.push(Line::from(std::mem::take(&mut cur)));
                    if led_lines.len() >= MAX_LED_LINES {
                        break 'kinds;
                    }
                    cur.push(Span::raw(" ".repeat(LABEL_W))); // 연속줄: 라벨 폭 들여쓰기
                    n = 0;
                }
                let (g, c) = if !a.alive {
                    ("✗", C_BAD())
                } else if a.throttle > 0.0 {
                    ("⚠", C_WARN())
                } else if a.util > IDLE_UTIL {
                    ("●", kc)
                } else {
                    ("○", C_DIM())
                };
                cur.push(Span::styled(format!("{} ", g), Style::default().fg(c)));
                n += 1;
            }
            if cur.len() > 1 {
                led_lines.push(Line::from(cur));
            }
            if led_lines.len() >= MAX_LED_LINES {
                break;
            }
        }
        if led_lines.is_empty() {
            led_lines.push(Line::from(Span::styled("(no accelerators)", Style::default().fg(C_DIM()))));
        }
        cluster_lines.extend(led_lines);
    }
    let cluster_h = cluster_lines.len() as u16 + 2; // 내용 줄 + 테두리(2)

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(cluster_h), Constraint::Length(6), Constraint::Length(5), Constraint::Min(4), Constraint::Length(3)])
        .split(area);
    f.render_widget(Paragraph::new(cluster_lines).block(block("Cluster")), rows[0]);

    // 가속기: (종류,노드)별 집계 — 한눈에 + 절대 메모리(GB) + health 아이콘
    let mut groups: Vec<(AccelKind, String, usize, f64, f64, f64, bool, bool, String)> = Vec::new();
    for a in &s.accel {
        if let Some(g) = groups.iter_mut().find(|g| g.0 == a.kind && g.1 == a.node) {
            g.2 += 1; g.3 += a.util; g.4 += a.mem_used_gb; g.5 += a.mem_total_gb;
            g.6 = g.6 && a.alive; g.7 = g.7 || a.throttle > 0.0;
        } else {
            groups.push((a.kind, a.node.clone(), 1, a.util, a.mem_used_gb, a.mem_total_gb, a.alive, a.throttle > 0.0, a.disp().to_string()));
        }
    }
    let mut al: Vec<Line> = Vec::new();
    for (kind, node, cnt, us, mu, mt, alive, thr, model) in &groups {
        let util = us / (*cnt as f64);
        let mempct = if *mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
        let (hi, hc) = if !*alive { ("✗", C_BAD()) } else if *thr { ("⚠", C_WARN()) } else { ("●", C_OK()) };
        let mut sp = vec![
            Span::styled(format!("{} ", hi), Style::default().fg(hc)),
            Span::styled(format!("{:<4}×{} ", model, cnt), Style::default().fg(kind_color(*kind)).add_modifier(Modifier::BOLD)),
            Span::styled(format!("@{:<16} ", truncw(node, 16)), Style::default().fg(C_DIM())),
        ];
        sp.extend(grad_bar(util, 10).spans);
        sp.push(Span::styled(format!(" {:>3.0}%  ", util), Style::default().fg(util_color(util))));
        sp.push(Span::styled(format!("mem {:.0}/{:.0} GB  ", mu, mt), Style::default().fg(mem_color(mempct)))); // 절대값
        let trend = sparkstr(&app.hist_for(&format!("sys:{}_util", kind.label())), 14, 100); // all-smi식 인라인 트렌드
        sp.push(Span::styled(trend, Style::default().fg(util_color(util))));
        al.push(Line::from(sp));
    }
    if al.is_empty() {
        al.push(Line::from(Span::styled("  (no accelerator metrics)", Style::default().fg(C_DIM()))));
    }
    f.render_widget(Paragraph::new(al).block(block("Accelerators (by kind / node)")), rows[1]);

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
    f.render_widget(Paragraph::new(pl).block(block("Inference (EPP / InferencePool)")), rows[2]);

    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(models_table(app, "Models"), rows[3], &mut st);

    let (txt, col) = diagnose(s);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(truncw(&txt, rows[4].width.saturating_sub(2) as usize), Style::default().fg(col))))
            .block(block("Diagnosis")),
        rows[4],
    );
}

// ── Detail (drill-down) ────────────────────────────────
fn detail_panel(f: &mut Frame, area: Rect, app: &App) {
    let (cur, tot) = app.detail_pos();
    let (prev, next) = app.neighbor_names();
    let nav = format!(" · ◂ {}  {}/{}  {} ▸ · esc back", truncw(&prev, 16), cur, tot, truncw(&next, 16));
    // Accelerator: info + util/mem/temp timeline
    if let Some(a) = app.selected_accel() {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(11), Constraint::Min(6)]).split(area);
        let mempct = if a.mem_total_gb > 0.0 { a.mem_used_gb / a.mem_total_gb * 100.0 } else { 0.0 };
        let health = if !a.alive {
            ("✗ not alive", C_BAD())
        } else if a.throttle > 0.0 {
            ("⚠ throttling", C_WARN())
        } else {
            ("● healthy", C_OK())
        };
        // 헤더 + 현재 포션 게이지(all-smi식)
        let barw = (rows[0].width as usize).saturating_sub(34).clamp(10, 46);
        let mut lines = vec![
            Line::from(vec![
                Span::styled(format!("{} ", a.disp()), Style::default().fg(kind_color(a.kind)).add_modifier(Modifier::BOLD)),
                Span::styled(format!("id {} @ {}   ", a.id, a.node), Style::default().fg(C_DIM())),
                Span::styled(health.0, Style::default().fg(health.1).add_modifier(Modifier::BOLD)),
                Span::styled(format!("   {:.0} W", a.power), Style::default().fg(C_DIM())),
                Span::styled(format!("   {}", if a.busy_model.is_empty() { "(idle)" } else { a.busy_model.as_str() }), Style::default().fg(C_ACC())),
            ]),
            Line::from(""),
            gauge_row("compute", a.util, &format!("{:.0} %", a.util), util_color(a.util), barw),
            gauge_row(
                if a.unified_mem { "mem∪" } else { "VRAM" },
                mempct,
                &format!(
                    "{:.1} / {:.1} GB  ({:.0}%){}",
                    a.mem_used_gb, a.mem_total_gb, mempct,
                    if a.unified_mem { "  unified w/ host" } else { "" }
                ),
                mem_color(mempct),
                barw,
            ),
            gauge_row("temp", a.temp.min(100.0), &format!("{:.0} °C", a.temp), temp_color(a.temp), barw),
        ];
        // 메모리 대역폭(통합 메모리에선 진짜 병목) — DCGM MEM_COPY_UTIL. 있을 때만.
        if !a.mem_bw.is_nan() {
            lines.push(gauge_row("mem bw", a.mem_bw, &format!("{:.0} %", a.mem_bw), grad_color(a.mem_bw), barw));
        }
        if !a.clock_mhz.is_nan() || !a.mem_temp.is_nan() {
            lines.push(Line::from(vec![
                Span::styled(format!("{:<8} ", "clock"), Style::default().fg(C_DIM())),
                Span::styled(
                    if a.clock_mhz.is_nan() { "–".into() } else { format!("{:.0} MHz", a.clock_mhz) },
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled("    mem temp ", Style::default().fg(C_DIM())),
                Span::styled(
                    if a.mem_temp.is_nan() { "–".into() } else { format!("{:.0} °C", a.mem_temp) },
                    Style::default().fg(temp_color(a.mem_temp)),
                ),
            ]));
        }
        f.render_widget(Paragraph::new(lines).block(block(&format!("Accelerator{}", nav))), rows[0]);
        // 타임라인: util% / VRAM% 두 개만 넓게(반응형). temp/power 는 위 게이지로.
        let k = format!("acc:{}:{}:{}", a.kind.label(), a.node, a.id);
        let (l, r) = two_panes(rows[1], 50);
        bar_timeline(f, l, app, &format!("{}:util", k), "compute util", "%", Some(100.0));
        bar_timeline(f, r, app, &format!("{}:mem", k), "VRAM", "%", Some(100.0));
        return;
    }
    // Node: info + cpu/mem/load timeline
    if let Some(n) = app.selected_node() {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(9), Constraint::Min(4), Constraint::Length(8)])
            .split(area);
        let (hg, hc) = if n.cordoned {
            ("⊘ cordoned", C_WARN())
        } else if !n.ready {
            ("✗ not ready", C_BAD())
        } else if n.pressure {
            ("⚠ pressure", C_WARN())
        } else {
            ("● ready", C_OK())
        };
        let mempct = if n.mem_total_gb > 0.0 { n.mem_used_gb / n.mem_total_gb * 100.0 } else { 0.0 };
        let barw = (rows[0].width as usize).saturating_sub(34).clamp(10, 46);
        let lines = vec![
            Line::from(vec![
                Span::styled(format!("{}  ", truncw(&n.name, 30)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(hg, Style::default().fg(hc).add_modifier(Modifier::BOLD)),
                Span::styled(format!("   kubelet {}", n.version), Style::default().fg(C_DIM())),
            ]),
            Line::from(""),
            gauge_row("cpu", if n.cpu_pct.is_nan() { 0.0 } else { n.cpu_pct }, &if n.cpu_pct.is_nan() { "–".into() } else { format!("{:.0} %", n.cpu_pct) }, util_color(n.cpu_pct.max(0.0)), barw),
            gauge_row("memory", mempct, &if n.mem_total_gb <= 0.0 { "–".into() } else { format!("{:.0} / {:.0} GB  ({:.0}%)", n.mem_used_gb, n.mem_total_gb, mempct) }, mem_color(mempct), barw),
            Line::from(vec![
                Span::styled(format!("{:<8} ", "load1"), Style::default().fg(C_DIM())),
                Span::styled(if n.load1.is_nan() { "–".into() } else { format!("{:.2}", n.load1) }, Style::default().fg(C_WARN()).add_modifier(Modifier::BOLD)),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).block(block(&format!("Node{}", nav))), rows[0]);
        // 이 노드가 가진 모든 디바이스(full 라인: util/mem/temp/pwr/bw/clock/model)
        let devs: Vec<&crate::collect::Accel> = app.snap.accel.iter().filter(|a| a.node == n.name).collect();
        let mut dl: Vec<Line> = Vec::new();
        if devs.is_empty() {
            dl.push(Line::from(Span::styled("(no accelerators on this node)", Style::default().fg(C_DIM()))));
        } else {
            let last = devs.len();
            for (j, a) in devs.iter().enumerate() {
                dl.push(accel_brief(a, if j + 1 == last { "└─" } else { "├─" }, true));
            }
        }
        f.render_widget(Paragraph::new(dl).block(block(&format!("devices on {} ({})", truncw(&n.name, 20), devs.len()))), rows[1]);
        let k = format!("nod:{}", n.name);
        let (l, r) = two_panes(rows[2], 50);
        bar_timeline(f, l, app, &format!("{}:cpu", k), "host cpu", "%", Some(100.0));
        bar_timeline(f, r, app, &format!("{}:mem", k), "host mem", "%", Some(100.0));
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut title = "Detail";
    if let Some(m) = app.selected_model() {
        title = "Model";
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
        lines.push(pivot_line(&[("p", "pods"), ("i", "infra"), ("r", "route"), ("e", "epp")]));
        lines.push(Line::from(Span::styled("  s = scale up/down", Style::default().fg(C_DIM()))));
    } else if let Some(p) = app.selected_pod() {
        title = "Pod";
        lines.push(kv("pod", &p.name, Color::White));
        lines.push(kv("phase", &p.phase, if p.phase == "Running" { C_OK() } else { C_DIM() }));
        lines.push(kv("ready", &p.ready, Color::White));
        lines.push(kv("node", &p.node, Color::White));
        lines.push(kv("restarts", &p.restarts.to_string(), if p.restarts > 0 { C_WARN() } else { Color::White }));
        lines.push(pivot_line(&[("i", "infra"), ("m", "model")]));
    } else {
        lines.push(Line::from(Span::styled("no item selected", Style::default().fg(C_DIM()))));
    }

    f.render_widget(
        Paragraph::new(lines).scroll((app.detail_scroll, 0)).wrap(Wrap { trim: false }).block(block(&format!("{}{}", title, nav))),
        area,
    );
}

/// 드릴 pivot 안내 줄 — `pivot  [p] pods  [i] infra …`. 상세 패널·크로스레이어 내비 광고.
fn pivot_line(pivots: &[(&str, &str)]) -> Line<'static> {
    let mut sp = vec![Span::styled("pivot  ", Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD))];
    for (k, label) in pivots {
        sp.push(Span::styled(format!("[{}]", k), Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)));
        sp.push(Span::styled(format!(" {}  ", label), Style::default().fg(C_DIM())));
    }
    Line::from(sp)
}

fn kv(k: &str, v: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<18} ", k), Style::default().fg(C_DIM())),
        Span::styled(v.to_string(), Style::default().fg(color)),
    ])
}

/// Perf 드릴다운 — 선택 모델 구간별 p50/p95/p99 + E2E 지연 버킷 히스토그램(온디맨드).
fn perf_detail_view(f: &mut Frame, area: Rect, d: &PerfDetail) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(4)])
        .split(area);
    // 구간별 percentile 테이블
    let qrow = |label: &str, a: &[f64; 3], col: Color| {
        Row::new(vec![
            Cell::from(Span::styled(label.to_string(), Style::default().fg(C_DIM()))),
            Cell::from(Span::styled(ms(a[0]), Style::default().fg(col))),
            Cell::from(Span::styled(ms(a[1]), Style::default().fg(col))),
            Cell::from(Span::styled(ms(a[2]), Style::default().fg(col).add_modifier(Modifier::BOLD))),
        ])
    };
    let qt = Table::new(
        vec![qrow("TTFT", &d.ttft, C_ACC()), qrow("TPOT", &d.tpot, C_DECODE()), qrow("E2E", &d.e2e, C_WARN())],
        [Constraint::Length(8), Constraint::Length(10), Constraint::Length(10), Constraint::Length(10)],
    )
    .header(hrow(&["METRIC", "p50", "p95", "p99"]))
    .column_spacing(2)
    .block(block(&format!("latency percentiles · {} · esc back", truncw(&d.model, 30))));
    f.render_widget(qt, rows[0]);

    // E2E 지연 버킷 분포(히스토그램) — 누적차 rate, 바 길이 = 상대 빈도.
    let maxc = d.buckets.iter().map(|(_, c)| *c).fold(0.0f64, f64::max).max(1e-9);
    let mut hl: Vec<Line> = Vec::new();
    if d.buckets.iter().all(|(_, c)| *c <= 0.0) {
        hl.push(Line::from(Span::styled("no request samples in the window (idle) — populates under traffic", Style::default().fg(C_DIM()))));
    } else {
        for (le, c) in &d.buckets {
            if *c <= 0.0 {
                continue;
            }
            let lbl = if le.is_infinite() { "  ∞".to_string() } else { format!("≤{}", ms(*le)) };
            let barw = ((c / maxc) * 34.0).round() as usize;
            hl.push(Line::from(vec![
                Span::styled(format!("{:>9} ", lbl), Style::default().fg(C_DIM())),
                Span::styled("█".repeat(barw), Style::default().fg(C_ACC())),
                Span::styled(format!(" {:.2}/s", c), Style::default().fg(C_DIM())),
            ]));
        }
    }
    f.render_widget(Paragraph::new(hl).block(block("E2E latency distribution · rate by bucket")), rows[1]);
}

// ── Perf (EPP 정책용 성능/분배) ─────────────────────────
fn ms(v: f64) -> String {
    if v.is_nan() { "–".into() } else if v >= 1.0 { format!("{:.2}s", v) } else { format!("{:.0}ms", v * 1000.0) }
}
fn rate(v: f64) -> String {
    if v.is_nan() { "–".into() } else { format!("{:.2}", v) }
}
/// 채움(area-fill) 타임라인. 컬럼당 값 1개를 세로 블록(▁▂▃▄▅▆▇█, 8단계)으로 채우고
/// 높이(값)에 따라 green→yellow→red 심각도색을 입힘(btop/nvtop식). 최신값을 오른쪽(now)에 고정.
/// 외부 크레이트 없이 프레임 버퍼에 직접 렌더(순수 Rust 원칙). ymax_opt=Some(100)→0~100 고정.
fn bar_timeline(f: &mut Frame, area: Rect, app: &App, key: &str, label: &str, unit: &str, ymax_opt: Option<f64>) {
    const LV: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let raw = app.hist_for(key);
    let cur = raw.last().copied().unwrap_or(0);
    let dmax = raw.iter().copied().max().unwrap_or(0);
    let ymax = ymax_opt.unwrap_or_else(|| nice_ceil((dmax as f64) * 1.1)).max(1.0);
    let cur_pct = (cur as f64 / ymax * 100.0).clamp(0.0, 100.0);
    let ttl = Line::from(vec![
        Span::styled(format!(" {} ", label), Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)),
        Span::styled(format!("▏ now {}{} ", cur, unit), Style::default().fg(grad_color(cur_pct)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("▏ max {}{} ", dmax, unit), Style::default().fg(C_DIM())),
    ]);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_TRACK()))
        .title(ttl);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    if inner.width == 0 || inner.height == 0 || raw.is_empty() {
        return;
    }
    let w = inner.width as usize;
    let rows_h = inner.height as usize;
    // 컬럼당 최신 값 1개(오른쪽 정렬). 폭보다 데이터가 많으면 최근 w개만.
    let data: Vec<u64> = raw.iter().rev().take(w).rev().copied().collect();
    let start_x = inner.x + inner.width - data.len() as u16; // 오른쪽 정렬(now)
    let buf = f.buffer_mut();
    for (ci, &v) in data.iter().enumerate() {
        let frac = (v as f64 / ymax).clamp(0.0, 1.0);
        let eighths_total = (frac * (rows_h as f64) * 8.0).round() as usize; // 전체 채움(1/8칸 단위)
        let color = grad_color(frac * 100.0);
        let x = start_x + ci as u16;
        for r in 0..rows_h {
            // r=0 = 맨 아래 행
            let filled = eighths_total.saturating_sub(r * 8).min(8);
            if filled == 0 {
                continue;
            }
            let y = inner.y + inner.height - 1 - r as u16;
            buf[(x, y)].set_char(LV[filled]).set_fg(color);
        }
    }
}

/// 1/2/5 ×10^n 로 올림(축 상한을 깔끔하게).
fn nice_ceil(v: f64) -> f64 {
    if v <= 1.0 {
        return 1.0;
    }
    let mag = 10f64.powf(v.log10().floor());
    let n = v / mag;
    let step = if n <= 1.0 { 1.0 } else if n <= 2.0 { 2.0 } else if n <= 5.0 { 5.0 } else { 10.0 };
    step * mag
}


fn view_perf(f: &mut Frame, area: Rect, app: &App) {
    // 드릴: 선택 모델 지연 분포(Enter). perf_detail 이 채워져 있으면 그것부터.
    if app.detail {
        if let Some(d) = &app.perf_detail {
            perf_detail_view(f, area, d);
            return;
        }
    }
    let p = &app.snap.perf;
    let any = [p.e2e_p95, p.ttft_p95, p.tps, p.req_rate].iter().any(|x| !x.is_nan());

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Length(3), Constraint::Min(5)])
        .split(area);

    // 실제 존재하는 것만 동적으로: 가속기 종류별(이름+수) util/mem + host cpu/mem
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for a in &app.snap.accel {
        *counts.entry(a.kind.label()).or_default() += 1;
    }
    let mut charts: Vec<(String, String)> = Vec::new();
    for (k, n) in &counts {
        charts.push((format!("sys:{}_util", k), format!("{} util x{}", k, n)));
        charts.push((format!("sys:{}_mem", k), format!("{} mem x{}", k, n)));
    }
    charts.push(("sys:cpu".to_string(), "CPU util (host)".to_string()));
    charts.push(("sys:host_mem".to_string(), "host mem".to_string()));
    let cols = if rows[0].width >= 100 { 3 } else { 2 };
    let nrows = charts.len().div_ceil(cols).max(1);
    let grid_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, nrows as u32); nrows])
        .split(rows[0]);
    for (i, (key, label)) in charts.iter().enumerate() {
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, cols as u32); cols])
            .split(grid_rows[i / cols]);
        bar_timeline(f, cells[i % cols], app, key, label, "%", Some(100.0));
    }

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
    let (bodyc_l, bodyc_r) = two_panes(rows[2], 72);

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
            bodyc_l,
        );
    } else {
        let mrows: Vec<Row> = app
            .snap
            .perf_rows
            .iter()
            .map(|r| {
                let preempt_cell = if r.preempt.is_nan() || r.preempt <= 0.0 {
                    Cell::from(Span::styled("·", Style::default().fg(C_DIM())))
                } else {
                    Cell::from(Span::styled(format!("{:.2}", r.preempt), Style::default().fg(C_BAD())))
                };
                Row::new(vec![
                    cellw(r.model.clone(), 16),
                    Cell::from(Span::styled(rate(r.req), Style::default().fg(C_OK()))),
                    Cell::from(Span::styled(rate(r.tps), Style::default().fg(C_OK()))),
                    cellw(ms(r.ttft_p95), 7),
                    Cell::from(Span::styled(ms(r.queue_p95), Style::default().fg(C_WARN()))), // 대기
                    Cell::from(Span::styled(ms(r.prefill_p95), Style::default().fg(C_PREFILL()))), // P
                    Cell::from(Span::styled(ms(r.decode_p95), Style::default().fg(C_DECODE()))), // D
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
        .header(hrow(&["MODEL", "req/s", "tok/s", "TTFT", "QUEUE", "PFILL", "DECODE", "TPOT", "E2E", "premt"]))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block(&format!("Per-model perf (p95) · ⏎ p50/p95/p99 + histogram{}", count_suffix(app.selected, app.snap.perf_rows.len()))));
        let mut st = TableState::default();
        st.select(Some(app.selected));
        f.render_stateful_widget(mt, bodyc_l, &mut st);
        list_scrollbar(f, bodyc_l, app.snap.perf_rows.len(), app.selected, 1);
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
    f.render_widget(Paragraph::new(ql).block(block("request distribution (per-pod queue, absolute)")), bodyc_r);
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

    let (body_l, body_r) = two_panes(rows[1], 40);

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
        .block(block_active(&format!("Catalog · up/dn to select{}", count_suffix(app.selected, app.catalog.len()))));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(lt, body_l, &mut st);
    list_scrollbar(f, body_l, app.catalog.len(), app.selected, 1);

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
    f.render_widget(Paragraph::new(pl).block(block("placements × live inventory")), body_r);
}

// ── Nodes (health / placement) — all-smi 식 트리: node → devices ──────
fn node_status(n: &crate::collect::NodeInfo) -> (&'static str, Color) {
    if n.cordoned {
        ("⊘", C_WARN())
    } else if !n.ready {
        ("✗", C_BAD())
    } else if n.pressure {
        ("⚠", C_WARN())
    } else {
        ("●", C_OK())
    }
}

/// 트리/상세용 디바이스 1줄. full=true 면 mem-bw/clock 도 붙임(상세).
fn accel_brief(a: &crate::collect::Accel, branch: &str, full: bool) -> Line<'static> {
    let mempct = if a.mem_total_gb > 0.0 { a.mem_used_gb / a.mem_total_gb * 100.0 } else { 0.0 };
    let (hg, hc) = if !a.alive { ("✗", C_BAD()) } else if a.throttle > 0.0 { ("⚠", C_WARN()) } else { ("●", C_OK()) };
    let mut sp = vec![
        Span::styled(format!("   {} ", branch), Style::default().fg(C_TRACK())),
        Span::styled(format!("{} ", hg), Style::default().fg(hc)),
        Span::styled(format!("{:<5}", a.disp()), Style::default().fg(kind_color(a.kind)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<6} ", a.id), Style::default().fg(C_DIM())),
    ];
    sp.extend(grad_bar(a.util, 8).spans);
    sp.push(Span::styled(format!(" {:>3.0}%", a.util), Style::default().fg(util_color(a.util))));
    sp.push(Span::styled(
        format!("  {:.0}/{:.0}GB{}", a.mem_used_gb, a.mem_total_gb, if a.unified_mem { "∪" } else { "" }),
        Style::default().fg(mem_color(mempct)),
    ));
    sp.push(Span::styled(format!("  {:.0}°C", a.temp), Style::default().fg(temp_color(a.temp))));
    sp.push(Span::styled(format!("  {:>3.0}W", a.power), Style::default().fg(C_DIM())));
    if full && !a.mem_bw.is_nan() {
        sp.push(Span::styled(format!("  bw {:>3.0}%", a.mem_bw), Style::default().fg(grad_color(a.mem_bw))));
        if !a.clock_mhz.is_nan() {
            sp.push(Span::styled(format!("  {:.0}MHz", a.clock_mhz), Style::default().fg(C_DIM())));
        }
    }
    if !a.busy_model.is_empty() {
        sp.push(Span::styled(format!("  {}", truncw(&a.busy_model, if full { 40 } else { 26 })), Style::default().fg(C_ACC())));
    }
    Line::from(sp)
}

fn view_nodes(f: &mut Frame, area: Rect, app: &App) {
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
        let memp = if n.mem_total_gb > 0.0 { n.mem_used_gb / n.mem_total_gb * 100.0 } else { 0.0 };
        let mut h = vec![
            Span::styled(if selected { "▎" } else { " " }, Style::default().fg(C_ACC())),
            Span::styled(format!("{} ", glyph), Style::default().fg(gc)),
            Span::styled(format!("{:<20} ", truncw(&n.name, 20)), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ];
        h.extend(grad_bar(if n.cpu_pct.is_nan() { 0.0 } else { n.cpu_pct }, 8).spans);
        h.push(Span::styled(
            if n.cpu_pct.is_nan() { " cpu   –".into() } else { format!(" cpu{:>3.0}%", n.cpu_pct) },
            Style::default().fg(C_DIM()),
        ));
        h.push(Span::styled(
            if n.mem_total_gb <= 0.0 { "  mem –".into() } else { format!("  mem {:.0}/{:.0}GB", n.mem_used_gb, n.mem_total_gb) },
            Style::default().fg(mem_color(memp)),
        ));
        h.push(Span::styled(
            if n.load1.is_nan() { "  load –".into() } else { format!("  load {:.1}", n.load1) },
            Style::default().fg(C_DIM()),
        ));
        let mut hline = Line::from(h);
        if selected {
            hline = hline.style(Style::default().bg(C_HL()).add_modifier(Modifier::BOLD));
        }
        lines.push(hline);
        // 이 노드의 디바이스들(트리 자식)
        let devs: Vec<&crate::collect::Accel> = app.snap.accel.iter().filter(|a| a.node == n.name).collect();
        if devs.is_empty() {
            lines.push(Line::from(Span::styled("   └─ (no accelerators)", Style::default().fg(C_TRACK()))));
        } else {
            let last = devs.len();
            for (j, a) in devs.iter().enumerate() {
                lines.push(accel_brief(a, if j + 1 == last { "└─" } else { "├─" }, false));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("(no nodes)", Style::default().fg(C_DIM()))));
    }
    // 선택 노드가 화면에 보이도록 세로 스크롤.
    let vis = (area.height as usize).saturating_sub(2);
    let scroll = if sel_line + 2 > vis { (sel_line + 3).saturating_sub(vis) as u16 } else { 0 };
    f.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .block(block(&format!("Nodes · node → devices · ⏎ detail{}", count_suffix(sel, order.len())))),
        area,
    );
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
        .block(block(&format!("Events (k8s + llm-d, newest first){}", count_suffix(app.selected, order.len()))));
    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(t, area, &mut st);
    list_scrollbar(f, area, order.len(), app.selected, 1);
}

// ── 진단 ───────────────────────────────────────────────
// 판정 로직은 app::diagnose(agent JSON 과 공유). 여기선 글리프+색만 입힌다.
fn diagnose(s: &Snapshot) -> (String, Color) {
    let (msg, sev) = crate::app::diagnose(s);
    let (glyph, col) = match sev {
        Some(Sev::Bad) => ("⚠", C_BAD()),
        Some(Sev::Warn) => ("⚠", C_WARN()),
        None => ("●", C_OK()),
    };
    (format!("{} {}", glyph, msg), col)
}
