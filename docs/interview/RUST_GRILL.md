# Rust grill — from zero, using **this** repo only

**Use when:** they say “why Rust?”, “explain ownership”, “what’s unsafe here?”, or a Rust JD.  
**Companion:** [CONCEPTS_GRILL.md](CONCEPTS_GRILL.md) · [SCRIPT_AND_QNA.md](SCRIPT_AND_QNA.md) · [RESUME_AND_STORY.md](RESUME_AND_STORY.md)

This file assumes you did **not** grow up in Rust. Every idea is tied to a file in `eBPF-Observability-Agent`. If you cannot point at the file, you do not know it yet.

**Honesty (read once, then stop talking about it):** the implementation was driven by you as product/architecture + an AI pair for code. Interviews still require **you** to walk types, compile story, and failure modes. Do **not** lead with “I don’t know Rust” or “ChatGPT wrote it.” Lead with the crates. If they ask how you worked: “I locked the ABI and gates; I can open any module and explain it.” If you cannot open the module, study this file until you can. Lying about authorship is worse than “I pair-programmed and own the design.”

**Rust level this repo justifies (if you study this file):** intern / junior systems — workspace, `no_std`, `repr(C)`, `Option`/`Result`, `Arc<Mutex<T>>`, Tokio tasks, small `unsafe`. It does **not** justify “senior Rustacean,” lifetimes gymnastics, or writing a proc-macro.

**Study order (5 sittings):**

1. §0–§2 crates, `cargo`, `no_std` vs std  
2. §3–§5 ownership, `Option`/`Result`, String vs bytes  
3. §6–§8 traits, HashMap, Arc/Mutex/Atomics  
4. §9–§11 async, unsafe islands, BPF Rust  
5. §12 quizzes + SCRIPT Rust Q&A  

---

## 0. What Rust is in one minute (then map to this repo)

Rust is a compiled language (like C) with **no garbage collector**. Memory is freed when the **owner** of a value goes out of scope. The compiler refuses programs that would use-after-free or data-race in safe code.

**Why this project used it (say this):**

1. **Aya** — the mature Rust eBPF toolchain (CO-RE, load/attach).  
2. **One language** for kernel ABI (`common`), BPF (`ebpf`), and agent (`obsagent`).  
3. **`#[repr(C)]` structs** shared with the kernel — same layout as C would use.  
4. **Tests** on the correlator without a kernel (`correlate.rs` `#[cfg(test)]`).

**Why not C + libbpf only?** You can. We chose Aya so the agent and ABI are Rust. The BPF still looks like tiny C structs + helpers.

**Why not Go?** Go is great for control planes. eBPF programs are not a normal Go runtime; you’d still write C/libbpf or bind. Tokio agent in Rust sits next to Aya cleanly.

**Trap:** “Rust is memory-safe so we have no unsafe.” We have **unsafe islands** (`decode.rs`, `setrlimit`, `aya::Pod`, BPF `bpf_probe_read_*`). Safety is **scoped**, not absolute.

---

## 1. Cargo workspace (they will ask “what’s a crate?”)

| Word | Meaning | Here |
| --- | --- | --- |
| **Workspace** | One git repo, many packages, shared `Cargo.toml` versions | root `Cargo.toml` `members` |
| **Crate** | A compile unit (lib or bin) | `obsagent-common`, `obsagent-ebpf`, `obsagent` |
| **Package** | Cargo folder with `Cargo.toml` | `common/`, `ebpf/`, `agent/` |
| **Module** | File or `mod foo;` inside a crate | `agent/src/correlate.rs` is module `correlate` |
| **Target** | What you actually build | agent **bin** `obsagent`; eBPF **bin** `probes` |

Root workspace:

- `edition = "2024"` on packages.  
- `default-members = ["agent", "common"]` — **`ebpf` is excluded** from `cargo build --workspace` because it only builds for `bpfel-unknown-none`, not the host.  
- Shared versions under `[workspace.dependencies]` (aya, tokio, anyhow, httparse, …).

**Agent `Cargo.toml`:** depends on `obsagent-common` with **`features = ["user"]`** — that turns on `aya::Pod` impls the kernel crate must not need.

**eBPF `Cargo.toml`:** `obsagent-common` **without** `user`. `[[bin]] name = "probes"` — Aya copies that ELF into the agent `OUT_DIR`.

**`agent/build.rs`:** cargo-in-cargo: compile the BPF crate with nightly + `-Z build-std`, emit `probes` object. Agent then `include_bytes_aligned!(concat!(env!("OUT_DIR"), "/probes"))`.

**Say:** “Three crates, two compile targets. BPF is a different triple. The agent **embeds** the ELF at build time.”

**Trap:** “`cargo test --workspace` runs BPF tests on Windows.” BPF is WSL; `test-agent` is userspace.

---

## 2. `no_std` vs `std` (the most important Rust fact in this repo)

**`std`:** OS, files, threads, `HashMap` from libstd, `String`, Tokio. **Agent uses this.**

**`core`:** language primitives that don’t need an OS (`Option`, `Result`, `size_of`, slices).

**`no_std`:** crate cannot use libstd. BPF **cannot** have an OS allocator like userspace.

Files:

```text
common/src/lib.rs     #![no_std]
ebpf/src/main.rs      #![no_std]  #![no_main]
ebpf/src/lib.rs       #![no_std]
agent/                std (implicit)
```

`#![no_main]` — no Rust `fn main` runtime. The kernel invokes probe functions. Aya macros generate the ELF sections.

**`common` is `no_std` so BPF and agent share types.** Agent still uses `String` in `http.rs` because that code is **not** in `common`.

**Say:** “The ABI crate is `no_std` so the same structs compile into `bpfel` and into the Tokio agent. HTTP parse lives in the agent because `httparse` + `String` want std.”

**Feature `user`:** `#[cfg(feature = "user")] unsafe impl aya::Pod for SockLatencyEvent {}` — only the agent crate enables this. BPF doesn’t implement `Pod`; it *is* the producer of those bytes.

---

## 3. Ownership, borrow, move (teach with `Correlator`)

Rust rule: **every value has one owner.** You can **borrow** (`&T` read, `&mut T` exclusive write) or **move** (owner changes).

### 3.1 This repo’s picture

`Correlator` **owns** `HashMap<SockKey, Pending>`.

```text
observe(&mut self, ev: &SockIoEvent, now: Instant) -> Option<Exchange>
```

| Piece | Why that type |
| --- | --- |
| `&mut self` | we insert/remove pending; exclusive |
| `ev: &SockIoEvent` | borrow the decoded event; don’t take it away from the drain loop |
| `now: Instant` | `Copy` — cheap pass by value |
| `Option<Exchange>` | maybe no pair yet |

`Pending` **owns** `prefix: Vec<u8>` — a heap buffer copied from the 256 B array. When we emit `Exchange`, we **move** `pending.prefix` into `req_prefix`. No clone of the whole FSM.

**`ev.prefix[..plen].to_vec()`** — the event’s `prefix` is a **fixed array** `[u8; 256]` (`Copy`). `to_vec()` **allocates** a `Vec` of the real length. That’s userspace; BPF cannot `Vec::new()`.

### 3.2 `&str` vs `String` vs `[u8]` vs `Vec<u8>`

| Type | Owns data? | Here |
| --- | --- | --- |
| `[u8; 256]` | yes, inline in the struct | `SockIoEvent.prefix` |
| `&[u8]` | no, borrow | `decode_event(bytes: &[u8])`, `looks_like_request` |
| `Vec<u8>` | yes, heap | `Pending.prefix`, `Exchange.req_prefix` |
| `&str` | borrow UTF-8 | `normalize_path(path: &str)` |
| `String` | yes, heap UTF-8 | `HttpEndpoint.method`, OTLP labels |

**Say:** “Kernel gives me bytes. I borrow them to sniff `GET `. I own a `Vec` if I keep the half. I only make a `String` after httparse gives me a method.”

**Clone:** `parsed.endpoint.clone()` when inserting into `HttpAggregator` HashMap — the map **must own** the key. Clone is explicit (not Java-silent). If they ask “isn’t clone slow?” — keys are short strings; we don’t clone 256 B prefixes into OTLP.

### 3.3 `Copy` vs `Clone`

`#[derive(Clone, Copy)]` on `SockKey`, `PeerV4`, `SockMeta`, `EventKind`.

- **`Copy`:** bitwise copy, no destructor. Integers, small structs of Copy fields. Passing `SockKey` doesn’t invalidate the old one.  
- **`Clone`:** might allocate (`String`, `Vec`). Must call `.clone()`.

`SockIoEvent` is `Copy` (fixed array, no Vec). After decode, the drain can copy it into `observe(&ev)`.

`Exchange` is **not** Copy — it holds `Vec<u8>`. Moving it into `handle_exchange` is a move.

### 3.4 Lifetimes (only what you need)

`fn lock_mut<T>(m: &Mutex<T>) -> MutexGuard<'_, T>`

The `'_` means: the guard **cannot outlive** the borrow of the mutex. You don’t write fancy `'a` anywhere in the correlator. **Say:** “I didn’t fight the borrow checker with explicit lifetimes. The compiler inferred them. The one I can name is MutexGuard tied to the lock.”

If they ask for a lifetime example: `find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize>` returns an **index**, not a reference into `hay`, so no lifetime on the return.

---

## 4. `Option` and `Result` (this is 40% of junior Rust interviews)

### 4.1 `Option<T>` = maybe missing

Used instead of null.

| Site | `None` means |
| --- | --- |
| `EventKind::from_u8` | unknown kind byte → skip record |
| `decode_event` | truncated / bad kind |
| `observe` | no completed exchange this event |
| `parse_exchange` | not HTTP / httparse fail |
| `request.method?` | `?` on `Option` inside a function that returns `Option` — missing method → `None` |

`bytes.first()?` in `decode_event`: empty slice → `None`. The `?` operator **returns early**.

**Say:** “I don’t unwrap on the drain path for parse failures. Bad events are skipped. Completeness is already a RingBuf story.”

### 4.2 `Result<T, E>` = recoverable failure

`main() -> anyhow::Result<()>` — attach/load failures **stop the process** (can’t run without programs).

`anyhow` is a **trait object error** crate: `context("EVENTS map missing")` adds a string. `?` bubbles `Err` to `main`.

| Path | Result vs Option |
| --- | --- |
| Load BPF / attach | `Result` — fatal |
| Decode one event | `Option` — skip |
| httparse | `Option` — skip |
| OTLP POST | `Result` inside export task — **log + `otlp_dropped`**, don’t kill drain |

**Trap:** `.unwrap()` on the drain. We use `lock_mut` poison recovery instead of unwrap-to-panic on mutex (see §8). `logger.readable_mut().await.unwrap()` in the log task is a weaker pattern — if they ask, “that log task can panic; the drain is more careful.”

### 4.3 `match` vs `if let` vs `?`

```text
let Some(decoded) = decode_event(...) else { continue; };
if let Some(ex) = correlator.observe(...) { handle_exchange(...) }
match decoded { Latency / Io / TlsIo => ... }
```

**Say:** “`else { continue }` is the let-else syntax: missing decode → next RingBuf item.”

---

## 5. Enums (Rust’s superpower vs C)

`EventKind { Connect=1, Accept=2, SockIo=3, TlsIo=4 }` — `#[repr(u8)]` so the **first byte on the wire** matches.

`DecodedEvent { Latency(...), Io(...), TlsIo(...) }` — after decode, the drain **must** handle all three or the compiler errors. That’s why we don’t forget TLS in the match.

`IoDir { Read=1, Write=2 }`.

**Say:** “Enums plus `match` are how we keep kernel kinds and userspace handling in sync. Unknown `u8` is `from_u8` → `None`, not a C default case that falls through.”

`View { Tcp, Http }` in `main.rs` — TUI mode. `Copy` enum.

---

## 6. Traits you actually use

You do **not** need to write a custom trait for the interview. You need **derives** and **one unsafe trait**.

| Trait | Why | Where |
| --- | --- | --- |
| `Clone` | copy `String` keys | `HttpEndpoint`, `SeriesKey` |
| `Copy` | pass keys cheaply | `SockKey`, `PeerV4` |
| `Debug` | tests / logs | almost everything |
| `PartialEq, Eq` | HashMap keys need Eq | `SockKey`, `SeriesKey` |
| `Hash` | HashMap | `SockKey`, `HttpEndpoint`, `SeriesKey` |
| `Default` | `Correlator::default()`, aggregators | empty maps |
| `aya::Pod` | “plain old data” — safe to dump into BPF maps | `unsafe impl` under `feature = "user"` |

**`Pod` sentence:** “The struct is `repr(C)`, no Rust pointers, no `String`. Aya can copy it to the kernel. Implementing `Pod` is `unsafe` because we promise that layout.”

**HashMap key rule:** `Eq + Hash`. That’s why `SeriesKey` has `String` fields not `&str` (the map owns the key).

---

## 7. Collections and iterators (concrete)

**`HashMap<SockKey, Pending>`** — correlator. `remove(&key)` takes ownership of pending. `insert` overwrites.

**`HashMap<SeriesKey, SeriesStats>`** — metrics. Cap 2048.

**`Vec<u8>`** — prefixes we keep.

**`windows(needle.len()).position(|w| w == needle)`** in `find_subslice` — iterator over overlapping slices. Classic “I can read iterator code.”

**`path.split('/').map(...).collect()`** in `normalize_path` — allocate `Vec<&str>` then `join`. Not the fastest; it’s clear. If they ask to optimize: in-place scan without Vec — we didn’t.

**`retain`** in `evict_stale` — drop pending older than 60s.

**`saturating_sub`** on timestamps — no panic on underflow if clocks look weird.

---

## 8. Concurrency: `Arc`, `Mutex`, `AtomicU64`, Tokio tasks

This is the **Rust systems** question. Draw it.

```text
main (Tokio runtime)
  ├─ drain task     (spawn)  locks correlator, aggs, maps
  ├─ export task    (spawn)  locks MetricsRegistry every 10s, reqwest POST
  ├─ pod index task (spawn)  locks PodIndex every 15s
  ├─ eBPF log task  (spawn)  AsyncFd on logger
  └─ TUI / headless (main)   locks aggs to print
```

### 8.1 Why `Arc<Mutex<T>>`

- **`Mutex<T>`:** only one thread/task mutates `T` at a time. Correlator is **not** Sync by itself in a useful way if we just used `&mut` from two tasks.  
- **`Arc`:** **A**tomic **R**eference **C**ounted. Multiple tasks **own** a handle to the same mutex. `Arc::clone` bumps the count; it does **not** clone the correlator.

Drain does `let corr_rb = Arc::clone(&correlator);` then `move` into `tokio::task::spawn`. Main keeps `correlator` for… actually drain **moves** the clones; TUI uses the originals that stayed in `main`. Same heap.

**Say:** “Arc is shared ownership. Mutex is exclusive mutation. Clone of Arc is cheap. Clone of Correlator would be wrong.”

### 8.2 `lock_mut`

```rust
fn lock_mut<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
```

If a thread **panicked** while holding the lock, the mutex is **poisoned**. Default `.unwrap()` would panic again. `into_inner()` recovers the data. **Say:** “I’d rather keep draining after a bug than freeze the agent on poison.”

Export snapshot: `reg.lock().unwrap_or_else(|p| p.into_inner())` then `to_otlp_json()` **inside** the lock, then POST **outside** (body is an owned `String`). That’s how we don’t block drain on HTTP — drain only `record()` which is a short lock.

### 8.3 `AtomicU64`

`ExportHub.dropped` / `exported` — `fetch_add(1, Ordering::Relaxed)` from the export task. Counters that don’t need a mutex. **Relaxed** is OK for stats, not for publishing a pointer.

**When mutex vs atomic:** complex struct → Mutex. Single counter from one writer mostly → Atomic.

### 8.4 `Send` / `Sync` (vocab)

- **`Send`:** can move to another thread.  
- **`Sync`:** can share `&T` across threads.

`Arc<Mutex<Correlator>>` is Send+Sync. That’s why spawn compiles. You didn’t implement these; they auto-impl.

**Trap:** “async means no locks.” We use **both**: async for I/O wait (RingBuf fd, HTTP POST), mutex for in-memory FSM.

---

## 9. Async / Tokio (what you must say)

**`#[tokio::main] async fn main()`** — starts a **multi-thread** runtime (`rt-multi-thread` in `agent/Cargo.toml`).

**`async` function:** can `.await`. It does not mean “runs in parallel by itself.”

**`.await`:** yield this task until the future is ready (socket readable, timer, HTTP response).

**`tokio::task::spawn`:** schedule another task on the runtime. The drain loop is a spawn so `main` can run TUI/`select` on Ctrl-C.

**`tokio::select!`** in the drain: wait for **either** RingBuf readable **or** 1s tick (drop counter scrape). Same idea in headless/TUI.

**`AsyncFd`:** wrap the RingBuf fd so Tokio wakes when the kernel has events. Without it you’d busy-poll.

**`MissedTickBehavior::Skip`:** if we were busy, don’t catch up 50 ticks.

**Export:** `interval(10s)` → snapshot JSON → `client.post().send().await`. Timeout 5s on reqwest client.

**Say:** “CPU work (httparse, FSM) is sync inside the drain task. I/O wait is async. I do not `.await` the collector on the RingBuf loop.”

**Trap:** “Rust async is like Node.” Similar event loop **idea**, different: no JS runtime, Send bounds, Mutex not a single thread.

**`async move { ... }`:** the closure **takes ownership** of cloned Arcs. That’s why we `Arc::clone` before spawn.

---

## 10. `unsafe` islands (list them; don’t be ashamed)

Safe Rust forbids raw pointer deref, calling FFI, implementing `Pod` incorrectly.

| Location | What | Why unsafe | How we contain it |
| --- | --- | --- | --- |
| `main.rs` `setrlimit` | `libc` FFI | C ABI | one call, ignore failure |
| `decode.rs` `ptr::read_unaligned` | interpret bytes as struct | alignment/size | **length check first**, check `kind` field |
| `common` `unsafe impl Pod` | tell Aya layout is C-like | we could lie | `repr(C)` + size tests |
| `ebpf/` `bpf_probe_read_user` | read userspace | kernel helper | Aya wrapper, Result |
| `ebpf/` map `.get` | kernel map | BPF context | Aya |

**Decode ritual (say it):** “I never transmute a short slice. I check `bytes.len()`, read_unaligned, then verify `ev.kind` matches. `prefix_len` is clamped to 256.”

**Trap:** “unsafe means the program is unsafe.” It means **you** promised the compiler something. The length check is the real safety.

**BPF `unsafe { ctx.read_at(offset) }`:** tracepoint field offsets (`ENTER_FD_OFF = 16`, …) from **this** kernel’s format. That’s CO-RE-ish / offset table — mention WSL 6.6 comments in `ebpf/src/main.rs`.

---

## 11. BPF-side Rust (different language dialect)

`ebpf/src/main.rs`:

- `#[map] static PENDING: HashMap<u32, PendingEnter>` — **kernel** HashMap, max 8192, not `std::collections`.  
- `#[tracepoint]`, `#[uprobe]`, `#[uretprobe]`, `#[kprobe]` — attach points.  
- Helpers: `bpf_get_current_pid_tgid`, `bpf_ktime_get_ns`, `bpf_probe_read_user`, `bpf_probe_read_user_buf`.  
- `Result<(), i64>` on try_* functions — BPF error codes, not anyhow.  
- Separate maps `PENDING_TLS` vs `PENDING_TLS_EX` so `_ex` doesn’t clobber classic SSL_read stash.

**Say:** “It looks like Rust but you don’t allocate, don’t panic, don’t iterate unbounded. The verifier is the real type system.”

**`no_main`:** entry is each probe, not `main`.

---

## 12. Generics (only this much)

`lock_mut<T>` — works for any `T` inside Mutex.  
`HashMap<K,V>` — K = SockKey or SeriesKey.  
`Array<MapData, u64>` — Aya typed map.  
`Arc<Mutex<T>>` — T = Correlator, etc.

You did not write a generic FSM. **Don’t pretend you designed a trait `Probe`.**

---

## 13. Tests, `cfg`, modules

`#[cfg(test)] mod tests` in correlate, http, decode, metrics_registry — compiled **only** for `cargo test`.

`wsl-run.sh test-agent` → 38 passed. Those tests construct `SockIoEvent` **by hand** (no kernel).

`mod correlate;` in `main.rs` — pulls `agent/src/correlate.rs`. Private by default; `pub fn observe` is public to the crate.

**`#[allow(dead_code)]` on `redact_headers`:** comment says keep off hot path; may warn if unused. Know it exists for security Qs.

---

## 14. Error / panic / logging

| Mechanism | Use |
| --- | --- |
| `log::{info,warn,debug}` + `env_logger` | `RUST_LOG=warn` in DaemonSet |
| `anyhow::Context` | attach failures |
| `warn!` + drop counter | OTLP 400 |
| `panic!` | avoid on drain; tests may assert |

**`expect("bind")`** in testdata Axum server is OK for a test binary, not for the agent.

---

## 15. Why these crates (dependency one-liners)

| Crate | Role |
| --- | --- |
| `aya` / `aya-ebpf` | load, maps, attach |
| `tokio` | runtime, AsyncFd, timers, spawn |
| `anyhow` | agent errors |
| `httparse` | HTTP/1.1 without a full hyper stack |
| `hdrhistogram` | TUI p50/p95 |
| `reqwest` + `rustls-tls` | OTLP HTTP JSON (userspace TLS to collector — **not** the uprobe story) |
| `serde_json` | build OTLP body |
| `ratatui` + `crossterm` | TUI |
| `libc` | setrlimit |
| `log` / `env_logger` | logging |

**Trap:** “We use reqwest rustls so we support rustls apps.” That’s the **agent talking to the collector**. Target-app rustls is still **not** hooked.

---

## 16. 30-second Rust pitch (for a Rust JD)

The workspace is three crates. `obsagent-common` is `no_std` `repr(C)` ABI — 48-byte latency events and 288-byte I/O events — compiled into both BPF and the agent. The BPF crate is `no_std` `no_main` Aya programs. The agent is Tokio: one drain task on `AsyncFd` over the RingBuf, `Arc<Mutex<Correlator>>` for the FSM, a second task that snapshots metrics and POSTs OTLP so the drain never awaits the collector. Decode is a small `unsafe` `read_unaligned` after a length check. I can walk `Option` skip paths and mutex poison recovery. I am not claiming I wrote a Rust compiler or a lock-free queue.

---

## 17. 2-minute Rust walk (open `main.rs` in your head)

1. `setrlimit` memlock so BPF maps can pin pages.  
2. `Ebpf::load(include_bytes_aligned!(OUT_DIR/probes))`.  
3. Attach tracepoints + try OpenSSL uprobes.  
4. `take_map` EVENTS, DROPS, SOCK_META.  
5. Wrap SOCK_META and aggregators in `Arc<Mutex<_>>`.  
6. `ExportHub::spawn` starts the 10s POST loop.  
7. `spawn` drain: `select` readable vs tick.  
8. `decode_event` → `observe` / `observe_client` → `handle_exchange` → parse → registry.record.  
9. Main thread TUI or headless until Ctrl-C.

---

## 18. Live-coding they might ask (you can do these on paper)

**Easy:** implement `looks_like_request` with `starts_with`.  
**Medium:** HashMap pending write then read → Option Exchange.  
**Medium:** `record` latency into first bucket `<= bound`.  
**Hard (don’t volunteer):** lock-free correlator, custom allocator, pin-accurate async.

If they ask you to write Aya boilerplate from scratch in 20 minutes: sketch maps + “userspace drain,” don’t fake macros.

---

## 19. Question bank — Rust (answers in this project)

**Q. What is ownership?**  
A. Each value has one owner. `Correlator` owns the HashMap. `observe` takes `&mut self`. Events are borrowed. Completed `Exchange` is moved to `handle_exchange`.

**Q. Stack vs heap?**  
A. `SockIoEvent` (288 B) can live on the stack after decode. `Vec<u8>` prefix copies to the heap. BPF doesn’t heap-alloc like that.

**Q. Borrow checker?**  
A. Can’t have `&mut correlator` and another `&mut` at once. That’s why one Mutex around it, or we’d split structs.

**Q. Why Mutex not RwLock?**  
A. Drain almost always writes (observe mutates). TUI reads aggs — RwLock could work; we used Mutex for simplicity. Honest: not a measured choice.

**Q. Deadlock?**  
A. Hold one lock at a time in `lock_mut` regions; export clones JSON out of the registry lock before `.await`. Nested locks (meta then cache) in `handle_exchange` — if they ask, “I’d order locks consistently; I should re-read that function before claiming no deadlock.”

**Q. Why `to_vec` instead of storing `[u8;256]` in Pending?**  
A. Save heap for actual `prefix_len`. Pending could have used the array; Vec is the current code.

**Q. `String` vs `&str` in SeriesKey?**  
A. HashMap must own keys. `&str` would need a lifetime tied to the exchange, which is dropped.

**Q. Interior mutability?**  
A. `Mutex` is interior mutability: `&ExportHub` can still `record` via `&self` + inner mutex. `AtomicU64` too.

**Q. `?` in `main`?**  
A. `main` returns `Result`; `?` exits with error. In `parse_exchange` `?` is on `Option` (method missing).

**Q. Two kinds of `?`?**  
A. `Option` and `Result` both implement `Try`. Function return type decides.

**Q. `from_u8` vs `as u8`?**  
A. `as u8` is infallible for our enum. `from_u8` is fallible for **unknown** kernel garbage.

**Q. Why `read_unaligned`?**  
A. RingBuf bytes might not be aligned for `u64` fields. Unaligned read is correct; aligned transmute could crash on some arch. BPF is the producer; still defensive.

**Q. Drop trait?**  
A. We don’t implement Drop. `Vec` frees on scope end. `Ebpf` drop unloads programs — Aya.

**Q. RAII?**  
A. MutexGuard unlocks when dropped — even if you `continue` the loop. That’s why we use blocks `{ let mut r = lock_mut(...); ... }` in the tick arm.

**Q. `clone` of Arc vs T?**  
A. Arc clone = +1 refcount. T clone = deep copy. Interviewers listen for this.

**Q. Tokio vs std::thread?**  
A. Many waits (fd, HTTP, timers). Threads would work; AsyncFd + select is the natural Aya example.

**Q. `block_in_place` / CPU heavy on runtime?**  
A. httparse is small. We didn’t offload to `spawn_blocking`. High RPS might warrant it — not done.

**Q. `unsafe impl Send`?**  
A. We didn’t. Don’t invent.

**Q. Pin / Future?**  
A. Tokio hides it. Don’t lecture Pin unless they ask; then: “self-referential futures, I didn’t write a manual Future.”

**Q. Macros?**  
A. Aya `#[map]`, `#[tracepoint]`, `tokio::main`, `select!`. I didn’t write a proc-macro crate.

**Q. `build.rs`?**  
A. Compile BPF with the right target and put ELF in OUT_DIR. Agent `include_bytes`.

**Q. Feature flags?**  
A. `common` `user` → Pod + aya dep. BPF builds common without it.

**Q. Edition 2024?**  
A. Workspace edition. Don’t claim you chose 2024 for a deep reason unless you did.

**Q. Why `default-features = false` on deps?**  
A. Smaller/controlled builds (Aya, tokio features listed explicitly).

**Q. `core::mem::size_of` tests?**  
A. ABI freeze: 48 B and 288 B. If someone adds a field, CI/test fails.

**Q. `type TlsIoEvent = SockIoEvent`?**  
A. Same layout; kind byte differs. One decode path.

**Q. Generics vs trait objects?**  
A. We use static types. `anyhow::Error` is a trait object under the hood. We didn’t design a plugin trait.

**Q. How do you handle UTF-8?**  
A. HTTP methods are ASCII. Prefixes are bytes. `String` after httparse. Invalid UTF-8 wouldn’t be in method if httparse succeeded.

**Q. `to_ascii_lowercase` in redact?**  
A. Header names case-insensitive. We allocate a lowered copy to search — simple, not zero-copy.

**Q. Could this be written in C?**  
A. Yes. Rust gave us tests, enums, and Aya. BPF constraints are the same.

---

## 20. If they start a generic Rust quiz (not this repo)

Map back or say “I’d use the same tools as in the agent”:

| Generic Q | Map |
| --- | --- |
| Linked list | We used HashMap + Vec, not lists |
| Implement Iterator | `windows` / `split` — we consume iterators, didn’t write `Iterator` impl |
| Lifetime `'a` on a struct | We owned `String`/`Vec` to avoid that |
| Unsafe queue | RingBuf is kernel; userspace is Mutex |
| Async recursive | No |
| GC vs Rust | Agent is long-running; we care about predictable maps, not GC pauses — secondary to Aya |

If you don’t know a generic puzzle: “I haven’t done that puzzle; in this project we avoided it by owning data.” Better than a wrong linked-list.

---

## 21. Mini programs to type until they stick

Do these in the playground or a scratch file **without** looking.

1. Function that takes `&[u8]` and returns `Option<&[u8]>` if it starts with `GET `.  
2. `struct Key { tgid: u32, fd: i32 }` with `Hash, Eq`. HashMap insert.  
3. `fn observe(...) -> Option<i32>` with `?`.  
4. `Arc::new(Mutex::new(0))`, spawn a thread or tokio task, increment.  
5. `match kind { 1 => ..., 3 => ..., _ => None }`.

Then open the real files and find the same shapes.

---

## 22. Honesty card — Rust seniority

**Can claim after studying this file:** workspace layout, why `no_std`, ownership of the FSM, Option skip, Arc/Mutex/async split, where unsafe is, Pod/repr(C), anyhow vs Option.

**Cannot claim:** I designed Aya; I am unsafe-expert; lock-free; I wrote 10k lines of Rust by hand from year one; I can implement a trait object allocator.

**If they ask “rate yourself 1–10 in Rust”:** “For this stack, I can maintain the agent. I’m not a language lawyer. Call it 4–5/10 language, 7/10 *this repo* if I’ve drilled the modules.” Don’t say 9.

---

## 23. Rapid-fire (≤6 words)

1. Crate vs module? — Package vs file.  
2. no_std where? — common and ebpf.  
3. Why no_std common? — Shared with BPF.  
4. user feature? — Pod impls for agent.  
5. Owner of pending map? — Correlator.  
6. observe self? — &mut self.  
7. Event passed how? — Shared reference.  
8. Exchange? — Owned Vecs, moved.  
9. Option decode? — Skip bad records.  
10. Result main? — Fatal attach.  
11. Arc clone? — Refcount, not deep.  
12. Mutex? — Exclusive FSM.  
13. Atomic? — Export counters.  
14. Poison? — into_inner, keep going.  
15. async wait? — RingBuf and HTTP.  
16. Drain awaits OTLP? — No.  
17. unsafe decode? — Length then unaligned.  
18. Pod? — repr C, no pointers.  
19. Tokio select? — Fd or timer.  
20. HashMap key traits? — Eq Hash.  
21. Copy? — SockKey, not Exchange.  
22. String key why? — Map owns it.  
23. ? on method? — Option early None.  
24. build.rs? — Embed BPF ELF.  
25. default-members skip ebpf? — Wrong triple.  
26. no_main? — Kernel calls probes.  
27. Send? — Arc Mutex is Send.  
28. RwLock? — We used Mutex.  
29. httparse crate? — Userspace only.  
30. reqwest rustls? — Collector client, not uprobes.

---

## 24. Files to open the night before a Rust round

1. `Cargo.toml` (workspace) + `agent/Cargo.toml` + `common/Cargo.toml`  
2. `common/src/lib.rs` — `no_std`, `repr(C)`, Pod cfg  
3. `agent/src/correlate.rs` — signatures, Vec prefix, Option  
4. `agent/src/decode.rs` — unsafe block  
5. `agent/src/main.rs` — Arc clone, spawn, select, lock_mut  
6. `agent/src/export.rs` — snapshot inside lock, await outside  
7. `ebpf/src/main.rs` — first 80 lines maps + no_std  

If you can narrate those seven, you can survive a Rust grill on **this** project.
