//! Traffic 뷰 — EPP(scorer/weight/picker + 요청분배)와 Flow(Gateway→route→pool + EPP 우회 진단).
//! 공유 렌더 헬퍼는 상위 `ui` 모듈에서 가져온다.
use super::*;

pub(super) fn view_epp(f: &mut Frame, area: Rect, app: &App) {
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

pub(super) fn view_routing(f: &mut Frame, area: Rect, app: &App) {
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
