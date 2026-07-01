//! lmd-top — llm-d 클러스터 터미널 관측 도구 (Phase 1: monitor).
//! 실행: `lmd-top`            → TUI
//!       `lmd-top --snapshot` → 1회 수집 후 텍스트 출력(헤드리스 검증용)

mod app;
mod collect;
mod kube;
mod prom;
mod ui;

use anyhow::Result;
use app::App;
use collect::{collect, Config};
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
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let cfg = Config::default();
    let args: Vec<String> = std::env::args().collect();

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

    run_tui(cfg).await
}

/// 헤드리스 렌더 검증 — TestBackend 로 각 뷰를 한 프레임 그려 텍스트로 출력.
fn render_dump(snap: collect::Snapshot) {
    use app::View;
    use ratatui::backend::TestBackend;
    let mut a = App::new();
    a.apply(snap);
    for v in View::ALL {
        a.view = v;
        a.selected = 0;
        let backend = TestBackend::new(100, 26);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| ui::draw(f, &a)).unwrap();
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

async fn run_tui(cfg: Config) -> Result<()> {
    let shared = Arc::new(Mutex::new(collect(&cfg).await)); // 첫 수집(즉시 표시)

    // 백그라운드 수집 루프 (2초)
    {
        let shared = shared.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
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

    // UI 루프(블로킹)
    let ns = cfg.ns.clone();
    let res = tokio::task::spawn_blocking(move || ui_loop(shared, ns)).await?;
    res
}

fn ui_loop(shared: Arc<Mutex<collect::Snapshot>>, ns: String) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let result = (|| -> Result<()> {
        loop {
            {
                let snap = shared.lock().map(|g| g.clone()).unwrap_or_default();
                app.apply(snap);
            }
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|f| ui::draw(f, &app))?;

            if event::poll(Duration::from_millis(250))? {
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
                    match k.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Esc => {
                            if app.detail {
                                app.detail = false;
                            } else if !app.filter.is_empty() {
                                app.clear_filter();
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('?') => app.toggle_help(),
                        KeyCode::Char('/') => app.start_filter(),
                        KeyCode::Enter => app.toggle_detail(),
                        KeyCode::Char('o') => app.cycle_sort(),
                        KeyCode::Tab => app.next_tab(),
                        KeyCode::Char(c @ '0'..='6') => {
                            app.set_view_idx(c as usize - '0' as usize)
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.move_sel(-1),
                        KeyCode::Down | KeyCode::Char('j') => app.move_sel(1),
                        KeyCode::Char('s') => {
                            if let Some(m) = app.selected_model() {
                                let (name, target) =
                                    (m.name.clone(), if m.desired == 0 { 1 } else { 0 });
                                match kube::scale_deploy(&ns, &name, target) {
                                    Ok(_) => {
                                        app.toast =
                                            Some(format!("scaled {} → {}", name, target))
                                    }
                                    Err(e) => app.toast = Some(format!("scale 실패: {}", e)),
                                }
                            } else {
                                app.toast = Some("scale: Models/Overview 뷰에서 모델 선택".into());
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
            a.kind.label(),
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
