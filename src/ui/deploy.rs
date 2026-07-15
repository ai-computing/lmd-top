//! Deploy(Library/Zoo) 뷰 — 배포 가능 모델 목록·zoo·통합 Activity 피드·상세.
//! 공유 렌더 헬퍼는 상위 `ui` 모듈에서 가져온다.
use super::*;

/// Deploy▸Library 선택 항목 상세 — 카탈로그 모델(배치 후보·가능성·수요) 또는 스토어 빌드(포맷·타깃·경로).
pub(super) fn library_detail(f: &mut Frame, area: Rect, app: &App) {
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

// ── Activity 패널 — compile Job + deploy rollout 통합 작업 피드(진행률 %/바 포함). ──
// Deploy 뷰 하단 패널로 렌더. active(포커스) 면 선택 하이라이트 + 활성 테두리.
pub(super) fn activity_panel(f: &mut Frame, area: Rect, app: &App, active: bool) {
    let data = app.activity_rows();
    let rows: Vec<Row> = data
        .iter()
        .map(|r| {
            let color = match r.sev {
                2 => C_BAD(),
                0 => C_OK(),
                _ => C_WARN(),
            };
            let kind = match r.kind {
                "compile" => Span::styled("compile", Style::default().fg(C_ACC())),
                "prefetch" => Span::styled("prefetch", Style::default().fg(C_WARN())),
                _ => Span::styled("deploy", Style::default().fg(C_OK())),
            };
            // STATUS 셀: 상태 텍스트(% 포함) + (진행 중 compile 이면) 진행바.
            let mut status_spans = vec![Span::styled(
                format!("{:<14} ", truncw(&r.status, 14)),
                Style::default().fg(color),
            )];
            if r.running_compile {
                status_spans.extend(compile_progress_bar(r.progress, app.tick, 10));
                // 실시간 힌트(파드 로그 마지막 줄) — 예: "downloading … 17G on disk".
                if !r.phase.is_empty() {
                    status_spans.push(Span::styled(
                        format!("  {}", truncw(&r.phase, 30)),
                        Style::default().fg(C_DIM()),
                    ));
                }
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
        Constraint::Min(18),    // TARGET
        Constraint::Length(8),  // STARTED
        Constraint::Length(46), // STATUS (%+bar+live hint)
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
pub(super) fn view_library(f: &mut Frame, area: Rect, app: &App) {
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

// ── Deploy▸Zoo — 벤더(Furiosa/Rebellions) 모델 zoo: prefetch/compile ──────────
// 2패널: 위=모델 목록(정렬/필터·⏎ 액션) · 아래=Activity(compile/prefetch/deploy 피드).
pub(super) fn view_zoo(f: &mut Frame, area: Rect, app: &App) {
    let focus = app.panel_focus;
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(9)])
        .split(area);

    // 위 패널이 포커스일 때만 정렬이 목록에 적용됨(order()가 panel_focus 를 봄).
    let order = if focus == 0 {
        app.order()
    } else {
        // 목록 정렬은 유지하되 선택 하이라이트는 활성 패널로만.
        let mut idx: Vec<usize> = (0..app.zoo.len()).collect();
        idx.sort_by(|&a, &b| app.zoo[a].display.to_lowercase().cmp(&app.zoo[b].display.to_lowercase()));
        idx
    };
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
            let (state_s, sev) = app.zoo_state(z);
            let state_c = match sev {
                0 => C_OK(),
                1 => C_WARN(),
                2 => C_BAD(),
                _ => C_DIM(),
            };
            Row::new(vec![
                Cell::from(truncw(&z.display, 30)).style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(truncw(&z.source, 34)).style(Style::default().fg(Color::Gray)),
                Cell::from(vlabel).style(Style::default().fg(vcolor)),
                Cell::from(truncw(&z.role, 9)).style(Style::default().fg(C_DIM())),
                Cell::from(state_s).style(Style::default().fg(state_c)),
                Cell::from(truncw(&z.note, 22)).style(Style::default().fg(C_DIM())),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(30), // MODEL
        Constraint::Min(18),    // SOURCE (HF)
        Constraint::Length(14), // COMPILE
        Constraint::Length(9),  // ROLE
        Constraint::Length(12), // STATUS
        Constraint::Length(22), // NOTE
    ];
    let header = ["MODEL", "SOURCE (HF)", "COMPILE", "ROLE", "STATUS", "NOTE"];
    let title = format!(
        "Deploy▸Zoo · vendor model zoo ({}) · ⏎ Prefetch/Compile · r refresh · o sort{}",
        app.zoo.len(),
        if focus == 0 {
            count_suffix(app.selected, order.len())
        } else {
            String::new()
        }
    );
    let blk = if focus == 0 {
        block_active(&title)
    } else {
        block(&title)
    };
    let mut st = TableState::default();
    if focus == 0 {
        st.select(Some(app.selected));
    }
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(hrow_sorted(&header, app.sort_header_label(), app.sort_arrow()))
            .column_spacing(1)
            .row_highlight_style(hl_style())
            .highlight_symbol("▎")
            .block(blk),
        panes[0],
        &mut st,
    );
    if focus == 0 {
        list_scrollbar(f, panes[0], order.len(), app.selected, 1);
    }
    // 아래: 통합 Activity 피드(compile/prefetch/deploy) — Library 와 동일.
    activity_panel(f, panes[1], app, focus == 1);
}
