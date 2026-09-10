//! Reusable render widgets/helpers — bars·gauges·not timelines (those live in timeline)·tables·blocks·string truncation, etc.
use super::theme::*;
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ── helpers ────────────────────────────────────────────
pub(crate) fn dwidth(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}
pub(crate) fn truncw(s: &str, max: usize) -> String {
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

/// Truncate to `n` display columns, then pad to exactly `n` columns.
///
/// `format!("{:<n}", truncw(s, n))` looks equivalent but is not: `{:<n}` counts **chars** while
/// `truncw` counts **columns**, so a CJK name (5 chars / 9 columns) got 15 spaces appended and
/// occupied 24 columns instead of 20 — shifting every column to its right (BUG-12).
/// Format a byte count the way `df -h` does, so it can be compared against a shell by eye.
///
/// Binary units with single-letter suffixes (`45.9T`), one decimal below 10 and none above,
/// which is `df -h`'s own rounding. Named for the base it uses rather than "si", because these
/// are powers of 1024 and calling them SI would be a lie the caller cannot see.
pub(crate) fn iec_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{}{}", n, UNITS[0])
    } else if v < 10.0 {
        format!("{:.1}{}", v, UNITS[i])
    } else {
        format!("{:.0}{}", v, UNITS[i])
    }
}

pub(crate) fn padw(s: &str, n: usize) -> String {
    let t = truncw(s, n);
    let w = dwidth(&t);
    let mut out = t;
    out.extend(std::iter::repeat_n(' ', n.saturating_sub(w)));
    out
}

/// Dot gauge — filled = colored dots (●), empty = dim dots (·). Each dot is a discrete tick. Font-safe.
pub(crate) fn dot_bar(pct: f64, cells: usize, color: Color) -> Line<'static> {
    let filled = ((pct.clamp(0.0, 100.0) / 100.0) * cells as f64).round() as usize;
    let tick_step = ((cells as f64) * 0.10).round() as usize; // 10% ticks (only on wide bars)
    let mut sp: Vec<Span> = Vec::with_capacity(cells);
    for i in 0..cells {
        if tick_step >= 2 && i > 0 && i % tick_step == 0 {
            sp.push(Span::styled(
                "│".to_string(),
                Style::default().fg(Color::Indexed(244)),
            ));
        } else if i < filled {
            sp.push(Span::styled("●".to_string(), Style::default().fg(color)));
        } else {
            sp.push(Span::styled(
                "·".to_string(),
                Style::default().fg(C_TRACK()),
            ));
        }
    }
    Line::from(sp)
}

pub(crate) fn bar_line(pct: f64, width: usize, color: Color) -> Line<'static> {
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

/// all-smi style stacked bar: joins segments (value, color) proportionally, the rest is track.
/// For showing "what takes up how much", e.g. heterogeneous accelerator VRAM makeup (GPU|RBLN|RNGD|free).
pub(crate) fn stacked_bar(
    segments: &[(f64, Color)],
    total: f64,
    width: usize,
) -> Vec<Span<'static>> {
    // Per-cell segment bar + 5% tick edges (┊). Ticks visible every 5% (roughly at width≥20).
    if width == 0 {
        return Vec::new();
    }
    let used: f64 = segments.iter().map(|(v, _)| *v).sum();
    let tick_step = ((width as f64) * 0.10).round() as usize; // 10% ticks
    let mut spans: Vec<Span> = Vec::with_capacity(width);
    for i in 0..width {
        if tick_step >= 2 && i > 0 && i % tick_step == 0 {
            spans.push(Span::styled(
                "│".to_string(),
                Style::default().fg(Color::Indexed(244)),
            ));
            continue;
        }
        let center = (i as f64 + 0.5) / width as f64 * total;
        if total > 0.0 && center < used {
            let mut cum = 0.0;
            let mut col = C_TRACK();
            for (v, c) in segments {
                if center < cum + *v {
                    col = *c;
                    break;
                }
                cum += *v;
            }
            spans.push(Span::styled("█".to_string(), Style::default().fg(col)));
        } else {
            spans.push(Span::styled(
                "░".to_string(),
                Style::default().fg(C_TRACK()),
            ));
        }
    }
    spans
}
/// all-smi style gauge row: `label  ██████░░░░  value`.
/// pct = bar fill (0~100), value = current-value text on the right, color = value color.
pub(crate) fn gauge_row(
    label: &str,
    pct: f64,
    value: &str,
    color: Color,
    barw: usize,
) -> Line<'static> {
    let mut sp = vec![Span::styled(
        format!("{:<8} ", label),
        Style::default().fg(C_DIM()),
    )];
    sp.extend(dot_bar(pct, barw, color).spans);
    sp.push(Span::styled(
        format!("  {}", value),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    Line::from(sp)
}

/// Colored inline sparkline — each dot colored by value (0..max) via grad_color (green→red). Last `width` points, right = now.
/// If data is short, pad the left with track dots (·) to fix the width (keeps alignment).
pub(crate) fn spark_colored(data: &[u64], width: usize, max: u64) -> Vec<Span<'static>> {
    const B: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let slice: &[u64] = if data.len() > width {
        &data[data.len() - width..]
    } else {
        data
    };
    let mx = if max > 0 {
        max
    } else {
        (*slice.iter().max().unwrap_or(&1)).max(1)
    };
    let mut sp: Vec<Span> = Vec::with_capacity(width);
    for _ in 0..width.saturating_sub(slice.len()) {
        sp.push(Span::styled(
            "·".to_string(),
            Style::default().fg(C_TRACK()),
        ));
    }
    for &v in slice {
        let frac = (v as f64 / mx as f64).clamp(0.0, 1.0);
        let idx = (frac * 7.0).round().clamp(0.0, 7.0) as usize;
        sp.push(Span::styled(
            B[idx].to_string(),
            Style::default().fg(grad_color(frac * 100.0)),
        ));
    }
    sp
}

/// Balance-lay out n tiles in the area (responsive column count, by width). Returned Rect order is left→right, top→bottom.
/// min_w = tile minimum width (limits columns so tiles never get narrower). If the last row is partial, grid alignment is kept (trailing gap).
pub(crate) fn tile_rects(area: Rect, n: usize, min_w: u16) -> Vec<Rect> {
    if n == 0 || area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let cols = ((area.width / min_w.max(1)) as usize).clamp(1, n);
    let rows = n.div_ceil(cols);
    let row_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, rows as u32); rows])
        .split(area);
    let mut out = Vec::with_capacity(n);
    for (r, rr) in row_rects.iter().enumerate() {
        let in_this = (n - r * cols).min(cols);
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, cols as u32); cols])
            .split(*rr);
        for c in 0..in_this {
            out.push(cells[c]);
        }
    }
    out
}

/// all-smi style inline text sparkline (▁▂▃▄▅▆▇█). Last `width` points, normalized against max.
pub(crate) fn sparkstr(data: &[u64], width: usize, max: u64) -> String {
    const B: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let slice: &[u64] = if data.len() > width {
        &data[data.len() - width..]
    } else {
        data
    };
    let mx = if max > 0 {
        max
    } else {
        (*slice.iter().max().unwrap_or(&1)).max(1)
    };
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

pub(crate) fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(x) if !x.is_nan() => format!("{:.0}", x),
        _ => "–".into(),
    }
}
pub(crate) fn fmt_nan(v: f64, dec: usize) -> String {
    if v.is_nan() {
        "–".into()
    } else {
        format!("{:.*}", dec, v)
    }
}

pub(crate) fn block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_TRACK()))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ))
}

/// 활성(선택 대상) 패널용 블록 — 멀티패널 뷰에서 ↑↓가 움직이는 패널을 밝은 테두리로 강조.
pub(crate) fn block_active(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            format!(" ▸ {} ", title),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ))
}

/// 애니메이션 마퀴 — 폭 초과 시 tick 에 따라 가로 스크롤(선택 행 강조용). 이름은 대개 ASCII.
pub(crate) fn marquee(s: &str, width: usize, tick: u64) -> String {
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
pub(crate) fn dot(up: bool) -> Span<'static> {
    if up {
        Span::styled("● ", Style::default().fg(C_OK()))
    } else {
        Span::styled("○ ", Style::default().fg(C_DIM()))
    }
}

pub(crate) fn hrow(cols: &[&str]) -> Row<'static> {
    hrow_sorted(cols, "", "")
}

/// 헤더 행 — mark 와 일치하는 컬럼에 정렬 방향 화살표(arrow)를 붙이고 accent 강조.
/// mark 가 빈 문자열이거나 매칭 컬럼이 없으면 일반 헤더(= hrow)와 동일.
pub(crate) fn hrow_sorted(cols: &[&str], mark: &str, arrow: &str) -> Row<'static> {
    Row::new(
        cols.iter()
            .map(|c| {
                if !mark.is_empty() && *c == mark {
                    Cell::from(Span::styled(
                        format!("{}{}", c, arrow),
                        Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Cell::from(Span::styled(
                        c.to_string(),
                        Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
                    ))
                }
            })
            .collect::<Vec<_>>(),
    )
}

/// Selection bar style for background lists/tables. Dims itself while a modal overlay is open
/// (see `C_SEL_BG`) so the floating window's cursor is the only live one on screen.
pub(crate) fn hl_style() -> Style {
    let mut st = Style::default().bg(C_SEL_BG());
    if !modal_open() {
        st = st.add_modifier(Modifier::BOLD);
    }
    st
}

/// 리스트/테이블 오른쪽에 스크롤바(오버플로 표시). 블록 테두리 안쪽 세로로 렌더.
/// total=전체 항목 수, pos=현재 선택 인덱스, header=본문 위 고정 행(테이블 헤더=1, 없으면 0).
/// 화면보다 짧으면 그리지 않음.
pub(crate) fn list_scrollbar(f: &mut Frame, area: Rect, total: usize, pos: usize, header: usize) {
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
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 0,
    });
    f.render_stateful_widget(sb, inner, &mut st);
}

/// 균일한 리스트 테이블 렌더 — Table+헤더+선택 하이라이트+스크롤바+위치카운터 보일러플레이트 1곳.
/// (Accel/Pods/Events 등 표준 리스트 뷰 공용. 커스텀 레이아웃 뷰는 직접 그림.)
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_list_table(
    f: &mut Frame,
    area: Rect,
    rows: Vec<Row<'static>>,
    widths: &[Constraint],
    headers: &[&str],
    title: &str,
    sel: usize,
    total: usize,
    sort_mark: &str,
    sort_arrow: &str,
) {
    let t = Table::new(rows, widths.to_vec())
        .header(hrow_sorted(headers, sort_mark, sort_arrow))
        .column_spacing(1)
        .row_highlight_style(hl_style())
        .highlight_symbol("▎")
        .block(block(&format!("{}{}", title, count_suffix(sel, total))));
    let mut st = TableState::default();
    st.select(Some(sel));
    f.render_stateful_widget(t, area, &mut st);
    list_scrollbar(f, area, total, sel, 1);
}

/// 블록 타이틀용 위치 카운터 접미사 " · sel/total". total<=0 이면 빈 문자열.
pub(crate) fn count_suffix(sel: usize, total: usize) -> String {
    if total == 0 {
        " · 0".to_string()
    } else {
        format!(" · {}/{}", sel + 1, total)
    }
}

pub(crate) fn cellw(text: String, w: usize) -> Cell<'static> {
    Cell::from(truncw(&text, w))
}
#[cfg(test)]
mod width_tests {
    use super::*;

    /// The values the cluster's own `df -h /mnt/store` prints, so the view and a shell agree.
    #[test]
    fn iec_bytes_matches_df_h_for_the_real_store() {
        assert_eq!(iec_bytes(57_504_801_730_560), "52T"); // df -h: 52.3T
        assert_eq!(iec_bytes(50_466_213_605_376), "46T"); // df -h: 45.9T
        assert_eq!(iec_bytes(7_038_588_125_184), "6.4T"); // df -h: 6.4T
    }

    #[test]
    fn iec_bytes_scales_and_never_loses_the_unit() {
        assert_eq!(iec_bytes(0), "0B");
        assert_eq!(iec_bytes(512), "512B");
        assert_eq!(iec_bytes(1024), "1.0K");
        assert_eq!(iec_bytes(109 * 1024 * 1024), "109M");
        assert_eq!(iec_bytes(5_374_743), "5.1M");
        // Beyond the table's largest unit it must still say something finite.
        let huge = iec_bytes(u64::MAX);
        assert!(huge.ends_with('P'), "{}", huge);
    }

    /// BUG-12 회귀: 표 셀은 문자 수가 아니라 **표시 폭**으로 맞춰져야 한다.
    /// (한글 이름 한 개가 열 정렬 전체를 밀어내던 원인.)
    #[test]
    fn padw_pads_and_truncates_by_display_width() {
        for (s, n) in [
            ("ascii", 20),
            ("모델-서버", 20),   // 5 char / 9 columns
            ("한글이름아주긴것", 10),
            ("mixed-한글-name", 16),
            ("", 8),
            ("정확히열폭", 10),  // exactly n columns → unchanged, no padding
        ] {
            assert_eq!(
                dwidth(&padw(s, n)),
                n,
                "padw({:?}, {}) must occupy exactly {} columns",
                s,
                n,
                n
            );
        }
        // 짧은 값은 잘리지 않고 뒤에 공백만 붙는다.
        assert!(padw("모델", 10).starts_with("모델"));
        // 넘치면 잘리고 말줄임표가 붙는다.
        assert!(padw("한글이름아주긴것", 6).contains('…'));
    }

    /// 기존 `format!("{:<N}", truncw(s, N))` 조합이 왜 틀렸는지 고정 — 회귀 방지 설명용.
    #[test]
    fn char_padding_would_overflow_for_cjk() {
        let cjk = "모델-서버"; // 5 chars, 9 columns
        assert_eq!(dwidth(&format!("{:<20}", truncw(cjk, 20))), 24);
        assert_eq!(dwidth(&padw(cjk, 20)), 20);
    }
}

