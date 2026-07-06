//! ratatui 렌더링 — 헤더/탭/본문(뷰별, 정렬·상세 포함)/footer.
//! 모든 문자열은 표시 폭(unicode-width) 기준으로 절단해 CJK/와이드 글자 깨짐을 방지.
//! 선택 하이라이트는 REVERSED 대신 은은한 배경색(htop/all-smi 스타일).

use crate::app::{App, Mode, Sev, View};
use crate::collect::{AccelKind, PerfDetail, Snapshot};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap,
};

mod theme;
pub(crate) use theme::*;
mod widgets;
pub(crate) use widgets::*;
mod fx;
pub use fx::FxState;
mod panel;
pub(crate) use panel::Dashboard;
mod overlays;
use overlays::*;

/// 오버레이 종류 — z-order(그리기 순서)와 입력 우선순위의 **단일 출처**.
/// 지금까지 draw()의 그리기 순서와 main 의 입력 처리 순서가 따로 관리돼 드리프트 위험이 있었다.
/// 여기 PRECEDENCE 하나로 통일: 위(topmost)일수록 나중에 그려지고(=화면 맨 앞), 키를 먼저 소비.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    Help,
    ExitConfirm,
    Confirm,
    Palette,
    Alerts,
    Preview,
    RouteForm,
    ObjectiveForm,
    PlacePicker,
    DeployForm,
    CompileForm,
    ActionMenu,
    Logs,
}

impl Overlay {
    /// topmost 우선. 입력 소비·z-order 공용. (Help 가 가장 위, Logs 가 가장 아래)
    pub const PRECEDENCE: [Overlay; 13] = [
        Overlay::Help,
        Overlay::ExitConfirm,
        Overlay::Confirm,
        Overlay::Palette,
        Overlay::Alerts,
        Overlay::Preview,
        Overlay::RouteForm,
        Overlay::ObjectiveForm,
        Overlay::PlacePicker,
        Overlay::DeployForm,
        Overlay::CompileForm,
        Overlay::ActionMenu,
        Overlay::Logs,
    ];

    /// 이 오버레이가 현재 열려 있는가(App 상태 기준).
    pub fn is_open(self, app: &App) -> bool {
        match self {
            Overlay::Help => app.help,
            Overlay::ExitConfirm => app.exit_confirm,
            Overlay::Confirm => app.confirm.is_some(),
            Overlay::Palette => app.palette.is_some(),
            Overlay::Alerts => app.alerts_panel,
            Overlay::Preview => app.preview.is_some(),
            Overlay::RouteForm => app.route_form.is_some(),
            Overlay::ObjectiveForm => app.objective_form.is_some(),
            Overlay::PlacePicker => app.place_picker.is_some(),
            Overlay::DeployForm => app.deploy_form.is_some(),
            Overlay::CompileForm => app.compile_form.is_some(),
            Overlay::ActionMenu => app.action_menu.is_some(),
            Overlay::Logs => app.logs_mode,
        }
    }

    /// 현재 최상위(키를 소비할) 오버레이. 없으면 None(단일키 디스패치로).
    pub fn top(app: &App) -> Option<Overlay> {
        Overlay::PRECEDENCE
            .iter()
            .copied()
            .find(|ov| ov.is_open(app))
    }

    fn render(self, f: &mut Frame, app: &App) {
        match self {
            Overlay::Help => help_overlay(f),
            Overlay::ExitConfirm => exit_confirm_overlay(f, app),
            Overlay::Confirm => confirm_overlay(f, app),
            Overlay::Palette => palette_overlay(f, app),
            Overlay::Alerts => alerts_overlay(f, app),
            Overlay::Preview => preview_overlay(f, app),
            Overlay::RouteForm => route_form_overlay(f, app),
            Overlay::ObjectiveForm => objective_form_overlay(f, app),
            Overlay::PlacePicker => place_picker_overlay(f, app),
            Overlay::DeployForm => deploy_form_overlay(f, app),
            Overlay::CompileForm => compile_form_overlay(f, app),
            Overlay::ActionMenu => action_menu_overlay(f, app),
            Overlay::Logs => logs_overlay(f, app),
        }
    }
}

pub fn draw(f: &mut Frame, app: &App, fxs: &mut FxState) {
    let dt = fxs.begin(app); // 경과시간 + 상태변화 감지(이펙트 무장)
    let (body, footer_area, summary) = if app.zoom {
        // 포커스 모드: 헤더/탭 숨기고 본문 최대화
        let c = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(f.area());
        title_bar(f, c[0], app);
        (c[1], c[2], None)
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
        (c[3], c[4], Some(c[1]))
    };
    if app.detail
        && matches!(
            app.view,
            View::Accel
                | View::Overview
                | View::Pods
                | View::Nodes
                | View::Events
                | View::Serving
                | View::Library
        )
    {
        detail_panel(f, body, app);
    } else {
        match app.view {
            View::Overview => view_overview(f, body, app),
            View::Accel => view_accel(f, body, app),
            View::Epp => view_epp(f, body, app),
            View::Routing => view_routing(f, body, app),
            View::Pods => view_pods(f, body, app),
            View::Perf => view_perf(f, body, app),
            View::Serving => view_serving(f, body, app),
            View::Library => view_library(f, body, app),
            View::Events => view_events(f, body, app),
            View::Nodes => view_nodes(f, body, app),
            View::Topo => view_topo(f, body, app),
            View::Zoo => view_zoo(f, body, app),
            View::Setup => view_setup(f, body, app),
        }
    }
    fxs.body(f, body, dt); // 본문 트랜지션(오버레이 전에)
    footer(f, footer_area, app);
    if let Some(sa) = summary {
        fxs.flash(f, sa, dt); // 신규 알림 플래시
    }
    // 오버레이 — 단일 우선순위(Overlay::PRECEDENCE)로 아래→위 순서로 그린다.
    // (역순 순회 = 가장 낮은 것 먼저 그림 → topmost 가 화면 맨 앞. 입력 우선순위와 동일 정의.)
    for ov in Overlay::PRECEDENCE.iter().rev().copied() {
        if ov.is_open(app) {
            ov.render(f, app);
        }
    }
}

/// Mutation confirmation popup. Defaults to No; ←→ selects Yes/No, Enter confirms.
fn confirm_overlay(f: &mut Frame, app: &App) {
    let Some(pending) = &app.confirm else { return };
    let full = f.area();
    let area = centered(full, 80, 9);
    f.render_widget(Clear, area);
    let yes = app.confirm_yes;
    let btn = |label: &str, on: bool| {
        if on {
            Span::styled(
                format!("  {}  ", label),
                Style::default()
                    .fg(Color::Black)
                    .bg(C_ACC())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("  {}  ", label), Style::default().fg(C_DIM()))
        }
    };
    let is_apply = matches!(pending, crate::app::Pending::Apply { .. });
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", pending.prompt()),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    if is_apply {
        lines.push(Line::from(Span::styled(
            "  Applies a generated manifest · e=edit in vi · v=validate dry-run",
            Style::default().fg(C_DIM()),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Run this operation? Default selection is No.",
            Style::default().fg(C_DIM()),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("      "),
        btn("Yes run", yes),
        Span::raw("        "),
        btn("No cancel", !yes),
    ]));
    let title = if is_apply {
        "confirm apply · default No · ←→ select · Enter confirm · e edit · v validate · Esc cancel"
    } else {
        "confirm · default No · ←→ select · Enter confirm · Esc cancel"
    };
    f.render_widget(
        Paragraph::new(lines).block(block(title).border_style(Style::default().fg(C_WARN()))),
        area,
    );
}

/// Quit confirmation popup. Opened by `q`; Enter/y exits, Esc/n returns to the TUI.
fn exit_confirm_overlay(f: &mut Frame, _app: &App) {
    let full = f.area();
    let area = centered(full, 58, 7);
    f.render_widget(Clear, area);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Quit lmd-top?",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  The terminal will be restored and the session will end.",
            Style::default().fg(C_DIM()),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("      "),
            Span::styled(
                "  Enter / y  Quit  ",
                Style::default()
                    .fg(Color::Black)
                    .bg(C_WARN())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled("Esc / n  Cancel", Style::default().fg(C_DIM())),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            block("quit confirmation · Enter/y quit · Esc/n cancel")
                .border_style(Style::default().fg(C_WARN())),
        ),
        area,
    );
}

/// 알림 히스토리 오버레이(A) — 최신 앞, 상대시각 + 심각도색.
fn alerts_overlay(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 78, 22);
    f.render_widget(Clear, area);
    let now = crate::collect::now_secs();
    let lines: Vec<Line> = if app.alerts.is_empty() {
        vec![Line::from(Span::styled(
            "  no alerts — all clear ●",
            Style::default().fg(C_OK()),
        ))]
    } else {
        app.alerts
            .iter()
            .map(|al| {
                let age = now.saturating_sub(al.ts);
                let (g, c) = if al.sev == Sev::Bad {
                    ("✗", C_BAD())
                } else {
                    ("⚠", C_WARN())
                };
                Line::from(vec![
                    Span::styled(
                        format!("  {} ", g),
                        Style::default().fg(c).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{:>4}s ago  ", age), Style::default().fg(C_DIM())),
                    Span::styled(
                        truncw(&al.msg, area.width.saturating_sub(18) as usize),
                        Style::default().fg(Color::White),
                    ),
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
                .title(Span::styled(
                    title,
                    Style::default().fg(C_BAD()).add_modifier(Modifier::BOLD),
                )),
        ),
        area,
    );
}

/// 2-패널: 넓으면 좌우, 좁으면(<100) 위아래로 — 반응형.
fn two_panes(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let dir = if area.width >= 100 {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };
    let c = Layout::default()
        .direction(dir)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(area);
    (c[0], c[1])
}

pub(super) fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w.min(area.width),
        height: h.min(area.height),
    }
}

fn help_overlay(f: &mut Frame) {
    let area = centered(f.area(), 68, 26);
    f.render_widget(Clear, area);
    let g = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(
                format!("  {:<10}", k),
                Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(d.to_string(), Style::default().fg(Color::White)),
        ])
    };
    let sec = |t: &str| {
        Line::from(Span::styled(
            format!(" {}", t),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        ))
    };
    let lines = vec![
        sec("navigate"),
        g(
            "0-6 / Tab",
            "switch section (Overview·Traffic·Serving·Infra·Deploy·Events·Setup); Shift+Tab back",
        ),
        g(
            "← / →  ([ ])",
            "cycle sub-tabs in the section (e.g. Models→Perf→Pods)",
        ),
        g(
            "Ctrl-w",
            "panel-focus mode → then h/j/k/l or arrows move, Esc exits",
        ),
        g("↑↓ j k", "move selection or scroll"),
        g("g / G", "jump to first / last row"),
        g("Ctrl-u/d", "half-page up / down"),
        g(
            "Enter / a",
            "open actions for the selection (detail when no menu)",
        ),
        g("Esc", "go back: detail → breadcrumb → filter → zoom"),
        g(
            "/ :",
            "filter rows · command palette (jump anywhere / run any action)",
        ),
        g("o / O", "cycle sort column / toggle direction"),
        sec("operations"),
        g("y / l", "YAML / logs (accelerators, also in the ⏎ menu)"),
        g("s S x", "scale / restart / stop"),
        g(
            "p i r e m",
            "pivot across pods, infra, routes, EPP, and models",
        ),
        g("A", "alert history"),
        g("R", "reset session energy"),
        g("t / f / z", "theme · animations · zoom (focus)"),
        g("Space", "pause/resume refresh   ( :graf opens Grafana )"),
        Line::from(""),
        sec("color / glyph"),
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(C_OK())),
            Span::styled("up  ", Style::default().fg(C_DIM())),
            Span::styled("○ ", Style::default().fg(C_DIM())),
            Span::styled("idle  ", Style::default().fg(C_DIM())),
            Span::styled("◐ ", Style::default().fg(C_WARN())),
            Span::styled("pending  ", Style::default().fg(C_DIM())),
            Span::styled("⚠ ", Style::default().fg(C_WARN())),
            Span::styled("throttle  ", Style::default().fg(C_DIM())),
            Span::styled("⊘ ", Style::default().fg(C_WARN())),
            Span::styled("cordoned  ", Style::default().fg(C_DIM())),
            Span::styled("✗ ", Style::default().fg(C_BAD())),
            Span::styled("down", Style::default().fg(C_DIM())),
        ]),
        Line::from(vec![
            Span::styled("  util/mem/temp: ", Style::default().fg(C_DIM())),
            Span::styled("low", Style::default().fg(C_OK())),
            Span::raw(" "),
            Span::styled("mid", Style::default().fg(C_WARN())),
            Span::raw(" "),
            Span::styled("high", Style::default().fg(C_BAD())),
            Span::styled(
                "   ∪ = unified memory (GB10 and similar shared CPU/GPU memory)",
                Style::default().fg(C_DIM()),
            ),
        ]),
        Line::from(vec![
            Span::styled("  vendor: ", Style::default().fg(C_DIM())),
            Span::styled("GPU ", Style::default().fg(Color::Green)),
            Span::styled("RBLN ", Style::default().fg(Color::Magenta)),
            Span::styled("RNGD", Style::default().fg(Color::Cyan)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(C_ACC()))
                .title(Span::styled(
                    " lmd-top · help (press any key to close) ",
                    Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
                )),
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
            let col = if low.contains("error")
                || low.contains("traceback")
                || low.contains("fatal")
                || low.contains("exception")
            {
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
                .title(Span::styled(
                    title,
                    Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
                )),
        ),
        area,
    );
    list_scrollbar(
        f,
        area,
        total,
        (app.logs_scroll as usize).min(total.saturating_sub(1)),
        0,
    );
}

// ── 헤더 ───────────────────────────────────────────────
fn title_bar(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let (tick, paused) = (app.tick, app.paused);
    let spin = if paused {
        "⏸"
    } else {
        SPINNER[(tick as usize) % SPINNER.len()]
    };
    let gw = if s.gw_addr.is_empty() {
        Span::styled("⌂ gw —", Style::default().fg(C_DIM()))
    } else if s.gw_ok {
        Span::styled(format!("⌂ gw {} ●", s.gw_addr), Style::default().fg(C_OK()))
    } else {
        Span::styled(
            format!("⌂ gw {} ○", s.gw_addr),
            Style::default().fg(C_WARN()),
        )
    };
    // 데이터 신선도: 마지막 스냅샷이 몇 초 전인지(수집 주기 3s). stale 판단용.
    let fresh = if s.ts == 0 {
        Span::styled("  · connecting…", Style::default().fg(C_DIM()))
    } else {
        let age = crate::collect::now_secs().saturating_sub(s.ts);
        let col = if age > 10 { C_WARN() } else { C_DIM() };
        Span::styled(
            format!("  · updated {}s ago", age),
            Style::default().fg(col),
        )
    };
    // 권한 모드 배지 — observe 는 은은하게, 상승 권한은 색+굵게(사고 방지 인지).
    let (mcol, mmod) = match app.mode {
        Mode::Observe => (C_DIM(), Modifier::empty()),
        Mode::Debug => (C_ACC(), Modifier::BOLD),
        Mode::Admin => (C_WARN(), Modifier::BOLD),
        Mode::Danger => (C_BAD(), Modifier::BOLD),
    };
    let mut spans = vec![
        Span::styled(
            format!("{} ", spin),
            Style::default().fg(if paused { C_WARN() } else { C_ACC() }),
        ),
        Span::styled(
            "lmd-top",
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" [{}]", app.mode.name()),
            Style::default().fg(mcol).add_modifier(mmod),
        ),
        Span::styled(
            format!("  llm-d · {} nodes  ", s.nodes.len()),
            Style::default().fg(C_DIM()),
        ),
        gw,
        fresh,
        Span::styled(
            if paused { "  ⏸ PAUSED (space)" } else { "" },
            Style::default().fg(C_WARN()),
        ),
    ];
    // 변경 작업 진행 중 — 스피너로 표시(UI 는 안 얼고 워커 스레드가 kube 수행).
    if let Some(label) = &app.inflight {
        let sp = SPINNER[(tick as usize) % SPINNER.len()];
        spans.push(Span::styled(
            format!("  {} {}", sp, truncw(label, 40)),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn summary_bar(f: &mut Frame, area: Rect, app: &App) {
    // 서빙/SLO 우선(왼쪽), 인프라는 뒤(오른쪽). "지금 서빙 건강한가?"가 항상 보이는 자리로.
    let s = &app.snap;
    let p = &s.perf;
    let (mut busy, mut mu, mut mt, mut pw) = (0usize, 0.0f64, 0.0f64, 0.0f64);
    for a in &s.accel {
        if a.util > IDLE_UTIL {
            busy += 1; // "busy" 기준을 LED/util_color 와 동일(IDLE_UTIL)하게 통일
        }
        mu += a.mem_used_gb;
        mt += a.mem_total_gb;
        pw += a.power;
    }
    let nacc = s.accel.len();
    let serving = s.serving_count();
    let total = s.models.len();
    let mempct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
    let err = p.err_rate;
    // 에러가 "의미있게" 있을 때만 경고: 비율(err/req) > 1% (또는 req 없는데 err 발생).
    let err_bad = !err.is_nan() && err > 0.0 && (err / p.req_rate.max(err) > 0.01);
    // 서빙 건강 글리프: 0서빙=✗, 유의미 에러=⚠, 아니면 ●.
    let (sg, sc) = if total == 0 {
        ("○", C_DIM())
    } else if serving == 0 {
        ("✗", C_BAD())
    } else if err_bad {
        ("⚠", C_WARN())
    } else {
        ("●", C_OK())
    };
    let mut spans = vec![
        Span::styled(
            format!("{} SERVING {}/{}  ", sg, serving, total),
            Style::default().fg(sc).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("req/s {}  ", rate(p.req_rate)),
            Style::default().fg(C_DIM()),
        ),
        Span::styled(
            format!("err {}  ", rate(err)),
            Style::default().fg(if err_bad { C_BAD() } else { C_DIM() }),
        ),
        Span::styled(
            format!("TTFT {}  ", ms(p.ttft_p95)),
            Style::default().fg(C_DIM()),
        ),
        Span::styled(
            format!("E2E {}  ", ms(p.e2e_p95)),
            Style::default().fg(C_DIM()),
        ),
        Span::raw("│ "),
        Span::styled(
            format!("accel {}/{} busy  ", busy, nacc),
            Style::default().fg(C_DIM()),
        ),
        Span::styled(
            format!("VRAM {:.0}%  ", mempct),
            Style::default().fg(mem_color(mempct)),
        ),
        Span::styled(format!("⚡{:.0}W", pw), Style::default().fg(C_DIM())),
    ];
    // Prometheus 도달 불가 — 빈 테이블이 "장비 없음"이 아니라 연결 문제임을 명시.
    if !s.prom_ok {
        spans.push(Span::styled(
            "  ⚠ Prometheus unreachable (check LMD_PROM)",
            Style::default().fg(C_BAD()).add_modifier(Modifier::BOLD),
        ));
    }
    // 활성 알림 카운트(A 로 히스토리)
    let nalert = app.active_alerts.len();
    if nalert > 0 {
        spans.push(Span::styled(
            format!("  ⚠{} alert (A)", nalert),
            Style::default().fg(C_BAD()).add_modifier(Modifier::BOLD),
        ));
    }
    let mut para = Paragraph::new(Line::from(spans));
    // 신규 알림 플래시: flash_until 이전이면 ~0.6s 주기로 요약바 전체를 반전.
    let now = crate::collect::now_secs();
    if now < app.flash_until && (app.tick / 3).is_multiple_of(2) {
        para = para.style(
            Style::default()
                .bg(C_BAD())
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );
    }
    f.render_widget(para, area);
}

fn tabs(f: &mut Frame, area: Rect, app: &App) {
    use crate::app::Section;
    let cur_sec = app.view.section();
    // Section tabs (0-6). Compact to number-only for inactive tabs when the strip won't fit.
    let full_w: usize = Section::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| format!(" {}:{} ", i, s.title()).len() + 1)
        .sum();
    let compact = full_w + 24 > area.width as usize;
    // 앞머리 마커 — Tab/0-6 로 섹션 전환됨을 명시.
    let mut spans: Vec<Span> = vec![Span::styled(
        "⇥ ",
        Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
    )];
    for (i, s) in Section::ALL.iter().enumerate() {
        let sel = *s == cur_sec;
        let st = if sel {
            Style::default()
                .fg(Color::Black)
                .bg(C_ACC())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(C_DIM())
        };
        let label = if sel || !compact {
            format!(" {}:{} ", i, s.title())
        } else {
            format!(" {} ", i)
        };
        spans.push(Span::styled(label, st));
        spans.push(Span::raw(" "));
    }
    // Sub-tab strip for the current section (only when it has >1 member).
    // `◂ … ▸` arrows signal ←/→ cycles; the active tab is a chip, siblings preview prev/next.
    let members = cur_sec.members();
    if members.len() > 1 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "◂ ",
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ));
        for (j, v) in members.iter().enumerate() {
            if j > 0 {
                spans.push(Span::styled(" · ", Style::default().fg(C_DIM())));
            }
            if *v == app.view {
                spans.push(Span::styled(
                    format!(" {} ", v.title()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(C_ACC())
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    v.title().to_string(),
                    Style::default().fg(C_DIM()),
                ));
            }
        }
        spans.push(Span::styled(
            " ▸",
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn compact_footer(parts: &[String], width: usize) -> String {
    let full = parts.join("  ");
    if dwidth(&full) <= width {
        return full;
    }
    let suffix_parts = ["? help", "q quit"];
    let suffix = suffix_parts.join("  ");
    let reserve = dwidth(&format!("  …  {}", suffix));
    let mut kept: Vec<String> = Vec::new();
    for p in parts {
        if suffix_parts.contains(&p.as_str()) {
            continue;
        }
        let candidate = if kept.is_empty() {
            p.clone()
        } else {
            format!("{}  {}", kept.join("  "), p)
        };
        if dwidth(&candidate) + reserve <= width {
            kept.push(p.clone());
        } else {
            break;
        }
    }
    if kept.is_empty() {
        truncw(&full, width)
    } else {
        format!("{}  …  {}", kept.join("  "), suffix)
    }
}

fn footer(f: &mut Frame, area: Rect, app: &App) {
    // vi/tmux panel-focus mode — a persistent banner so the user knows arrows now move panels.
    if app.panel_move {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    " panel focus {}/{} — h/j/k/l or ←↑↓→ move · Esc exits",
                    app.panel_focus + 1,
                    app.panel_count()
                ),
                Style::default()
                    .fg(Color::Black)
                    .bg(C_ACC())
                    .add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    if app.exit_confirm {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " quit confirmation — Enter/y quit · Esc/n cancel",
                Style::default().fg(C_WARN()).add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    // 확인은 이제 팝업(confirm_overlay)으로 표시 — 푸터에는 안내만.
    if app.confirm.is_some() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " confirm popup — default No · ←→ Yes/No · Enter confirm · Esc cancel",
                Style::default().fg(C_WARN()).add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    // 필터 입력 모드
    if app.filtering {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " / ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(C_ACC())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}", app.filter),
                    Style::default().fg(Color::White),
                ),
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
                Paragraph::new(Line::from(Span::styled(
                    format!(" {} ", msg),
                    Style::default().fg(Color::Black).bg(bg),
                ))),
                area,
            );
            return;
        }
    }
    let mut spans: Vec<Span> = Vec::new();
    if !app.filter.is_empty() {
        spans.push(Span::styled(
            format!("[filter: {}] ", app.filter),
            Style::default().fg(Color::Black).bg(C_ACC()),
        ));
        spans.push(Span::raw(" "));
    }
    // 컨텍스트 푸터: 현재 뷰가 실제 할 수 있는 액션만(no-op 숨김).
    use View::*;
    let v = app.view;
    let mut parts: Vec<String> = Vec::new();
    parts.push("↑↓ sel".into());
    match v {
        // Models/Overview/Pods/Launch 은 Enter=액션 메뉴(아래 "⏎ actions") 라 여기선 제외.
        Accel | Nodes | Events => parts.push("⏎ detail".into()),
        Perf => parts.push("⏎ p50/95/99".into()),
        _ => {}
    }
    if matches!(
        v,
        Accel | Overview | Pods | Serving | Library | Epp | Events | Nodes
    ) {
        parts.push("/ filter".into());
    }
    if app.sort_modes() > 1 {
        // 컬럼 + 방향(▼/▲). O 로 방향 토글.
        parts.push(format!(
            "o sort:{}{} O⇅",
            app.sort_label(),
            app.sort_arrow()
        ));
    }
    if app.panel_count() > 1 {
        parts.push(format!(
            "^w+hjkl panel {}/{}",
            app.panel_focus + 1,
            app.panel_count()
        ));
    }
    // Sub-tab strip: `←`/`→` (or `[`/`]`) cycle the current section's views (e.g. Models→Perf→Pods).
    let members = v.section().members();
    if members.len() > 1 {
        let strip: Vec<&str> = members.iter().map(|m| m.title()).collect();
        parts.push(format!("←→ {}", strip.join("·")));
    }
    // Enter/action behavior: views with action menus expose a single compact hint.
    let has_menu = matches!(v, Overview | Pods | Serving | Library | Routing);
    if has_menu {
        parts.push("a/⏎ actions".into());
    }
    // pivot(액션 메뉴 밖 — 크로스레이어 점프).
    match v {
        Overview | Serving => parts.push("p/i/r/e pivot".into()),
        Accel => parts.push("p/m/n pivot".into()),
        Pods => parts.push("i/m pivot".into()),
        Nodes => parts.push("i pivot".into()),
        Perf => parts.push("p/i/e pivot".into()),
        Routing => parts.push("p/i/m/e pivot".into()),
        Epp => parts.push("+/- weight".into()),
        _ => {}
    }
    // 액션 메뉴가 없는 뷰의 개별 액션만 표기.
    if matches!(v, Accel) {
        parts.push("l logs".into());
    }
    if matches!(v, Nodes) {
        parts.push("y yaml".into());
    }
    // 전역
    parts.push("⇥/0-6 section".into());
    parts.push("A alerts".into());
    parts.push("t theme".into());
    parts.push("? help".into());
    parts.push("q quit".into());
    spans.push(Span::styled(
        compact_footer(&parts, area.width.saturating_sub(1) as usize),
        Style::default().fg(C_DIM()),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
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

// ── Models ─────────────────────────────────────────────
const MODEL_COLS: [&str; 10] = [
    "name", "engine", "accel", "ready", "run", "wait", "kv", "tps", "path", "status",
];

fn model_col_header(k: &str) -> &'static str {
    match k {
        "name" => "MODEL",
        "engine" => "ENGINE",
        "accel" => "ACCEL",
        "ready" => "READY",
        "run" => "RUN",
        "wait" => "WAIT",
        "kv" => "KV",
        "tps" => "t/s",
        "path" => "PATH",
        "status" => "STATUS",
        _ => "?",
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
        "name" => Cell::from(if selected {
            marquee(&m.name, 20, tick)
        } else {
            truncw(&m.name, 20)
        }),
        "engine" => Cell::from(Span::styled(
            truncw(&m.engine, 12),
            Style::default().fg(C_ACC()),
        )),
        "accel" => Cell::from(Span::styled(
            truncw(&m.accel, 13),
            Style::default().fg(C_DIM()),
        )),
        "ready" => cellw(format!("{}/{}", m.ready, m.desired), 6),
        "run" => cellw(fmt_opt(m.running), 4),
        "wait" => cellw(fmt_opt(m.waiting), 4),
        "kv" => cellw(
            m.kv.map(|x| format!("{:.0}%", x * 100.0))
                .unwrap_or("–".into()),
            5,
        ),
        "tps" => cellw(m.tps.map(|x| format!("{:.0}", x)).unwrap_or("–".into()), 5),
        "path" => cellw(
            if m.route.is_empty() {
                "–".into()
            } else {
                m.route.clone()
            },
            11,
        ),
        "status" => {
            let color = if m.status.contains("Running") {
                C_OK()
            } else if m.status.contains("Pending") {
                C_WARN()
            } else {
                C_DIM()
            };
            Cell::from(Span::styled(m.status.clone(), Style::default().fg(color)))
        }
        _ => Cell::from(""),
    }
}

fn models_table<'a>(app: &'a App, title: &'a str) -> Table<'static> {
    let cols = app.columns("models", &MODEL_COLS); // 설정파일 columns.models 순서/표시
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let m = &app.snap.models[i];
            Row::new(
                cols.iter()
                    .map(|c| model_cell(c, m, pos == app.selected, app.tick))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let widths: Vec<Constraint> = cols.iter().map(|c| model_col_width(c)).collect();
    let header: Vec<&str> = cols.iter().map(|c| model_col_header(c)).collect();
    Table::new(rows, widths)
        .header(hrow_sorted(
            &header,
            app.sort_header_label(),
            app.sort_arrow(),
        ))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block(title))
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
            let sfocus = app.panel_focus == 0; // scorers 패널 포커스
                                               // 유효 가중치(what-if 오버라이드 반영) + 상대 영향도(%).
            let eff: Vec<f64> = cfg
                .scorers
                .iter()
                .map(|(n, w)| app.epp_weight(n, *w))
                .collect();
            let maxw = eff.iter().cloned().fold(1.0, f64::max);
            let total: f64 = eff.iter().sum::<f64>().max(1e-9);
            let simulating = !app.epp_weights.is_empty();
            let srows: Vec<Row> = cfg
                .scorers
                .iter()
                .map(|(name, base)| {
                    let w = app.epp_weight(name, *base);
                    let ov = app.epp_weights.contains_key(name);
                    let infl = w / total * 100.0;
                    Row::new(vec![
                        cellw(name.clone(), 26),
                        Cell::from(Span::styled(
                            format!("{:.0}", w),
                            Style::default()
                                .fg(if ov { C_ACC() } else { C_WARN() })
                                .add_modifier(if ov {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        )),
                        Cell::from(bar_line(w / maxw * 100.0, 8, C_ACC())), // 고정폭 + track(░)
                        Cell::from(Span::styled(
                            format!("{:>3.0}%", infl),
                            Style::default().fg(C_DIM()),
                        )),
                    ])
                })
                .collect();
            // 정직한 문구: +/- 는 가중치를 조정하고 infl=상대 점유율(weight share)을 보여줄 뿐,
            // 실제 라우팅 결정 재시뮬이 아님(그건 per-endpoint score 필요 → 인프라 대기).
            let ns = cfg.scorers.len();
            let cnt = if sfocus {
                count_suffix(app.selected, ns)
            } else {
                String::new()
            };
            let title = if simulating {
                format!("EPP scorers · +/- weight (sim) · infl=share{}", cnt)
            } else {
                format!("EPP scorers · +/- weight · infl=share{}", cnt)
            };
            let mut t = Table::new(
                srows,
                [
                    Constraint::Min(14),
                    Constraint::Length(3),
                    Constraint::Length(9),
                    Constraint::Length(4),
                ],
            )
            .header(hrow(&["SCORER", "WT", "WEIGHT", "infl"]))
            .column_spacing(1)
            .block(if sfocus {
                block_active(&title)
            } else {
                block(&title)
            });
            if sfocus {
                t = t.row_highlight_style(hl_style()).highlight_symbol("▎");
            }
            let mut st = TableState::default();
            st.select(if sfocus { Some(app.selected) } else { None });
            f.render_stateful_widget(t, top_l, &mut st);
            if sfocus {
                list_scrollbar(f, top_l, ns, app.selected, 1);
            }

            let sel = if sfocus {
                cfg.scorers.get(app.selected)
            } else {
                None
            };
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
                    Span::styled(
                        name.clone(),
                        Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  (weight {:.0})", w), Style::default().fg(C_DIM())),
                ]));
                dl.push(Line::from(""));
                dl.push(Line::from(Span::styled(
                    scorer_desc(name),
                    Style::default().fg(Color::White),
                )));
            }
            f.render_widget(
                Paragraph::new(dl)
                    .wrap(Wrap { trim: true })
                    .block(block("what this scorer does")),
                top_r,
            );
        }
        None => f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "EPP ConfigMap not found (llmd-router-epp)",
                Style::default().fg(C_DIM()),
            )))
            .block(block("EPP scorers")),
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
                    Style::default().fg(if p.sat > 0.8 {
                        C_BAD()
                    } else if p.sat > 0.5 {
                        C_WARN()
                    } else {
                        C_DIM()
                    }),
                )),
            ])
        })
        .collect();
    let pfocus = app.panel_focus == 1; // InferencePool 패널 포커스
    let ptitle = format!(
        "InferencePool{}",
        if pfocus {
            count_suffix(app.selected, app.snap.pools.len())
        } else {
            String::new()
        }
    );
    let mut t = Table::new(
        rows,
        [
            Constraint::Min(12),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(6),
        ],
    )
    .header(hrow(&["POOL", "EP r/t", "QUEUE", "SAT"]))
    .block(if pfocus {
        block_active(&ptitle)
    } else {
        block(&ptitle)
    });
    if pfocus {
        t = t.row_highlight_style(hl_style()).highlight_symbol("▎");
    }
    let mut pst = TableState::default();
    pst.select(if pfocus { Some(app.selected) } else { None });
    f.render_stateful_widget(t, bottom_l, &mut pst);
    if pfocus {
        list_scrollbar(f, bottom_l, app.snap.pools.len(), app.selected, 1);
    }

    // request distribution
    let mut dl: Vec<Line> = vec![Line::from(vec![
        Span::styled("EPP in path: ", Style::default().fg(C_DIM())),
        Span::styled(
            if app.snap.epp_in_path {
                "yes"
            } else {
                "no (bypassed)"
            },
            Style::default().fg(if app.snap.epp_in_path {
                C_OK()
            } else {
                C_WARN()
            }),
        ),
        Span::styled(
            format!(
                "   prefix idx: {}",
                if app.snap.prefix_idx.is_nan() {
                    "-".into()
                } else {
                    format!("{:.0}", app.snap.prefix_idx)
                }
            ),
            Style::default().fg(C_DIM()),
        ),
    ])];
    let total: f64 = app.snap.decisions.iter().map(|(_, c)| c).sum();
    if app.snap.decisions.is_empty() || total <= 0.0 {
        dl.push(Line::from(Span::styled(
            if app.snap.epp_in_path {
                "no distribution data (waiting for traffic)"
            } else {
                "no distribution data (EPP bypassed - see Topo)"
            },
            Style::default().fg(C_DIM()),
        )));
    } else {
        for (pod, cnt) in app.snap.decisions.iter().take(5) {
            let share = cnt / total * 100.0;
            let mut sp = vec![Span::styled(
                format!("{:<20} ", truncw(pod, 20)),
                Style::default().fg(Color::White),
            )];
            sp.extend(bar_line(share, 8, C_ACC()).spans);
            sp.push(Span::styled(
                format!(" {:>3.0}%", share),
                Style::default().fg(C_DIM()),
            ));
            dl.push(Line::from(sp));
        }
    }
    f.render_widget(
        Paragraph::new(dl).block(block("request distribution (routing decisions)")),
        bottom_r,
    );
}

// ── Topology (구성/라우팅/분배 한눈에) ──────────────────
// Flow 컴포넌트 타입별 색(구별용): 게이트웨이/EPP/모델/직결서비스/인프라.
fn c_gw() -> Color {
    C_ACC()
}
fn c_epp() -> Color {
    Color::Magenta
}
fn c_svc() -> Color {
    C_WARN()
}
/// `[TAG]` 형태의 타입 배지 span.
fn tag(t: &str, c: Color) -> Span<'static> {
    Span::styled(
        format!("[{}]", t),
        Style::default().fg(c).add_modifier(Modifier::BOLD),
    )
}

fn view_routing(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line = 0usize; // 선택 route 의 줄 위치(스크롤용)

    // 범례 — 각 계층 컴포넌트 타입을 색으로 구별.
    lines.push(Line::from(vec![
        Span::styled("layers: ", Style::default().fg(C_DIM())),
        tag("GW", c_gw()),
        Span::styled(" gateway  ", Style::default().fg(C_DIM())),
        tag("EPP", c_epp()),
        Span::styled(" picker  ", Style::default().fg(C_DIM())),
        tag("MODEL", C_OK()),
        Span::styled(" serving  ", Style::default().fg(C_DIM())),
        tag("SVC", c_svc()),
        Span::styled(" direct  ", Style::default().fg(C_DIM())),
        tag("INFRA", C_DIM()),
        Span::styled(" pod/node", Style::default().fg(C_DIM())),
    ]));

    // [GW] Gateway → [ROUTE] HTTPRoute → 각 route
    let gw = if s.gw_addr.is_empty() {
        "llm-d-gateway  (—)".to_string()
    } else {
        format!(
            "llm-d-gateway  {}  {}",
            s.gw_addr,
            if s.gw_ok {
                "●Programmed"
            } else {
                "○ pending"
            }
        )
    };
    lines.push(Line::from(vec![
        tag("GW", c_gw()),
        Span::raw(" "),
        Span::styled(gw, Style::default().fg(c_gw()).add_modifier(Modifier::BOLD)),
    ]));
    if s.routes.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("└─ ", Style::default().fg(C_DIM())),
            tag("ROUTE", Color::White),
            Span::styled(
                " no HTTPRoute discovered in namespace",
                Style::default().fg(C_DIM()),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("└─ ", Style::default().fg(C_DIM())),
            tag("ROUTE", Color::White),
            Span::styled(
                format!(
                    " {} route rule{}",
                    s.routes.len(),
                    if s.routes.len() == 1 { "" } else { "s" }
                ),
                Style::default().fg(Color::White),
            ),
        ]));
    }
    let n = s.routes.len();
    for (i, r) in s.routes.iter().enumerate() {
        let last = i + 1 == n;
        let rbr = if last { "   └─ " } else { "   ├─ " };
        let m = s.models.iter().find(|m| m.name == r.backend);
        let up = m.map(|m| m.ready > 0).unwrap_or(false);
        let is_pool = r.kind == "InferencePool";
        // backend 계층 배지: InferencePool → [EPP], Service → [SVC](직결).
        let (btag, bcol) = if is_pool {
            ("EPP", c_epp())
        } else {
            ("SVC", c_svc())
        };
        let sel = app.panel_focus == 0 && i == app.selected; // routes 패널 포커스일 때만 강조
        if sel {
            sel_line = lines.len();
        }
        let mut spans = vec![
            Span::styled(rbr, Style::default().fg(C_DIM())),
            dot(up),
            Span::styled(
                format!("{:<13} ", truncw(&r.path, 13)),
                Style::default().fg(Color::White),
            ),
            Span::styled("→", Style::default().fg(C_DIM())),
            tag(btag, bcol),
            Span::styled(
                format!("{} ", truncw(&r.backend, 20)),
                Style::default().fg(bcol),
            ),
        ];
        // 모델 계층 배지 + 상태/가속기/엔진.
        spans.push(Span::styled("→", Style::default().fg(C_DIM())));
        spans.push(tag("MODEL", if up { C_OK() } else { C_DIM() }));
        match m {
            Some(m) => spans.push(Span::styled(
                format!(
                    "{} {}/{} {} [{}]",
                    truncw(&m.name, 16),
                    m.ready,
                    m.desired,
                    m.accel,
                    m.engine
                ),
                Style::default().fg(if up { C_OK() } else { C_DIM() }),
            )),
            None => spans.push(Span::styled("(no serving)", Style::default().fg(C_WARN()))),
        }
        let mut rl = Line::from(spans);
        if sel {
            rl = rl.style(Style::default().bg(C_HL()).add_modifier(Modifier::BOLD));
            // 선택 route(정렬 유지 위해 배경만)
        }
        lines.push(rl);
        // 하위: [INFRA] 이 backend 의 파드들(트리 자식)
        let cont = if last { "      " } else { "   │  " };
        let pods: Vec<&crate::collect::PodRow> = s
            .pods
            .iter()
            .filter(|p| p.name.starts_with(&r.backend))
            .collect();
        for (j, p) in pods.iter().enumerate() {
            let pbr = if j + 1 == pods.len() {
                "└─ "
            } else {
                "├─ "
            };
            let pc = if p.phase == "Running" {
                C_OK()
            } else {
                C_DIM()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}", cont, pbr), Style::default().fg(C_TRACK())),
                tag("INFRA", C_DIM()),
                Span::styled(
                    format!(" {} ", truncw(&p.name, 30)),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(format!("{} @{}", p.phase, p.node), Style::default().fg(pc)),
            ]));
        }
    }
    // EPP 경유 여부 진단
    if !s.routes.is_empty() {
        if s.epp_in_path {
            lines.push(Line::from(Span::styled(
                "  ✓ routes go through InferencePool (EPP-routed)",
                Style::default().fg(C_OK()),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  ⚠ some HTTPRoutes point to Service directly → EPP bypassed (no model-aware routing)",
                Style::default().fg(C_WARN()),
            )));
        }
    }

    let top = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(10)])
        .split(area);
    // 트리가 길면 선택 route 가 보이도록 세로 스크롤(무언의 잘림 방지).
    let vis = (top[0].height as usize).saturating_sub(2);
    let scroll = if sel_line + 2 > vis {
        (sel_line + 3).saturating_sub(vis) as u16
    } else {
        0
    };
    let rfocus = app.panel_focus == 0;
    let rtitle = format!(
        "Flow · Gateway→EPP→Model→Infra · ↑↓ route · p/i/m/e pivot{}",
        if rfocus {
            count_suffix(app.selected, s.routes.len())
        } else {
            String::new()
        }
    );
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).block(if rfocus {
            block_active(&rtitle)
        } else {
            block(&rtitle)
        }),
        top[0],
    );

    // InferencePool + EPP + SLO
    let pfocus = app.panel_focus == 1; // InferencePool 패널 포커스
    let mut pl: Vec<Line> = Vec::new();
    if s.pools.is_empty() {
        pl.push(Line::from(Span::styled(
            "(no InferencePool)",
            Style::default().fg(C_DIM()),
        )));
    }
    for (pi, p) in s.pools.iter().enumerate() {
        let sel = pfocus && pi == app.selected;
        let epc = if p.ep_total == 0 { C_WARN() } else { C_OK() };
        // 이 pool 을 가리키는 route 경로(들어오는 트래픽 입구).
        let route_path = s
            .routes
            .iter()
            .find(|r| r.kind == "InferencePool" && r.backend == p.name)
            .map(|r| r.path.clone())
            .unwrap_or_else(|| "(no route)".into());
        // 메트릭 요약(값 있을 때만).
        let mut metrics = String::new();
        if p.kv.is_finite() && p.kv > 0.0 {
            metrics.push_str(&format!("kv{:.0}% ", p.kv * 100.0));
        }
        if p.sat.is_finite() && p.sat > 0.0 {
            metrics.push_str(&format!("sat{:.2} ", p.sat));
        }
        if p.queue.is_finite() && p.queue > 0.0 {
            metrics.push_str(&format!("q{:.0}", p.queue));
        }
        let mut pline = Line::from(vec![
            tag("EPP", c_epp()),
            Span::styled(
                format!(" {:<18}", truncw(&p.name, 18)),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("●{}/{} ep ", p.ep_ready, p.ep_total),
                Style::default().fg(epc),
            ),
            Span::styled(
                format!("←{} ", truncw(&route_path, 14)),
                Style::default().fg(c_gw()),
            ),
            Span::styled(
                format!("picker:{} ", if p.epp.is_empty() { "–" } else { &p.epp }),
                Style::default().fg(c_epp()),
            ),
            Span::styled(metrics, Style::default().fg(C_DIM())),
        ]);
        if sel {
            pline.style = Style::default().bg(C_HL()).add_modifier(Modifier::BOLD);
        }
        pl.push(pline);
        // 선택된 pool 은 selector 상세 한 줄 더(어떤 파드를 고르는지).
        if sel {
            pl.push(Line::from(vec![
                Span::styled("    selector ", Style::default().fg(C_DIM())),
                Span::styled(
                    if p.selector.is_empty() {
                        "–".into()
                    } else {
                        p.selector.clone()
                    },
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    "   ⏎ actions (rename/retarget/delete)",
                    Style::default().fg(C_DIM()),
                ),
            ]));
        }
    }
    if !s.objectives.is_empty() {
        let so: Vec<String> = s
            .objectives
            .iter()
            .map(|o| format!("{}(p{}→{})", o.name, o.priority, o.pool))
            .collect();
        pl.push(Line::from(vec![
            Span::styled("SLO  ", Style::default().fg(C_DIM())),
            Span::styled(so.join("  "), Style::default().fg(Color::White)),
        ]));
    }
    for a in &s.autoscalers {
        pl.push(Line::from(vec![
            Span::styled("autoscale ", Style::default().fg(C_DIM())),
            Span::styled(truncw(&a.target, 26), Style::default().fg(Color::White)),
            Span::styled(
                format!("  {}↔{} rep={} ", a.min, a.max, a.replicas),
                Style::default().fg(C_DIM()),
            ),
            Span::styled(
                if a.active { "active" } else { "idle" },
                Style::default().fg(if a.active { C_OK() } else { C_DIM() }),
            ),
            Span::styled(
                if a.ready { " ✓" } else { " ⚠notready" },
                Style::default().fg(if a.ready { C_OK() } else { C_WARN() }),
            ),
            Span::styled(format!(" [{}]", a.triggers), Style::default().fg(C_DIM())),
        ]));
    }
    let ptitle = format!(
        "InferencePool / EPP / SLO / Autoscale{}",
        if pfocus {
            count_suffix(app.selected, s.pools.len())
        } else {
            String::new()
        }
    );
    f.render_widget(
        Paragraph::new(pl).block(if pfocus {
            block_active(&ptitle)
        } else {
            block(&ptitle)
        }),
        top[1],
    );
}

// ── Overview ───────────────────────────────────────────
fn view_overview(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;

    // ── 클러스터 요약 카드(all-smi 식 aggregate) ──────────
    // Σ 요약 1줄 + LED 그리드(폭에 맞춰 줄바꿈). 카드 높이는 LED 줄 수에 맞춰 가변.
    let mut cluster_lines: Vec<Line> = Vec::new();
    {
        // 벤더별 (수, 색, 사용메모리GB) — 스택 바용.
        let mut kinds: std::collections::BTreeMap<&str, (usize, Color, f64)> =
            std::collections::BTreeMap::new();
        let (mut usum, mut mu, mut mt, mut pw, mut tsum) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for a in &s.accel {
            let e = kinds
                .entry(a.disp())
                .or_insert((0, kind_color(a.kind), 0.0));
            e.0 += 1;
            e.2 += a.mem_used_gb;
            usum += a.util;
            mu += a.mem_used_gb;
            mt += a.mem_total_gb;
            pw += a.power;
            tsum += a.temp;
        }
        let ncnt = s.accel.len().max(1);
        let avg = usum / ncnt as f64;
        let avg_temp = tsum / ncnt as f64;
        let mempct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
        let ready = s.serving_count();
        // 인벤토리 라벨(GB10×2 …) + 라벨된 집계(util·temp·VRAM·W·models). req/s·TTFT 는 상단바에 있어 생략.
        let mut sp = vec![Span::styled(
            format!("{} accel  ", s.accel.len()),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        )];
        for (k, (c, col, _)) in &kinds {
            sp.push(Span::styled(
                format!("{}×{} ", k, c),
                Style::default().fg(*col).add_modifier(Modifier::BOLD),
            ));
        }
        sp.push(Span::styled(
            format!("│ util {:.0}% ", avg),
            Style::default().fg(util_color(avg)),
        ));
        sp.push(Span::styled(
            format!("temp {:.0}°C ", avg_temp),
            Style::default().fg(temp_color(avg_temp)),
        ));
        sp.push(Span::styled(
            format!("│ VRAM {:.0}/{:.0}GB {:.0}% ", mu, mt, mempct),
            Style::default().fg(mem_color(mempct)),
        ));
        // 노드 루트 디스크 집계(존재하는 노드만)
        let (du, dt): (f64, f64) = s.nodes.iter().fold((0.0, 0.0), |(u, t), n| {
            (u + n.disk_used_gb, t + n.disk_total_gb)
        });
        if dt > 0.0 {
            let dp = du / dt * 100.0;
            sp.push(Span::styled(
                format!("disk {:.0}% ", dp),
                Style::default().fg(mem_color(dp)),
            ));
        }
        sp.push(Span::styled(
            format!("⚡{:.0}W ", pw),
            Style::default().fg(C_DIM()),
        ));
        sp.push(Span::styled(
            format!("│ models {}/{} ", ready, s.models.len()),
            Style::default().fg(if ready > 0 { C_OK() } else { C_DIM() }),
        ));
        // 세션 에너지 총합(R 리셋)
        let ewh: f64 = s
            .accel
            .iter()
            .map(|a| app.energy_session_wh(a))
            .filter(|x| !x.is_nan())
            .sum();
        if ewh > 0.0 {
            sp.push(Span::styled(
                format!("· E {:.1}Wh", ewh),
                Style::default().fg(C_ACC()),
            ));
        }
        cluster_lines.push(Line::from(sp));

        // VRAM 구성(벤더별 스택 바 + free) — 이종 가속기 메모리 점유를 한눈에.
        if mt > 0.0 {
            let barw = ((area.width as usize).saturating_sub(24)).clamp(10, 48);
            let segs: Vec<(f64, Color)> = kinds.values().map(|(_, col, m)| (*m, *col)).collect();
            let mut vsp = vec![Span::styled(
                format!("{:<6}", "VRAM"),
                Style::default().fg(C_DIM()),
            )];
            vsp.extend(stacked_bar(&segs, mt, barw));
            vsp.push(Span::styled(
                format!(" {:.0}/{:.0}GB used", mu, mt),
                Style::default().fg(C_DIM()),
            ));
            cluster_lines.push(Line::from(vsp));
        }

        // all-smi 식 LED 그리드: 디바이스 1개=글리프 1개. vendor=색, util=●채움/○유휴, dead=✗, throttle=⚠.
        // 폭 초과 시 다음 줄로 감싸고(라벨 폭만큼 들여쓰기), 큰 fleet 대비 최대 줄 수 제한.
        const MAX_LED_LINES: usize = 8;
        const LABEL_W: usize = 5; // "{:<4} "
        let iw = (area.width as usize).saturating_sub(2); // 카드 내부 폭(테두리 제외)
        let per_line = iw.saturating_sub(LABEL_W) / 2; // 글리프 "● " = 2칸씩
        let per_line = per_line.max(1);
        let mut bykind: std::collections::BTreeMap<&str, Vec<&crate::collect::Accel>> =
            std::collections::BTreeMap::new();
        for a in &s.accel {
            bykind.entry(a.disp()).or_default().push(a);
        }
        let mut led_lines: Vec<Line> = Vec::new();
        'kinds: for (k, list) in &bykind {
            let kc = kind_color(list[0].kind);
            let mut cur: Vec<Span> = vec![Span::styled(
                format!("{:<4} ", k),
                Style::default().fg(kc).add_modifier(Modifier::BOLD),
            )];
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
                // 점 색 = util 히트(레인보우: 파랑 저부하 → 빨강 고부하) → fleet 핫스팟이 한눈에(all-smi식).
                let (g, c) = if !a.alive {
                    ("✗", C_BAD())
                } else if a.throttle > 0.0 {
                    ("⚠", C_WARN())
                } else if a.util > IDLE_UTIL {
                    ("●", util_color(a.util))
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
            led_lines.push(Line::from(Span::styled(
                "(no accelerators)",
                Style::default().fg(C_DIM()),
            )));
        }
        cluster_lines.extend(led_lines);
    }
    let cluster_h = cluster_lines.len() as u16 + 2; // 내용 줄 + 테두리(2)

    // 위계(눈이 가는 순서 = 중요도): 히어로(용량·부하) → 판정(문제?) → 가속기 → 서빙경로 → 모델 리스트.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(cluster_h),
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Min(4),
        ])
        .split(area);
    f.render_widget(
        Paragraph::new(cluster_lines).block(block("Cluster")),
        rows[0],
    );

    // 판정(plain-language verdict) — 히어로 바로 밑으로 올려 "지금 문제 있나?"에 즉답.
    let (txt, col) = diagnose(s);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncw(&txt, rows[1].width.saturating_sub(2) as usize),
            Style::default().fg(col).add_modifier(Modifier::BOLD),
        )))
        .block(block("Status")),
        rows[1],
    );

    // 가속기: (종류,노드)별 집계 — 한눈에 + 절대 메모리(GB) + health 아이콘
    #[allow(clippy::type_complexity)]
    let mut groups: Vec<(AccelKind, String, usize, f64, f64, f64, bool, bool, String)> = Vec::new();
    for a in &s.accel {
        if let Some(g) = groups.iter_mut().find(|g| g.0 == a.kind && g.1 == a.node) {
            g.2 += 1;
            g.3 += a.util;
            g.4 += a.mem_used_gb;
            g.5 += a.mem_total_gb;
            g.6 = g.6 && a.alive;
            g.7 = g.7 || a.throttle > 0.0;
        } else {
            groups.push((
                a.kind,
                a.node.clone(),
                1,
                a.util,
                a.mem_used_gb,
                a.mem_total_gb,
                a.alive,
                a.throttle > 0.0,
                a.disp().to_string(),
            ));
        }
    }
    let mut al: Vec<Line> = Vec::new();
    for (kind, node, cnt, us, mu, mt, alive, thr, model) in &groups {
        let util = us / (*cnt as f64);
        let mempct = if *mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
        let (hi, hc) = if !*alive {
            ("✗", C_BAD())
        } else if *thr {
            ("⚠", C_WARN())
        } else {
            ("●", C_OK())
        };
        let mut sp = vec![
            Span::styled(format!("{} ", hi), Style::default().fg(hc)),
            Span::styled(
                format!("{:<4}×{} ", model, cnt),
                Style::default()
                    .fg(kind_color(*kind))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("@{:<16} ", truncw(node, 16)),
                Style::default().fg(C_DIM()),
            ),
        ];
        sp.extend(dot_bar(util, 10, util_color(util)).spans); // overview 는 레인보우 바(장식) — 수치는 severity 색으로 의미 유지
        sp.push(Span::styled(
            format!(" {:>3.0}% ", util),
            Style::default().fg(util_color(util)),
        ));
        sp.push(Span::styled("mem ", Style::default().fg(C_DIM())));
        sp.extend(dot_bar(mempct, 10, mem_color(mempct)).spans); // MEM 도 레인보우 바 — 유휴 때도 채움이 보임
                                                                 // GB 필드 고정폭(우측정렬) → 뒤따르는 트렌드 스파크라인이 열 정렬됨.
        sp.push(Span::styled(
            format!(" {:>3.0}/{:>3.0}GB  ", mu, mt),
            Style::default().fg(mem_color(mempct)),
        ));
        sp.push(Span::styled("trend ", Style::default().fg(C_DIM())));
        let trend = sparkstr(
            &app.hist_for(&format!("sys:{}_util", kind.label())),
            14,
            100,
        ); // all-smi식 인라인 트렌드
        sp.push(Span::styled(trend, Style::default().fg(util_color(util))));
        al.push(Line::from(sp));
    }
    if al.is_empty() {
        al.push(Line::from(Span::styled(
            "  (no accelerator metrics)",
            Style::default().fg(C_DIM()),
        )));
    }
    // 무언의 잘림 방지: (종류,노드) 그룹이 패널보다 많으면 마지막 줄을 "+N more" 로.
    let acap = (rows[2].height as usize).saturating_sub(2);
    if al.len() > acap && acap > 0 {
        let hidden = al.len() - (acap - 1);
        al.truncate(acap - 1);
        al.push(Line::from(Span::styled(
            format!("  … +{} more (see Accel / Nodes tab)", hidden),
            Style::default().fg(C_DIM()),
        )));
    }
    f.render_widget(
        Paragraph::new(al).block(block("Accelerators (by kind / node)")),
        rows[2],
    );

    // Inference: EPP 경로 + 풀 endpoints + scorers + autoscale
    let mut pl: Vec<Line> = Vec::new();
    pl.push(Line::from(vec![
        Span::styled("EPP path ", Style::default().fg(C_DIM())),
        Span::styled(
            if s.epp_in_path {
                "via InferencePool ●"
            } else {
                "bypassed (HTTPRoute→Service) ⚠"
            },
            Style::default().fg(if s.epp_in_path { C_OK() } else { C_WARN() }),
        ),
    ]));
    for p in s.pools.iter().take(2) {
        pl.push(Line::from(vec![
            dot(p.ep_ready > 0),
            Span::styled(
                format!("{:<16} ", truncw(&p.name, 16)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(
                    "endpoints {}/{}  sat {}",
                    p.ep_ready,
                    p.ep_total,
                    fmt_nan(p.sat, 2)
                ),
                Style::default().fg(C_DIM()),
            ),
        ]));
    }
    if let Some(cfg) = &s.epp {
        let names: Vec<String> = cfg
            .scorers
            .iter()
            .map(|(n, w)| format!("{}·{:.0}", n.replace("-scorer", ""), w))
            .collect();
        pl.push(Line::from(Span::styled(
            format!("scorers: {}", names.join("  ")),
            Style::default().fg(C_DIM()),
        )));
    }
    f.render_widget(
        Paragraph::new(pl).block(block("Inference (EPP / InferencePool)")),
        rows[3],
    );

    let mut st = TableState::default();
    st.select(Some(app.selected));
    f.render_stateful_widget(models_table(app, "Models · ⏎ detail"), rows[4], &mut st);
}

// ── Detail (drill-down) ────────────────────────────────
fn detail_panel(f: &mut Frame, area: Rect, app: &App) {
    // Deploy▸Library 통합 트리 선택 항목 상세 — 카탈로그 모델(배치 후보/가능성) 또는 스토어 빌드.
    if app.view == View::Library && app.panel_focus == 0 {
        library_detail(f, area, app);
        return;
    }
    let (cur, tot) = app.detail_pos();
    let (prev, next) = app.neighbor_names();
    let nav = format!(
        " · ◂ {}  {}/{}  {} ▸ · esc back",
        truncw(&prev, 16),
        cur,
        tot,
        truncw(&next, 16)
    );
    // Accelerator: info + util/mem/temp timeline
    if let Some(a) = app.selected_accel() {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(11), Constraint::Min(6)])
            .split(area);
        let mempct = if a.mem_total_gb > 0.0 {
            a.mem_used_gb / a.mem_total_gb * 100.0
        } else {
            0.0
        };
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
                Span::styled(
                    format!("{} ", a.disp()),
                    Style::default()
                        .fg(kind_color(a.kind))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("id {} @ {}   ", a.id, a.node),
                    Style::default().fg(C_DIM()),
                ),
                Span::styled(
                    health.0,
                    Style::default().fg(health.1).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("   {:.0} W", a.power), Style::default().fg(C_DIM())),
                Span::styled(
                    format!(
                        "   {}",
                        if a.busy_model.is_empty() {
                            "(idle)"
                        } else {
                            a.busy_model.as_str()
                        }
                    ),
                    Style::default().fg(C_ACC()),
                ),
            ]),
            Line::from(""),
            gauge_row(
                "compute",
                a.util,
                &format!("{:.0} %", a.util),
                util_color(a.util),
                barw,
            ),
            gauge_row(
                if a.unified_mem { "mem∪" } else { "VRAM" },
                mempct,
                &format!(
                    "{:.1} / {:.1} GB  ({:.0}%){}",
                    a.mem_used_gb,
                    a.mem_total_gb,
                    mempct,
                    if a.unified_mem {
                        "  unified w/ host"
                    } else {
                        ""
                    }
                ),
                mem_color(mempct),
                barw,
            ),
            gauge_row(
                "temp",
                a.temp.min(100.0),
                &format!("{:.0} °C", a.temp),
                temp_color(a.temp),
                barw,
            ),
        ];
        // 메모리 대역폭(통합 메모리에선 진짜 병목) — DCGM MEM_COPY_UTIL. 있을 때만.
        if !a.mem_bw.is_nan() {
            lines.push(gauge_row(
                "mem bw",
                a.mem_bw,
                &format!("{:.0} %", a.mem_bw),
                grad_color(a.mem_bw),
                barw,
            ));
        }
        if !a.clock_mhz.is_nan() || !a.mem_temp.is_nan() {
            lines.push(Line::from(vec![
                Span::styled(format!("{:<8} ", "clock"), Style::default().fg(C_DIM())),
                Span::styled(
                    if a.clock_mhz.is_nan() {
                        "–".into()
                    } else {
                        format!("{:.0} MHz", a.clock_mhz)
                    },
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("    mem temp ", Style::default().fg(C_DIM())),
                Span::styled(
                    if a.mem_temp.is_nan() {
                        "–".into()
                    } else {
                        format!("{:.0} °C", a.mem_temp)
                    },
                    Style::default().fg(temp_color(a.mem_temp)),
                ),
            ]));
        }
        // 세션 에너지(누적) — R 로 리셋
        let ewh = app.energy_session_wh(a);
        if !ewh.is_nan() {
            let hrs = crate::collect::now_secs().saturating_sub(app.energy_since) as f64 / 3600.0;
            let avg = if hrs > 1e-6 { ewh / hrs } else { f64::NAN };
            lines.push(Line::from(vec![
                Span::styled(format!("{:<8} ", "energy"), Style::default().fg(C_DIM())),
                Span::styled(
                    format!("{:.2} Wh", ewh),
                    Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  (session · avg {})",
                        if avg.is_nan() {
                            "–".into()
                        } else {
                            format!("{:.0} W", avg)
                        }
                    ),
                    Style::default().fg(C_DIM()),
                ),
                Span::styled("  R reset", Style::default().fg(C_DIM())),
            ]));
        }
        f.render_widget(
            Paragraph::new(lines).block(block(&format!("Accelerator{}", nav))),
            rows[0],
        );
        // 타임라인: util% / VRAM% 두 개만 넓게(반응형). temp/power 는 위 게이지로.
        let k = format!("acc:{}:{}:{}", a.kind.label(), a.node, a.id);
        let (l, r) = two_panes(rows[1], 50);
        bar_timeline(
            f,
            l,
            app,
            &format!("{}:util", k),
            "compute util",
            "%",
            Some(100.0),
        );
        bar_timeline(f, r, app, &format!("{}:mem", k), "VRAM", "%", Some(100.0));
        return;
    }
    // Node: info + cpu/mem/load timeline
    if let Some(n) = app.selected_node() {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10),
                Constraint::Min(4),
                Constraint::Length(8),
            ])
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
        let mempct = if n.mem_total_gb > 0.0 {
            n.mem_used_gb / n.mem_total_gb * 100.0
        } else {
            0.0
        };
        let barw = (rows[0].width as usize).saturating_sub(34).clamp(10, 46);
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{}  ", truncw(&n.name, 30)),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(hg, Style::default().fg(hc).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("   kubelet {}", n.version),
                    Style::default().fg(C_DIM()),
                ),
                Span::styled(
                    if n.npu.is_empty() {
                        String::new()
                    } else {
                        format!("   accel {}", n.npu)
                    },
                    Style::default().fg(C_ACC()),
                ),
            ]),
            Line::from(""),
            gauge_row(
                "cpu",
                if n.cpu_pct.is_nan() { 0.0 } else { n.cpu_pct },
                &if n.cpu_pct.is_nan() {
                    "–".into()
                } else {
                    format!("{:.0} %", n.cpu_pct)
                },
                util_color(n.cpu_pct.max(0.0)),
                barw,
            ),
            gauge_row(
                "memory",
                mempct,
                &if n.mem_total_gb <= 0.0 {
                    "–".into()
                } else {
                    format!(
                        "{:.0} / {:.0} GB  ({:.0}%)",
                        n.mem_used_gb, n.mem_total_gb, mempct
                    )
                },
                mem_color(mempct),
                barw,
            ),
            {
                let dp = if n.disk_total_gb > 0.0 {
                    n.disk_used_gb / n.disk_total_gb * 100.0
                } else {
                    0.0
                };
                gauge_row(
                    "disk /",
                    dp,
                    &if n.disk_total_gb <= 0.0 {
                        "–".into()
                    } else {
                        format!(
                            "{:.0} / {:.0} GB  ({:.0}%)",
                            n.disk_used_gb, n.disk_total_gb, dp
                        )
                    },
                    mem_color(dp),
                    barw,
                )
            },
            Line::from(vec![
                Span::styled(format!("{:<8} ", "load1"), Style::default().fg(C_DIM())),
                Span::styled(
                    if n.load1.is_nan() {
                        "–".into()
                    } else {
                        format!("{:.2}", n.load1)
                    },
                    Style::default().fg(C_WARN()).add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        f.render_widget(
            Paragraph::new(lines).block(block(&format!("Node{}", nav))),
            rows[0],
        );
        // 이 노드가 가진 모든 디바이스(full 라인). ↑↓ 로 커서 이동(0=노드요약, i=개별 device 히스토리).
        let devs: Vec<&crate::collect::Accel> =
            app.snap.accel.iter().filter(|a| a.node == n.name).collect();
        let mut dl: Vec<Line> = Vec::new();
        if devs.is_empty() {
            dl.push(Line::from(Span::styled(
                "(no accelerators on this node)",
                Style::default().fg(C_DIM()),
            )));
        } else {
            let last = devs.len();
            for (j, a) in devs.iter().enumerate() {
                let sel = app.dev_sel == j + 1;
                let branch = if sel {
                    "▸ "
                } else if j + 1 == last {
                    "└─"
                } else {
                    "├─"
                };
                let mut line = accel_brief(a, branch, true);
                if sel {
                    line.style = Style::default().bg(C_HL()).add_modifier(Modifier::BOLD);
                }
                dl.push(line);
            }
        }
        let dtitle = if app.dev_sel == 0 {
            format!(
                "devices on {} ({}) · ↑↓ pick device → history",
                truncw(&n.name, 16),
                devs.len()
            )
        } else {
            format!(
                "devices on {} ({}) · ↑↓ move · ▸#{} history below",
                truncw(&n.name, 16),
                devs.len(),
                app.dev_sel
            )
        };
        f.render_widget(Paragraph::new(dl).block(block(&dtitle)), rows[1]);
        // 하단 타임라인: dev_sel==0 → 노드 host cpu/mem/disk 요약, 아니면 선택 device 의 util/VRAM.
        if app.dev_sel == 0 || devs.is_empty() {
            let k = format!("nod:{}", n.name);
            let mut dash = Dashboard::new().min_width(24);
            let kc = k.clone();
            dash = dash.cell(move |f, r| {
                bar_timeline(
                    f,
                    r,
                    app,
                    &format!("{}:cpu", kc),
                    "host cpu",
                    "%",
                    Some(100.0),
                )
            });
            let km = k.clone();
            dash = dash.cell(move |f, r| {
                bar_timeline(
                    f,
                    r,
                    app,
                    &format!("{}:mem", km),
                    "host mem",
                    "%",
                    Some(100.0),
                )
            });
            if n.disk_total_gb > 0.0 {
                let kd = k.clone();
                dash = dash.cell(move |f, r| {
                    bar_timeline(
                        f,
                        r,
                        app,
                        &format!("{}:disk", kd),
                        "disk /",
                        "%",
                        Some(100.0),
                    )
                });
            }
            dash.render(f, rows[2]);
        } else if let Some(a) = devs.get(app.dev_sel - 1) {
            let (l, r) = two_panes(rows[2], 50);
            let k = format!("acc:{}:{}:{}", a.kind.label(), a.node, a.id);
            let name = format!("{} {}", a.disp(), a.id);
            bar_timeline(
                f,
                l,
                app,
                &format!("{}:util", k),
                &format!("{} util", name),
                "%",
                Some(100.0),
            );
            bar_timeline(
                f,
                r,
                app,
                &format!("{}:mem", k),
                &format!("{} VRAM", name),
                "%",
                Some(100.0),
            );
        }
        return;
    }

    // Event 상세 — 표에서 잘리는 전체 메시지를 읽기 위한 뷰.
    if let Some(e) = app.selected_event() {
        let (tg, tc) = if e.typ == "Warning" {
            ("⚠ Warning", C_WARN())
        } else {
            ("● Normal", C_OK())
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{}  ", tg),
                    Style::default().fg(tc).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("×{}", e.count),
                    Style::default().fg(if e.count > 1 { C_WARN() } else { C_DIM() }),
                ),
            ]),
            Line::from(""),
            kv("reason", &e.reason, Color::White),
            kv("object", &e.object, C_ACC()),
            Line::from(""),
            Line::from(Span::styled(
                "message",
                Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                e.message.clone(),
                Style::default().fg(Color::White),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block(&format!("Event{}", nav))),
            area,
        );
        return;
    }

    // Model artifact 상세 — 저장 위치 + 컴파일/서빙 옵션 전체.
    if let Some(a) = app.selected_artifact() {
        let mut lines = vec![
            kv("model", &a.model, Color::White),
            kv("family", &a.family, C_DIM()),
            kv("engine", &a.engine, C_ACC()),
            kv(
                "image",
                if a.image.is_empty() { "–" } else { &a.image },
                C_DIM(),
            ),
            Line::from(""),
            kv(
                "source",
                if a.source.is_empty() {
                    "– (not in container args/env)"
                } else {
                    &a.source
                },
                Color::White,
            ),
            kv(
                "storage node",
                if a.node.is_empty() { "–" } else { &a.node },
                Color::White,
            ),
            kv(
                "storage path",
                if a.mount.is_empty() {
                    "– (no volume mount)"
                } else {
                    &a.mount
                },
                Color::White,
            ),
            Line::from(""),
            Line::from(Span::styled(
                "compile / serve options",
                Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
            )),
        ];
        if a.opts.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (none detected in the container spec)",
                Style::default().fg(C_DIM()),
            )));
        }
        for (k, v) in &a.opts {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<18} ", k), Style::default().fg(C_DIM())),
                Span::styled(v.clone(), Style::default().fg(Color::White)),
            ]));
        }
        f.render_widget(
            Paragraph::new(lines)
                .scroll((app.detail_scroll, 0))
                .wrap(Wrap { trim: false })
                .block(block(&format!("Model artifact{}", nav))),
            area,
        );
        return;
    }

    // Model 상세 — 정보 + per-model perf 지표 시계열(있으면 하단에 타임라인 그리드).
    if let Some(m) = app.selected_model() {
        let mut lines: Vec<Line> = Vec::new();
        lines.push(kv("model", &m.name, Color::White));
        lines.push(kv(
            "status",
            &m.status,
            if m.ready > 0 { C_OK() } else { C_DIM() },
        ));
        lines.push(kv(
            "replicas",
            &format!("{}/{} (ready/desired)", m.ready, m.desired),
            Color::White,
        ));
        lines.push(kv("engine", &m.engine, C_ACC()));
        lines.push(kv("accelerator", &m.accel, C_ACC()));
        lines.push(kv(
            "route",
            if m.route.is_empty() {
                "–"
            } else {
                m.route.as_str()
            },
            Color::White,
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "inference (vLLM)",
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        )));
        lines.push(kv(
            "  running / waiting",
            &format!("{} / {}", fmt_opt(m.running), fmt_opt(m.waiting)),
            Color::White,
        ));
        lines.push(kv(
            "  KV cache",
            &m.kv
                .map(|x| format!("{:.0}%", x * 100.0))
                .unwrap_or("- (no vLLM metrics)".into()),
            Color::White,
        ));
        lines.push(kv(
            "  tokens/s",
            &m.tps.map(|x| format!("{:.1}", x)).unwrap_or("–".into()),
            Color::White,
        ));
        lines.push(kv(
            "  TTFT p95",
            &m.ttft
                .map(|x| format!("{:.0} ms", x * 1000.0))
                .unwrap_or("–".into()),
            Color::White,
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "pivot ▸ peek (press key to open)",
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        )));
        // [p] pods — 매칭 파드 수/running + 첫 이름
        let mpods: Vec<&crate::collect::PodRow> = app
            .snap
            .pods
            .iter()
            .filter(|p| p.name.starts_with(&m.name))
            .collect();
        let running = mpods.iter().filter(|p| p.phase == "Running").count();
        let pods_prev = if mpods.is_empty() {
            "(none)".to_string()
        } else {
            format!(
                "{} pod(s) · {} running · {}",
                mpods.len(),
                running,
                truncw(&mpods[0].name, 26)
            )
        };
        lines.push(pivot_prev("p", "pods", &pods_prev));
        // [i] infra — 이 모델을 돌리는 디바이스(있으면 util 집계), 없으면 배치 문자열
        let macc: Vec<&crate::collect::Accel> = app
            .snap
            .accel
            .iter()
            .filter(|a| !a.busy_model.is_empty() && a.busy_model.starts_with(&m.name))
            .collect();
        let infra_prev = if !macc.is_empty() {
            let u = macc.iter().map(|a| a.util).sum::<f64>() / macc.len() as f64;
            format!(
                "{}×{} @{} · util {:.0}%",
                macc[0].disp(),
                macc.len(),
                truncw(&macc[0].node, 16),
                u
            )
        } else if !m.accel.is_empty() && m.accel != "-" {
            m.accel.clone()
        } else {
            "no device bound (scaled to 0?)".into()
        };
        lines.push(pivot_prev("i", "infra", &infra_prev));
        // [r] route — HTTPRoute 경로
        lines.push(pivot_prev(
            "r",
            "route",
            if m.route.is_empty() {
                "no route"
            } else {
                m.route.as_str()
            },
        ));
        // [e] epp — EPP 경유 여부
        lines.push(pivot_prev(
            "e",
            "epp",
            if app.snap.epp_in_path {
                "via InferencePool ●"
            } else {
                "bypassed → Service ⚠"
            },
        ));
        lines.push(Line::from(Span::styled(
            "  s scale · S restart",
            Style::default().fg(C_DIM()),
        )));
        // 매칭되는 per-model perf 시계열(이름 정확/포함 일치) → 하단 타임라인.
        let mkey = app
            .snap
            .perf_rows
            .iter()
            .find(|r| r.model == m.name || m.name.contains(&r.model) || r.model.contains(&m.name))
            .map(|r| format!("mperf:{}", r.model));
        let series: [(&str, &str, &str); 4] = [
            ("tps", "tok/s", ""),
            ("ttft", "TTFT", "ms"),
            ("decode", "DECODE", "ms"),
            ("e2e", "E2E", "ms"),
        ];
        let present: Vec<&(&str, &str, &str)> = match &mkey {
            Some(k) => series
                .iter()
                .filter(|(s, _, _)| !app.hist_for(&format!("{}:{}", k, s)).is_empty())
                .collect(),
            None => Vec::new(),
        };
        let n_lines = lines.len();
        let pblk = Paragraph::new(lines)
            .scroll((app.detail_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(block(&format!("Model{}", nav)));
        if present.is_empty() {
            f.render_widget(pblk, area);
        } else {
            let mk = mkey.unwrap();
            let text_h = (n_lines as u16 + 2).clamp(12, 24); // 내용에 맞춘 텍스트 패널 높이
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(text_h), Constraint::Min(6)])
                .split(area);
            f.render_widget(pblk, split[0]);
            let mut dash = Dashboard::new().min_width(30);
            for (s, label, unit) in present {
                let key = format!("{}:{}", mk, s);
                dash =
                    dash.cell(move |f, rect| bar_timeline(f, rect, app, &key, label, unit, None));
            }
            dash.render(f, split[1]);
        }
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut title = "Detail";
    if let Some(p) = app.selected_pod() {
        title = "Pod";
        lines.push(kv("pod", &p.name, Color::White));
        lines.push(kv(
            "phase",
            &p.phase,
            if p.phase == "Running" {
                C_OK()
            } else {
                C_DIM()
            },
        ));
        lines.push(kv("ready", &p.ready, Color::White));
        lines.push(kv("node", &p.node, Color::White));
        lines.push(kv(
            "restarts",
            &p.restarts.to_string(),
            if p.restarts > 0 {
                C_WARN()
            } else {
                Color::White
            },
        ));
        lines.push(pivot_line(&[("i", "infra"), ("m", "model")]));
    } else {
        lines.push(Line::from(Span::styled(
            "no item selected",
            Style::default().fg(C_DIM()),
        )));
    }

    f.render_widget(
        Paragraph::new(lines)
            .scroll((app.detail_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(block(&format!("{}{}", title, nav))),
        area,
    );
}

/// Deploy▸Library 선택 항목 상세 — 카탈로그 모델(배치 후보·가능성·수요) 또는 스토어 빌드(포맷·타깃·경로).
fn library_detail(f: &mut Frame, area: Rect, app: &App) {
    use crate::app::LibItem;
    let mut lines: Vec<Line> = Vec::new();
    let title = match app.selected_lib_item() {
        Some(LibItem::Catalog(k)) => {
            let m = &app.catalog[k];
            lines.push(kv("id", &m.id, Color::White));
            if !m.display.is_empty() {
                lines.push(kv("display", &m.display, C_ACC()));
            }
            lines.push(kv("role", if m.role.is_empty() { "-" } else { &m.role }, C_DIM()));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("placement candidates ({}) — 배치 후보 × 라이브 재고", m.placements.len()),
                Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
            )));
            for p in &m.placements {
                let (state, free, need) = crate::catalog::solve(p, &app.snap.inventory);
                let (g, gc) = match state {
                    crate::catalog::Ready::Ready => ("✓ ready", C_OK()),
                    crate::catalog::Ready::NeedsArtifact => ("⚙ needs-compile/artifact", C_WARN()),
                    crate::catalog::Ready::NoCapacity => ("✗ no-capacity", C_BAD()),
                };
                lines.push(Line::from(vec![
                    Span::styled("  • ", Style::default().fg(C_TRACK())),
                    Span::styled(
                        format!("{} ", p.engine),
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("on {} ", p.accel),
                        Style::default().fg(C_ACC()),
                    ),
                    Span::styled(g, Style::default().fg(gc)),
                ]));
                lines.push(Line::from(Span::styled(
                    format!(
                        "      {} · {}×{} = {} device(s) · free {} / need {} · artifact:{}",
                        p.resource,
                        p.count.max(1),
                        p.replicas.max(1),
                        p.count.max(1) * p.replicas.max(1),
                        free,
                        need,
                        if p.requires_artifact { "required" } else { "not needed" },
                    ),
                    Style::default().fg(C_DIM()),
                )));
                if !p.uri.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("      source: {}", p.uri),
                        Style::default().fg(C_TRACK()),
                    )));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "d deploy · c RBLN / f Furiosa compile (지원 모델) · esc back",
                Style::default().fg(C_DIM()),
            )));
            format!("catalog · {}", m.id)
        }
        Some(LibItem::Stored(k)) => {
            let s = &app.snap.stored[k];
            lines.push(kv("repo", &s.repo, Color::White));
            lines.push(kv("family", &s.family, C_ACC()));
            lines.push(kv("format", &s.format, C_ACC()));
            lines.push(kv(
                "compiled-for",
                if s.compiled_for.is_empty() { "-" } else { &s.compiled_for },
                if s.format == "hf" { C_DIM() } else { C_WARN() },
            ));
            // compiled_for 인코딩을 풀어 "무슨 옵션으로 컴파일됐는지" 명시.
            let opts = crate::app::decode_compiled_for(&s.compiled_for);
            if s.format != "hf" && !opts.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  compile options — 이 빌드에 박힌 옵션",
                    Style::default().fg(C_ACC()),
                )));
                for (label, val) in opts {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("    {:<18}", label),
                            Style::default().fg(C_DIM()),
                        ),
                        Span::styled(val, Style::default().fg(Color::White)),
                    ]));
                }
            }
            lines.push(kv(
                "revision",
                if s.revision.is_empty() { "-" } else { &s.revision },
                C_DIM(),
            ));
            lines.push(kv("size", &s.size, C_DIM()));
            lines.push(kv("path", &s.path, C_TRACK()));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "d deploy this build → Deployment · esc back",
                Style::default().fg(C_DIM()),
            )));
            format!("store build · {}", s.repo)
        }
        None => {
            lines.push(Line::from(Span::styled(
                "(no selection)",
                Style::default().fg(C_DIM()),
            )));
            "detail".to_string()
        }
    };
    f.render_widget(
        Paragraph::new(lines).block(block(&format!("{} · esc back", title))),
        area,
    );
}

/// 드릴 pivot 안내 줄 — `pivot  [p] pods  [i] infra …`. 상세 패널·크로스레이어 내비 광고.
fn pivot_line(pivots: &[(&str, &str)]) -> Line<'static> {
    let mut sp = vec![Span::styled(
        "pivot  ",
        Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
    )];
    for (k, label) in pivots {
        sp.push(Span::styled(
            format!("[{}]", k),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ));
        sp.push(Span::styled(
            format!(" {}  ", label),
            Style::default().fg(C_DIM()),
        ));
    }
    Line::from(sp)
}

/// pivot 미리보기 한 줄 — `[k] label  <peek>`. 키 누르면 해당 레이어로 이동(preview 로 먼저 엿봄).
fn pivot_prev(key: &str, label: &str, preview: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  [{}] ", key),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{:<6} ", label), Style::default().fg(C_DIM())),
        Span::styled(preview.to_string(), Style::default().fg(Color::White)),
    ])
}

fn kv(k: &str, v: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<18} ", k), Style::default().fg(C_DIM())),
        Span::styled(v.to_string(), Style::default().fg(color)),
    ])
}

/// Perf 드릴다운 — 선택 모델 구간별 p50/p95/p99 + 지표별 시계열 타임라인 + E2E 버킷 히스토그램.
fn perf_detail_view(f: &mut Frame, area: Rect, app: &App, d: &PerfDetail) {
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

// ── Perf (EPP 정책용 성능/분배) ─────────────────────────
fn ms(v: f64) -> String {
    if v.is_nan() {
        "–".into()
    } else if v >= 1.0 {
        format!("{:.2}s", v)
    } else {
        format!("{:.0}ms", v * 1000.0)
    }
}
fn rate(v: f64) -> String {
    if v.is_nan() {
        "–".into()
    } else {
        format!("{:.2}", v)
    }
}
/// htop 식 braille 영역 그래프 타임라인 — 셀당 2×4 점으로 고해상도, 시점 값별 색(초록→빨강).
/// 최신값 오른쪽(now) 고정. 외부 크레이트 없이 프레임 버퍼 직접 렌더. ymax_opt=Some(100)→0~100.
#[allow(clippy::needless_range_loop)] // DOTS[dr][dc] 2D 인덱싱이 더 명확
fn bar_timeline(
    f: &mut Frame,
    area: Rect,
    app: &App,
    key: &str,
    label: &str,
    unit: &str,
    ymax_opt: Option<f64>,
) {
    let raw = app.hist_for(key);
    let cur = raw.last().copied().unwrap_or(0);
    let dmax = raw.iter().copied().max().unwrap_or(0);
    // 자동 축: 피크가 높이의 ~80~90% 를 채우도록(살짝 헤드룸). 세밀 계단이라 과도한 여백 없음.
    let ymax = ymax_opt
        .unwrap_or_else(|| nice_ceil((dmax as f64) * 1.05))
        .max(1.0);
    let cur_pct = (cur as f64 / ymax * 100.0).clamp(0.0, 100.0);
    let ttl = Line::from(vec![
        Span::styled(
            format!(" {} ", label),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("▏ now {}{} ", cur, unit),
            Style::default()
                .fg(grad_color(cur_pct))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("▏ max {}{} ", dmax, unit),
            Style::default().fg(C_DIM()),
        ),
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
    // htop 식 braille 영역 그래프: 셀당 2×4 점 → 가로2·세로4 고해상도. 시점 값에 따라 색(초록→빨강).
    let cols = inner.width as usize;
    let rows = inner.height as usize;
    let sub_cols = cols * 2; // braille 서브열(시간 샘플)
    let sub_rows = rows * 4; // braille 서브행(세로 해상도)
    let data: Vec<u64> = raw.iter().rev().take(sub_cols).rev().copied().collect(); // 오른쪽=now
    let n = data.len();
    // 서브열 gsc(왼→오)의 채움 높이(서브행, 바닥부터) / 값%
    let sample = |gsc: usize| -> Option<f64> {
        if gsc + n < sub_cols {
            return None;
        } // 데이터 없는 왼쪽
        Some(data[gsc + n - sub_cols] as f64)
    };
    // 점 비트: [세로행 0..4(위→아래)][가로열 0..2]
    const DOTS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let buf = f.buffer_mut();
    for cy in 0..rows {
        for cx in 0..cols {
            let mut bits: u8 = 0u8;
            let mut vmax = 0.0f64; // 이 셀 대표값(색)
            for dc in 0..2 {
                let gsc = cx * 2 + dc;
                let (h, vpct) = match sample(gsc) {
                    Some(v) => {
                        let frac = (v / ymax).clamp(0.0, 1.0);
                        ((frac * sub_rows as f64).round() as usize, frac * 100.0)
                    }
                    None => (0, 0.0),
                };
                if vpct > vmax {
                    vmax = vpct;
                }
                for dr in 0..4 {
                    let gsr = cy * 4 + dr; // 0=맨 위
                    if h > 0 && gsr >= sub_rows.saturating_sub(h) {
                        bits |= DOTS[dr][dc];
                    }
                }
            }
            if bits != 0 {
                let ch = char::from_u32(0x2800 + bits as u32).unwrap_or('⣿');
                buf[(inner.x + cx as u16, inner.y + cy as u16)]
                    .set_char(ch)
                    .set_fg(grad_color(vmax));
            }
        }
    }
}

/// "깔끔한" 축 상한으로 올림 — 세밀한 계단(1·1.5·2·2.5·3·4·5·6·8·10)으로 과도한 여백 방지.
/// 1/2/5 만 쓰면 231→500 처럼 2배 넘게 튀어 그래프가 납작해짐. 세밀 계단은 231→250(피크 92%).
fn nice_ceil(v: f64) -> f64 {
    if v <= 1.0 {
        return 1.0;
    }
    let mag = 10f64.powf(v.log10().floor());
    let n = v / mag;
    const STEPS: [f64; 10] = [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0];
    let step = STEPS
        .iter()
        .copied()
        .find(|&s| n <= s + 1e-9)
        .unwrap_or(10.0);
    step * mag
}

fn view_perf(f: &mut Frame, area: Rect, app: &App) {
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

/// 컴파일 진행바 스팬 — progress Some 이면 실측 determinate([███░░] 45%),
/// None 이면 indeterminate(2칸 이동 블록 + "···")로 "살아있음"을 표시(NPU 컴파일은 표준 % 부재).
fn compile_progress_bar(progress: Option<f32>, tick: u64, width: usize) -> Vec<Span<'static>> {
    let width = width.max(4);
    match progress {
        Some(p) => {
            let p = p.clamp(0.0, 1.0);
            let filled = ((p * width as f32).round() as usize).min(width);
            let bar: String = "█".repeat(filled) + &"░".repeat(width - filled);
            let col = if p >= 1.0 { C_OK() } else { C_ACC() };
            vec![
                Span::styled(bar, Style::default().fg(col)),
                Span::styled(
                    format!(" {:>3.0}%", p * 100.0),
                    Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
                ),
            ]
        }
        None => {
            let pos = (tick as usize) % width;
            let mut cells = vec!['░'; width];
            cells[pos] = '█';
            cells[(pos + 1) % width] = '█';
            let bar: String = cells.into_iter().collect();
            vec![
                Span::styled(bar, Style::default().fg(C_ACC())),
                Span::styled("  ···".to_string(), Style::default().fg(C_DIM())),
            ]
        }
    }
}

// ── Serving — 현재 서빙 중인 배포(라이브 아티팩트)를 균일 정렬표로. ──
// 컬럼: ● 상태(좌) · MODEL · ENGINE · TARGET(opts) · REP · NODE · t/s. o/O 정렬 · x/s/r/y/l/⏎ 액션.
fn view_serving(f: &mut Frame, area: Rect, app: &App) {
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

// ── Activity 패널 — compile Job + deploy rollout 통합 작업 피드(진행률 %/바 포함). ──
// Deploy 뷰 하단 패널로 렌더. active(포커스) 면 선택 하이라이트 + 활성 테두리.
fn activity_panel(f: &mut Frame, area: Rect, app: &App, active: bool) {
    let data = app.activity_rows();
    let rows: Vec<Row> = data
        .iter()
        .map(|r| {
            let color = match r.sev {
                2 => C_BAD(),
                0 => C_OK(),
                _ => C_WARN(),
            };
            let kind = if r.kind == "compile" {
                Span::styled("compile", Style::default().fg(C_ACC()))
            } else {
                Span::styled("deploy", Style::default().fg(C_OK()))
            };
            // STATUS 셀: 상태 텍스트(% 포함) + (진행 중 compile 이면) 진행바.
            let mut status_spans = vec![Span::styled(
                format!("{:<14} ", truncw(&r.status, 14)),
                Style::default().fg(color),
            )];
            if r.running_compile {
                status_spans.extend(compile_progress_bar(r.progress, app.tick, 10));
            }
            Row::new(vec![
                Cell::from(kind),
                Cell::from(truncw(&r.target, 40)),
                Cell::from(r.started()).style(Style::default().fg(C_DIM())),
                Cell::from(Line::from(status_spans)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),  // KIND
        Constraint::Min(20),    // TARGET
        Constraint::Length(8),  // STARTED
        Constraint::Length(32), // STATUS (%+bar)
    ];
    let title = format!(
        "Activity · compile + deploy · l logs · D delete{}",
        if active {
            count_suffix(app.selected, data.len())
        } else {
            String::new()
        }
    );
    let blk = if active {
        block_active(&title)
    } else {
        block(&title)
    };
    let table = Table::new(rows, widths)
        .header(hrow_sorted(&["KIND", "TARGET", "STARTED", "STATUS"], "", ""))
        .column_spacing(1)
        .block(blk);
    if active {
        let mut st = TableState::default();
        st.select(Some(app.selected));
        f.render_stateful_widget(
            table.row_highlight_style(hl_style()).highlight_symbol("▎"),
            area,
            &mut st,
        );
        list_scrollbar(f, area, data.len(), app.selected, 1);
    } else {
        f.render_widget(table, area);
    }
}

// ── Deploy▸Model List — 배포 가능한 모델(카탈로그 조직정의 + 스토어 컴파일본, family 로 묶음). ──
// 단일 패널: 배포 가능한 모든 것을 한 곳에서 고른다(진행 중 작업은 Activity 탭에서).
fn view_library(f: &mut Frame, area: Rect, app: &App) {
    use crate::app::LibItem;
    // Deploy = 2패널 세로 배치: 위 Model List(포커스 0) · 아래 Activity(포커스 1). Ctrl+w 로 전환.
    let focus = app.panel_focus;
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(9)])
        .split(area);
    let list_area = panes[0];

    // ── 통합 배포 트리: family › (카탈로그 모델 · 스토어 컴파일본). 재고 가능성 ✓⚙✗ / ◇ 스토어 빌드. ──
    let items = app.library_items();
    let fam_of = |it: LibItem| match it {
        LibItem::Catalog(k) => crate::app::App::catalog_family(&app.catalog[k]).to_lowercase(),
        LibItem::Stored(k) => app.snap.stored[k].family.to_lowercase(),
    };
    let mut ll: Vec<Line> = Vec::new();
    ll.push(Line::from(vec![
        Span::styled("catalog: ", Style::default().fg(C_DIM())),
        Span::styled("✓ ready ", Style::default().fg(C_OK())),
        Span::styled("⚙ needs-compile ", Style::default().fg(C_WARN())),
        Span::styled("✗ no-cap   ", Style::default().fg(C_BAD())),
        Span::styled("store: ", Style::default().fg(C_DIM())),
        Span::styled("◇ ", Style::default().fg(C_ACC())),
        tag("HF", C_ACC()),
        tag("RBLN", Color::Magenta),
        tag("RNGD", C_WARN()),
    ]));
    if items.is_empty() {
        ll.push(Line::from(Span::styled(
            "(배포 가능한 모델 없음 — catalog/models.yaml 또는 스토어 인벤토리)",
            Style::default().fg(C_DIM()),
        )));
    }
    let mut lsel_line = 0usize;
    let mut last_fam = String::new();
    for (pos, &it) in items.iter().enumerate() {
        let fam = fam_of(it);
        let fam_cnt = items.iter().filter(|&&x| fam_of(x) == fam).count();
        if fam != last_fam {
            if fam_cnt > 1 {
                ll.push(Line::from(vec![
                    Span::styled("▪ ", Style::default().fg(C_ACC())),
                    Span::styled(
                        fam.clone(),
                        Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            last_fam = fam.clone();
        }
        let indent = if fam_cnt > 1 { "  └ " } else { "" };
        let selected = focus == 0 && pos == app.selected;
        if selected {
            lsel_line = ll.len();
        }
        let mut sp: Vec<Span> = vec![Span::styled(indent.to_string(), Style::default().fg(C_TRACK()))];
        match it {
            LibItem::Catalog(k) => {
                let m = &app.catalog[k];
                let any_ready = m.placements.iter().any(|p| {
                    matches!(
                        crate::catalog::solve(p, &app.snap.inventory).0,
                        crate::catalog::Ready::Ready
                    )
                });
                let needs = m.placements.iter().any(|p| {
                    matches!(
                        crate::catalog::solve(p, &app.snap.inventory).0,
                        crate::catalog::Ready::NeedsArtifact
                    )
                });
                let (g, c) = if any_ready {
                    ("✓", C_OK())
                } else if needs {
                    ("⚙", C_WARN())
                } else {
                    ("✗", C_BAD())
                };
                sp.push(Span::styled(format!("{} ", g), Style::default().fg(c)));
                sp.push(Span::styled(
                    format!("{:<22} ", truncw(&m.id, 22)),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
                sp.push(Span::styled(
                    format!("{:<8} ", truncw(&m.role, 8)),
                    Style::default().fg(C_DIM()),
                ));
                let mut seen = std::collections::BTreeSet::new();
                for p in &m.placements {
                    let sig =
                        format!("{} {} {} {}", p.engine, p.accel, p.resource, p.uri).to_lowercase();
                    let (lbl, col) = if sig.contains("rbln")
                        || sig.contains("rebellions")
                        || sig.contains("atom")
                    {
                        ("RBLN", Color::Magenta)
                    } else if sig.contains("furiosa") || sig.contains("rngd") {
                        ("RNGD", C_WARN())
                    } else {
                        ("GPU", C_ACC())
                    };
                    if seen.insert(lbl) {
                        // 벤더 배지 + 디바이스 수요(count×replicas) — "어떤 가속기 몇 개로".
                        sp.push(tag(lbl, col));
                        sp.push(Span::styled(
                            format!("×{} ", p.count.max(1) * p.replicas.max(1)),
                            Style::default().fg(col),
                        ));
                    }
                }
                // 소스(HF id / PVC 경로) — 어디서 오는지.
                if let Some(p) = m.placements.first() {
                    sp.push(Span::styled(
                        truncw(&p.uri, 34),
                        Style::default().fg(C_TRACK()),
                    ));
                }
            }
            LibItem::Stored(k) => {
                let s = &app.snap.stored[k];
                let (tbadge, is_src) = match s.format.as_str() {
                    "rbln" => (tag("RBLN", Color::Magenta), false),
                    "furiosa" => (tag("RNGD", C_WARN()), false),
                    _ => (tag("HF", C_ACC()), true),
                };
                let label = if is_src {
                    if s.revision.is_empty() || s.revision == "-" {
                        "source weights".to_string()
                    } else {
                        format!("source @{}", truncw(&s.revision, 8))
                    }
                } else {
                    // compiled_for 를 사람이 읽는 옵션(tp/pp/seq/칩)으로 풀어 표시.
                    let toks: Vec<String> = crate::app::decode_compiled_for(&s.compiled_for)
                        .into_iter()
                        .filter_map(|(k, v)| match k {
                            "tensor-parallel" => Some(format!("tp{}", v)),
                            "pipeline-parallel" => Some(format!("pp{}", v)),
                            "max-seq-len" => Some(format!("seq{}", v)),
                            "npu-chip" => Some(v),
                            _ => None,
                        })
                        .collect();
                    if toks.is_empty() {
                        format!("compiled · {}", s.compiled_for)
                    } else {
                        format!("compiled · {}", toks.join(" "))
                    }
                };
                sp.push(Span::styled("◇ ", Style::default().fg(C_ACC())));
                sp.push(tbadge);
                sp.push(Span::styled(
                    format!(" {:<22} ", truncw(s.repo.rsplit('/').next().unwrap_or(&s.repo), 22)),
                    Style::default().fg(Color::White),
                ));
                sp.push(Span::styled(
                    format!("{:<22} ", truncw(&label, 22)),
                    Style::default().fg(if is_src { C_DIM() } else { C_WARN() }),
                ));
                sp.push(Span::styled(
                    format!("{} ", s.size),
                    Style::default().fg(C_DIM()),
                ));
            }
        }
        let mut line = Line::from(sp);
        if selected {
            line.style = Style::default().bg(C_HL()).add_modifier(Modifier::BOLD);
        }
        ll.push(line);
    }
    ll.push(Line::from(Span::styled(
        "⏎/a actions · d deploy · c RBLN / f Furiosa compile (지원 모델) · ◇=스토어 실재 빌드 · 아래 Activity 패널은 ^w",
        Style::default().fg(C_DIM()),
    )));
    let lvis = (list_area.height as usize).saturating_sub(2);
    let lscroll = if lsel_line + 2 > lvis {
        (lsel_line + 3).saturating_sub(lvis) as u16
    } else {
        0
    };
    let list_title = format!(
        "Model List · 배포 가능 (family › model/build › target){}",
        if focus == 0 {
            count_suffix(app.selected, items.len())
        } else {
            String::new()
        }
    );
    let list_blk = if focus == 0 {
        block_active(&list_title)
    } else {
        block(&list_title)
    };
    f.render_widget(
        Paragraph::new(ll).scroll((lscroll, 0)).block(list_blk),
        list_area,
    );

    // 아래 패널: 통합 Activity 피드(compile + deploy 상태·진행률).
    activity_panel(f, panes[1], app, focus == 1);
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
    let mempct = if a.mem_total_gb > 0.0 {
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
        Span::styled(format!("   {} ", branch), Style::default().fg(C_TRACK())),
        Span::styled(format!("{} ", hg), Style::default().fg(hc)),
        Span::styled(
            format!("{:<5}", a.disp()),
            Style::default()
                .fg(kind_color(a.kind))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{:<6} ", a.id), Style::default().fg(C_DIM())),
    ];
    sp.extend(dot_bar(a.util, 8, util_color(a.util)).spans);
    sp.push(Span::styled(
        format!(" {:>3.0}%", a.util),
        Style::default().fg(util_color(a.util)),
    ));
    sp.push(Span::styled(
        format!(
            "  {:>3.0}/{:>3.0}GB{} ",
            a.mem_used_gb,
            a.mem_total_gb,
            if a.unified_mem { "∪" } else { " " }
        ),
        Style::default().fg(mem_color(mempct)),
    ));
    sp.push(Span::styled(
        format!("  {:.0}°C", a.temp),
        Style::default().fg(temp_color(a.temp)),
    ));
    sp.push(Span::styled(
        format!("  {:>3.0}W", a.power),
        Style::default().fg(C_DIM()),
    ));
    if full && !a.mem_bw.is_nan() {
        sp.push(Span::styled(
            format!("  bw {:>3.0}%", a.mem_bw),
            Style::default().fg(grad_color(a.mem_bw)),
        ));
        if !a.clock_mhz.is_nan() {
            sp.push(Span::styled(
                format!("  {:.0}MHz", a.clock_mhz),
                Style::default().fg(C_DIM()),
            ));
        }
    }
    if !a.busy_model.is_empty() {
        sp.push(Span::styled(
            format!("  {}", truncw(&a.busy_model, if full { 40 } else { 26 })),
            Style::default().fg(C_ACC()),
        ));
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
fn view_topo(f: &mut Frame, area: Rect, app: &App) {
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

// ── Deploy▸Zoo — 벤더(Furiosa/Rebellions) 모델 zoo: prefetch/compile ──────────
fn view_zoo(f: &mut Frame, area: Rect, app: &App) {
    let order = app.order();
    let rows: Vec<Row> = order
        .iter()
        .map(|&i| {
            let z = &app.zoo[i];
            let vendors = App::zoo_vendors(&z.source);
            let vlabel = if vendors.is_empty() {
                "GPU only".to_string()
            } else {
                vendors
                    .iter()
                    .map(|v| match *v {
                        "furiosa" => "Furiosa",
                        "rbln" => "RBLN",
                        _ => *v,
                    })
                    .collect::<Vec<_>>()
                    .join("+")
            };
            let vcolor = if vendors.is_empty() { C_DIM() } else { C_WARN() };
            let (store_s, store_c) = if app.zoo_in_store(&z.source) {
                ("● built", C_OK())
            } else {
                ("○ –", C_DIM())
            };
            Row::new(vec![
                Cell::from(truncw(&z.display, 30)).style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(truncw(&z.source, 34)).style(Style::default().fg(Color::Gray)),
                Cell::from(vlabel).style(Style::default().fg(vcolor)),
                Cell::from(truncw(&z.role, 10)).style(Style::default().fg(C_DIM())),
                Cell::from(store_s).style(Style::default().fg(store_c)),
                Cell::from(truncw(&z.note, 24)).style(Style::default().fg(C_DIM())),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(30), // MODEL
        Constraint::Min(20),    // SOURCE (HF)
        Constraint::Length(14), // COMPILE
        Constraint::Length(10), // ROLE
        Constraint::Length(8),  // STORE
        Constraint::Length(24), // NOTE
    ];
    let header = ["MODEL", "SOURCE (HF)", "COMPILE", "ROLE", "STORE", "NOTE"];
    let title = format!(
        "Deploy▸Zoo · vendor model zoo · ⏎ Prefetch / Compile→vendor · then deploy from Library{}",
        count_suffix(app.selected, order.len())
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
    list_scrollbar(f, area, order.len(), app.selected, 1);
}

// ── Setup(Doctor) — 새 환경 부트스트랩 전제조건 점검 + 가이드된 조치 ──────────
fn view_setup(f: &mut Frame, area: Rect, app: &App) {
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
fn view_events(f: &mut Frame, area: Rect, app: &App) {
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

// ── Models artifacts 관점 헬퍼 ───────────
/// 변형 한 줄에 넣을 컴파일 옵션 요약 — 중요 순으로 몇 개만.
fn opts_summary(a: &crate::collect::ModelArtifact) -> String {
    const PRI: [&str; 12] = [
        "tp", "pp", "dp", "max-len", "batch", "bucket", "quant", "dtype", "kv-dtype", "npu",
        "devices", "device",
    ];
    let mut out: Vec<String> = Vec::new();
    for k in PRI {
        if let Some((_, v)) = a.opts.iter().find(|(kk, _)| kk == k) {
            out.push(format!("{}={}", k, v));
        }
        if out.len() >= 5 {
            break;
        }
    }
    // NPU 벤더 특유 키(rbln_*/furiosa_*)도 하나 끌어올림.
    if out.len() < 6 {
        if let Some((k, v)) = a.opts.iter().find(|(k, _)| {
            let l = k.to_lowercase();
            l.starts_with("rbln") || l.starts_with("furiosa")
        }) {
            out.push(format!("{}={}", k, truncw(v, 14)));
        }
    }
    out.join(" ")
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
