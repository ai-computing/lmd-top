//! 오버레이 렌더러 — 컴파일/배포/목표 폼, 액션 메뉴, 매니페스트 미리보기.
//! (mod.rs 에서 분리 — 파일 크기·응집도 개선)

use super::*;
use crate::app::App;
use ratatui::widgets::{Clear, Paragraph, Wrap};

/// NPU 컴파일 옵션 편집 폼 오버레이 — ↑↓ 필드, ←→ 프리셋, e 커스텀 입력, Enter 매니페스트.
/// 선택 인프라 대비 메모리 적합성(OOM/tight)과 조정 제안을 실시간 표시.
pub(super) fn compile_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.compile_form else {
        return;
    };
    let fit = app.compile_fit(form);
    let full = f.area();
    // 높이: 헤더3 + family1 + 필드 + 도움말1 + fit 2 + 추정근거1 + tips + 여백.
    let h = (form.fields.len() as u16)
        + (fit.tips.len() as u16)
        + (app.compile_preflight(form).len() as u16)
        + {
            let a = app.compile_advice(&form.model_id, form.vendor);
            1 + a.avoid.len().min(2) as u16
                + u16::from(a.next.is_some())
                + crate::advisor::toolchain_warnings(&app.history)
                    .iter()
                    .filter(|(v, _)| v == form.vendor)
                    .count() as u16
        }
        + 14;
    let area = centered(full, 92, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  model  ", Style::default().fg(C_DIM())),
        Span::styled(
            form.model_id.clone(),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "   engine {}   ~{}",
                form.engine,
                fit.params_b
                    .map(|p| format!("{}B", fmt_num(p)))
                    .unwrap_or_else(|| "?".into())
            ),
            Style::default().fg(C_DIM()),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  target ", Style::default().fg(C_DIM())),
        Span::styled(
            format!(
                "compiled/{}/{}/{}",
                form.model_id.replace('/', "--"),
                form.vendor,
                form.target()
            ),
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
    // Active field help.
    if let Some(fld) = form.fields.get(form.cursor) {
        lines.push(Line::from(Span::styled(
            format!("  {}", fld.help),
            Style::default().fg(C_DIM()),
        )));
    }
    let (vcol, glyph, hint) = match fit.verdict {
        crate::app::FitVerdict::Fits => (C_OK(), "●", "fits"),
        crate::app::FitVerdict::Tight => (C_WARN(), "◐", "tight"),
        crate::app::FitVerdict::Oom => (
            C_WARN(),
            "◐",
            "may not fit per chip; consider tp↑ or max-len↓",
        ),
        crate::app::FitVerdict::Unknown => (C_DIM(), "○", "unknown"),
    };
    lines.push(Line::from(vec![
        Span::styled("  serving fit~ ", Style::default().fg(C_DIM())),
        Span::styled(
            format!("{} {}", glyph, hint),
            Style::default().fg(vcol).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "   ≈{:.0}/{:.0} GiB/chip × {} chip   (w {:.0} + kv {:.0} + oh {:.0})",
                fit.per_chip_gb,
                fit.avail_gb,
                fit.chips as i64,
                fit.weight_gb,
                fit.kv_gb,
                fit.overhead_gb
            ),
            Style::default().fg(C_DIM()),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "  Advisory estimate only (name-based params + linear KV approximation); compile is not blocked.",
        Style::default().fg(C_DIM()),
    )));
    for tip in &fit.tips {
        let tcol = if tip.starts_with('⚠') {
            C_BAD()
        } else {
            C_WARN()
        };
        lines.push(Line::from(Span::styled(
            format!("   → {}", tip),
            Style::default().fg(tcol),
        )));
    }
    // What has actually worked here before. Silent when there is no history — a recommendation
    // drawn from nothing would read as authoritative.
    let advice = app.compile_advice(&form.model_id, form.vendor);
    if let Some(best) = &advice.best {
        lines.push(Line::from(vec![
            Span::styled("  ↺ history  ", Style::default().fg(C_DIM())),
            Span::styled(
                best.options_line(),
                Style::default().fg(C_OK()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({})", best.reason),
                Style::default().fg(C_DIM()),
            ),
        ]));
    }
    // A dependency skew known to break this vendor's compiler outranks any option advice —
    // no parameter fixes it, and every build will fail until it is addressed on the host.
    for (_, why) in crate::advisor::toolchain_warnings(&app.history)
        .into_iter()
        .filter(|(v, _)| v == form.vendor)
    {
        lines.push(Line::from(vec![
            Span::styled("  ⚠ toolchain ", Style::default().fg(C_BAD())),
            Span::styled(
                truncw(&why, 86),
                Style::default().fg(C_BAD()).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    if let Some(next) = &advice.next {
        lines.push(Line::from(vec![
            Span::styled("  ↺ next     ", Style::default().fg(C_DIM())),
            Span::styled(
                next.options_line(),
                Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  ({})", next.reason), Style::default().fg(C_DIM())),
        ]));
    }
    for bad in advice.avoid.iter().take(2) {
        lines.push(Line::from(vec![
            Span::styled("   ✗ avoid   ", Style::default().fg(C_BAD())),
            Span::styled(bad.options_line(), Style::default().fg(C_WARN())),
            Span::styled(format!("  ({})", bad.reason), Style::default().fg(C_DIM())),
        ]));
    }
    if advice.best.is_none() && advice.avoid.is_empty() && advice.compiles_seen == 0 {
        lines.push(Line::from(Span::styled(
            "  ↺ history  no previous build of this model on this accelerator yet",
            Style::default().fg(C_DIM()),
        )));
    }

    let pf = app.compile_preflight(form);
    let pf_ok = pf.iter().all(|(ok, _)| *ok);
    lines.push(Line::from(Span::styled(
        format!(
            "  preflight {}",
            if pf_ok { "● ready" } else { "✗ blocked" }
        ),
        Style::default()
            .fg(if pf_ok { C_OK() } else { C_BAD() })
            .add_modifier(Modifier::BOLD),
    )));
    for (ok, msg) in &pf {
        let warn = msg.starts_with('⚠');
        let (g, c) = if warn {
            ("⚠", C_WARN())
        } else if *ok {
            ("✓", C_DIM())
        } else {
            ("✗", C_BAD())
        };
        let text = msg.trim_start_matches('⚠').trim_start();
        lines.push(Line::from(Span::styled(
            format!("   {} {}", g, text),
            Style::default().fg(c),
        )));
    }
    let title = if form.editing {
        format!(
            "compile · {} · TYPING custom — Enter/Esc confirm · Backspace del",
            form.vendor
        )
    } else {
        format!(
            "compile · {} · ↑↓ field · ←→ value · e edit · Enter→confirm apply · q cancel",
            form.vendor
        )
    };
    f.render_widget(Paragraph::new(lines).block(block(&title)), area);
}

/// Prefetch(zoo 다운로드) 대상 저장소 선택 폼 오버레이.
pub(super) fn prefetch_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.prefetch_form else {
        return;
    };
    let full = f.area();
    let h = (form.fields.len() as u16) + 10;
    let area = centered(full, 84, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let get = |k: &str| {
        form.fields
            .iter()
            .find(|x| x.key == k)
            .map(|x| x.value.clone())
            .unwrap_or_default()
    };
    let pvc = get("pvc");
    let dir = get("dir");
    let dest = format!("{}:/mnt/store/{}", pvc, dir.trim_matches('/'));
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  model  ", Style::default().fg(C_DIM())),
        Span::styled(
            form.model_id.clone(),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  saves to ", Style::default().fg(C_DIM())),
        Span::styled(dest, Style::default().fg(C_ACC())),
        Span::styled("  (HF cache; reused by compile/serve)", Style::default().fg(C_DIM())),
    ]));
    lines.push(Line::from(""));
    for (i, fld) in form.fields.iter().enumerate() {
        let active = i == form.cursor;
        lines.push(choice_row(fld, active, active && form.editing));
    }
    lines.push(Line::from(""));
    if let Some(fld) = form.fields.get(form.cursor) {
        lines.push(Line::from(Span::styled(
            format!("  {}", fld.help),
            Style::default().fg(C_DIM()),
        )));
    }
    let title = if form.editing {
        "prefetch · TYPING custom — Enter/Esc confirm · Backspace del".to_string()
    } else {
        "prefetch (download) · ↑↓ field · ←→ value · e edit · Enter→confirm · q cancel".to_string()
    };
    f.render_widget(Paragraph::new(lines).block(block(&title)), area);
}

/// Deploy options form overlay.
pub(super) fn deploy_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.deploy_form else { return };
    let fit = app.deploy_fit(form);
    let full = f.area();
    let h = (form.fields.len() as u16)
        + (fit.tips.len() as u16)
        + (app.deploy_preflight(form).len() as u16)
        + 17;
    let w = full.width.saturating_sub(6).clamp(72, 104);
    let area = centered(full, w, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let section = |t: &str| {
        Line::from(Span::styled(
            format!("── {} ", t),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ))
    };
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  model  ", Style::default().fg(C_DIM())),
        Span::styled(
            form.model_id.clone(),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   engine {}", form.engine),
            Style::default().fg(C_DIM()),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  serve   ", Style::default().fg(C_DIM())),
        Span::styled(form.mount.clone(), Style::default().fg(C_ACC())),
    ]));
    lines.push(Line::from(""));
    lines.push(section("settings  (↑↓ field · ←→ value · e edit)"));
    for (i, fld) in form.fields.iter().enumerate() {
        let active = i == form.cursor;
        lines.push(choice_row(fld, active, active && form.editing));
    }
    if let Some(fld) = form.fields.get(form.cursor) {
        lines.push(Line::from(Span::styled(
            format!("   ↳ {}", fld.help),
            Style::default().fg(C_DIM()),
        )));
    }
    lines.push(Line::from(""));
    lines.push(section("capacity  (requested devices vs free devices)"));
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
        Span::styled(
            format!("{} {}", glyph, fit.verdict.label()),
            Style::default().fg(vcol).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "   demand {} dev   free {}{} / {} over {} node",
                fit.demand,
                fit.free,
                if fit.metric_free != fit.free {
                    format!(" (idle {})", fit.metric_free)
                } else {
                    String::new()
                },
                fit.total,
                fit.nodes
            ),
            Style::default().fg(C_DIM()),
        ),
    ]));
    for tip in &fit.tips {
        let tcol = if tip.starts_with('⚠') {
            C_BAD()
        } else {
            C_WARN()
        };
        lines.push(Line::from(Span::styled(
            format!("   → {}", tip),
            Style::default().fg(tcol),
        )));
    }
    let pf = app.deploy_preflight(form);
    let pf_ok = pf.iter().all(|(ok, _)| *ok);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "── preflight ",
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "(checks likely apply failures) ",
            Style::default().fg(C_DIM()),
        ),
        Span::styled(
            if pf_ok {
                "● passed — apply ready"
            } else {
                "✗ resolve failed checks below"
            },
            Style::default()
                .fg(if pf_ok { C_OK() } else { C_BAD() })
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    for (ok, msg) in &pf {
        let (g, c) = if *ok {
            ("✓", C_OK())
        } else {
            ("✗", C_BAD())
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {} ", g),
                Style::default().fg(c).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                msg.clone(),
                Style::default().fg(if *ok { C_DIM() } else { C_WARN() }),
            ),
        ]));
    }
    let title = if form.editing {
        "deploy · editing custom value — Enter/Esc confirm · Backspace delete".to_string()
    } else if pf_ok {
        "deploy · ↑↓ field · ←→ value · e edit · Enter→placement · q cancel".to_string()
    } else {
        "deploy · preflight has failures · Enter→placement anyway · q cancel".to_string()
    };
    f.render_widget(Paragraph::new(lines).block(block(&title)), area);
}

/// Route edit form — rename path text or retarget backend.
pub(super) fn route_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.route_form else { return };
    let full = f.area();
    let h = if form.rename {
        8
    } else {
        (form.choices.len() as u16).min(10) + 8
    };
    let area = centered(full, 76, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  route  ", Style::default().fg(C_DIM())),
        Span::styled(
            form.path.clone(),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   in {}", form.route),
            Style::default().fg(C_DIM()),
        ),
    ]));
    lines.push(Line::from(""));
    let title = if form.rename {
        lines.push(Line::from(Span::styled(
            "  New gateway path:",
            Style::default().fg(C_DIM()),
        )));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("{}_", form.value),
                Style::default()
                    .fg(C_WARN())
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Examples: /gpu/qwen · /rngd/exaone · /atom/gemma  (URLRewrite maps to /v1)",
            Style::default().fg(C_DIM()),
        )));
        "route rename · type · Enter confirm · Backspace delete · Esc cancel"
    } else {
        lines.push(Line::from(Span::styled(
            "  Select the backend for this path (↑↓ · Enter):",
            Style::default().fg(C_DIM()),
        )));
        for (i, c) in form.choices.iter().enumerate() {
            let active = i == form.cursor;
            let (g, st) = if active {
                (
                    "▶ ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(C_ACC())
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("  ", Style::default().fg(Color::Gray))
            };
            lines.push(Line::from(vec![Span::styled(format!("  {}{}", g, c), st)]));
        }
        "route retarget · ↑↓ select · Enter confirm · Esc cancel"
    };
    f.render_widget(
        Paragraph::new(lines).block(block(title).border_style(Style::default().fg(C_WARN()))),
        area,
    );
}

/// Shared-store move form — one editable destination path, with the source shown above it.
pub(super) fn store_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.store_form else { return };
    let full = f.area();
    let area = centered(full, 84, 10u16.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  build  ", Style::default().fg(C_DIM())),
        Span::styled(
            form.repo.clone(),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("   {}", form.size), Style::default().fg(C_DIM())),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  from   ", Style::default().fg(C_DIM())),
        Span::styled(form.src.clone(), Style::default().fg(Color::Gray)),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Destination (relative to the store root):",
        Style::default().fg(C_DIM()),
    )));
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!("{}_", form.value),
            Style::default()
                .fg(C_WARN())
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Must stay under hub/ or compiled/ — the move runs as a Job you review first.",
        Style::default().fg(C_DIM()),
    )));
    f.render_widget(
        Paragraph::new(lines).block(
            block("store move · type · Enter preview · Backspace delete · Esc cancel")
                .border_style(Style::default().fg(C_WARN())),
        ),
        area,
    );
}

/// Enter 컨텍스트 액션 메뉴 오버레이 — 가능한 동작을 라벨+설명+단축키로. 발견 가능한 UX.
pub(super) fn action_menu_overlay(f: &mut Frame, app: &App) {
    let Some(menu) = &app.action_menu else { return };
    let full = f.area();
    let w = 70u16.min(full.width.saturating_sub(2));
    let h = (menu.items.len() as u16) + 4;
    let area = centered(full, w, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    for (i, it) in menu.items.iter().enumerate() {
        let active = i == menu.cursor;
        let mode = it.action.required_mode();
        let locked = !app.can(mode); // current permission mode can't run it — show, but dimmed with ⊘
        let marker = if active {
            "▶ "
        } else if locked {
            "⊘ "
        } else {
            "  "
        };
        let base = if locked {
            Style::default().fg(C_DIM())
        } else if active {
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let mcol = if locked {
            C_DIM()
        } else {
            match mode {
                crate::app::Mode::Observe => C_DIM(),
                crate::app::Mode::Debug => C_ACC(),
                crate::app::Mode::Admin => C_WARN(),
                crate::app::Mode::Danger => C_BAD(),
            }
        };
        let key_col = if locked { C_DIM() } else { C_ACC() };
        let mut line = Line::from(vec![
            Span::styled(marker, base),
            Span::styled(
                format!("[{}] ", it.key),
                Style::default().fg(key_col).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<9}", it.label), base),
            Span::styled(
                format!("{:<7}", it.action.risk_label()),
                Style::default().fg(mcol),
            ),
            Span::styled(it.desc.to_string(), Style::default().fg(C_DIM())),
        ]);
        if active {
            line.style = Style::default().bg(C_HL());
        }
        lines.push(line);
    }
    f.render_widget(
        Paragraph::new(lines).block(block(&format!(
            "{} · ↑↓ Enter · [key] · ⊘=needs higher mode · q cancel",
            truncw(&menu.title, 34)
        ))),
        area,
    );
}

/// 커맨드 팔레트 오버레이(`:`) — 쿼리 줄 + 퍼지 매칭 결과(매칭 문자 강조·점수순).
pub(super) fn palette_overlay(f: &mut Frame, app: &App) {
    let Some(p) = &app.palette else { return };
    let full = f.area();
    let rows = p.rows();
    let w = 56u16;
    // 최대 12행 표시(넘치면 커서 주변만).
    let visible = rows.len().min(12);
    let h = (visible as u16) + 4;
    let area = centered(full, w, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);

    let mut lines: Vec<Line> = Vec::new();
    // 쿼리 프롬프트.
    lines.push(Line::from(vec![
        Span::styled(
            " : ",
            Style::default()
                .fg(Color::Black)
                .bg(C_ACC())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", p.query),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("▏", Style::default().fg(C_ACC())),
    ]));
    lines.push(Line::from(""));

    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no match)",
            Style::default().fg(C_DIM()),
        )));
    } else {
        // 커서가 보이도록 윈도우 시작 계산.
        let cursor = rows.iter().position(|r| r.3).unwrap_or(0);
        let start = cursor.saturating_sub(visible.saturating_sub(1));
        for (label, hint, idx, active) in rows.iter().skip(start).take(visible) {
            let marker = if *active { "▶ " } else { "  " };
            let base = if *active {
                Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let mut sp = vec![Span::styled(marker, base)];
            // 라벨 — 매칭된 char 는 accent+bold 로 강조.
            for (ci, ch) in label.chars().enumerate() {
                let st = if idx.contains(&ci) {
                    Style::default()
                        .fg(C_ACC())
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    base
                };
                sp.push(Span::styled(ch.to_string(), st));
            }
            // 힌트(오른쪽 흐리게).
            sp.push(Span::styled(
                format!("  · {}", hint),
                Style::default().fg(C_DIM()),
            ));
            let mut line = Line::from(sp);
            if *active {
                line.style = Style::default().bg(C_HL());
            }
            lines.push(line);
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(block("command palette · type to filter · ↑↓ + Enter · Esc")),
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
    let h: u32 = name
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    PAL[(h as usize) % PAL.len()]
}

/// 옵션 한 줄 — 모든 후보를 인라인으로(선택만 강조). k9s 처럼 후보군이 한눈에.
/// editing 이면 자유 입력 버퍼를 보여줌. 커스텀 값(프리셋에 없음)은 노란 강조 토큰.
pub(super) fn choice_row(
    fld: &crate::app::CompileField,
    active: bool,
    editing: bool,
) -> Line<'static> {
    let name_style = if active {
        Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let mut sp = vec![
        Span::styled(if active { "▶ " } else { "  " }, name_style),
        Span::styled(format!("{:<17}", fld.label), name_style),
    ];
    if editing {
        sp.push(Span::styled(
            format!("[ {}_ ]", fld.value),
            Style::default()
                .fg(C_WARN())
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ));
    } else {
        let mut matched = false;
        for c in &fld.choices {
            let is = *c == fld.value;
            matched |= is;
            let st = if is {
                Style::default()
                    .fg(Color::Black)
                    .bg(C_ACC())
                    .add_modifier(Modifier::BOLD)
            } else if active {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().fg(C_DIM())
            };
            sp.push(Span::styled(format!(" {} ", c), st));
            sp.push(Span::raw(" "));
        }
        if !matched && !fld.value.is_empty() {
            sp.push(Span::styled(
                format!(" {} ", fld.value),
                Style::default()
                    .fg(Color::Black)
                    .bg(C_WARN())
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    Line::from(sp)
}

/// Serving objective edit form overlay.
pub(super) fn objective_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = &app.objective_form else {
        return;
    };
    let full = f.area();
    let area = centered(full, 72, (form.fields.len() as u16) + 8);
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  model  ", Style::default().fg(C_DIM())),
        Span::styled(
            form.model.clone(),
            Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "   Perf shows pass/fail status and tuning advice when objectives are set",
            Style::default().fg(C_DIM()),
        ),
    ]));
    lines.push(Line::from(""));
    for (i, fld) in form.fields.iter().enumerate() {
        let active = i == form.cursor;
        lines.push(choice_row(fld, active, active && form.editing));
    }
    lines.push(Line::from(""));
    if let Some(fld) = form.fields.get(form.cursor) {
        lines.push(Line::from(Span::styled(
            format!("  {}", fld.help),
            Style::default().fg(C_DIM()),
        )));
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
    let Some((title, body)) = &app.preview else {
        return;
    };
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
        // 실패 이유(kubectl 에러)는 한 줄이 매우 길어 가로로 잘리므로 줄바꿈(wrap)으로 전부 보이게.
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.preview_scroll, 0))
            .block(block(&format!(
                "{} · ↑↓ scroll · w save{} · q close",
                title,
                if app.preview_apply {
                    " · v validate · a apply(admin)"
                } else {
                    ""
                }
            ))),
        area,
    );
}

/// Placement 선택 화면 — deploy 폼의 place 필드에서 열리는 후보 노드 상태 목록(컬럼).
/// 유휴/전체 디바이스·평균 util·메모리·스케줄 가능 여부를 보고 배치를 고른다.
pub(super) fn place_picker_overlay(f: &mut Frame, app: &App) {
    let Some(p) = &app.place_picker else { return };
    let full = f.area();
    let h = (p.rows.len() as u16) + 5;
    let w = full.width.saturating_sub(6).clamp(74, 110);
    let area = centered(full, w, h.min(full.height.saturating_sub(2)));
    f.render_widget(Clear, area);
    // 목적지 종류(2단계 picker 라우팅과 동일): compile=실행 노드, prefetch=저장 PVC, 그 외 deploy 배치.
    let (name_hdr, kind_label) = if app.compile_form.is_some() {
        ("NODE", "compile node")
    } else if app.prefetch_form.is_some() {
        ("PVC", "store PVC")
    } else {
        ("NODE", "placement")
    };
    // info_only picker(디바이스 예약 없음)면 디바이스 열을 숨기고 이름+설명만.
    let info_picker = p.rows.iter().all(|r| r.info_only);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        if info_picker {
            format!("  {:<24} {}", name_hdr, "DETAIL")
        } else {
            format!("  {:<20} {:>7}  {:>5}  {:>13}  {}", name_hdr, "FREE", "UTIL", "MEM(GB)", "STATUS")
        },
        Style::default().fg(C_HEAD()).add_modifier(Modifier::BOLD),
    )));
    for (i, r) in p.rows.iter().enumerate() {
        let sel = i == p.cursor;
        // info_only 행(PVC·컴파일 노드): 디바이스 열 없이 이름 + 설명만 깔끔하게.
        if r.info_only {
            let name_c = if !r.schedulable {
                C_BAD()
            } else if r.value == "any" {
                C_ACC()
            } else {
                Color::White
            };
            let (note_s, note_c) = if r.schedulable {
                (r.note.clone(), C_DIM())
            } else {
                (format!("✗ {}", r.note), C_BAD())
            };
            let mut line = Line::from(vec![
                Span::styled(
                    format!("{} {} ", if sel { "▎" } else { " " }, padw(&r.label, 24)),
                    Style::default().fg(name_c).add_modifier(if sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                ),
                Span::styled(note_s, Style::default().fg(note_c)),
            ]);
            if sel {
                line.style = Style::default().bg(C_HL());
            }
            lines.push(line);
            continue;
        }
        let pseudo = r.value == "any" || r.value == "spread";
        let name_c = if !r.schedulable {
            C_BAD()
        } else if pseudo {
            C_ACC()
        } else {
            Color::White
        };
        let (free_s, util_s, mem_s, status_s, status_c) = if pseudo {
            (
                String::new(),
                String::new(),
                String::new(),
                r.note.clone(),
                C_DIM(),
            )
        } else if r.schedulable {
            let (st, sc) = if r.free > 0 {
                (format!("✓ {} free", r.free), C_OK())
            } else {
                // ready·드라이버 OK 지만 전부 할당됨 → 배치 불가.
                (format!("· full ({} allocated)", r.total - r.free), C_WARN())
            };
            (
                format!("{}/{}", r.free, r.total),
                if r.util.is_nan() {
                    "–".into()
                } else {
                    format!("{:.0}%", r.util)
                },
                format!("{:.0}/{:.0}", r.mem_used, r.mem_total),
                st,
                sc,
            )
        } else {
            (
                format!("{}/{}", r.free, r.total),
                if r.util.is_nan() {
                    "–".into()
                } else {
                    format!("{:.0}%", r.util)
                },
                format!("{:.0}/{:.0}", r.mem_used, r.mem_total),
                format!("✗ {}", r.note),
                C_BAD(),
            )
        };
        let mut line = Line::from(vec![
            Span::styled(
                format!("{} {} ", if sel { "▎" } else { " " }, padw(&r.label, 20)),
                Style::default().fg(name_c).add_modifier(if sel {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
            Span::styled(
                format!("{:>7}  ", free_s),
                Style::default().fg(if r.free > 0 { C_OK() } else { C_DIM() }),
            ),
            Span::styled(format!("{:>5}  ", util_s), Style::default().fg(C_DIM())),
            Span::styled(format!("{:>13}  ", mem_s), Style::default().fg(C_DIM())),
            Span::styled(status_s, Style::default().fg(status_c)),
        ]);
        if sel {
            line.style = Style::default().bg(C_HL());
        }
        lines.push(line);
    }
    f.render_widget(
        Paragraph::new(lines).block(block_active(&format!(
            "{} · ↑↓ 선택 · Enter 적용 · Esc 취소",
            kind_label
        ))),
        area,
    );
}

/// Masked secret entry. Draws the value's *length* and nothing else — the characters never
/// reach a buffer cell, so a screen share or a terminal scrollback cannot leak the token.
pub(super) fn secret_form_overlay(f: &mut Frame, app: &App) {
    let Some(form) = app.secret_form.as_ref() else {
        return;
    };
    let area = centered(f.area(), 74, 11);
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  secret  ", Style::default().fg(C_DIM())),
        Span::styled(
            format!("{}/{}", form.name, form.key),
            Style::default().fg(C_ACC()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  in namespace {}", app.ns),
            Style::default().fg(C_DIM()),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!("  {}", form.purpose),
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  value   ", Style::default().fg(C_DIM())),
        Span::styled(
            format!("[ {}_ ]", form.masked()),
            Style::default()
                .fg(C_WARN())
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {}", form.help),
        Style::default().fg(C_DIM()),
    )));
    lines.push(Line::from(Span::styled(
        "  the value is masked here, kept out of the audit log, and passed to kubectl on stdin",
        Style::default().fg(C_DIM()),
    )));
    lines.push(Line::from(Span::styled(
        "  so it never appears in the process table.",
        Style::default().fg(C_DIM()),
    )));
    f.render_widget(
        Paragraph::new(lines).block(block("enter secret · type/paste · Enter store · Esc cancel")),
        area,
    );
}
