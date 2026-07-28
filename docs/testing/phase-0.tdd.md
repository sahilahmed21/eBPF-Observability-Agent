# Phase 0 — TDD evidence report

Milestone 0 is **achieved**. This records what was actually run, on what, and what the passing
gates do and do not guarantee.

- **Source plan:** [phase-0-implementation-plan.md](../phases/phase-0-implementation-plan.md)
- **Executed:** 2026-07-28
- **Target:** WSL2 Ubuntu, kernel `6.6.114.1-microsoft-standard-WSL2`, 16 vCPU / 7 GB

## Environment discovery (the plan's biggest assumption was wrong, in our favour)

The plan assumed a VM was needed because WSL2 usually lacks BTF (assumption A2). It was worth
60 seconds to check rather than accept:

```
PASS  kernel 6.6.114.1-microsoft-standard-WSL2 (>= 5.15)
PASS  BTF at /sys/kernel/btf/vmlinux
PASS  perf-based kprobe attach available
PASS  kernel lockdown not enforced
```

Microsoft's current WSL2 kernel ships `CONFIG_DEBUG_INFO_BTF=y` and the kprobe perf PMU, so the
entire "provision a VM" step (plan Step 1, 45 min) was skipped. The plan's warning is still correct
as general advice — older WSL2 kernels do lack BTF — but on this machine it does not apply.

## Toolchain baseline

Record this. It is the diff you want the first time "it worked last week".

| Component | Version |
|---|---|
| kernel | `6.6.114.1-microsoft-standard-WSL2` |
| rustc stable | `1.97.1 (8bab26f4f 2026-07-14)` |
| rustc nightly | `1.99.0-nightly (09ee43b2d 2026-07-27)` |
| cargo | `1.97.1 (c980f4866 2026-06-30)` |
| bpf-linker | `0.10.4` |
| bpftool | `v7.7.0` / libbpf `v1.7` |
| gcc | `15.2.0` |
| aya / aya-ebpf / aya-log / aya-log-ebpf / aya-build | `0.14.0` / `0.2.1` / `0.3.0` / `0.2.0` / `0.2.0` |

## User journeys

Phase 0 has no user-facing behaviour. Its journeys are operator journeys, taken from the plan:

1. As an operator, I want to know before I write code whether this host can run eBPF at all, so that
   I do not debug my program when the problem is the kernel.
2. As a developer, I want one command that compiles Rust into a loadable BPF object and embeds it in
   the agent, so that the build is not a ritual.
3. As an operator, I want proof the agent loads, reports, and **detaches cleanly leaving nothing
   behind**, so that running it on a node is reversible.

## Task report

### 1. Environment gate — `scripts/preflight.sh`

RED (before toolchain install):

```
FAIL  bpf-linker missing
FAIL  bpftool missing
FAIL  cargo missing
FAIL  rustup missing
FAIL  stable toolchain missing
FAIL  nightly toolchain missing
FAIL  nightly rust-src missing: rustup component add rust-src --toolchain nightly
preflight FAILED          # exit 1
```

GREEN (after install): all 13 checks `PASS`, `preflight OK`, exit 0. Two `WARN` lines remain by
design — `perf_event_paranoid=2` and `unprivileged_bpf_disabled=2` are informational and explain why
loading needs privileges.

**Guarantees:** kernel floor, BTF presence, kprobe attach surface, lockdown state, and every
required tool are verified mechanically, not by memory.

### 2. Workspace scaffold — `agent/`, `ebpf/`, `common/`

Deviation from plan Step 4: `cargo-generate` was **not** used. The template's eleven files were
written directly, with dependency versions taken verbatim from the current `aya-template` manifest.
Rationale: installing `cargo-generate` is a multi-minute compile whose only product is those eleven
small files, and the build is the test — a transcription slip fails immediately and loudly.

Locked decisions realised in code: one BPF ELF object with bin target `probes` (L2), embedded at
compile time via `OUT_DIR/probes` (L3), `default-members` excluding the BPF crate so plain
`cargo build` works on the host, and `obsagent-common` depended on by both sides so the shared ABI
crate is proven to compile for host **and** `bpfel-unknown-none`.

### 3. Build — `cargo build --release`

Exit 0. Chain exercised: stable host build -> `agent/build.rs` -> `aya-build` shells out
`rustup run nightly cargo build -p obsagent-ebpf --target bpfel-unknown-none -Z build-std=core`
-> `bpf-linker` -> artifact copied to `OUT_DIR/probes` -> embedded via `include_bytes_aligned!`.

**One warning was investigated rather than ignored:**

```
warning: linker stderr: unable to open LLVM shared lib
  /home/sahil/.rustup/toolchains/nightly-.../lib/libLLVM-22-rust-1.99.0-nightly.so: dlopen failed
```

`bpf-linker` tried to `dlopen` the nightly's LLVM and fell back to its statically linked copy. The
output object was inspected to confirm the fallback produced a complete object:

```
probes: ELF 64-bit LSB relocatable, eBPF, version 1 (SYSV), with debug_info, not stripped
  [ 3] kprobe            PROGBITS
  [ 5] license           PROGBITS
  [15] .BTF              PROGBITS
  [17] .BTF.ext          PROGBITS
  Machine: Linux BPF
```

`.BTF` and `.BTF.ext` are present, which is what `aya-log` and (from Phase 1) CO-RE relocations
need. **Conclusion: the warning is cosmetic here.** If a future aya/CO-RE feature misbehaves, this
warning is the first suspect and the fix is reinstalling bpf-linker against the nightly LLVM.

### 4. Milestone 0 — `scripts/smoke-milestone0.sh`

RED (before the workspace existed):

```
FAIL: /tmp/obsagent-target/release/obsagent not built - run: cargo build --release   # exit 1
```

GREEN:

```
PASS: loaded
PASS: aya-log output visible
PASS: clean unload
PASS: no leaked pins          # exit 0
```

Human-visible run, for the record:

```
Waiting for Ctrl-C...
[INFO  probes] kprobe called
[INFO  probes] kprobe called
[INFO  probes] kprobe called
```

**Guarantees:** the program is present in `bpftool prog list` while running; `aya-log` reaches
userspace; after SIGINT the program is gone from the kernel and `/sys/fs/bpf` holds no pins. The
SIGINT path — not a `kill -9` — is what is tested, because that is the real shutdown path.

## Test specification

| # | What is guaranteed | Test | Type | Result | Evidence |
|---|---|---|---|---|---|
| 1 | Host kernel is >= 5.15 and exposes BTF, or the gate refuses to proceed | `scripts/preflight.sh` | environment | PASS | `preflight OK`, exit 0 |
| 2 | Every required tool (bpf-linker, bpftool, stable, nightly + rust-src) is present | `scripts/preflight.sh` | environment | PASS | 13 PASS lines |
| 3 | Rust compiles to a valid BPF ELF containing BTF and a kprobe program | `cargo build --release` + `readelf -S` | build | PASS | `.BTF`, `.BTF.ext`, `kprobe` sections; `Machine: Linux BPF` |
| 4 | The ABI crate `obsagent-common` compiles for both host and `bpfel-unknown-none` | `cargo build --release` | build | PASS | compiled in both dependency graphs |
| 5 | The probe loads into the kernel and is visible to `bpftool` | `scripts/smoke-milestone0.sh` | integration | PASS | `PASS: loaded` |
| 6 | `aya-log` output reaches userspace | `scripts/smoke-milestone0.sh` | integration | PASS | `PASS: aya-log output visible` |
| 7 | SIGINT unloads the program, leaving nothing in the kernel | `scripts/smoke-milestone0.sh` | integration | PASS | `PASS: clean unload` |
| 8 | No BPF pins are leaked into `/sys/fs/bpf` | `scripts/smoke-milestone0.sh` | integration | PASS | `PASS: no leaked pins` |
| 9 | `try_to_wake_up` exists on this kernel (the attach target) | `grep -w try_to_wake_up /proc/kallsyms` | environment | PASS | 1 match |

Coverage: line/branch coverage is meaningless for Phase 0 — the deliverable is a toolchain path and
~50 lines of glue with no branching logic. The meaningful coverage question is "is every checklist
item in `phase-0.md` mechanically verified", and it is: 9 guarantees over 11 checklist rows, the
other two being the deferred items below.

## Environment gotchas found while executing (not in the original plan)

| Gotcha | Consequence | Handling |
|---|---|---|
| WSL2 `sudo` requires a password | Autonomous `sudo -E cargo run` and any scripted `sudo` blocks on a prompt | Build as the normal user, run privileged steps via `wsl -u root` (WSL grants root without a password). `smoke-milestone0.sh` now detects `id -u = 0` and skips `sudo` |
| WSL2 idles the VM out and systemd wipes `/tmp` on the next boot | `CARGO_TARGET_DIR=/tmp/...` silently lost the whole build between two commands | Use a persistent path: `CARGO_TARGET_DIR=$HOME/.cache/obsagent-target` |
| Source tree lives on `/mnt/c` (9p) | Slow builds, and `target/` churn on a translated filesystem | Source stays on `/mnt/c` (single source of truth with the Windows checkout); only `CARGO_TARGET_DIR` moves to ext4 |
| `bpf-linker` cannot `dlopen` the nightly LLVM | Alarming warning on every build | Verified benign by inspecting the object (see task 3) |
| `rustup` state is per-user | `preflight.sh` run as root reported `rust-src missing` while the build user had it — a misleading FAIL, found by the final re-run | `preflight.sh` now WARNs when run as root. Run it as the user who builds; run only `smoke-milestone0.sh` as root |
| `rustup` auto-installs on demand | Invoking `rustc +nightly` as root silently installed a 1.3 GB nightly into `/root/.rustup` | Left in place rather than deleting root-owned data: remove with `sudo rm -rf /root/.rustup` if you want the space back. Nothing depends on it |

## Deferred, with reasons

| Item | Why deferred | Command when wanted |
|---|---|---|
| `setcap cap_bpf,cap_perfmon` least-privilege run (plan Step 9.2, decision L6) | Optional hardening; changes a host security setting, so it is the operator's call. The mandatory gate passes without it | `sudo setcap cap_bpf,cap_perfmon=ep <binary>` then run without sudo |
| Repoint probe to `tcp_v4_connect` + `sys_enter_connect` (plan Step 8) | De-risks Phase 1, but is not part of the Milestone 0 gate. Attach symbol already confirmed present | Edit `program.attach(...)`, trigger with `curl` |
| `cargo clippy` / `cargo fmt --check` | `clippy` and `rustfmt` components are not installed (minimal rustup profile) | `rustup component add clippy rustfmt` |
| CI workflow (plan Step 9.3) | No CI configured for this repo yet | Crib `.github/workflows/ci.yml` from `aya-template` |
| `aya-tool` | Deferred to Phase 1 by decision L9 — only needed for CO-RE struct bindings | `cargo install --git https://github.com/aya-rs/aya -- aya-tool` |

## How to re-run the whole gate

```bash
# in WSL, from the repo root
export CARGO_TARGET_DIR="$HOME/.cache/obsagent-target"
bash scripts/preflight.sh          # environment
cargo build --release              # build (as the normal user)
# then, as root (wsl -u root), with the same CARGO_TARGET_DIR exported:
bash scripts/smoke-milestone0.sh   # load / log / unload / leak
```

Note: scripts are stored with CRLF from the Windows editor. `.gitattributes` normalizes `*.sh` to LF
on commit; when running them straight off `/mnt/c` without a fresh clone, strip CR first
(`sed 's/\r$//'`), which is what was done for every run recorded above.
