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
mod perf;
use perf::*;
mod overview;
use overview::*;
mod detail;
use detail::*;
mod traffic;
use traffic::*;
mod serving;
use serving::*;
mod infra;
use infra::*;
mod deploy;
use deploy::*;
mod extras;
use extras::*;

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
    PrefetchForm,
    ActionMenu,
    Logs,
}

impl Overlay {
    /// topmost 우선. 입력 소비·z-order 공용. (Help 가 가장 위, Logs 가 가장 아래)
    pub const PRECEDENCE: [Overlay; 14] = [
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
        Overlay::PrefetchForm,
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
            Overlay::PrefetchForm => app.prefetch_form.is_some(),
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
            Overlay::PrefetchForm => prefetch_form_overlay(f, app),
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
