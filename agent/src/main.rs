mod agg;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agg::Aggregator;
use anyhow::Context as _;
use aya::maps::{Array, MapData, RingBuf};
use aya::programs::{KProbe, TracePoint};
#[rustfmt::skip]
use log::{debug, info, warn};
use obsagent_common::SockLatencyEvent;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table};
use tokio::io::unix::AsyncFd;
use tokio::signal;
use tokio::time::MissedTickBehavior;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        debug!("remove limit on locked memory failed, ret is: {ret}");
    }

    let mut ebpf = aya::Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/probes"
    )))?;
    match aya_log::EbpfLogger::init(&mut ebpf) {
        Err(e) => {
            warn!("failed to initialize eBPF logger: {e}");
        }
        Ok(logger) => {
            let mut logger =
                AsyncFd::with_interest(logger, tokio::io::Interest::READABLE)?;
            tokio::task::spawn(async move {
                loop {
                    let mut guard = logger.readable_mut().await.unwrap();
                    guard.get_inner_mut().flush();
                    guard.clear_ready();
                }
            });
        }
    }

    // Q9: keep smoke_probe for Phase 0 regression.
    let smoke: &mut KProbe = ebpf.program_mut("smoke_probe").unwrap().try_into()?;
    smoke.load()?;
    smoke.attach("try_to_wake_up", 0)?;

    attach_tp(&mut ebpf, "enter_connect", "syscalls", "sys_enter_connect")?;
    attach_tp(&mut ebpf, "exit_connect", "syscalls", "sys_exit_connect")?;
    attach_tp(&mut ebpf, "enter_accept4", "syscalls", "sys_enter_accept4")?;
    attach_tp(&mut ebpf, "exit_accept4", "syscalls", "sys_exit_accept4")?;

    let events_map = ebpf
        .take_map("EVENTS")
        .context("EVENTS map missing")?;
    let drops_map = ebpf
        .take_map("DROPS")
        .context("DROPS map missing")?;

    let ring = RingBuf::try_from(events_map)?;
    let drops: Array<MapData, u64> = Array::try_from(drops_map)?;

    let agg = Arc::new(Mutex::new(Aggregator::default()));
    let rates = Arc::new(Mutex::new(Vec::new()));
    let drop_count = Arc::new(Mutex::new(0u64));

    let agg_rb = Arc::clone(&agg);
    let rates_rb = Arc::clone(&rates);
    let drop_rb = Arc::clone(&drop_count);
    let mut poll = AsyncFd::with_interest(ring, tokio::io::Interest::READABLE)?;
    tokio::task::spawn(async move {
        let mut events_in_tick = 0u64;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                ready = poll.readable_mut() => {
                    let mut guard = match ready {
                        Ok(g) => g,
                        Err(_) => break,
                    };
                    let rb = guard.get_inner_mut();
                    while let Some(item) = rb.next() {
                        let ev = read_event(&item);
                        if let Ok(mut a) = agg_rb.lock() {
                            a.record(&ev, Instant::now());
                        }
                        events_in_tick += 1;
                    }
                    guard.clear_ready();
                }
                _ = tick.tick() => {
                    if let Ok(mut r) = rates_rb.lock() {
                        r.push(events_in_tick);
                        if r.len() > 60 {
                            r.remove(0);
                        }
                    }
                    events_in_tick = 0;
                    if let Ok(v) = drops.get(&0, 0) {
                        if let Ok(mut d) = drop_rb.lock() {
                            *d = v;
                        }
                    }
                }
            }
        }
    });

    info!("Phase 1 agent running (connect/accept4 latency). q/Ctrl-C to quit.");
    // Latency is syscall enter→exit only (Q7); -EINPROGRESS is not TCP established.
    let headless = std::env::var_os("OBSAGENT_HEADLESS").is_some()
        || !std::io::IsTerminal::is_terminal(&std::io::stdout());
    if headless {
        run_headless(agg, drop_count).await?;
    } else {
        run_tui(agg, rates, drop_count).await?;
    }

    // Keep ebpf alive until UI/headless exits (maps moved out; programs drop with ebpf).
    drop(ebpf);
    Ok(())
}

fn attach_tp(
    ebpf: &mut aya::Ebpf,
    name: &str,
    category: &str,
    tp_name: &str,
) -> anyhow::Result<()> {
    let program: &mut TracePoint = ebpf
        .program_mut(name)
        .with_context(|| format!("program {name}"))?
        .try_into()?;
    program.load()?;
    program.attach(category, tp_name)?;
    Ok(())
}

fn read_event(item: &aya::maps::ring_buf::RingBufItem<'_>) -> SockLatencyEvent {
    let bytes = item.as_ref();
    assert!(bytes.len() >= core::mem::size_of::<SockLatencyEvent>());
    // SAFETY: kernel writes SockLatencyEvent; size checked above.
    unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<SockLatencyEvent>()) }
}

async fn run_headless(
    agg: Arc<Mutex<Aggregator>>,
    drop_count: Arc<Mutex<u64>>,
) -> anyhow::Result<()> {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = signal::ctrl_c() => break,
            _ = tick.tick() => {
                let now = Instant::now();
                let rows = agg.lock().map(|a| a.rows(now)).unwrap_or_default();
                let total: u64 = rows.iter().map(|(_, r)| r.count).sum();
                let drops = drop_count.lock().map(|d| *d).unwrap_or(0);
                println!("events_60s={total} endpoints={} drops={drops}", rows.len());
                for (k, r) in rows.iter().take(8) {
                    println!(
                        "  {} count={} err={} p50={}",
                        k.label(),
                        r.count,
                        r.errors,
                        fmt_ns(r.p50_ns)
                    );
                }
            }
        }
    }
    Ok(())
}

async fn run_tui(
    agg: Arc<Mutex<Aggregator>>,
    rates: Arc<Mutex<Vec<u64>>>,
    drop_count: Arc<Mutex<u64>>,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => break,
            _ = tick.tick() => {
                if event::poll(Duration::from_millis(0))? {
                    if let Event::Key(key) = event::read()? {
                        if key.code == KeyCode::Char('q')
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL))
                        {
                            break;
                        }
                    }
                }
                let now = Instant::now();
                let rows = agg.lock().map(|a| a.rows(now)).unwrap_or_default();
                let spark: Vec<u64> = rates.lock().map(|r| r.clone()).unwrap_or_default();
                let drops = drop_count.lock().map(|d| *d).unwrap_or(0);
                terminal.draw(|f| draw(f, &rows, &spark, drops))?;
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn draw(
    f: &mut Frame<'_>,
    rows: &[(agg::EndpointKey, agg::RowSnapshot)],
    spark: &[u64],
    drops: u64,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(5),
        ])
        .split(f.area());

    let header = Paragraph::new(format!(
        "obsagent Phase 1 — connect/accept latency (60s) | drops={drops} | q quit\nQ7: latency = syscall enter→exit (EINPROGRESS ≠ connected)"
    ))
    .block(Block::default().borders(Borders::ALL).title("status"));
    f.render_widget(header, chunks[0]);

    let header_cells = ["endpoint", "count", "err", "p50", "p95", "p99"]
        .into_iter()
        .map(Cell::from);
    let table_rows = rows.iter().take(32).map(|(k, r)| {
        Row::new(vec![
            Cell::from(k.label()),
            Cell::from(r.count.to_string()),
            Cell::from(r.errors.to_string()),
            Cell::from(fmt_ns(r.p50_ns)),
            Cell::from(fmt_ns(r.p95_ns)),
            Cell::from(fmt_ns(r.p99_ns)),
        ])
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Percentage(40),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(header_cells).style(Style::new().bold()))
    .block(Block::default().borders(Borders::ALL).title("endpoints"));
    f.render_widget(table, chunks[1]);

    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("events/s"))
        .data(spark);
    f.render_widget(sparkline, chunks[2]);
}

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.2}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}µs", ns as f64 / 1_000.0)
    } else {
        format!("{ns}ns")
    }
}
