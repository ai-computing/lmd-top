//! 우측 상세(drill-down) 패널 — 선택 리소스의 확장 정보를 그린다.
//! 공유 렌더 헬퍼는 상위 `ui` 모듈에서 가져온다.
use super::*;

// ── Detail (drill-down) ────────────────────────────────
pub(super) fn detail_panel(f: &mut Frame, area: Rect, app: &App) {
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
