# Phase 0 — Implementation Plan

Companion to [phase-0.md](phase-0.md) (the checklist). This is the *how*: environment topology,
locked decisions, ordered steps with verification commands, failure playbook, and the handoff
contract to Phase 1.

**Phase 0 is a gate, not a feature.** It buys exactly one thing: proof that the
`Rust → bpf-linker → BPF ELF → verifier → kernel → aya-log → userspace` path works end to end on
our target, and that it tears down cleanly. Everything after Phase 0 assumes this path is boring.

Timebox: **one focused day** (~5h work + slack). If it stretches past two days, the problem is the
environment, not the code — see [Stop rules](#9-stop-rules-and-timeboxes).

> **Status: executed 2026-07-28, Milestone 0 achieved.** Results, toolchain baseline, and four
> environment gotchas found on the way: [`docs/testing/phase-0.tdd.md`](../testing/phase-0.tdd.md).
> The headline correction: this machine's **WSL2 kernel (6.6.114.1) already has BTF**, so Step 1
> (provision a VM) was skipped entirely. Sections below are kept as written; the evidence report
> records every deviation.

---

## 1. Scope

### In scope

| # | Deliverable | Why it exists |
|---|---|---|
| D1 | Linux target with BTF, provisioned and reachable from the editor | Nothing else can be verified without it |
| D2 | `scripts/preflight.sh` — machine-checkable environment gate | Same checks are reused as the Phase 4 DaemonSet init check and in CI |
| D3 | Cargo workspace in the repo's target shape: `agent/`, `ebpf/`, `common/` | Layout is an ABI decision; changing it later touches every path |
| D4 | One trivial kprobe that loads, logs via `aya-log`, and unloads | Milestone 0 |
| D5 | `scripts/smoke-milestone0.sh` — asserts load + log + clean unload + no leaked programs | A milestone you verify by eyeballing a terminal is a milestone that silently regresses |
| D6 | Recorded toolchain versions + the first `docs/verifier-rejection-log.md` habit | Phase 1+ debugging depends on knowing what changed |

### Explicitly not in Phase 0 (YAGNI)

RingBuf, drop counters, real event structs in `common/`, correlation, `httparse`, Ratatui, OTLP,
Dockerfile, k8s manifests, `testdata/` servers, CO-RE struct bindings (`aya-tool`), overhead
measurement. Every one of these has a phase that owns it. Adding any of them now buys nothing and
makes the gate ambiguous: when the build breaks you want exactly one suspect.

---

## 2. Assumptions (correct me if any of these is wrong)

| # | Assumption | Blast radius if wrong |
|---|---|---|
| A1 | Dev workstation is Windows; **no BPF code is ever built or run on it** | Changes the editor/sync topology (§4), nothing else |
| A2 | We can create a local VM (Hyper-V/Multipass) *or* spin a cloud VM. 2 vCPU / 4 GB / 20 GB minimum | Without a Linux target Phase 0 cannot start at all |
| A3 | Development kernel is Ubuntu 24.04 (6.8); the *supported floor* stays 5.15 as the README claims | Using a 6.x-only feature in Phase 1–3 would break the floor claim silently |
| A4 | Root (or `CAP_BPF`+`CAP_PERFMON`) is available on the target | No load, no Phase 0 |
| A5 | Crate/package prefix is `obsagent` (`obsagent`, `obsagent-ebpf`, `obsagent-common`) | Cheapest thing here to reverse — a rename before the first commit is a 5-minute `sed` |

---

## 3. Decisions locked in Phase 0

These are cheap now and expensive in Phase 2+. Each is a real fork in the road, so it is recorded
with its reversal cost.

| # | Decision | Choice | Why | Cost to reverse later |
|---|---|---|---|---|
| L1 | Directory ↔ package mapping | `agent/` = `obsagent`, `ebpf/` = `obsagent-ebpf`, `common/` = `obsagent-common` | Repo README already promises these directory names; the Aya template's `myapp-*` naming is a template artifact, not a constraint | Low (rename) |
| L2 | **One BPF ELF object** containing every program | `ebpf/` has exactly one `[[bin]]` (`probes`) | Programs must share maps (the correlation `HashMap` + one `RingBuf`). Separate objects cannot share maps without pinning, and pinning adds lifecycle bugs we do not need | **High** — splitting/merging objects later rewrites the loader and map wiring |
| L3 | Loader embeds the object at compile time | `include_bytes_aligned!(concat!(env!("OUT_DIR"), "/probes"))` | Single self-contained binary; no "where is the .o" problem in a container. `aya-build` runs from `agent/build.rs`, so plain `cargo build` is the only command anyone needs to learn | Low |
| L4 | Kernel→user timestamps are **CLOCK_MONOTONIC ns** (`bpf_ktime_get_ns`) | Events carry raw monotonic ns; userspace captures a boot↔wall offset **once at startup** and converts only at export | Doing wall-clock math in the kernel is expensive and impossible under the verifier; converting per-event in userspace at export time is free. This shapes the Phase 1 event struct | Medium (event ABI change) |
| L5 | `aya-log` is a **debug tool, never a hot-path tool** | `info!` allowed in Phase 0/1 probes; from Phase 2 any per-syscall/per-byte log lives behind a `debug-log` cargo feature, off by default | In-kernel string formatting + a buffer write per event is exactly the kind of cost that eats the <2% CPU budget, and it will not show up until Phase 2 load tests | Low if decided now, annoying later |
| L6 | Least privilege is proven in Phase 0, not discovered in Phase 4 | `sudo -E` for the dev loop (via `.cargo/config.toml` runner), **plus** one verified non-root run with `cap_bpf,cap_perfmon` | If the agent turns out to need full root, that is a deployment-design regression found at the worst possible time | Low now, high in Phase 4 |
| L7 | Nightly is used **only** to build the BPF object | `aya-build` shells out via `rustup run nightly`; the workspace itself stays on stable | Keeps the userspace crate on a stable toolchain (needed for a credible production binary) while satisfying `-Z build-std=core` for the `bpfel-unknown-none` target | Low |
| L8 | Nightly is floating **until it breaks us once** | Start with `Toolchain::default()` (= `nightly`). On the first nightly-induced BPF build break, pin `Toolchain::Custom("nightly-YYYY-MM-DD")` in `agent/build.rs` and note the date in this doc | A preemptive pin means manually chasing nightly forever; an unpinned nightly costs one bad afternoon at most, and the fix is one line | Low (one line) |
| L9 | `bpftool` is a **required** Phase 0 dependency; `aya-tool` is **deferred to Phase 1** | Install `bpftool` now, skip `aya-tool` | Every Phase 0 verification step (is it loaded? did it unload? did it leak?) is a `bpftool` command. `aya-tool` only matters once we read kernel structs, which is Phase 1 | None |

> **Checklist amendments** to `phase-0.md` implied by L9: add `bpftool` to the checklist; mark
> `aya-tool` optional/deferred. Also note the root `README.md` prerequisite `cargo install aya-tool`
> is wrong — `aya-tool` is installed from git:
> `cargo install --git https://github.com/aya-rs/aya -- aya-tool`.

---

## 4. Environment topology

Windows editor, Linux kernel. Three ways to bridge that; only one of them is not a trap.

| Option | Verdict |
|---|---|
| **Repo cloned inside the Linux VM, edited over Remote-SSH** | **Chosen.** Build, load, and test all happen where the kernel is. `target/` lives on native ext4 |
| Repo on Windows, shared/mounted into the VM (`/mnt`, 9p, virtiofs) | Rejected for `target/`; **acceptable for source** if `CARGO_TARGET_DIR` points at a native path. This is what we ended up doing — see the evidence report |
| WSL2 as the target | Only if the 60-second BTF probe passes (below). Do not build a custom WSL2 kernel to get BTF — the phase doc already calls this a week-burner. **On this machine the probe passed**, so WSL2 is the target |

**Source of truth is git.** The Windows checkout stays useful for docs; all code lands from the VM.

### Provisioning (pick one)

```bash
# 60-second WSL2 probe — either it already has BTF or we move on. No kernel building.
uname -r && ls -l /sys/kernel/btf/vmlinux    # missing file => abandon WSL2, use a VM

# Local VM (Windows host, Hyper-V backed)
winget install Canonical.Multipass
multipass launch 24.04 --name ebpf-dev --cpus 2 --memory 4G --disk 20G
multipass shell ebpf-dev

# Cloud VM: any Ubuntu 24.04 LTS image, 2 vCPU / 4 GB. Nothing special required.
```

Then: install `git`, generate an SSH key, connect the editor over Remote-SSH, `git clone` the repo
**inside the VM**, and work there. 8 GB RAM is more comfortable than 4 GB once `bpf-linker` and
rust-analyzer are both resident.

---

## 5. Step-by-step plan

Every step has a verification command and a pass condition. A step is not done because it printed
something; it is done because its check passed.

### Step 1 — Provision the Linux target · 45 min

Do §4. Pass: `ssh` in from the editor, `uname -r` reports 5.15 or newer, `sudo -v` works.

### Step 2 — Write and run the preflight gate · 25 min

Write `scripts/preflight.sh` (contents in [Appendix A](#appendix-a-scriptspreflightsh)) and run it.
This is D2 and it is the single most reused artifact of Phase 0 — the same checks become the
Phase 4 init-container gate and the CI environment assertion.

It checks, in order of "how badly does this ruin the day":

1. Kernel ≥ 5.15 — our floor.
2. `/sys/kernel/btf/vmlinux` — **hard gate**. No BTF, no CO-RE, no `aya-log` relocations, no project.
3. `/sys/bus/event_source/devices/kprobe/type` — modern perf-based kprobe attach. Its absence means
   Aya falls back to legacy `tracefs` `kprobe_events`, which leaks probe entries if we crash.
4. Kernel lockdown state (`/sys/kernel/security/lockdown`) — `confidentiality` blocks BPF outright
   (Secure Boot surprise).
5. `kernel.unprivileged_bpf_disabled` / `kernel.perf_event_paranoid` — informational, but they
   explain `EPERM` later.
6. `bpf-linker`, `bpftool`, stable toolchain, nightly toolchain **with `rust-src`**.

Pass: exit code 0. FAIL on BTF → go back to §4 and change environment. Do not proceed.

### Step 3 — Toolchain · 30 min (mostly compile wait)

```bash
sudo apt update && sudo apt install -y build-essential pkg-config libssl-dev \
    linux-tools-common "linux-tools-$(uname -r)"     # provides bpftool
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install stable
rustup toolchain install nightly --component rust-src   # rust-src is required: -Z build-std=core
cargo install bpf-linker
cargo install cargo-generate
```

Notes that save an hour:

- The template targets **edition 2024**, so stable must be ≥ 1.85. `rustup update` if in doubt.
- `rust-src` on nightly is not optional. Without it the BPF build fails deep inside `build-std`
  with a message that does not mention `rust-src`.
- Ubuntu's `apt` Rust is too old. Use `rustup` only.
- If `cargo install bpf-linker` fails on LLVM, follow the bpf-linker README rather than guessing;
  on Ubuntu the usual fix is installing the matching `llvm-<N>-dev` / `libpolly-<N>-dev`.

Pass: `scripts/preflight.sh` now exits 0 with every line PASS.

### Step 4 — Scaffold the workspace · 30 min

Generate into a scratch directory, then relocate. Generating *into* the repo would fight the
existing `README.md` files and the directory names we committed to.

```bash
cd /tmp
cargo generate --name obsagent -d program_type=kprobe -d kprobe=try_to_wake_up \
    https://github.com/aya-rs/aya-template
```

Why `try_to_wake_up` for the smoke probe: it fires constantly on any live system, so "no log output"
unambiguously means *broken*, never *idle*. A probe on something interesting (like `tcp_v4_connect`)
cannot distinguish "did not load" from "nothing connected" — which is precisely the ambiguity a gate
must not have. We repoint to the interesting one in Step 8, once the toolchain is proven.

Relocate into the repo shape:

| Generated | Repo destination | Package name |
|---|---|---|
| `obsagent/` | `agent/` | `obsagent` (bin `obsagent`) |
| `obsagent-ebpf/` | `ebpf/` | `obsagent-ebpf` (bin **`probes`**) |
| `obsagent-common/` | `common/` | `obsagent-common` |
| `Cargo.toml`, `.cargo/config.toml`, `rustfmt.toml` | repo root | — |

Keep each generated `README.md` where it is (they describe the directories); the template's
`README.md` and license files go in the root or the bin.

Then make exactly these edits — nothing else:

1. Root `Cargo.toml`: `members = ["agent", "common", "ebpf"]`,
   `default-members = ["agent", "common"]`, and the profile override key becomes
   `[profile.release.package.obsagent-ebpf]`.
2. `agent/Cargo.toml`: path deps → `obsagent-common = { path = "../common", features = ["user"] }`
   and `obsagent-ebpf = { path = "../ebpf" }` (the build-dependency).
3. `ebpf/Cargo.toml`: path dep → `obsagent-common = { path = "../common" }`; set
   `[[bin]] name = "probes"`.
4. `agent/build.rs`: the package-name string it searches for → `"obsagent-ebpf"`.
5. `ebpf/src/main.rs`: rename the program fn to `smoke_probe`.
6. `agent/src/main.rs`: `include_bytes_aligned!(concat!(env!("OUT_DIR"), "/probes"))` and
   `ebpf.program_mut("smoke_probe")`.

**Do not copy dependency versions out of this document.** Whatever the template generates at scaffold
time is authoritative; record the versions in Step 9 so we know what "working" was. At time of
writing the template pins `aya 0.14`, `aya-ebpf 0.2.1`, `aya-log 0.3`, `aya-log-ebpf 0.2`,
`aya-build 0.2`, edition 2024.

Three naming rules that cause real, confusing failures:

- The **`[[bin]]` target name** — not the package name — is what `aya-build` copies into `OUT_DIR`.
  `ebpf/`'s bin is `probes`, so the loader path is `OUT_DIR/probes`. Mismatch = compile-time
  "file not found" inside a macro.
- `program_mut("...")` takes the **Rust function name** of the probe, which becomes the program name
  in the ELF. Mismatch = `unwrap()` panic at load.
- The kernel truncates program names to **15 characters**. Keep probe fn names short or `bpftool`
  output will not match what you grep for.

Pass: `git status` shows the three crates in the right directories and nothing under `/tmp` is still
referenced (`grep -rn "obsagent-ebpf\|\.\./" */Cargo.toml`).

### Step 5 — First build · 20 min

```bash
cargo build --release          # from the repo root
```

What actually happens, because knowing this makes build failures readable:

`cargo` builds `agent` for the host → `agent/build.rs` runs → `aya_build::build_ebpf` shells out
`rustup run nightly cargo build -p obsagent-ebpf --bins --release --target bpfel-unknown-none
-Z build-std=core` with `RUSTFLAGS` carrying `--cfg bpf_target_arch="x86_64"`, `-Cdebuginfo=2` and
`-Clink-arg=--btf` → `bpf-linker` emits the BPF ELF **with BTF** (aya-log and CO-RE both need it) →
the artifact is copied to `OUT_DIR/probes` → `include_bytes_aligned!` embeds it in the agent binary.

Two workspace gotchas that follow from `default-members`:

- **Never** run `cargo build/clippy/test --workspace` on the host: `--workspace` overrides
  `default-members` and tries to compile the `no_std`/`no_main` BPF crate for the host. Plain
  `cargo build` and `cargo clippy` are correct.
- To lint the BPF crate specifically:
  `cargo +nightly clippy -p obsagent-ebpf --target bpfel-unknown-none -Zbuild-std=core`.
- `AYA_BUILD_SKIP=1` makes `build.rs` skip the BPF build — useful for a docs/lint-only CI job.

Pass: `target/release/obsagent` exists; `file` reports a Linux ELF.

### Step 6 — Milestone 0: load → log → unload · 15 min

```bash
RUST_LOG=info cargo run --release
# expect: "Waiting for Ctrl-C..." then a stream of "[INFO] ... kprobe called"
# Ctrl-C to exit
```

- `.cargo/config.toml` sets `runner = "sudo -E"`, so **compilation runs as you and only the binary
  runs as root**. Never run `sudo cargo run`: it root-owns `target/` and every later build fails in
  a way that looks like a cargo bug.
- `RUST_LOG=info` is mandatory — `env_logger` defaults to `error` and you will conclude the probe
  is not firing when it is.
- `-E` is what carries `RUST_LOG` across `sudo`.
- If it loads but prints nothing: the logger task must be polled. The template spawns an `AsyncFd`
  task that flushes the logger's single fd; if that task was dropped during editing, logs vanish
  silently.

Pass: `kprobe called` appears within a couple of seconds.

### Step 7 — Prove clean unload and no leaks · 20 min

The checklist item "clean unload works" is the one people fake. Aya detaches on `Drop` (link close),
and the kernel frees the program when its refcount hits zero — so the real question is whether our
exit path actually drops the `Ebpf` handle.

```bash
sudo bpftool prog list | grep smoke_probe     # while running: exactly one entry
sudo bpftool map  list                        # note the AYA_LOGS map
# after Ctrl-C:
sudo bpftool prog list | grep smoke_probe     # expect: nothing
ls /sys/fs/bpf                                # expect: empty — we pin nothing in Phase 0
sudo cat /sys/kernel/debug/tracing/kprobe_events   # expect: empty (legacy-attach leak check)
```

Then encode all of it in `scripts/smoke-milestone0.sh` ([Appendix B](#appendix-b-scriptssmoke-milestone0sh)),
which asserts: not-already-loaded → loaded → logged → exited → not-loaded. Run it once and commit it.
From here on, "Milestone 0 still works" is one command, which matters every time we bump aya or the
kernel.

Pass: `./scripts/smoke-milestone0.sh` prints four PASS lines and exits 0.

**Runbook — force cleanup** if something ever survives: `sudo pkill -INT obsagent`, then
`sudo bpftool link detach <id>` / `sudo rm /sys/fs/bpf/<pin>` for stragglers, and clear
`kprobe_events` if the legacy path was used. Worst case, a reboot clears all BPF state — never
persist anything in Phase 0 that a reboot would not clear.

### Step 8 — De-risk Phase 1's attach point · 30 min (strongly recommended)

Still Phase 0 cost, but it answers the first three questions Phase 1 would otherwise ask.

1. Repoint the probe: `program.attach("tcp_v4_connect", 0)` and rename the fn to `connect_probe`.
2. Confirm the symbol exists on this kernel first: `sudo grep -w tcp_v4_connect /proc/kallsyms`.
3. Trigger it: `curl -s http://example.com >/dev/null` → expect exactly one log line per connect.
4. Also attach the tracepoint variant (`syscalls:sys_enter_connect`) and compare. The architecture
   doc prefers **tracepoints** over kprobes on syscall internals for ABI stability
   ([OVERVIEW.md](../architecture/OVERVIEW.md)); proving both attach paths work now means Phase 1 is
   a data-modelling exercise, not a toolchain exercise.

Pass: one log line per `curl`. Any verifier rejection encountered on the way → one entry in
[`docs/verifier-rejection-log.md`](../verifier-rejection-log.md). Start that habit here, while the
programs are trivial and the rejections are easy to understand.

### Step 9 — Record and harden · 30 min (optional, do it before Phase 1)

1. **Record the working toolchain** in this doc or `notes/`: `rustc -V`,
   `rustc +nightly -V`, `bpf-linker --version`, `uname -r`, and the aya versions from `Cargo.lock`.
   This is the diff you will want the first time "it worked last week".
2. **Least privilege (L6)** — prove the production path once:
   ```bash
   sudo setcap cap_bpf,cap_perfmon=ep target/release/obsagent
   RUST_LOG=info ./target/release/obsagent      # no sudo
   ```
   Pass: loads and logs as a normal user. This is the capability set the Phase 4 DaemonSet requests;
   finding out here that it is insufficient is cheap, finding out in Phase 4 is not.
3. **CI, two tiers** (crib `.github/workflows/ci.yml` from the template):
   - *Tier 1, guaranteed:* `ubuntu-24.04`, install nightly + `rust-src` + `bpf-linker`, then
     `cargo fmt --check`, `cargo clippy`, `cargo build --release`. This alone catches "BPF object no
     longer compiles", which is most of what breaks.
   - *Tier 2, best effort:* run `scripts/preflight.sh` and `scripts/smoke-milestone0.sh` on the
     runner. GitHub's Ubuntu runners are VMs with `sudo`, so this usually works; if the runner lacks
     BTF, do not fight it — leave Tier 2 to a `vmtest`/QEMU job in Phase 4, where a 5.15 kernel needs
     testing anyway.

---

## 6. Definition of Done

Phase 0 is complete when every row passes on the Linux target, from a clean `git clone`.

| Checklist item (phase-0.md) | Verification | Pass |
|---|---|---|
| Linux 5.15+ target | `uname -r` | ≥ 5.15 |
| BTF present | `ls /sys/kernel/btf/vmlinux` | exists |
| stable + nightly (`rust-src`) | `rustup component list --toolchain nightly \| grep rust-src` | installed |
| `bpf-linker` present | `bpf-linker --version` | prints a version |
| `bpftool` present (**added**, L9) | `bpftool version` | prints a version |
| `aya-tool` (**deferred to Phase 1**, L9) | — | n/a |
| Workspace scaffolded into `ebpf/`+`agent/`+`common/` | `cargo build --release` from a clean clone | succeeds |
| Template kprobe loads | `bpftool prog list \| grep smoke_probe` | one entry |
| `aya-log` output visible | `RUST_LOG=info` run | `kprobe called` |
| Clean unload | `bpftool prog list` after exit | no entry, no pins |
| **Gate is automated** (D5) | `./scripts/smoke-milestone0.sh` | exit 0 |

---

## 7. Failure playbook

| Symptom | Likely cause | Fix |
|---|---|---|
| `/sys/kernel/btf/vmlinux` missing | Kernel built without `CONFIG_DEBUG_INFO_BTF` (typical WSL2) | Change environment (§4). Do **not** build a kernel |
| `build-std` / `core` errors during BPF build | nightly missing `rust-src` | `rustup component add rust-src --toolchain nightly` |
| `error: edition 2024 is unstable` | stable < 1.85 | `rustup update stable` |
| `bpf-linker: not found` | not installed, or installed for a different user than the one building | `cargo install bpf-linker`; check `$PATH` under `sudo` |
| BPF build works, host build fails on the `ebpf` crate | someone ran `--workspace` | Use plain `cargo build`; lint the BPF crate with `-p obsagent-ebpf --target bpfel-unknown-none` |
| Load fails `EPERM` / `Operation not permitted` | missing caps, `perf_event_paranoid`, or lockdown | Run under `sudo -E`; check the preflight sysctl/lockdown lines |
| Load fails with a verifier dump | Actual verifier rejection | Read the dump bottom-up; log it in `verifier-rejection-log.md` |
| `unwrap()` panic at `program_mut(...)` | name ≠ Rust probe fn name (or > 15 chars) | Match the fn name exactly |
| Compile error: no such file in `include_bytes_aligned!` | `OUT_DIR` path ≠ the `[[bin]]` name of the BPF crate | Align both on `probes` |
| Loads but zero log output | `RUST_LOG` unset, `sudo` without `-E`, or the logger flush task not running | `RUST_LOG=info` + `sudo -E`; confirm the `AsyncFd` task still exists |
| Builds break right after a `rustup update` | nightly drift (L8) | Pin `Toolchain::Custom("nightly-YYYY-MM-DD")` in `agent/build.rs` |
| Everything breaks after one `sudo cargo run` | `target/` is root-owned | `sudo chown -R "$USER" target` and never again |
| `warning: linker stderr: unable to open LLVM shared lib ... dlopen failed` | `bpf-linker` prefers the toolchain's shared LLVM and fell back to its static copy | Benign if the object is complete — confirm with `readelf -S <obj> \| grep BTF`. Only reinstall bpf-linker if `.BTF` is absent |
| Whole build disappears between two commands (WSL2) | WSL idled the VM out and systemd wiped `/tmp`; `CARGO_TARGET_DIR` was under `/tmp` | Point `CARGO_TARGET_DIR` at a persistent path such as `$HOME/.cache/obsagent-target` |
| Scripted `sudo` hangs or fails (WSL2) | WSL `sudo` wants a password | Run privileged steps via `wsl -u root` (no password needed); `smoke-milestone0.sh` skips `sudo` when already root |

---

## 8. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| BTF/environment rabbit hole | High | 60-minute stop rule → switch environment (§4, §9) |
| Nightly/aya version drift breaking the BPF build | Medium | Record versions (Step 9.1); pin nightly on first break (L8) |
| Dev kernel 6.8 lets us use features absent from the 5.15 floor | Medium | Floor documented (A3); Phase 4 adds a 5.15 CI job before any deployment claim |
| Phase 0 quietly grows into Phase 1 | Medium | The "not in Phase 0" list in §1 is the contract |
| Milestone 0 verified by eyeball, silently regresses | High without D5 | `smoke-milestone0.sh`, run in CI Tier 2 |

---

## 9. Stop rules and timeboxes

| Step | Budget | Stop rule |
|---|---|---|
| 1–2 Environment + preflight | 70 min | BTF still missing after 60 min → change environment, do not debug the kernel |
| 3 Toolchain | 30 min | `bpf-linker` won't install after 30 min → follow its README exactly; do not hand-roll LLVM |
| 4 Scaffold + relocate | 30 min | Fighting the relocation → keep the template's `myapp-*` layout, ship Milestone 0, rename in a follow-up commit |
| 5–7 Build, load, verify | 55 min | Verifier rejection on the *template* probe means the toolchain is wrong, not the code — re-run preflight |
| 8–9 De-risk + harden | 60 min | Skippable. Never skip Step 7 |

---

## 10. Handoff to Phase 1

Phase 0 leaves behind, in order of value: a workspace where `cargo build` is the only build command;
`preflight.sh` (reused as the Phase 4 init check); `smoke-milestone0.sh` (reused as the CI gate); a
proven attach path for `connect` (Step 8); the recorded toolchain baseline; and the
verifier-log habit.

Phase 1 then starts on its actual problem — event ABI in `common/`, the enter/exit `HashMap`, the
RingBuf + drop counter, and the Tokio consumer — with zero open questions about the toolchain. The
two Phase 0 decisions Phase 1 immediately consumes: **L2** (one ELF object, so the enter/exit
programs share a map) and **L4** (monotonic ns in the event struct, converted only at export).

---

Both scripts are committed under `scripts/`. The exec bit does not survive a Windows checkout, so on
the target: `chmod +x scripts/*.sh`. (`.gitattributes` pins `*.sh` to LF so the VM never sees CRLF.)

## Appendix A: `scripts/preflight.sh`

Environment gate. Reused verbatim as the Phase 4 init-container check and CI assertion.

```bash
#!/usr/bin/env bash
# Phase 0 environment gate. Exits non-zero if anything required is missing.
set -uo pipefail

fail=0
pass() { printf 'PASS  %s\n' "$1"; }
warn() { printf 'WARN  %s\n' "$1"; }
bad()  { printf 'FAIL  %s\n' "$1"; fail=1; }

# --- kernel ---
kver=$(uname -r)
if [ "$(printf '%s\n5.15\n' "${kver%%-*}" | sort -V | head -1)" = "5.15" ]; then
  pass "kernel $kver (>= 5.15)"
else
  bad "kernel $kver is below the 5.15 floor"
fi

# --- hard gate: BTF ---
if [ -r /sys/kernel/btf/vmlinux ]; then
  pass "BTF at /sys/kernel/btf/vmlinux"
else
  bad "no /sys/kernel/btf/vmlinux - CO-RE and aya-log cannot work. Change environment."
fi

# --- attach surface ---
if [ -r /sys/bus/event_source/devices/kprobe/type ]; then
  pass "perf-based kprobe attach available"
else
  warn "no perf kprobe PMU; aya will fall back to legacy tracefs kprobe_events (leaks on crash)"
fi

# --- things that turn into EPERM later ---
lockdown=$(cat /sys/kernel/security/lockdown 2>/dev/null || echo "none")
case "$lockdown" in
  *"[confidentiality]"*) bad "kernel lockdown=confidentiality blocks BPF" ;;
  none) pass "kernel lockdown not enforced" ;;
  *)    warn "kernel lockdown: $lockdown" ;;
esac
warn "perf_event_paranoid=$(sysctl -n kernel.perf_event_paranoid 2>/dev/null || echo '?')"
warn "unprivileged_bpf_disabled=$(sysctl -n kernel.unprivileged_bpf_disabled 2>/dev/null || echo '?')"

# --- toolchain ---
for tool in bpf-linker bpftool cargo rustup; do
  if command -v "$tool" >/dev/null 2>&1; then pass "$tool on PATH"; else bad "$tool missing"; fi
done
rustup toolchain list 2>/dev/null | grep -q '^stable'  && pass "stable toolchain"  || bad "stable toolchain missing"
rustup toolchain list 2>/dev/null | grep -q '^nightly' && pass "nightly toolchain" || bad "nightly toolchain missing"
if rustup component list --toolchain nightly 2>/dev/null | grep -q 'rust-src.*installed'; then
  pass "nightly rust-src (required by -Z build-std=core)"
else
  bad "nightly rust-src missing: rustup component add rust-src --toolchain nightly"
fi

[ "$fail" -eq 0 ] && echo "preflight OK" || echo "preflight FAILED"
exit "$fail"
```

## Appendix B: `scripts/smoke-milestone0.sh`

The Milestone 0 gate: load → log → clean unload → no leak. Four assertions, no framework.

```bash
#!/usr/bin/env bash
# Milestone 0 gate: the kprobe loads, logs, and unloads without leaking.
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-target}/release/obsagent"
PROG=smoke_probe          # eBPF program name == the Rust probe fn name (kernel truncates at 15 chars)
LOG=$(mktemp)

# Loading BPF needs root. `env` is a no-op prefix for when we are already root.
if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi
loaded() { "${PRIV[@]}" bpftool prog list | grep -c " name ${PROG} " || true; }

# This script verifies; it does not build. Keeps `cargo` off the privileged path.
[ -x "$BIN" ] || { echo "FAIL: $BIN not built - run: cargo build --release"; exit 1; }

[ "$(loaded)" -eq 0 ] || { echo "FAIL: ${PROG} already loaded (leak from an earlier run)"; exit 1; }

# try_to_wake_up fires constantly, so 8s is generous. SIGINT exercises the real shutdown path.
RUST_LOG=info timeout --signal=INT 8 "${PRIV[@]}" "$BIN" >"$LOG" 2>&1 &
runner=$!

sleep 3
[ "$(loaded)" -eq 1 ] || { echo "FAIL: program not loaded"; cat "$LOG"; exit 1; }
echo "PASS: loaded"

wait "$runner" || true

grep -q "kprobe called" "$LOG" || { echo "FAIL: no aya-log output"; cat "$LOG"; exit 1; }
echo "PASS: aya-log output visible"

[ "$(loaded)" -eq 0 ] || { echo "FAIL: still loaded after exit"; exit 1; }
echo "PASS: clean unload"

[ -z "$(ls -A /sys/fs/bpf 2>/dev/null)" ] || { echo "FAIL: leftover pins in /sys/fs/bpf"; exit 1; }
echo "PASS: no leaked pins"
```
