//! `--cast` — emit synthetic animation frames as asciicast v2 (no external recorder needed).
//! Pipeline: keep the real snapshot structure, animate only the values with a sin waveform to render frames →
//! serialize per-cell ANSI (SGR) → asciicast events. Convert to a GIF with `agg <file>.cast <file>.gif`.
//! Effects (tachyonfx) are off in the render path, so the GIF shows "data motion + view transitions".

use crate::app::{App, View};
use crate::collect::{PerfRow, Snapshot};
use crate::config::Config;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

const W: u16 = 148;
const H: u16 = 42;
const DT: f64 = 0.13; // frame interval (s) — makes transitions a bit leisurely (≈7.7fps)

pub async fn run(cfg: &Config, out: &str) {
    let mut base = crate::collect::collect(cfg).await;
    // If traffic metrics are empty (idle cluster), synthesize demo per-model rows from the models.
    if base.perf_rows.is_empty() {
        for m in base.models.iter().filter(|m| m.ready > 0).take(5) {
            base.perf_rows.push(nan_row(&m.name));
        }
    }
    crate::app::set_theme(cfg.theme); // usually soft
    let mut app = App::new();
    // Preroll: fill history so sparklines/timelines start already full.
    for i in 0..HISTORY_PREROLL {
        app.apply(evolve(&base, i));
    }

    // For the SLO advisor demo: set objectives on the first perf model (verdict/advice vs. observed renders).
    if let Some(r) = base.perf_rows.first() {
        app.objectives.insert(
            r.model.clone(),
            crate::app::Objective {
                ttft_ms: Some(2000.0),
                tpot_ms: Some(50.0),
                e2e_ms: Some(1500.0),
                min_tps: Some(150.0),
            },
        );
    }

    // Storyboard: (view, detail, frame count, special staging) — showcases the new UI (hub, topology, action menu, SLO).
    let story: &[(View, bool, u32, Extra)] = &[
        (View::Overview, false, 28, Extra::None),
        (View::Nodes, false, 20, Extra::None), // Nodes hub: nodes + disk (switch with w)
        (View::Accel, false, 18, Extra::None), // hub: device pressure
        (View::Topo, false, 26, Extra::None),  // hub: topology / pressure map (Canvas)
        (View::Serving, false, 22, Extra::None), // Serving: 정렬 가능한 배포 표(상태/시도/실패)
        (View::Library, false, 26, Extra::ActionMenu), // Deploy: Model List(위) + Activity(아래)
        (View::Perf, false, 26, Extra::Slo),   // serving perf + SLO advisor
        (View::Serving, true, 24, Extra::None), // 배포 상세(상태·타깃·옵션)
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
            app.action_menu = None; // closed by default (staged per scene below)
            let n = app.list_len().max(1);
            match extra {
                Extra::None => {
                    app.selected = if detail {
                        (i as usize / 40) % n
                    } else {
                        (lf as usize / 5) % n
                    };
                }
                Extra::ActionMenu => {
                    // Pick a Deploy variant → open the Enter action menu + move cursor slowly.
                    app.panel_focus = 0;
                    app.selected = 0;
                    app.open_action_menu();
                    if let Some(m) = app.action_menu.as_mut() {
                        let mi = m.items.len().max(1);
                        m.cursor = (lf as usize / 5) % mi;
                    }
                }
                Extra::Slo => {
                    // Serving perf: pin selection to the model with objectives (0) so the SLO advisor is visible.
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

    let header = format!(
        "{{\"version\": 2, \"width\": {}, \"height\": {}, \"idle_time_limit\": 2}}\n",
        W, H
    );
    let doc = format!("{}{}", header, events);
    match std::fs::write(out, &doc) {
        Ok(_) => eprintln!(
            "wrote {} ({} frames, {:.1}s) → agg {} <out>.gif",
            out,
            (t / DT) as u64,
            t,
            out
        ),
        Err(e) => eprintln!("cast write failed: {}", e),
    }
}

const HISTORY_PREROLL: u64 = 50;

/// Storyboard special staging — surfaces new UI elements in the cast.
#[derive(Clone, Copy)]
enum Extra {
    None,
    ActionMenu, // Enter action-menu overlay
    Slo,        // SLO advisor (verdict vs. objectives)
}

fn nan_row(model: &str) -> PerfRow {
    let n = f64::NAN;
    PerfRow {
        model: model.to_string(),
        req: n,
        tps: n,
        ttft_p95: n,
        tpot_p95: n,
        e2e_p95: n,
        in_tok_p95: n,
        out_tok_p95: n,
        queue_p95: n,
        prefill_p95: n,
        decode_p95: n,
        preempt: n,
    }
}

/// Keep the real structure; animate only the varying metrics with a sin waveform. Bump ts so history appends.
fn evolve(base: &Snapshot, i: u64) -> Snapshot {
    let mut s = base.clone();
    let ph = i as f64;
    let wave = |a: f64, b: f64| (a + b).sin() * 0.5 + 0.5; // 0..1
    for (idx, ac) in s.accel.iter_mut().enumerate() {
        let w = wave(ph * 0.25, idx as f64 * 0.7);
        ac.util = (w * 88.0 + 4.0).clamp(0.0, 100.0);
        let cap = ac.mem_total_gb.max(1.0);
        let used0 = if ac.mem_used_gb > 0.0 {
            ac.mem_used_gb
        } else {
            cap * 0.5
        };
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
    s.sources.clear(); // Synthetic animation uses its own timeline, never live freshness metadata.
    s
}

fn render_frame(app: &App) -> String {
    let mut term = Terminal::new(TestBackend::new(W, H)).unwrap();
    let mut fx = crate::ui::FxState::disabled();
    term.draw(|fr| crate::ui::draw(fr, app, &mut fx)).unwrap();
    serialize(term.backend().buffer())
}

/// Buffer → ANSI. Emit SGR only when style changes (saves file size). Reset at the end of each row.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_specials_controls_and_unicode() {
        assert_eq!(json_escape("plain text"), "plain text");
        // 따옴표·역슬래시.
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
        // 개행·탭·캐리지리턴.
        assert_eq!(json_escape("l1\nl2\tx\r"), "l1\\nl2\\tx\\r");
        // 기타 제어문자(NUL, ESC)는 \uXXXX.
        assert_eq!(json_escape("\u{0}\u{1b}"), "\\u0000\\u001b");
        // 비제어 유니코드는 그대로 유지.
        assert_eq!(json_escape("한글✓"), "한글✓");
    }
}
