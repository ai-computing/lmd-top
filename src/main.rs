//! lmd-top — llm-d 클러스터 터미널 관측 도구 (Phase 1: monitor).
//! 실행: `lmd-top`            → TUI
//!       `lmd-top --snapshot` → 1회 수집 후 텍스트 출력(헤드리스 검증용)

mod agent;
mod app;
mod cast;
mod catalog;
mod collect;
mod config;
mod doctor;
mod kube;
mod metrics;
mod prom;
mod ui;

use anyhow::Result;
use app::{App, Mode, Pending, View};
use collect::collect;
use config::Config;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let cfg = Config::default();
    let args: Vec<String> = std::env::args().collect();

    // 메트릭 전수조사 + 갭 분석(왜 뷰가 비었나 진단).
    if args.iter().any(|a| a == "--doctor") {
        doctor::run(&cfg).await;
        return Ok(());
    }

    // 기계가독 상태(agent) — --json (또는 --snapshot --json). 1회 수집 후 JSON 출력.
    if args.iter().any(|a| a == "--json") {
        let snap = collect(&cfg).await;
        agent::emit_json(&snap, &cfg);
        return Ok(());
    }

    if args.iter().any(|a| a == "--snapshot" || a == "-s") {
        let snap = collect(&cfg).await;
        print_snapshot(&snap, &cfg);
        return Ok(());
    }

    if args.iter().any(|a| a == "--render") {
        let snap = collect(&cfg).await;
        render_dump(snap);
        return Ok(());
    }

    // 데모 asciicast 생성(agg 로 GIF 변환용). --cast [out.cast]
    if let Some(pos) = args.iter().position(|a| a == "--cast") {
        let out = args.get(pos + 1).filter(|s| !s.starts_with('-')).cloned().unwrap_or_else(|| "docs/demo.cast".to_string());
        cast::run(&cfg, &out).await;
        return Ok(());
    }

    // 권한 모드(기동 시) — --mode observe|debug|admin|danger (기본 observe)
    let mode = args
        .iter()
        .position(|a| a == "--mode")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| Mode::parse(s))
        .unwrap_or(Mode::Observe);

    run_tui(cfg, mode).await
}

/// 헤드리스 렌더 검증 — TestBackend 로 각 뷰를 한 프레임 그려 텍스트로 출력.
fn render_dump(snap: collect::Snapshot) {
    use app::View;
    use ratatui::backend::TestBackend;
    let mut a = App::new();
    a.apply(snap);
    let rw: u16 = std::env::var("LMD_W").ok().and_then(|s| s.parse().ok()).unwrap_or(100);
    let rh: u16 = std::env::var("LMD_H").ok().and_then(|s| s.parse().ok()).unwrap_or(26);
    let mut fx = ui::FxState::disabled(); // 텍스트 덤프 — 이펙트 끔(부분 프레임 방지)
    for v in View::ALL {
        a.view = v;
        a.selected = 0;
        let backend = TestBackend::new(rw, rh);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| ui::draw(f, &a, &mut fx)).unwrap();
        let buf = term.backend().buffer().clone();
        println!("\n========== VIEW: {} ==========", v.title());
        let area = buf.area;
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                if let Some(c) = buf.cell((x, y)) {
                    line.push_str(c.symbol());
                }
            }
            println!("{}", line.trim_end());
        }
    }
}

async fn run_tui(cfg: Config, mode: Mode) -> Result<()> {
    let shared = Arc::new(Mutex::new(collect(&cfg).await)); // 첫 수집(즉시 표시)

    // full 수집 루프 (3초) — 모델/EPP/perf/events 등 무거운 것
    {
        let shared = shared.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(cfg.interval_full));
            tick.tick().await;
            loop {
                tick.tick().await;
                let snap = collect(&cfg).await;
                if let Ok(mut g) = shared.lock() {
                    *g = snap;
                }
            }
        });
    }
    // fast tier (1초) — 가속기 util/mem/temp + 노드만 빠르게 갱신(반응성)
    {
        let shared = shared.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(cfg.interval_fast));
            tick.tick().await;
            loop {
                tick.tick().await;
                let (accel, nodes) = collect::collect_fast(&cfg).await;
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if let Ok(mut g) = shared.lock() {
                    g.accel = accel;
                    g.nodes = nodes;
                    g.ts = ts;
                }
            }
        });
    }

    // UI 루프(블로킹)
    let rt = tokio::runtime::Handle::current(); // Perf 드릴 온디맨드 조회용
    let res = tokio::task::spawn_blocking(move || ui_loop(shared, cfg, mode, rt)).await?;
    res
}

fn ui_loop(shared: Arc<Mutex<collect::Snapshot>>, cfg: Config, mode: Mode, rt: tokio::runtime::Handle) -> Result<()> {
    let ns = cfg.ns.clone();
    let prom = cfg.prom.clone();
    // 패닉 시에도 터미널 복원(raw mode/alt-screen 해제) — 안 하면 셸이 망가짐.
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        orig_hook(info);
    }));
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.mode = mode;
    app::set_theme(cfg.theme); // 시작 테마(LMD_THEME / yaml)
    let mut fx = ui::FxState::new();
    let result = (|| -> Result<()> {
        loop {
            if !app.paused {
                let snap = shared.lock().map(|g| g.clone()).unwrap_or_default();
                app.apply(snap);
            }
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|f| ui::draw(f, &app, &mut fx))?;

            // 애니메이션 중엔 짧게 폴링(부드러운 프레임), 평시엔 100ms.
            let poll_ms = if fx.animating() { 16 } else { 100 };
            if event::poll(Duration::from_millis(poll_ms))? {
                let ev = event::read()?;
                if let Event::Mouse(m) = ev {
                    match m.kind {
                        MouseEventKind::ScrollDown => app.move_sel(1),
                        MouseEventKind::ScrollUp => app.move_sel(-1),
                        _ => {}
                    }
                    continue;
                }
                if let Event::Key(k) = ev {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    // 필터 입력 모드: 텍스트 캡처
                    if app.filtering {
                        match k.code {
                            KeyCode::Esc | KeyCode::Enter => app.stop_filter(),
                            KeyCode::Backspace => app.filter_pop(),
                            KeyCode::Char(c) => app.filter_push(c),
                            _ => {}
                        }
                        continue;
                    }
                    // 도움말 오버레이: 아무 키나 닫기
                    if app.help {
                        app.help = false;
                        continue;
                    }
                    // 변경 작업 확인(y/n) — 다른 키 무시. 실행은 여기서(권한 검증은 트리거 시점).
                    if let Some(pending) = app.confirm.clone() {
                        match k.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                match pending {
                                    Pending::Scale { name, target } => match kube::scale_deploy(&ns, &name, target) {
                                        Ok(_) => app.notify(format!("scaled {} → {}", name, target)),
                                        Err(e) => app.notify(format!("scale failed: {}", e)),
                                    },
                                    Pending::Restart { name } => match kube::rollout_restart(&ns, &name) {
                                        Ok(_) => app.notify(format!("rollout restart {}", name)),
                                        Err(e) => app.notify(format!("restart failed: {}", e)),
                                    },
                                }
                                app.confirm = None;
                            }
                            _ => {
                                app.confirm = None;
                                app.notify("cancelled".to_string());
                            }
                        }
                        continue;
                    }
                    // 알림 히스토리 오버레이: esc/q/A 로 닫기(다른 키 무시)
                    if app.alerts_panel {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('A') | KeyCode::Char('a') => {
                                app.alerts_panel = false
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // 로그 오버레이 모드
                    if app.logs_mode {
                        match k.code {
                            KeyCode::Esc | KeyCode::Char('q') => app.logs_mode = false,
                            KeyCode::Up | KeyCode::Char('k') => app.logs_scroll = app.logs_scroll.saturating_sub(1),
                            KeyCode::Down | KeyCode::Char('j') => app.logs_scroll = app.logs_scroll.saturating_add(3),
                            KeyCode::Char('r') => {
                                if let Ok(l) = kube::logs(&ns, &app.logs_target, 400) {
                                    app.logs = l;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('q') => break, // 종료는 q 만
                        KeyCode::Esc => {
                            // 뒤로가기만: 상세→브레드크럼→필터→줌 순 (종료 안 함)
                            if app.detail {
                                app.detail = false;
                            } else if app.nav_back() {
                                // 크로스레이어 드릴 되짚기
                            } else if !app.filter.is_empty() {
                                app.clear_filter();
                            } else if app.zoom {
                                app.zoom = false;
                            }
                        }
                        // 크로스레이어 드릴 pivot — 선택 엔티티에서 관련 레이어로 점프
                        KeyCode::Char(c @ ('p' | 'i' | 'r' | 'e' | 'm' | 'n')) => app.pivot(c),
                        // EPP scorer 가중치 what-if(로컬 시뮬) — EPP 뷰에서만 반응
                        KeyCode::Char('+') | KeyCode::Char('=') => app.epp_adjust(1.0),
                        KeyCode::Char('-') | KeyCode::Char('_') => app.epp_adjust(-1.0),
                        // 세션 에너지 리셋(all-smi 식)
                        KeyCode::Char('R') => app.reset_energy(),
                        KeyCode::Char('?') => app.toggle_help(),
                        KeyCode::Char('A') | KeyCode::Char('a') => app.toggle_alerts(),
                        KeyCode::Char('t') => app.cycle_theme(),
                        KeyCode::Char('f') => {
                            let on = fx.toggle();
                            app.notify(format!("animations {}", if on { "on" } else { "off" }));
                        }
                        KeyCode::Char('z') => app.zoom = !app.zoom,
                        KeyCode::Char(' ') => app.paused = !app.paused,
                        KeyCode::Char('g') => {
                            let base = cfg.grafana.clone();
                            // best-effort 브라우저 오픈 — stdio를 null로(터미널 화면 깨짐 방지)
                            let _ = std::process::Command::new("xdg-open")
                                .arg(&base)
                                .stdin(Stdio::null())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn();
                            app.notify(format!("Grafana: {}  (open in browser · llm-models dashboard)", base));
                            terminal.clear().ok(); // 혹시 모를 잔상 제거 → 전체 재그리기
                        }
                        KeyCode::Char('/') => app.start_filter(),
                        KeyCode::Enter => {
                            if app.view == View::Perf {
                                // 선택 모델 지연 분포 온디맨드 조회 → 히스토그램 드릴
                                if let Some(model) = app.selected_perf_model() {
                                    let d = rt.block_on(collect::perf_detail(&prom, &model));
                                    app.perf_detail = Some(d);
                                    app.detail = true;
                                }
                            } else if app.view == View::Routing {
                                app.drill_route(); // Flow: route → backend 모델 상세
                            } else {
                                app.toggle_detail();
                            }
                        }
                        KeyCode::Char('o') => app.cycle_sort(),
                        KeyCode::Tab => app.next_tab(),
                        KeyCode::BackTab => app.prev_tab(), // Shift+Tab → 이전 뷰
                        KeyCode::Char(c @ '0'..='9') => {
                            app.set_view_idx(c as usize - '0' as usize)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if app.detail && app.view == View::Nodes {
                                app.dev_cursor(-1) // Node 상세: device 커서(0=요약)
                            } else if app.detail {
                                app.scroll_detail(-1)
                            } else {
                                app.move_sel(-1)
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.detail && app.view == View::Nodes {
                                app.dev_cursor(1)
                            } else if app.detail {
                                app.scroll_detail(1)
                            } else {
                                app.move_sel(1)
                            }
                        }
                        KeyCode::Left => app.move_sel(-1), // 이전 항목
                        KeyCode::Right => app.move_sel(1),  // 다음 항목
                        KeyCode::Char('l') if !app.can(Mode::Debug) => {
                            app.notify(format!("logs needs --mode debug+ (current: {})", app.mode.name()));
                        }
                        KeyCode::Char('l') => {
                            // 선택 pod/모델의 로그 오버레이
                            if let Some(pod) = app.logs_target_pod() {
                                match kube::logs(&ns, &pod, 400) {
                                    Ok(l) => {
                                        app.logs_scroll = l.len().saturating_sub(30) as u16;
                                        app.logs = l;
                                        app.logs_target = pod;
                                        app.logs_mode = true;
                                    }
                                    Err(e) => app.notify(format!("logs: {}", e)),
                                }
                            } else {
                                app.notify("logs: Pods/Models 뷰에서 pod/model 선택".to_string());
                            }
                        }
                        KeyCode::Char('s') if !app.can(Mode::Admin) => {
                            app.notify(format!("scale needs --mode admin+ (current: {})", app.mode.name()));
                        }
                        KeyCode::Char('s') => {
                            // Admin+ : 즉시 실행하지 않고 확인(y/n) 대기로 — dry-run→confirm.
                            if let Some(m) = app.selected_model() {
                                let (name, target) =
                                    (m.name.clone(), if m.desired == 0 { 1 } else { 0 });
                                app.confirm = Some(Pending::Scale { name, target });
                            } else {
                                app.notify("scale: select a model in Models/Overview".to_string());
                            }
                        }
                        // 롤아웃 재시작(admin+, 확인)
                        KeyCode::Char('S') if !app.can(Mode::Admin) => {
                            app.notify(format!("restart needs --mode admin+ (current: {})", app.mode.name()));
                        }
                        KeyCode::Char('S') => {
                            if let Some(m) = app.selected_model() {
                                app.confirm = Some(Pending::Restart { name: m.name.clone() });
                            } else {
                                app.notify("restart: select a model in Models/Overview".to_string());
                            }
                        }
                        _ => {
                            app.toast = None;
                        }
                    }
                }
            }
        }
        Ok(())
    })();

    // teardown (에러여도 복원)
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).ok();
    terminal.show_cursor().ok();
    result
}

fn print_snapshot(s: &collect::Snapshot, cfg: &Config) {
    println!("== lmd-top snapshot (prom={}, ns={}) ==", cfg.prom, cfg.ns);
    println!(
        "gateway: {} {}",
        if s.gw_addr.is_empty() { "—" } else { &s.gw_addr },
        if s.gw_ok { "Programmed" } else { "" }
    );
    println!("\n[nodes] {}", s.nodes.len());
    for n in &s.nodes {
        println!(
            "  {:<24} load1 {:>5.2}  mem {:.0}/{:.0}G",
            n.name, n.load1, n.mem_used_gb, n.mem_total_gb
        );
    }
    println!("\n[accelerators] {}", s.accel.len());
    for a in &s.accel {
        println!(
            "  {:<5} {:<6} {:<16} util {:>5.1}%  mem {:.0}/{:.0}G  {:.0}°C {:.0}W  {}",
            a.disp(),
            a.id,
            a.node,
            a.util,
            a.mem_used_gb,
            a.mem_total_gb,
            a.temp,
            a.power,
            a.busy_model
        );
    }
    println!("\n[inference pools] {}", s.pools.len());
    for p in &s.pools {
        println!(
            "  {:<16} ready {:.0}  queue {:.1}  kv {:.2}  sat {:.2}",
            p.name, p.ready, p.queue, p.kv, p.sat
        );
    }
    if let Some(e) = &s.epp {
        println!(
            "\n[EPP] profile={} picker={}\n  scorers: {}",
            e.profile,
            e.picker,
            e.scorers
                .iter()
                .map(|(n, w)| format!("{}·w{:.0}", n, w))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("\n[models] {}", s.models.len());
    for m in &s.models {
        println!(
            "  {:<24} {}/{}  run={} wait={} tps={}  path={}  {}",
            m.name,
            m.ready,
            m.desired,
            m.running.map(|x| format!("{:.0}", x)).unwrap_or("-".into()),
            m.waiting.map(|x| format!("{:.0}", x)).unwrap_or("-".into()),
            m.tps.map(|x| format!("{:.0}", x)).unwrap_or("-".into()),
            if m.route.is_empty() { "-" } else { &m.route },
            m.status
        );
    }
    println!("\n[routes] {} (epp_in_path={})", s.routes.len(), s.epp_in_path);
    for r in &s.routes {
        println!("  {:<10} → {}/{}", r.path, r.kind, r.backend);
    }
    if !s.objectives.is_empty() {
        println!("[SLO] {}", s.objectives.iter().map(|o| format!("{}(p{})", o.name, o.priority)).collect::<Vec<_>>().join(", "));
    }
    println!("\n[pods] {}", s.pods.len());
    for p in &s.pods {
        println!("  {:<40} {:<8} {} {} restarts={}", p.name, p.phase, p.ready, p.node, p.restarts);
    }
    if !s.warnings.is_empty() {
        println!("\n[warnings] {}", s.warnings.len());
        for w in &s.warnings {
            println!("  ! {}", w);
        }
    }
}
