//! `--cast` — 합성 애니메이션 프레임을 asciicast v2 로 출력(외부 녹화 도구 없이).
//! 파이프라인: 실측 스냅샷 구조를 기반으로 값만 sin 파형으로 움직여 프레임 렌더 →
*Demo recording omitted from the public export.*
//! 이펙트(tachyonfx)는 렌더 경로에서 꺼지므로 GIF 는 "데이터 움직임 + 뷰 전환"을 보여줌.

use crate::app::{App, View};
use crate::collect::{PerfRow, Snapshot};
use crate::config::Config;
use ratatui::buffer::Buffer;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

const W: u16 = 148;
const H: u16 = 42;
const DT: f64 = 0.13; // 프레임 간격(초) — 전환을 살짝 느긋하게(≈7.7fps)

pub async fn run(cfg: &Config, out: &str) {
    let mut base = crate::collect::collect(cfg).await;
    // 트래픽 지표가 비어있으면(유휴 클러스터) 데모용 per-model 행을 모델에서 합성.
    if base.perf_rows.is_empty() {
        for m in base.models.iter().filter(|m| m.ready > 0).take(5) {
            base.perf_rows.push(nan_row(&m.name));
        }
    }
    crate::app::set_theme(cfg.theme); // 보통 soft
    let mut app = App::new();
    // 프리롤: 히스토리를 채워 스파크라인/타임라인이 꽉 찬 상태로 시작.
    for i in 0..HISTORY_PREROLL {
        app.apply(evolve(&base, i));
    }

    // SLO advisor 데모용: 첫 perf 모델에 목표 설정(관측 대비 판정·조언이 렌더됨).
    if let Some(r) = base.perf_rows.first() {
        app.objectives.insert(
            r.model.clone(),
            crate::app::Objective { ttft_ms: Some(2000.0), tpot_ms: Some(50.0), e2e_ms: Some(1500.0), min_tps: Some(150.0) },
        );
    }

    // 스토리보드: (뷰, detail, 프레임수, 특수연출) — 새 UI(허브·토폴로지·액션메뉴·SLO)를 두루 보여줌.
    let story: &[(View, bool, u32, Extra)] = &[
        (View::Overview, false, 28, Extra::None),
        (View::Nodes, false, 20, Extra::None),     // Nodes 허브: 노드+disk (w 로 전환)
        (View::Accel, false, 18, Extra::None),     // 허브: 디바이스 pressure
        (View::Topo, false, 26, Extra::None),      // 허브: 토폴로지/pressure 맵(Canvas)
        (View::Launch, false, 20, Extra::None),    // Deploy 변형(컴파일·배치)
        (View::Launch, false, 24, Extra::ActionMenu), // Enter 액션 메뉴
        (View::Perf, false, 26, Extra::Slo),       // 서빙 perf + SLO advisor
        (View::Models, true, 24, Extra::None),     // 모델 상세
    ];

    let mut events = String::new();
    let mut t = 0.0f64;
    let mut i = HISTORY_PREROLL;
    for &(view, detail, frames, extra) in story {
        for lf in 0..frames {
            app.apply(evolve(&base, i));
            app.tick = app.tick.wrapping_add(1);
            app.view = view;
            app.detail = detail;
            app.action_menu = None; // 기본은 닫힘(scene 별로 아래에서 연출)
            let n = app.list_len().max(1);
            match extra {
                Extra::None => {
                    app.selected = if detail { (i as usize / 40) % n } else { (lf as usize / 5) % n };
                }
                Extra::ActionMenu => {
                    // Deploy 변형 선택 → Enter 액션 메뉴 오픈 + 커서 천천히 이동.
                    app.panel_focus = 0;
                    app.selected = 0;
                    app.open_action_menu();
                    if let Some(m) = app.action_menu.as_mut() {
                        let mi = m.items.len().max(1);
                        m.cursor = (lf as usize / 5) % mi;
                    }
                }
                Extra::Slo => {
                    // 서빙 perf: SLO advisor 가 보이도록 선택을 목표설정 모델(0)에 고정.
                    app.panel_focus = 0;
                    app.selected = 0;
                }
            }
            let frame = render_frame(&app);
            events.push_str(&event_line(t, &frame));
            t += DT;
            i += 1;
        }
    }

    let header = format!("{{\"version\": 2, \"width\": {}, \"height\": {}, \"idle_time_limit\": 2}}\n", W, H);
    let doc = format!("{}{}", header, events);
    match std::fs::write(out, &doc) {
*Demo recording omitted from the public export.*
        Err(e) => eprintln!("cast write failed: {}", e),
    }
}

const HISTORY_PREROLL: u64 = 50;

/// 스토리보드 특수 연출 — 새 UI 요소를 캐스트에 노출.
#[derive(Clone, Copy)]
enum Extra {
    None,
    ActionMenu, // Enter 액션 메뉴 오버레이
    Slo,        // SLO advisor(목표 대비 판정)
}

fn nan_row(model: &str) -> PerfRow {
    let n = f64::NAN;
    PerfRow {
        model: model.to_string(),
        req: n, tps: n, ttft_p95: n, tpot_p95: n, e2e_p95: n, in_tok_p95: n, out_tok_p95: n,
        queue_p95: n, prefill_p95: n, decode_p95: n, preempt: n,
    }
}

/// 실측 구조를 유지한 채 변동 지표만 sin 파형으로 움직임. ts 를 바꿔 히스토리가 append 되게 함.
fn evolve(base: &Snapshot, i: u64) -> Snapshot {
    let mut s = base.clone();
    let ph = i as f64;
    let wave = |a: f64, b: f64| (a + b).sin() * 0.5 + 0.5; // 0..1
    for (idx, ac) in s.accel.iter_mut().enumerate() {
        let w = wave(ph * 0.25, idx as f64 * 0.7);
        ac.util = (w * 88.0 + 4.0).clamp(0.0, 100.0);
        let cap = ac.mem_total_gb.max(1.0);
        let used0 = if ac.mem_used_gb > 0.0 { ac.mem_used_gb } else { cap * 0.5 };
        ac.mem_used_gb = (used0 * (0.9 + 0.1 * wave(ph * 0.12, idx as f64))).min(cap);
        ac.temp = 34.0 + w * 38.0;
        ac.power = 18.0 + w * 90.0;
        ac.alive = true;
    }
    for (idx, n) in s.nodes.iter_mut().enumerate() {
        if !n.cpu_pct.is_nan() {
            n.cpu_pct = (wave(ph * 0.2, idx as f64) * 55.0).clamp(0.0, 100.0);
        }
    }
    s.perf.tps = 180.0 + 160.0 * wave(ph * 0.3, 0.0);
    s.perf.req_rate = 4.0 + 5.0 * wave(ph * 0.33, 1.0);
    s.perf.ttft_p95 = 0.08 + 0.25 * wave(ph * 0.3, 2.0);
    s.perf.e2e_p95 = 0.4 + 1.4 * wave(ph * 0.27, 3.0);
    for (idx, r) in s.perf_rows.iter_mut().enumerate() {
        let w = wave(ph * 0.28, idx as f64 * 0.9);
        r.tps = 40.0 + w * 260.0;
        r.req = 1.0 + w * 8.0;
        r.ttft_p95 = 0.05 + w * 0.4;
        r.tpot_p95 = 0.01 + w * 0.04;
        r.e2e_p95 = 0.3 + w * 1.5;
        r.queue_p95 = w * 0.2;
        r.prefill_p95 = 0.05 + w * 0.3;
        r.decode_p95 = 0.02 + w * 0.05;
    }
    s.ts = base.ts.wrapping_add(i).wrapping_add(1);
    s
}

fn render_frame(app: &App) -> String {
    let mut term = Terminal::new(TestBackend::new(W, H)).unwrap();
    let mut fx = crate::ui::FxState::disabled();
    term.draw(|fr| crate::ui::draw(fr, app, &mut fx)).unwrap();
    serialize(term.backend().buffer())
}

/// 버퍼 → ANSI. 스타일이 바뀔 때만 SGR 방출(파일 크기 절약). 각 행 끝에 리셋.
fn serialize(buf: &Buffer) -> String {
    let a = buf.area;
    let mut out = String::from("\x1b[H");
    for y in 0..a.height {
        let mut cur: Option<(Color, Color, Modifier)> = None;
        for x in 0..a.width {
            let c = buf.cell((x, y)).unwrap();
            let st = (c.fg, c.bg, c.modifier);
            if cur != Some(st) {
                out.push_str(&sgr(c.fg, c.bg, c.modifier));
                cur = Some(st);
            }
            out.push_str(c.symbol());
        }
        out.push_str("\x1b[0m\r\n");
    }
    out
}

fn sgr(fg: Color, bg: Color, m: Modifier) -> String {
    let mut codes: Vec<String> = vec!["0".into()];
    if m.contains(Modifier::BOLD) {
        codes.push("1".into());
    }
    if m.contains(Modifier::DIM) {
        codes.push("2".into());
    }
    if m.contains(Modifier::REVERSED) {
        codes.push("7".into());
    }
    codes.push(color_code(fg, true));
    codes.push(color_code(bg, false));
    format!("\x1b[{}m", codes.join(";"))
}

fn color_code(c: Color, fg: bool) -> String {
    let (b3, b8, bdef) = if fg { (30, 38, 39) } else { (40, 48, 49) };
    let bright = if fg { 90 } else { 100 };
    match c {
        Color::Reset => format!("{}", bdef),
        Color::Rgb(r, g, b) => format!("{};2;{};{};{}", b8, r, g, b),
        Color::Indexed(i) => format!("{};5;{}", b8, i),
        Color::Black => format!("{}", b3),
        Color::Red => format!("{}", b3 + 1),
        Color::Green => format!("{}", b3 + 2),
        Color::Yellow => format!("{}", b3 + 3),
        Color::Blue => format!("{}", b3 + 4),
        Color::Magenta => format!("{}", b3 + 5),
        Color::Cyan => format!("{}", b3 + 6),
        Color::Gray => format!("{}", b3 + 7),
        Color::DarkGray => format!("{}", bright),
        Color::LightRed => format!("{}", bright + 1),
        Color::LightGreen => format!("{}", bright + 2),
        Color::LightYellow => format!("{}", bright + 3),
        Color::LightBlue => format!("{}", bright + 4),
        Color::LightMagenta => format!("{}", bright + 5),
        Color::LightCyan => format!("{}", bright + 6),
        Color::White => format!("{}", bright + 7),
    }
}

fn event_line(t: f64, data: &str) -> String {
    format!("[{:.3}, \"o\", \"{}\"]\n", t, json_escape(data))
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}
