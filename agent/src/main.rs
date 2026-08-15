mod agg;
mod correlate;
mod decode;
mod export;
mod http;
mod http_agg;
mod identity;
mod k8s_index;
mod metrics_registry;
mod peer_cache;
mod service_map;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;

use agg::Aggregator;
use anyhow::Context as _;
use aya::maps::{Array, HashMap as AyaHashMap, MapData, RingBuf};
use aya::programs::uprobe::UProbeScope;
use aya::programs::{KProbe, TracePoint, UProbe};
use correlate::{Correlator, Exchange, PeerV4};
use decode::{DecodedEvent, decode_event};
use export::ExportHub;
use http::parse_exchange;
use http_agg::HttpAggregator;
use identity::IdentityResolver;
use k8s_index::{spawn_pod_index_if_configured, PodIndex};
use obsagent_common::SockMeta;
use peer_cache::PeerCache;
use service_map::{format_ip_port, DstId, MapRecord, ServiceMap};
#[rustfmt::skip]
use log::{debug, info, warn};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Tcp,
    Http,
}

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

    // Q15: smoke_probe only when OBSAGENT_SMOKE_PROBE is set (smoke0/1/2).
    if std::env::var_os("OBSAGENT_SMOKE_PROBE").is_some() {
        let smoke: &mut KProbe = ebpf
            .program_mut("smoke_probe")
            .context("program smoke_probe")?
            .try_into()?;
        smoke.load()?;
        smoke.attach("try_to_wake_up", 0)?;
    }

    attach_tp(&mut ebpf, "enter_connect", "syscalls", "sys_enter_connect")?;
    attach_tp(&mut ebpf, "exit_connect", "syscalls", "sys_exit_connect")?;
    attach_tp(&mut ebpf, "enter_accept4", "syscalls", "sys_enter_accept4")?;
    attach_tp(&mut ebpf, "exit_accept4", "syscalls", "sys_exit_accept4")?;
    attach_tp(&mut ebpf, "enter_close", "syscalls", "sys_enter_close")?;
    attach_tp(&mut ebpf, "enter_read", "syscalls", "sys_enter_read")?;
    attach_tp(&mut ebpf, "exit_read", "syscalls", "sys_exit_read")?;
    attach_tp(&mut ebpf, "enter_write", "syscalls", "sys_enter_write")?;
    attach_tp(&mut ebpf, "exit_write", "syscalls", "sys_exit_write")?;
    // Q2 revised: glibc TcpStream uses sendto/recvfrom (strace evidence 2026-08-07).
    attach_tp(&mut ebpf, "enter_sendto", "syscalls", "sys_enter_sendto")?;
    attach_tp(&mut ebpf, "exit_sendto", "syscalls", "sys_exit_sendto")?;
    attach_tp(&mut ebpf, "enter_recvfrom", "syscalls", "sys_enter_recvfrom")?;
    attach_tp(&mut ebpf, "exit_recvfrom", "syscalls", "sys_exit_recvfrom")?;

    // Phase 3 Q5/Q6/Q12: try-attach libssl; soft-fail keeps cleartext working.
    attach_openssl_uprobes(&mut ebpf);

    let events_map = ebpf
        .take_map("EVENTS")
        .context("EVENTS map missing")?;
    let drops_map = ebpf
        .take_map("DROPS")
        .context("DROPS map missing")?;
    let sock_meta_map = ebpf
        .take_map("SOCK_META")
        .context("SOCK_META map missing")?;

    let ring = RingBuf::try_from(events_map)?;
    let drops: Array<MapData, u64> = Array::try_from(drops_map)?;
    let sock_meta: Arc<Mutex<AyaHashMap<MapData, u64, SockMeta>>> =
        Arc::new(Mutex::new(AyaHashMap::try_from(sock_meta_map)?));

    let tcp_agg = Arc::new(Mutex::new(Aggregator::default()));
    let http_agg = Arc::new(Mutex::new(HttpAggregator::default()));
    let correlator = Arc::new(Mutex::new(Correlator::default()));
    let identity = Arc::new(Mutex::new(IdentityResolver::default()));
    let service_map = Arc::new(Mutex::new(ServiceMap::default()));
    let peer_cache = Arc::new(Mutex::new(PeerCache::default()));
    let pod_index = spawn_pod_index_if_configured();
    let export = Arc::new(ExportHub::spawn());
    let rates = Arc::new(Mutex::new(Vec::new()));
    let drop_count = Arc::new(Mutex::new(0u64));
    let sockio_count = Arc::new(Mutex::new(0u64));
    let tlsio_count = Arc::new(Mutex::new(0u64));

    let tcp_rb = Arc::clone(&tcp_agg);
    let http_rb = Arc::clone(&http_agg);
    let corr_rb = Arc::clone(&correlator);
    let id_rb = Arc::clone(&identity);
    let map_rb = Arc::clone(&service_map);
    let peers_rb = Arc::clone(&peer_cache);
    let pods_rb = Arc::clone(&pod_index);
    let export_rb = Arc::clone(&export);
    let meta_rb = Arc::clone(&sock_meta);
    let rates_rb = Arc::clone(&rates);
    let drop_rb = Arc::clone(&drop_count);
    let sockio_rb = Arc::clone(&sockio_count);
    let tlsio_rb = Arc::clone(&tlsio_count);
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
                        let Some(decoded) = decode_event(item.as_ref()) else {
                            continue;
                        };
                        let now = Instant::now();
                        match decoded {
                            DecodedEvent::Latency(ev) => {
                                lock_mut(&tcp_rb).record(&ev, now);
                            }
                            DecodedEvent::Io(ev) => {
                                *lock_mut(&sockio_rb) += 1;
                                if let Some(ex) = lock_mut(&corr_rb).observe(&ev, now) {
                                    handle_exchange(
                                        ex, now, &meta_rb, &peers_rb, &id_rb, &pods_rb, &map_rb,
                                        &http_rb, &export_rb,
                                    );
                                }
                            }
                            DecodedEvent::TlsIo(ev) => {
                                *lock_mut(&tlsio_rb) += 1;
                                // Client-only: same-host OpenSSL server halves would 2× rates.
                                if let Some(ex) = lock_mut(&corr_rb).observe_client(&ev, now) {
                                    handle_exchange(
                                        ex, now, &meta_rb, &peers_rb, &id_rb, &pods_rb, &map_rb,
                                        &http_rb, &export_rb,
                                    );
                                }
                            }
                        }
                        events_in_tick += 1;
                    }
                    guard.clear_ready();
                }
                _ = tick.tick() => {
                    {
                        let mut r = lock_mut(&rates_rb);
                        r.push(events_in_tick);
                        if r.len() > 60 {
                            r.remove(0);
                        }
                    }
                    events_in_tick = 0;
                    if let Ok(v) = drops.get(&0, 0) {
                        *lock_mut(&drop_rb) = v;
                        export_rb.set_events_dropped(v);
                    }
                    let edges = lock_mut(&map_rb).edge_count() as u64;
                    export_rb.set_edge_count(edges);
                }
            }
        }
    });

    info!("Phase 5 agent running (hardened export + service map). q quit, t toggle view.");
    let headless = std::env::var_os("OBSAGENT_HEADLESS").is_some()
        || !std::io::IsTerminal::is_terminal(&std::io::stdout());
    if headless {
        run_headless(
            tcp_agg,
            http_agg,
            service_map,
            drop_count,
            sockio_count,
            tlsio_count,
            export,
        )
        .await?;
    } else {
        run_tui(tcp_agg, http_agg, rates, drop_count).await?;
    }

    drop(ebpf);
    Ok(())
}

fn handle_exchange(
    mut ex: Exchange,
    now: Instant,
    sock_meta: &Mutex<AyaHashMap<MapData, u64, SockMeta>>,
    peer_cache: &Mutex<PeerCache>,
    identity: &Mutex<IdentityResolver>,
    pods: &Mutex<PodIndex>,
    service_map: &Mutex<ServiceMap>,
    http_agg: &Mutex<HttpAggregator>,
    export: &ExportHub,
) {
    if ex.peer.is_none() {
        ex.peer = lookup_peer(sock_meta, peer_cache, ex.tgid, ex.fd, now);
    }
    let Some(parsed) = parse_exchange(&ex) else {
        return;
    };
    lock_mut(http_agg).record(&parsed, now);

    let mut src = lock_mut(identity).resolve(ex.tgid, now);
    if let Some(uid) = src.pod_uid.as_deref() {
        if let Ok(idx) = pods.lock() {
            if let Some((ns, name)) = idx.lookup_uid(uid) {
                src.label = format!("{ns}/{name}");
            }
        }
    }
    let dst = resolve_dst(ex.peer, pods, &src);
    let rec = MapRecord {
        src: src.clone(),
        dst: dst.clone(),
        endpoint: parsed.endpoint.clone(),
        latency_ns: parsed.latency_ns,
        status: parsed.status,
    };
    lock_mut(service_map).record(&rec, now);
    export.record_exchange(
        &src.label,
        &dst,
        &parsed.endpoint.method,
        &parsed.endpoint.path,
        parsed.latency_ns,
        parsed.status,
    );
}

fn lookup_peer(
    sock_meta: &Mutex<AyaHashMap<MapData, u64, SockMeta>>,
    peer_cache: &Mutex<PeerCache>,
    tgid: u32,
    fd: i32,
    now: Instant,
) -> Option<PeerV4> {
    if fd < 0 {
        return None;
    }
    if let Some(p) = lock_mut(peer_cache).get(tgid, fd, now) {
        return Some(p);
    }
    let key = ((tgid as u64) << 32) | (fd as u32 as u64);
    let map = lock_mut(sock_meta);
    let meta = map.get(&key, 0).ok()?;
    if !meta.has_addr() {
        return None;
    }
    let peer = PeerV4 {
        daddr_be: meta.daddr_be,
        dport_be: meta.dport_be,
    };
    drop(map);
    lock_mut(peer_cache).insert(tgid, fd, peer, now);
    Some(peer)
}

fn resolve_dst(peer: Option<PeerV4>, pods: &Mutex<PodIndex>, _src: &identity::NodeId) -> DstId {
    let Some(p) = peer else {
        return DstId::Unknown;
    };
    if let Ok(idx) = pods.lock() {
        if let Some((ns, name)) = idx.lookup_ip(p.daddr_be) {
            return DstId::Pod {
                namespace: ns,
                name,
            };
        }
    }
    DstId::IpPort {
        addr: format_ip_port(p.daddr_be, p.dport_be),
    }
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

/// Phase 3 Q5: try-attach `libssl.so.3` / `libssl.so.1.1`. Q6/Q12: soft-fail.
fn attach_openssl_uprobes(ebpf: &mut aya::Ebpf) {
    const CANDIDATES: &[&str] = &[
        "/lib/x86_64-linux-gnu/libssl.so.3",
        "/usr/lib/x86_64-linux-gnu/libssl.so.3",
        "/lib/x86_64-linux-gnu/libssl.so.1.1",
        "/usr/lib/x86_64-linux-gnu/libssl.so.1.1",
    ];

    // Classic + set_fd required; `_ex` / `SSL_free` soft-loaded.
    for name in [
        "enter_ssl_set_fd",
        "enter_ssl_write",
        "exit_ssl_write",
        "enter_ssl_read",
        "exit_ssl_read",
    ] {
        if let Err(e) = load_uprobe(ebpf, name) {
            warn!("failed to load {name}: {e:#} (continuing; cleartext still active)");
            return;
        }
    }
    let mut have_ex = true;
    for name in [
        "enter_ssl_write_ex",
        "exit_ssl_write_ex",
        "enter_ssl_read_ex",
        "exit_ssl_read_ex",
    ] {
        if let Err(e) = load_uprobe(ebpf, name) {
            warn!("failed to load optional {name}: {e:#}");
            have_ex = false;
            break;
        }
    }
    let have_free = match load_uprobe(ebpf, "enter_ssl_free") {
        Ok(()) => true,
        Err(e) => {
            debug!("failed to load optional enter_ssl_free: {e:#}");
            false
        }
    };

    let mut attached_any = false;
    let mut seen_inodes = HashSet::new();
    for path in CANDIDATES {
        if !Path::new(path).exists() {
            continue;
        }
        // Dedup /lib vs /usr/lib when they are the same inode (avoids double-fire).
        match std::fs::metadata(path) {
            Ok(meta) => {
                let inode_key = (meta.dev(), meta.ino());
                if !seen_inodes.insert(inode_key) {
                    debug!("skip duplicate libssl path {path} (same inode)");
                    continue;
                }
            }
            Err(e) => {
                debug!("stat {path}: {e}");
                continue;
            }
        }

        let required = [
            ("enter_ssl_set_fd", "SSL_set_fd"),
            ("enter_ssl_write", "SSL_write"),
            ("exit_ssl_write", "SSL_write"),
            ("enter_ssl_read", "SSL_read"),
            ("exit_ssl_read", "SSL_read"),
        ];
        let mut path_ok = true;
        for (prog, sym) in required {
            if let Err(e) = attach_uprobe(ebpf, prog, sym, path) {
                warn!("attach {prog} → {sym} in {path}: {e:#}");
                path_ok = false;
                break;
            }
        }
        if !path_ok {
            continue;
        }

        // Soft: CPython uses `_ex`; older OpenSSL may only have classic.
        let mut ex_ok = 0usize;
        if have_ex {
            let optional_ex = [
                ("enter_ssl_write_ex", "SSL_write_ex"),
                ("exit_ssl_write_ex", "SSL_write_ex"),
                ("enter_ssl_read_ex", "SSL_read_ex"),
                ("exit_ssl_read_ex", "SSL_read_ex"),
            ];
            for (prog, sym) in optional_ex {
                match attach_uprobe(ebpf, prog, sym, path) {
                    Ok(()) => ex_ok += 1,
                    Err(e) => debug!("optional {prog} → {sym} in {path}: {e:#}"),
                }
            }
            if ex_ok == 0 {
                debug!("no SSL_*_ex symbols in {path}; classic SSL_read/write only");
            }
        }

        for sym in ["SSL_set_rfd", "SSL_set_wfd"] {
            if let Err(e) = attach_uprobe(ebpf, "enter_ssl_set_fd", sym, path) {
                debug!("optional {sym} in {path}: {e:#}");
            }
        }
        if have_free {
            if let Err(e) = attach_uprobe(ebpf, "enter_ssl_free", "SSL_free", path) {
                debug!("optional SSL_free in {path}: {e:#}");
            }
        }

        info!("attached OpenSSL uprobes to {path} (classic + {ex_ok}/4 _ex)");
        attached_any = true;
    }
    if !attached_any {
        warn!("no libssl uprobes attached (Q6/Q12 soft-fail); cleartext HTTP still active");
    }
}

fn load_uprobe(ebpf: &mut aya::Ebpf, name: &str) -> anyhow::Result<()> {
    let program: &mut UProbe = ebpf
        .program_mut(name)
        .with_context(|| format!("program {name}"))?
        .try_into()?;
    program.load()?;
    Ok(())
}

fn attach_uprobe(ebpf: &mut aya::Ebpf, name: &str, symbol: &str, lib: &str) -> anyhow::Result<()> {
    let program: &mut UProbe = ebpf
        .program_mut(name)
        .with_context(|| format!("program {name}"))?
        .try_into()?;
    program
        .attach(symbol, lib, UProbeScope::AllProcesses)
        .with_context(|| format!("attach {name} → {symbol} in {lib}"))?;
    Ok(())
}

fn lock_mut<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

async fn run_headless(
    tcp_agg: Arc<Mutex<Aggregator>>,
    http_agg: Arc<Mutex<HttpAggregator>>,
    service_map: Arc<Mutex<ServiceMap>>,
    drop_count: Arc<Mutex<u64>>,
    sockio_count: Arc<Mutex<u64>>,
    tlsio_count: Arc<Mutex<u64>>,
    export: Arc<ExportHub>,
) -> anyhow::Result<()> {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = signal::ctrl_c() => break,
            _ = tick.tick() => {
                let now = Instant::now();
                let tcp_rows = lock_mut(&tcp_agg).rows(now);
                let http_rows = lock_mut(&http_agg).rows(now);
                let map_rows = lock_mut(&service_map).rows(now);
                let tcp_total: u64 = tcp_rows.iter().map(|(_, r)| r.count).sum();
                let http_total: u64 = http_rows.iter().map(|(_, r)| r.count).sum();
                let drops = *lock_mut(&drop_count);
                let sockio = *lock_mut(&sockio_count);
                let tlsio = *lock_mut(&tlsio_count);
                let edges = lock_mut(&service_map).edge_count();
                let export_drop = export.dropped.load(std::sync::atomic::Ordering::Relaxed);
                let export_ok = export.exported.load(std::sync::atomic::Ordering::Relaxed);
                let otlp = if export.enabled() { "on" } else { "off" };
                println!(
                    "events_60s={tcp_total} tcp_60s={tcp_total} http_60s={http_total} sockio={sockio} tlsio={tlsio} drops={drops} edges={edges} otlp={otlp} otlp_ok={export_ok} otlp_drop={export_drop}"
                );
                for (k, r) in http_rows.iter().take(8) {
                    println!(
                        "  {} count={} rate={:.2}/s p50={} 4xx={:.0}% 5xx={:.0}%",
                        k.label(),
                        r.count,
                        r.rate_per_s,
                        fmt_ns(r.p50_ns),
                        r.pct_4xx,
                        r.pct_5xx
                    );
                }
                for (k, r) in map_rows.iter().take(8) {
                    println!(
                        "  [edge] {} -> {} count={} rate={:.2}/s p99={} 5xx={:.0}%",
                        k.src,
                        k.dst.label(),
                        r.count,
                        r.rate_per_s,
                        fmt_ns(r.p99_ns),
                        r.pct_5xx
                    );
                }
                for (k, r) in tcp_rows.iter().take(4) {
                    println!(
                        "  [tcp] {} count={} p50={}",
                        k.label(),
                        r.count,
                        fmt_ns(r.p50_ns)
                    );
                }
            }
        }
    }
    Ok(())
}

async fn run_tui(
    tcp_agg: Arc<Mutex<Aggregator>>,
    http_agg: Arc<Mutex<HttpAggregator>>,
    rates: Arc<Mutex<Vec<u64>>>,
    drop_count: Arc<Mutex<u64>>,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut view = View::Http;

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
                        if key.code == KeyCode::Char('t') {
                            view = match view {
                                View::Tcp => View::Http,
                                View::Http => View::Tcp,
                            };
                        }
                    }
                }
                let now = Instant::now();
                let spark: Vec<u64> = lock_mut(&rates).clone();
                let drops = *lock_mut(&drop_count);
                match view {
                    View::Tcp => {
                        let rows = lock_mut(&tcp_agg).rows(now);
                        terminal.draw(|f| draw_tcp(f, &rows, &spark, drops))?;
                    }
                    View::Http => {
                        let rows = lock_mut(&http_agg).rows(now);
                        terminal.draw(|f| draw_http(f, &rows, &spark, drops))?;
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn draw_tcp(
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
        "obsagent Phase 2 — TCP connect/accept (60s) | drops={drops} | t toggle | q quit\nQ7: latency = syscall enter→exit"
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
    .block(Block::default().borders(Borders::ALL).title("tcp endpoints"));
    f.render_widget(table, chunks[1]);

    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("events/s"))
        .data(spark);
    f.render_widget(sparkline, chunks[2]);
}

fn draw_http(
    f: &mut Frame<'_>,
    rows: &[(http::HttpEndpoint, http_agg::HttpRow)],
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
        "obsagent Phase 3 — HTTP/HTTPS endpoints (60s) | drops={drops} | t toggle | q quit\nTLS-only latency (Q2); metrics only — never log raw prefixes (Q11)"
    ))
    .block(Block::default().borders(Borders::ALL).title("status"));
    f.render_widget(header, chunks[0]);

    let header_cells = ["endpoint", "count", "rate", "p50", "p95", "p99", "4xx%", "5xx%"]
        .into_iter()
        .map(Cell::from);
    let table_rows = rows.iter().take(32).map(|(k, r)| {
        Row::new(vec![
            Cell::from(k.label()),
            Cell::from(r.count.to_string()),
            Cell::from(format!("{:.1}", r.rate_per_s)),
            Cell::from(fmt_ns(r.p50_ns)),
            Cell::from(fmt_ns(r.p95_ns)),
            Cell::from(fmt_ns(r.p99_ns)),
            Cell::from(format!("{:.0}", r.pct_4xx)),
            Cell::from(format!("{:.0}", r.pct_5xx)),
        ])
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Percentage(30),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Length(6),
        ],
    )
    .header(Row::new(header_cells).style(Style::new().bold()))
    .block(Block::default().borders(Borders::ALL).title("http endpoints"));
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
