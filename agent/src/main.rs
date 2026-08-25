mod agg;
mod cgroup_index;
mod correlate;
mod decode;
mod dual_plane;
mod export;
mod filter;
mod h2;
mod http;
mod http_agg;
mod identity;
mod k8s_index;
mod metrics_registry;
mod peer_cache;
mod profile;
mod reassemble;
mod sample;
mod service_map;
mod stack_decode;
mod symbolize;
mod trace_export;

use std::path::Path;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;

use agg::Aggregator;
use anyhow::Context as _;
use aya::maps::{Array, HashMap as AyaHashMap, MapData, RingBuf};
use aya::programs::uprobe::UProbeScope;
use aya::programs::perf_event::{
    PerfEvent, PerfEventConfig, PerfEventScope, SamplePolicy, SoftwareEvent,
};
use aya::programs::{KProbe, TracePoint, UProbe};
use aya::util::online_cpus;
use cgroup_index::{spawn_cgroup_index_if_configured, CgroupPodIndex};
use correlate::{Correlator, Exchange, PeerAddr};
use decode::{DecodedEvent, decode_event};
use dual_plane::DualPlane;
use export::ExportHub;
use filter::{
    allowed_tgids_from_proc, classify_ingest, collect_self_ids, denied_tgids_from_proc,
    is_otlp_export_route, tgid_map_delta, CommFilter, IngestSkip,
};
use h2::{looks_like_h2, looks_like_http11, H2Exchange, H2Registry};
use http::{normalize_path, parse_exchange, HttpEndpoint, ParsedExchange};
use http_agg::HttpAggregator;
use identity::IdentityResolver;
use k8s_index::{spawn_pod_index_if_configured, PodIndex};
use obsagent_common::{IoDir, SockIoEvent, SockMeta};
use peer_cache::PeerCache;
use profile::{ProfileConfig, ProfileStore};
use reassemble::Reassembler;
use stack_decode::decode_stack_sample;
use symbolize::StackSymbolizer;
use service_map::{format_peer, DstId, MapRecord, ServiceMap};
use trace_export::TraceMeta;
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
    attach_tp(&mut ebpf, "enter_readv", "syscalls", "sys_enter_readv")?;
    attach_tp(&mut ebpf, "exit_readv", "syscalls", "sys_exit_readv")?;
    attach_tp(&mut ebpf, "enter_writev", "syscalls", "sys_enter_writev")?;
    attach_tp(&mut ebpf, "exit_writev", "syscalls", "sys_exit_writev")?;
    attach_tp(&mut ebpf, "enter_recvmsg", "syscalls", "sys_enter_recvmsg")?;
    attach_tp(&mut ebpf, "exit_recvmsg", "syscalls", "sys_exit_recvmsg")?;
    attach_tp(&mut ebpf, "enter_sendmsg", "syscalls", "sys_enter_sendmsg")?;
    attach_tp(&mut ebpf, "exit_sendmsg", "syscalls", "sys_exit_sendmsg")?;

    // Phase 3 Q5/Q6/Q12: try-attach libssl; soft-fail keeps cleartext working.
    attach_openssl_uprobes(&mut ebpf);

    let profile_cfg = ProfileConfig::from_env();
    let mut profile_active = profile_cfg.enabled;
    if profile_cfg.enabled {
        match attach_perf(&mut ebpf, profile_cfg.freq_hz) {
            Ok(()) => info!(
                "OBSAGENT_PROFILE=1: perf_event ~{} Hz per CPU (STACKS RingBuf)",
                profile_cfg.freq_hz
            ),
            Err(e) => {
                warn!("OBSAGENT_PROFILE=1 but perf attach failed: {e:#}; profiles disabled");
                profile_active = false;
            }
        }
    }

    let events_map = ebpf
        .take_map("EVENTS")
        .context("EVENTS map missing")?;
    let drops_map = ebpf
        .take_map("DROPS")
        .context("DROPS map missing")?;
    let sock_meta_map = ebpf
        .take_map("SOCK_META")
        .context("SOCK_META map missing")?;
    let inflight_map = ebpf
        .take_map("INFLIGHT")
        .context("INFLIGHT map missing")?;
    let unmapped_map = ebpf
        .take_map("TLS_UNMAPPED")
        .context("TLS_UNMAPPED map missing")?;
    let sample_n_map_obj = ebpf.take_map("SAMPLE_N").context("SAMPLE_N map missing")?;
    let denied_map_obj = ebpf
        .take_map("DENIED_TGID")
        .context("DENIED_TGID map missing")?;
    let allowed_map_obj = ebpf
        .take_map("ALLOWED_TGID")
        .context("ALLOWED_TGID map missing")?;
    let allow_only_map_obj = ebpf
        .take_map("ALLOW_ONLY")
        .context("ALLOW_ONLY map missing")?;

    let ring = RingBuf::try_from(events_map)?;
    let drops: Array<MapData, u64> = Array::try_from(drops_map)?;
    let sock_meta: Arc<Mutex<AyaHashMap<MapData, u64, SockMeta>>> =
        Arc::new(Mutex::new(AyaHashMap::try_from(sock_meta_map)?));
    let inflight: Arc<Mutex<AyaHashMap<MapData, u64, u8>>> =
        Arc::new(Mutex::new(AyaHashMap::try_from(inflight_map)?));
    let tls_unmapped: Array<MapData, u64> = Array::try_from(unmapped_map)?;
    let mut sample_n_map: Array<MapData, u32> = Array::try_from(sample_n_map_obj)?;
    let denied_map: Arc<Mutex<AyaHashMap<MapData, u32, u8>>> =
        Arc::new(Mutex::new(AyaHashMap::try_from(denied_map_obj)?));
    let allowed_map: Arc<Mutex<AyaHashMap<MapData, u32, u8>>> =
        Arc::new(Mutex::new(AyaHashMap::try_from(allowed_map_obj)?));
    let allow_only_map: Arc<Mutex<Array<MapData, u32>>> =
        Arc::new(Mutex::new(Array::try_from(allow_only_map_obj)?));

    let tcp_agg = Arc::new(Mutex::new(Aggregator::default()));
    let http_agg = Arc::new(Mutex::new(HttpAggregator::default()));
    let correlator = Arc::new(Mutex::new(Correlator::default()));
    let identity = Arc::new(Mutex::new(IdentityResolver::default()));
    let service_map = Arc::new(Mutex::new(ServiceMap::default()));
    let peer_cache = Arc::new(Mutex::new(PeerCache::default()));
    let pod_index = spawn_pod_index_if_configured();
    let cgroup_index = spawn_cgroup_index_if_configured();
    let profile_store = Arc::new(Mutex::new(ProfileStore::new(profile_cfg.clone())));
    let export = Arc::new(ExportHub::spawn(
        profile_active
            .then(|| Arc::clone(&profile_store)),
    ));
    let rates = Arc::new(Mutex::new(Vec::new()));
    let drop_count = Arc::new(Mutex::new(0u64));
    let sockio_count = Arc::new(Mutex::new(0u64));
    let tlsio_count = Arc::new(Mutex::new(0u64));
    let socktimes_count = Arc::new(Mutex::new(0u64));
    let unmapped_count = Arc::new(Mutex::new(0u64));
    let denied_count = Arc::new(Mutex::new(0u64));
    let reasm_count = Arc::new(Mutex::new(0u64));
    let comm_filter = CommFilter::from_env();
    let agent_tgid = std::process::id();
    let pinned_sample_n = sample::bpf_sample_n_from_env();
    let init_sample_n = pinned_sample_n.unwrap_or(1);
    sample_n_map
        .set(0, init_sample_n, 0)
        .context("SAMPLE_N init")?;
    export.set_bpf_sample_n(init_sample_n);
    if let Some(n) = pinned_sample_n {
        info!("BPF SAMPLE_N pinned at {n} (OBSAGENT_SAMPLE_N); auto disabled");
    }
    let published_deny: Arc<Mutex<HashSet<u32>>> = Arc::new(Mutex::new(HashSet::new()));
    let published_allow: Arc<Mutex<HashSet<u32>>> = Arc::new(Mutex::new(HashSet::new()));
    let bpf_self_ids = Arc::new(Mutex::new(HashSet::<u32>::new()));
    let self_ids = Arc::new(Mutex::new(collect_self_ids(agent_tgid)));
    {
        let flag = if comm_filter.is_allow_only() { 1u32 } else { 0 };
        if let Err(e) = lock_mut(&allow_only_map).set(0, flag, 0) {
            warn!("ALLOW_ONLY init {flag}: {e}");
        }
    }
    let calib_dport = Arc::new(AtomicU16::new(0));
    let reassembler = Arc::new(Mutex::new(Reassembler::default()));
    let h2 = Arc::new(Mutex::new(H2Registry::default()));
    let dual = Arc::new(Mutex::new(DualPlane::from_env()));
    let tls_client_only = std::env::var_os("OBSAGENT_TLS_SERVER").is_none();

    let tcp_rb = Arc::clone(&tcp_agg);
    let http_rb = Arc::clone(&http_agg);
    let corr_rb = Arc::clone(&correlator);
    let id_rb = Arc::clone(&identity);
    let map_rb = Arc::clone(&service_map);
    let peers_rb = Arc::clone(&peer_cache);
    let pods_rb = Arc::clone(&pod_index);
    let cgroups_rb = Arc::clone(&cgroup_index);
    let export_rb = Arc::clone(&export);
    let meta_rb = Arc::clone(&sock_meta);
    let rates_rb = Arc::clone(&rates);
    let drop_rb = Arc::clone(&drop_count);
    let sockio_rb = Arc::clone(&sockio_count);
    let tlsio_rb = Arc::clone(&tlsio_count);
    let denied_rb = Arc::clone(&denied_count);
    let reasm_rb = Arc::clone(&reasm_count);
    let inflight_rb = Arc::clone(&inflight);
    let filter_rb = comm_filter.clone();
    let reassemble_rb = Arc::clone(&reassembler);
    let h2_rb = Arc::clone(&h2);
    let dual_rb = Arc::clone(&dual);
    let bpf_ids_rb = Arc::clone(&bpf_self_ids);
    let self_ids_rb = Arc::clone(&self_ids);
    let denied_map_rb = Arc::clone(&denied_map);
    let published_deny_rb = Arc::clone(&published_deny);
    let calib_rb = Arc::clone(&calib_dport);
    let socktimes_rb = Arc::clone(&socktimes_count);
    let unmapped_rb = Arc::clone(&unmapped_count);
    if profile_active {
        let stacks_map = ebpf.take_map("STACKS").context("STACKS map missing")?;
        let stack_drops_map = ebpf
            .take_map("STACK_DROPS")
            .context("STACK_DROPS map missing")?;
        let stacks_ring = RingBuf::try_from(stacks_map)?;
        let stack_drops: Array<MapData, u64> = Array::try_from(stack_drops_map)?;
        let store_rb = Arc::clone(&profile_store);
        std::thread::spawn(move || {
            let mut symbolizer = StackSymbolizer::new();
            let mut stacks_ring = stacks_ring;
            let mut samples_tick = 0u64;
            let mut last_log = Instant::now();
            loop {
                while let Some(item) = stacks_ring.next() {
                    if let Some(raw) = decode_stack_sample(item.as_ref()) {
                        let tgid = raw.tgid;
                        let ts_ns = raw.ts_ns;
                        let n = raw.frame_count as usize;
                        let ips = raw.ips[..n].to_vec();
                        let frames = symbolizer.symbolize_ips(tgid, &ips);
                        if !frames.is_empty() {
                            lock_mut(&store_rb).push_symbolized(tgid, ts_ns, frames);
                        }
                        samples_tick += 1;
                    }
                }
                if last_log.elapsed() >= Duration::from_secs(1) {
                    let ingested = lock_mut(&store_rb).samples_ingested;
                    let dropped = lock_mut(&store_rb).samples_dropped;
                    if let Ok(v) = stack_drops.get(&0, 0) {
                        debug!(
                            "profile stacks/s={samples_tick} ingested={ingested} store_drop={dropped} ring_drop={v}"
                        );
                    }
                    samples_tick = 0;
                    last_log = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
    }
    {
        let identity = Arc::clone(&identity);
        let bpf_self_ids = Arc::clone(&bpf_self_ids);
        let self_ids = Arc::clone(&self_ids);
        let denied_map = Arc::clone(&denied_map);
        let allowed_map = Arc::clone(&allowed_map);
        let allow_only_map = Arc::clone(&allow_only_map);
        let published_deny = Arc::clone(&published_deny);
        let published_allow = Arc::clone(&published_allow);
        let filter = comm_filter.clone();
        tokio::task::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                sweep_capture_maps(
                    agent_tgid,
                    &filter,
                    &identity,
                    &bpf_self_ids,
                    &self_ids,
                    &denied_map,
                    &allowed_map,
                    &allow_only_map,
                    &published_deny,
                    &published_allow,
                );
            }
        });
    }
    let mut poll = AsyncFd::with_interest(ring, tokio::io::Interest::READABLE)?;
    tokio::task::spawn(async move {
        let mut events_in_tick = 0u64;
        let mut auto_n = init_sample_n;
        let mut quiet_ticks = 0u32;
        let mut last_drops = 0u64;
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
                    let calib = calib_rb.load(Ordering::Relaxed);
                    while let Some(item) = rb.next() {
                        let Some(decoded) = decode_event(item.as_ref()) else {
                            continue;
                        };
                        if let DecodedEvent::Latency(ev) = &decoded {
                            if calib != 0 && u16::from_be(ev.dport_be) == calib {
                                let mut g = lock_mut(&bpf_ids_rb);
                                g.insert(ev.tgid);
                                g.insert(ev.pid);
                            }
                        }
                        let now = Instant::now();
                        let comm = lock_mut(&id_rb).comm_pair(
                            decoded.tgid(),
                            decoded.pid(),
                            now,
                        );
                        let mut agent_ids = lock_mut(&self_ids_rb).clone();
                        agent_ids.extend(lock_mut(&bpf_ids_rb).iter().copied());
                        match classify_ingest(
                            decoded.tgid(),
                            decoded.pid(),
                            &agent_ids,
                            &comm,
                            &filter_rb,
                        ) {
                            IngestSkip::Keep => {}
                            IngestSkip::SelfProcess => {
                                lazy_deny_insert(
                                    &filter_rb,
                                    &denied_map_rb,
                                    &published_deny_rb,
                                    decoded.tgid(),
                                );
                                continue;
                            }
                            IngestSkip::Denied => {
                                *lock_mut(&denied_rb) += 1;
                                lazy_deny_insert(
                                    &filter_rb,
                                    &denied_map_rb,
                                    &published_deny_rb,
                                    decoded.tgid(),
                                );
                                continue;
                            }
                        }
                        match decoded {
                            DecodedEvent::Latency(ev) => {
                                lock_mut(&tcp_rb).record(&ev, now);
                            }
                            DecodedEvent::Io(ev) => {
                                *lock_mut(&sockio_rb) += 1;
                                ingest_http_io(
                                    ev,
                                    false,
                                    false,
                                    now,
                                    &filter_rb,
                                    &id_rb,
                                    &reassemble_rb,
                                    &h2_rb,
                                    &dual_rb,
                                    &inflight_rb,
                                    &denied_rb,
                                    &reasm_rb,
                                    &corr_rb,
                                    &meta_rb,
                                    &peers_rb,
                                    &pods_rb,
                                    &cgroups_rb,
                                    &map_rb,
                                    &http_rb,
                                    &export_rb,
                                );
                            }
                            DecodedEvent::TlsIo(ev) => {
                                *lock_mut(&tlsio_rb) += 1;
                                ingest_http_io(
                                    ev,
                                    true,
                                    tls_client_only,
                                    now,
                                    &filter_rb,
                                    &id_rb,
                                    &reassemble_rb,
                                    &h2_rb,
                                    &dual_rb,
                                    &inflight_rb,
                                    &denied_rb,
                                    &reasm_rb,
                                    &corr_rb,
                                    &meta_rb,
                                    &peers_rb,
                                    &pods_rb,
                                    &cgroups_rb,
                                    &map_rb,
                                    &http_rb,
                                    &export_rb,
                                );
                            }
                            DecodedEvent::SockIoTimes(ev) => {
                                *lock_mut(&socktimes_rb) += 1;
                                if let Some(dir) = IoDir::from_u8(ev.dir) {
                                    lock_mut(&dual_rb).note(ev.tgid, ev.fd, dir, ev.ts_ns, now);
                                }
                            }
                            DecodedEvent::TlsHandshake(ev) => {
                                lock_mut(&dual_rb).record_handshake(ev.latency_ns, now);
                                export_rb.note_handshake(
                                    ev.tgid,
                                    ev.fd,
                                    ev.latency_ns,
                                    ev.ts_ns,
                                    now,
                                );
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
                        if let Some(n) = pinned_sample_n {
                            set_sample_n(&mut sample_n_map, n, &export_rb);
                        } else {
                            let delta = v.saturating_sub(last_drops);
                            last_drops = v;
                            let (next, q) = sample::next_auto_n(auto_n, delta, quiet_ticks);
                            if next != auto_n {
                                info!("SAMPLE_N {auto_n} -> {next} (drops_delta={delta})");
                            }
                            auto_n = next;
                            quiet_ticks = q;
                            set_sample_n(&mut sample_n_map, auto_n, &export_rb);
                        }
                    }
                    if let Ok(v) = tls_unmapped.get(&0, 0) {
                        *lock_mut(&unmapped_rb) = v;
                        export_rb.set_tls_unmapped(v);
                    }
                    let edges = lock_mut(&map_rb).edge_count() as u64;
                    export_rb.set_edge_count(edges);
                    let now = Instant::now();
                    let mut unmarks = {
                        let mut r = lock_mut(&reassemble_rb);
                        r.evict_stale(now);
                        r.take_unmark()
                    };
                    {
                        let mut h = lock_mut(&h2_rb);
                        h.evict_stale(now);
                        {
                            let meta = lock_mut(&meta_rb);
                            let inf = lock_mut(&inflight_rb);
                            h.evict_closed(|tgid, fd| {
                                if fd < 0 {
                                    return false;
                                }
                                let key = ((tgid as u64) << 32) | (fd as u32 as u64);
                                meta.get(&key, 0).is_ok() || inf.get(&key, 0).is_ok()
                            });
                        }
                        unmarks.extend(h.take_unmark());
                    }
                    {
                        let mut d = lock_mut(&dual_rb);
                        d.evict_stale(now);
                        {
                            let meta = lock_mut(&meta_rb);
                            let inf = lock_mut(&inflight_rb);
                            let still_open = |tgid: u32, fd: i32| {
                                if fd < 0 {
                                    return false;
                                }
                                let key = ((tgid as u64) << 32) | (fd as u32 as u64);
                                meta.get(&key, 0).is_ok() || inf.get(&key, 0).is_ok()
                            };
                            d.evict_closed(&still_open);
                            export_rb.evict_handshake(now, &still_open);
                        }
                    }
                    for (tgid, fd) in unmarks {
                        clear_inflight(&inflight_rb, tgid, fd);
                    }
                }
            }
        }
    });

    learn_bpf_self_tgid(&calib_dport);

    let self_src = lock_mut(&identity)
        .resolve(agent_tgid, Instant::now())
        .label;
    info!("Phase 7 agent running (h2/gRPC + HTTP/1.1). q quit, t toggle view.");
    if profile_active {
        info!("Phase 12 CPU profiles enabled (join on spans >= {} ms)", profile_cfg.min_latency_ns / 1_000_000);
    }
    println!("obsagent ready pid={agent_tgid} src={self_src}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
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
            socktimes_count,
            unmapped_count,
            denied_count,
            reasm_count,
            h2,
            dual,
            export,
            profile_store,
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
    from_tls: bool,
    sock_meta: &Mutex<AyaHashMap<MapData, u64, SockMeta>>,
    peer_cache: &Mutex<PeerCache>,
    identity: &Mutex<IdentityResolver>,
    pods: &Mutex<PodIndex>,
    cgroups: &Mutex<CgroupPodIndex>,
    service_map: &Mutex<ServiceMap>,
    http_agg: &Mutex<HttpAggregator>,
    dual: &Mutex<DualPlane>,
    export: &ExportHub,
) {
    if ex.peer.is_none() {
        ex.peer = lookup_peer(sock_meta, peer_cache, ex.tgid, ex.fd, now);
    }
    let Some(mut parsed) = parse_exchange(&ex) else {
        return;
    };
    if from_tls {
        if let Some(j) = lock_mut(dual).join(
            ex.tgid,
            ex.fd,
            ex.t_start_ns,
            ex.t_end_ns,
            ex.req_is_write,
            now,
        ) {
            parsed.wire_ns = Some(j.wire_ns);
        }
    }
    publish_parsed(
        ex.tgid,
        ex.fd,
        0,
        ex.t_start_ns,
        ex.req_is_write,
        ex.peer,
        ex.cgroup_id,
        parsed,
        now,
        sock_meta,
        peer_cache,
        identity,
        pods,
        cgroups,
        service_map,
        http_agg,
        export,
    );
}

fn handle_h2_exchange(
    x: H2Exchange,
    now: Instant,
    sock_meta: &Mutex<AyaHashMap<MapData, u64, SockMeta>>,
    peer_cache: &Mutex<PeerCache>,
    identity: &Mutex<IdentityResolver>,
    pods: &Mutex<PodIndex>,
    cgroups: &Mutex<CgroupPodIndex>,
    service_map: &Mutex<ServiceMap>,
    http_agg: &Mutex<HttpAggregator>,
    export: &ExportHub,
) {
    let path = normalize_path(&x.path);
    debug!(
        "h2 exchange tgid={} fd={} stream={} proto={} method={} status={} lat_ns={}",
        x.tgid,
        x.fd,
        x.stream_id,
        x.protocol,
        x.method,
        x.status,
        x.latency_ns()
    );
    let latency_ns = x.latency_ns();
    let parsed = ParsedExchange {
        endpoint: HttpEndpoint::labeled(x.protocol, x.method, path),
        status: x.status,
        latency_ns,
        wire_ns: None,
    };
    publish_parsed(
        x.tgid,
        x.fd,
        x.stream_id,
        x.t_start_ns,
        x.req_dir == IoDir::Write,
        None,
        x.cgroup_id,
        parsed,
        now,
        sock_meta,
        peer_cache,
        identity,
        pods,
        cgroups,
        service_map,
        http_agg,
        export,
    );
}

fn publish_parsed(
    tgid: u32,
    fd: i32,
    stream_id: u32,
    t_start_ns: u64,
    is_client: bool,
    mut peer: Option<PeerAddr>,
    cgroup_id: u64,
    parsed: ParsedExchange,
    now: Instant,
    sock_meta: &Mutex<AyaHashMap<MapData, u64, SockMeta>>,
    peer_cache: &Mutex<PeerCache>,
    identity: &Mutex<IdentityResolver>,
    pods: &Mutex<PodIndex>,
    cgroups: &Mutex<CgroupPodIndex>,
    service_map: &Mutex<ServiceMap>,
    http_agg: &Mutex<HttpAggregator>,
    export: &ExportHub,
) {
    if is_otlp_export_route(&parsed.endpoint.method, &parsed.endpoint.path) {
        return;
    }
    if peer.is_none() {
        peer = lookup_peer(sock_meta, peer_cache, tgid, fd, now);
    }
    lock_mut(http_agg).record(&parsed, now);

    let mut src = lock_mut(identity).resolve(tgid, now);
    let dst = {
        let idx = lock_mut(pods);
        if src.pod_uid.is_none() {
            if let Some(uid) = lock_mut(cgroups).lookup(cgroup_id) {
                src.pod_uid = Some(uid.to_string());
            }
        }
        if let Some(uid) = src.pod_uid.as_deref() {
            if let Some((ns, name)) = idx.lookup_uid(uid) {
                src.label = format!("{ns}/{name}");
            }
        }
        dst_from_index(peer, &idx)
    };
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
        &parsed.endpoint.protocol,
        parsed.wire_ns,
        TraceMeta {
            tgid,
            fd,
            stream_id,
            t_start_ns,
            is_client,
        },
        now,
    );
}

fn lookup_peer(
    sock_meta: &Mutex<AyaHashMap<MapData, u64, SockMeta>>,
    peer_cache: &Mutex<PeerCache>,
    tgid: u32,
    fd: i32,
    now: Instant,
) -> Option<PeerAddr> {
    if fd < 0 {
        return None;
    }
    if let Some(p) = lock_mut(peer_cache).get(tgid, fd, now) {
        return Some(p);
    }
    let key = ((tgid as u64) << 32) | (fd as u32 as u64);
    let map = lock_mut(sock_meta);
    let meta = map.get(&key, 0).ok()?;
    let peer = PeerAddr::from_meta(meta)?;
    drop(map);
    lock_mut(peer_cache).insert(tgid, fd, peer, now);
    Some(peer)
}

fn dst_from_index(peer: Option<PeerAddr>, idx: &PodIndex) -> DstId {
    let Some(p) = peer else {
        return DstId::Unknown;
    };
    // Pod IP first, then Service ClusterIP → Service ns/name (not a replica).
    if let Some(daddr) = p.v4_addr() {
        if let Some((ns, name)) = idx.lookup_ip(daddr) {
            return DstId::Pod {
                namespace: ns,
                name,
            };
        }
    }
    DstId::IpPort {
        addr: format_peer(&p),
    }
}

fn attach_perf(ebpf: &mut aya::Ebpf, freq_hz: u64) -> anyhow::Result<()> {
    let prog: &mut PerfEvent = ebpf
        .program_mut("profile_sample")
        .context("program profile_sample")?
        .try_into()?;
    prog.load()?;
    let config = PerfEventConfig::Software(SoftwareEvent::CpuClock);
    let cpus = online_cpus().map_err(|(_, e)| e)?;
    for cpu in cpus {
        prog.attach(
            config,
            PerfEventScope::AllProcessesOneCpu { cpu },
            SamplePolicy::Frequency(freq_hz),
            false,
        )
        .with_context(|| format!("profile_sample attach cpu={cpu}"))?;
    }
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

/// Phase 3 Q5: try-attach `libssl.so.3` / `libssl.so.1.1`. Q6/Q12: soft-fail.
fn libssl_candidates() -> Vec<String> {
    const KNOWN: &[&str] = &[
        "/lib/x86_64-linux-gnu/libssl.so.3",
        "/usr/lib/x86_64-linux-gnu/libssl.so.3",
        "/lib/x86_64-linux-gnu/libssl.so.1.1",
        "/usr/lib/x86_64-linux-gnu/libssl.so.1.1",
        "/lib64/libssl.so.3",
        "/usr/lib64/libssl.so.3",
    ];
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |p: String| {
        if Path::new(&p).exists() && seen.insert(p.clone()) {
            out.push(p);
        }
    };
    for p in KNOWN {
        push((*p).to_string());
    }
    if let Ok(p) = std::env::var("OBSAGENT_LIBSSL") {
        push(p);
    }
    out
}

fn attach_openssl_uprobes(ebpf: &mut aya::Ebpf) {
    let candidates = libssl_candidates();

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
    let mut have_hs = true;
    for name in ["enter_ssl_do_handshake", "exit_ssl_do_handshake"] {
        if let Err(e) = load_uprobe(ebpf, name) {
            warn!("failed to load optional {name}: {e:#}");
            have_hs = false;
            break;
        }
    }

    let mut attached_any = false;
    let mut seen_inodes = HashSet::new();
    for path in &candidates {
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
                    Err(e) => warn!("optional {prog} → {sym} in {path}: {e:#}"),
                }
            }
            if ex_ok == 0 {
                warn!("no SSL_*_ex symbols in {path}; CPython will not emit TlsIo");
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
        if have_hs {
            match attach_uprobe(ebpf, "enter_ssl_do_handshake", "SSL_do_handshake", path) {
                Ok(()) => {
                    if let Err(e) =
                        attach_uprobe(ebpf, "exit_ssl_do_handshake", "SSL_do_handshake", path)
                    {
                        warn!("optional exit SSL_do_handshake in {path}: {e:#}");
                    } else {
                        info!("attached SSL_do_handshake in {path}");
                    }
                }
                Err(e) => warn!("optional SSL_do_handshake in {path}: {e:#}"),
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

/// One localhost connect so drain can record the tgid BPF actually emits
/// (may differ from `getpid()` when pid namespaces do not match).
fn learn_bpf_self_tgid(calib_dport: &AtomicU16) {
    let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else {
        return;
    };
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    let _ = listener.set_nonblocking(true);
    calib_dport.store(addr.port(), Ordering::Relaxed);
    let _ = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300));
    let _ = listener.accept();
    std::thread::sleep(Duration::from_millis(200));
    calib_dport.store(0, Ordering::Relaxed);
}

fn set_sample_n(map: &mut Array<MapData, u32>, n: u32, export: &ExportHub) {
    match map.set(0, n, 0) {
        Ok(_) => export.set_bpf_sample_n(n),
        Err(e) => warn!("SAMPLE_N set {n}: {e}"),
    }
}

fn lazy_deny_insert(
    filter: &CommFilter,
    denied_map: &Mutex<AyaHashMap<MapData, u32, u8>>,
    published: &Mutex<HashSet<u32>>,
    tgid: u32,
) {
    if filter.is_allow_only() {
        return;
    }
    if lock_mut(published).contains(&tgid) {
        return;
    }
    if let Err(e) = lock_mut(denied_map).insert(tgid, 1u8, 0) {
        warn!("DENIED_TGID lazy insert {tgid}: {e}");
        return;
    }
    lock_mut(published).insert(tgid);
}

fn apply_tgid_hashmap(
    map: &Mutex<AyaHashMap<MapData, u32, u8>>,
    published: &Mutex<HashSet<u32>>,
    desired: &HashSet<u32>,
    name: &str,
) {
    let (remove, insert) = {
        let pubd = lock_mut(published);
        tgid_map_delta(&pubd, desired)
    };
    for t in remove {
        match lock_mut(map).remove(&t) {
            Ok(_) => {
                lock_mut(published).remove(&t);
            }
            Err(e) => warn!("{name} remove {t}: {e}"),
        }
    }
    for t in insert {
        if let Err(e) = lock_mut(map).insert(t, 1u8, 0) {
            warn!("{name} insert {t}: {e}");
            continue;
        }
        lock_mut(published).insert(t);
    }
}

fn sweep_capture_maps(
    agent_tgid: u32,
    filter: &CommFilter,
    identity: &Mutex<IdentityResolver>,
    bpf_self_ids: &Mutex<HashSet<u32>>,
    self_ids: &Mutex<HashSet<u32>>,
    denied_map: &Mutex<AyaHashMap<MapData, u32, u8>>,
    allowed_map: &Mutex<AyaHashMap<MapData, u32, u8>>,
    allow_only_map: &Mutex<Array<MapData, u32>>,
    published_deny: &Mutex<HashSet<u32>>,
    published_allow: &Mutex<HashSet<u32>>,
) {
    *lock_mut(self_ids) = collect_self_ids(agent_tgid);
    let allow_only = filter.is_allow_only();
    let flag = if allow_only { 1u32 } else { 0 };
    if let Err(e) = lock_mut(allow_only_map).set(0, flag, 0) {
        warn!("ALLOW_ONLY set {flag}: {e}");
    }
    let root = lock_mut(identity).proc_root().to_path_buf();
    if allow_only {
        let desired = allowed_tgids_from_proc(&root, filter);
        apply_tgid_hashmap(allowed_map, published_allow, &desired, "ALLOWED_TGID");
    } else {
        let mut extras = HashSet::from([agent_tgid]);
        extras.extend(lock_mut(bpf_self_ids).iter().copied());
        let desired = denied_tgids_from_proc(&root, filter, &extras);
        apply_tgid_hashmap(denied_map, published_deny, &desired, "DENIED_TGID");
    }
}

fn lock_mut<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn clear_inflight(map: &Mutex<AyaHashMap<MapData, u64, u8>>, tgid: u32, fd: i32) {
    if fd < 0 {
        return;
    }
    let key = ((tgid as u64) << 32) | (fd as u32 as u64);
    let _ = lock_mut(map).remove(&key);
}

fn ingest_http_io(
    ev: SockIoEvent,
    from_tls: bool,
    client_only: bool,
    now: Instant,
    filter: &CommFilter,
    identity: &Mutex<IdentityResolver>,
    reassembler: &Mutex<Reassembler>,
    h2: &Mutex<H2Registry>,
    dual: &Mutex<DualPlane>,
    inflight: &Mutex<AyaHashMap<MapData, u64, u8>>,
    denied: &Mutex<u64>,
    reasm_count: &Mutex<u64>,
    correlator: &Mutex<Correlator>,
    sock_meta: &Mutex<AyaHashMap<MapData, u64, SockMeta>>,
    peer_cache: &Mutex<PeerCache>,
    pods: &Mutex<PodIndex>,
    cgroups: &Mutex<CgroupPodIndex>,
    service_map: &Mutex<ServiceMap>,
    http_agg: &Mutex<HttpAggregator>,
    export: &ExportHub,
) {
    if !filter.is_passthrough() {
        let comm = lock_mut(identity).comm(ev.tgid, now);
        if !filter.allow(&comm) {
            *lock_mut(denied) += 1;
            lock_mut(h2).drop_conn(ev.tgid, ev.fd);
            clear_inflight(inflight, ev.tgid, ev.fd);
            return;
        }
    }

    let plen = (ev.prefix_len as usize).min(ev.prefix.len());
    let chunk = if plen == 0 {
        &[][..]
    } else {
        &ev.prefix[..plen]
    };

    if lock_mut(h2).is_marked(ev.tgid, ev.fd) && looks_like_http11(chunk) {
        lock_mut(h2).drop_conn(ev.tgid, ev.fd);
        for (tgid, fd) in lock_mut(h2).take_unmark() {
            clear_inflight(inflight, tgid, fd);
        }
    }

    let use_h2 = lock_mut(h2).is_marked(ev.tgid, ev.fd) || looks_like_h2(chunk);
    if use_h2 {
        let (exchanges, unmarks) = {
            let mut h = lock_mut(h2);
            let xs = h.feed(&ev, now, client_only);
            (xs, h.take_unmark())
        };
        for (tgid, fd) in unmarks {
            clear_inflight(inflight, tgid, fd);
        }
        for x in exchanges {
            handle_h2_exchange(
                x,
                now,
                sock_meta,
                peer_cache,
                identity,
                pods,
                cgroups,
                service_map,
                http_agg,
                export,
            );
        }
        return;
    }

    let (flushed, unmarks) = {
        let mut r = lock_mut(reassembler);
        let flushed = r.push(&ev, now);
        (flushed, r.take_unmark())
    };
    for (tgid, fd) in unmarks {
        clear_inflight(inflight, tgid, fd);
    }
    let Some(ev) = flushed else {
        return;
    };
    *lock_mut(reasm_count) += 1;
    clear_inflight(inflight, ev.tgid, ev.fd);
    let ex = if client_only {
        lock_mut(correlator).observe_client(&ev, now)
    } else {
        lock_mut(correlator).observe(&ev, now)
    };
    if let Some(ex) = ex {
        handle_exchange(
            ex,
            now,
            from_tls,
            sock_meta,
            peer_cache,
            identity,
            pods,
            cgroups,
            service_map,
            http_agg,
            dual,
            export,
        );
    }
}

async fn run_headless(
    tcp_agg: Arc<Mutex<Aggregator>>,
    http_agg: Arc<Mutex<HttpAggregator>>,
    service_map: Arc<Mutex<ServiceMap>>,
    drop_count: Arc<Mutex<u64>>,
    sockio_count: Arc<Mutex<u64>>,
    tlsio_count: Arc<Mutex<u64>>,
    socktimes_count: Arc<Mutex<u64>>,
    unmapped_count: Arc<Mutex<u64>>,
    denied_count: Arc<Mutex<u64>>,
    reasm_count: Arc<Mutex<u64>>,
    h2: Arc<Mutex<H2Registry>>,
    dual: Arc<Mutex<DualPlane>>,
    export: Arc<ExportHub>,
    profile_store: Arc<Mutex<ProfileStore>>,
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
                let socktimes = *lock_mut(&socktimes_count);
                let unmapped = *lock_mut(&unmapped_count);
                let denied = *lock_mut(&denied_count);
                let reasm = *lock_mut(&reasm_count);
                let h2s = lock_mut(&h2).stats.clone();
                let d = lock_mut(&dual);
                let jw = d.window(now);
                let join_hit = jw.hit;
                let join_miss = jw.miss;
                let join_fb = jw.fallback;
                let join_pct = d.join_hit_rate(now) * 100.0;
                let hs_p50 = d.handshake_p50_ns().map(fmt_ns).unwrap_or_else(|| "-".into());
                let hs = d.handshake_count();
                drop(d);
                let edges = lock_mut(&service_map).edge_count();
                let export_drop = export.dropped.load(std::sync::atomic::Ordering::Relaxed);
                let export_ok = export.exported.load(std::sync::atomic::Ordering::Relaxed);
                let trace_s = export.traces_sampled.load(std::sync::atomic::Ordering::Relaxed);
                let trace_ns = export
                    .traces_not_sampled
                    .load(std::sync::atomic::Ordering::Relaxed);
                let trace_drop = export
                    .traces_dropped
                    .load(std::sync::atomic::Ordering::Relaxed);
                let trace_fail = export
                    .traces_export_failed
                    .load(std::sync::atomic::Ordering::Relaxed);
                let otlp = if export.enabled() { "on" } else { "off" };
                let sample_n = export.bpf_sample_n();
                let prof_hit = export.profile_join_hits.load(Ordering::Relaxed);
                let prof_miss = export.profile_join_miss.load(Ordering::Relaxed);
                let (prof_ing, prof_store_drop) = if lock_mut(&profile_store).config().enabled {
                    let s = lock_mut(&profile_store);
                    (s.samples_ingested, s.samples_dropped)
                } else {
                    (0, 0)
                };
                println!(
                    "events_60s={tcp_total} tcp_60s={tcp_total} http_60s={http_total} sockio={sockio} tlsio={tlsio} socktimes={socktimes} denied={denied} reasm={reasm} h2_conns={} h2_xchg={} h2_drop={} h2_nopath={} h2_hreq={} h2_hresp={} h2_unpair={} h2_desync={} join_hit={join_hit} join_miss={join_miss} join_fb={join_fb} join={join_pct:.0}% hs={hs} hs_p50={hs_p50} tls_unmapped={unmapped} drops={drops} sample_n={sample_n} prof_hit={prof_hit} prof_miss={prof_miss} prof_ing={prof_ing} prof_store_drop={prof_store_drop} edges={edges} otlp={otlp} otlp_ok={export_ok} otlp_drop={export_drop} trace_s={trace_s} trace_ns={trace_ns} trace_drop={trace_drop} trace_fail={trace_fail}",
                    h2s.conn_count, h2s.exchanges, h2s.dropped_streams, h2s.missing_path, h2s.headers_req, h2s.headers_resp, h2s.unpaired_status, h2s.desync
                );
                let _ = std::io::Write::flush(&mut std::io::stdout());
                for (k, r) in http_rows.iter().take(8) {
                    let wire = r
                        .wire_p50_ns
                        .map(fmt_ns)
                        .unwrap_or_else(|| "-".into());
                    println!(
                        "  {} count={} rate={:.2}/s p50={} wire_p50={} 4xx={:.0}% 5xx={:.0}%",
                        k.label(),
                        r.count,
                        r.rate_per_s,
                        fmt_ns(r.p50_ns),
                        wire,
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
