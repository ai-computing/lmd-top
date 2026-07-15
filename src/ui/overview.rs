//! Overview 뷰 — 클러스터 요약·LED 그리드·VRAM 바·Status 진단 밴드·EPP 경로.
//! 공유 렌더 헬퍼는 상위 `ui` 모듈에서 가져온다.
use super::*;

// ── Overview ───────────────────────────────────────────
pub(super) fn view_overview(f: &mut Frame, area: Rect, app: &App) {
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
