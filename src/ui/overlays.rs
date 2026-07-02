//! 오버레이 렌더러 — 컴파일/배포/목표 폼, 액션 메뉴, 매니페스트 미리보기.
//! (mod.rs 에서 분리 — 파일 크기·응집도 개선)

use super::*;
use crate::app::App;
use ratatui::widgets::{Clear, Paragraph};

/// NPU 컴파일 옵션 편집 폼 오버레이 — ↑↓ 필드, ←→ 프리셋, e 커스텀 입력, Enter 매니페스트.
/// 선택 인프라 대비 메모리 적합성(OOM/tight)과 조정 제안을 실시간 표시.
pub(super) fn compile_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.compile_form else { return };
    let fit = app.compile_fit(form);
    let full = f.area();
    // 높이: 헤더3 + family1 + 필드 + 도움말1 + fit 2 + 추정근거1 + tips + 여백.
    let h = (form.fields.len() as u16) + (fit.tips.len() as u16) + 13;
    let area = centered(full, 92, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  model  ", Style::default().fg(C_DIM())),
        Span::styled(form.model_id.clone(), Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("   engine {}   ~{}", form.engine, fit.params_b.map(|p| format!("{}B", fmt_num(p))).unwrap_or_else(|| "?".into())),
            Style::default().fg(C_DIM()),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  target ", Style::default().fg(C_DIM())),
        Span::styled(
            format!("compiled/{}/{}/{}", form.model_id.replace('/', "--"), form.vendor, form.target()),
            Style::default().fg(C_ACC()),
        ),
    ]));
    // 지원 계열(벤더 공식 목록) — 매칭되면 family/arch/note 표시.
    if let Some(fam) = crate::compat::family_of(&form.model_id) {
        let mut txt = format!("  family {} ({})", fam.name, fam.arch);
        if !fam.note.is_empty() {
            txt.push_str(&format!(" · {}", fam.note));
        }
        lines.push(Line::from(Span::styled(txt, Style::default().fg(C_OK()))));
    }
    lines.push(Line::from(""));
    for (i, fld) in form.fields.iter().enumerate() {
        let active = i == form.cursor;
        lines.push(choice_row(fld, active, active && form.editing));
    }
    lines.push(Line::from(""));
    // 활성 필드 도움말
    if let Some(fld) = form.fields.get(form.cursor) {
        lines.push(Line::from(Span::styled(format!("  {}", fld.help), Style::default().fg(C_DIM()))));
    }
    // ── fit 추정 ──
    let vcol = match fit.verdict {
        crate::app::FitVerdict::Fits => C_OK(),
        crate::app::FitVerdict::Tight => C_WARN(),
        crate::app::FitVerdict::Oom => C_BAD(),
        crate::app::FitVerdict::Unknown => C_DIM(),
    };
    let glyph = match fit.verdict {
        crate::app::FitVerdict::Fits => "●",
        crate::app::FitVerdict::Tight => "◐",
        crate::app::FitVerdict::Oom => "✗",
        crate::app::FitVerdict::Unknown => "○",
    };
    lines.push(Line::from(vec![
        Span::styled("  fit~   ", Style::default().fg(C_DIM())),
        Span::styled(format!("{} {}", glyph, fit.verdict.label()), Style::default().fg(vcol).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(
                "   ≈{:.0}/{:.0} GiB/chip × {} chip   (w {:.0} + kv {:.0} + oh {:.0})",
                fit.per_chip_gb, fit.avail_gb, fit.chips as i64, fit.weight_gb, fit.kv_gb, fit.overhead_gb
            ),
            Style::default().fg(C_DIM()),
        ),
    ]));
    // 추정 근거·한계 명시(이름 기반 휴리스틱 — 과신 방지).
    lines.push(Line::from(Span::styled(
        "  rough estimate: params from name · KV linear proxy · dtype guessed — verify on real compile",
        Style::default().fg(C_DIM()),
    )));
    for tip in &fit.tips {
        let tcol = if tip.starts_with('⚠') { C_BAD() } else { C_WARN() };
        lines.push(Line::from(Span::styled(format!("   → {}", tip), Style::default().fg(tcol))));
    }
    let title = if form.editing {
        format!("compile · {} · TYPING custom — Enter/Esc confirm · Backspace del", form.vendor)
    } else {
        format!("compile · {} · ↑↓ row · ←→ pick · e custom · Enter → manifest · q cancel", form.vendor)
    };
    f.render_widget(Paragraph::new(lines).block(block(&title)), area);
}

/// 배포(서빙) 옵션 편집 폼 오버레이 — replicas·디바이스·노드 배치. 용량(수요 대 총/유휴) 표시.
pub(super) fn deploy_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.deploy_form else { return };
    let fit = app.deploy_fit(form);
    let full = f.area();
    let h = (form.fields.len() as u16) + (fit.tips.len() as u16) + 12;
    let area = centered(full, 92, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  model  ", Style::default().fg(C_DIM())),
        Span::styled(form.model_id.clone(), Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD)),
        Span::styled(format!("   engine {}", form.engine), Style::default().fg(C_DIM())),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  serve  ", Style::default().fg(C_DIM())),
        Span::styled(form.mount.clone(), Style::default().fg(C_ACC())),
    ]));
    lines.push(Line::from(""));
    for (i, fld) in form.fields.iter().enumerate() {
        let active = i == form.cursor;
        lines.push(choice_row(fld, active, active && form.editing));
    }
    lines.push(Line::from(""));
    if let Some(fld) = form.fields.get(form.cursor) {
        lines.push(Line::from(Span::styled(format!("  {}", fld.help), Style::default().fg(C_DIM()))));
    }
    let vcol = match fit.verdict {
        crate::app::FitVerdict::Fits => C_OK(),
        crate::app::FitVerdict::Tight => C_WARN(),
        crate::app::FitVerdict::Oom => C_BAD(),
        crate::app::FitVerdict::Unknown => C_DIM(),
    };
    let glyph = match fit.verdict {
        crate::app::FitVerdict::Fits => "●",
        crate::app::FitVerdict::Tight => "◐",
        crate::app::FitVerdict::Oom => "✗",
        crate::app::FitVerdict::Unknown => "○",
    };
    lines.push(Line::from(vec![
        Span::styled("  cap    ", Style::default().fg(C_DIM())),
        Span::styled(format!("{} {}", glyph, fit.verdict.label()), Style::default().fg(vcol).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("   demand {} dev   free {} (res {}) / {} over {} node", fit.demand, fit.free, fit.resource_free, fit.total, fit.nodes),
            Style::default().fg(C_DIM()),
        ),
    ]));
    for tip in &fit.tips {
        let tcol = if tip.starts_with('⚠') { C_BAD() } else { C_WARN() };
        lines.push(Line::from(Span::styled(format!("   → {}", tip), Style::default().fg(tcol))));
    }
    let title = if form.editing {
        "deploy · TYPING custom — Enter/Esc confirm · Backspace del".to_string()
    } else {
        "deploy · ↑↓ row · ←→ pick · e custom · Enter → manifest · q cancel".to_string()
    };
    f.render_widget(Paragraph::new(lines).block(block(&title)), area);
}

/// Enter 컨텍스트 액션 메뉴 오버레이 — 가능한 동작을 라벨+설명+단축키로. 발견 가능한 UX.
pub(super) fn action_menu_overlay(f: &mut Frame, app: &App) {
    let Some(menu) = &app.action_menu else { return };
    let full = f.area();
    let w = 58u16;
    let h = (menu.items.len() as u16) + 4;
    let area = centered(full, w, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    for (i, it) in menu.items.iter().enumerate() {
        let active = i == menu.cursor;
        let marker = if active { "▶ " } else { "  " };
        let base = if active {
            Style::default().fg(C_HL()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let mut line = Line::from(vec![
            Span::styled(marker, base),
            Span::styled(format!("[{}] ", it.key), Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<9}", it.label), base),
            Span::styled(it.desc.to_string(), Style::default().fg(C_DIM())),
        ]);
        if active {
            line.style = Style::default().bg(C_HL());
        }
        lines.push(line);
    }
    f.render_widget(
        Paragraph::new(lines).block(block(&format!("{} · ↑↓ + Enter · [key] shortcut · q cancel", truncw(&menu.title, 40)))),
        area,
    );
}

/// 모델명 → 안정적인 색(디바이스 점유 LED 를 모델별로 구분). 이름 해시로 팔레트 선택.
pub(super) fn model_color(name: &str) -> Color {
    // 테마 무관하게 잘 보이는 6색 팔레트.
    const PAL: [Color; 6] = [
        Color::Rgb(137, 180, 250), // blue
        Color::Rgb(166, 227, 161), // green
        Color::Rgb(250, 179, 135), // peach
        Color::Rgb(203, 166, 247), // mauve
        Color::Rgb(148, 226, 213), // teal
        Color::Rgb(242, 205, 205), // rosewater
    ];
    let h: u32 = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    PAL[(h as usize) % PAL.len()]
}

/// 옵션 한 줄 — 모든 후보를 인라인으로(선택만 강조). k9s 처럼 후보군이 한눈에.
/// editing 이면 자유 입력 버퍼를 보여줌. 커스텀 값(프리셋에 없음)은 노란 강조 토큰.
pub(super) fn choice_row(fld: &crate::app::CompileField, active: bool, editing: bool) -> Line<'static> {
    let name_style = if active {
        Style::default().fg(C_HL()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let mut sp = vec![
        Span::styled(if active { "▶ " } else { "  " }, name_style),
        Span::styled(format!("{:<17}", fld.label), name_style),
    ];
    if editing {
        sp.push(Span::styled(format!("[ {}_ ]", fld.value), Style::default().fg(C_WARN()).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)));
    } else {
        let mut matched = false;
        for c in &fld.choices {
            let is = *c == fld.value;
            matched |= is;
            let st = if is {
                Style::default().fg(Color::Black).bg(C_ACC()).add_modifier(Modifier::BOLD)
            } else if active {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().fg(C_DIM())
            };
            sp.push(Span::styled(format!(" {} ", c), st));
            sp.push(Span::raw(" "));
        }
        if !matched && !fld.value.is_empty() {
            sp.push(Span::styled(format!(" {} ", fld.value), Style::default().fg(Color::Black).bg(C_WARN()).add_modifier(Modifier::BOLD)));
        }
    }
    Line::from(sp)
}

/// 서빙 목표(SLO) 편집 폼 오버레이 — TTFT/TPOT/E2E/tok·s 목표를 그리드로.
pub(super) fn objective_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.objective_form else { return };
    let full = f.area();
    let area = centered(full, 72, (form.fields.len() as u16) + 8);
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  model  ", Style::default().fg(C_DIM())),
        Span::styled(form.model.clone(), Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD)),
        Span::styled("   목표를 정하면 Perf 뷰에서 충족여부·조정제안을 표시", Style::default().fg(C_DIM())),
    ]));
    lines.push(Line::from(""));
    for (i, fld) in form.fields.iter().enumerate() {
        let active = i == form.cursor;
        lines.push(choice_row(fld, active, active && form.editing));
    }
    lines.push(Line::from(""));
    if let Some(fld) = form.fields.get(form.cursor) {
        lines.push(Line::from(Span::styled(format!("  {}", fld.help), Style::default().fg(C_DIM()))));
    }
    let title = if form.editing {
        "objective · TYPING — Enter/Esc confirm".to_string()
    } else {
        "objective (SLO) · ↑↓ row · ←→ pick · e custom · Enter save · q cancel".to_string()
    };
    f.render_widget(Paragraph::new(lines).block(block(&title)), area);
}

/// 파라미터 수 표시 — 정수면 정수, 소수면 한 자리(예: 8, 1.5, 0.5).
pub(super) fn fmt_num(v: f64) -> String {
    if (v.fract()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{:.1}", v)
    }
}

/// 매니페스트 미리보기 오버레이(compile/deploy dry-run) — YAML 을 그대로 표시. `q`/esc 닫기, ↑↓ 스크롤.
pub(super) fn preview_overlay(f: &mut Frame, app: &App) {
    let Some((title, body)) = &app.preview else { return };
    let full = f.area();
    let area = centered(full, 92, 30);
    f.render_widget(Clear, area);
    let lines: Vec<Line> = body
        .lines()
        .map(|l| {
            let col = if l.trim_start().starts_with("# TODO") || l.contains("TODO-") {
                C_WARN()
            } else if l.trim_start().starts_with('#') {
                C_DIM()
            } else {
                Color::Gray
            };
            Line::from(Span::styled(l.to_string(), Style::default().fg(col)))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).scroll((app.preview_scroll, 0)).block(block(&format!(
            "{} · ↑↓ scroll · w save{} · q close",
            title,
            if app.preview_apply { " · v validate · a apply(admin)" } else { "" }
        ))),
        area,
    );
}
