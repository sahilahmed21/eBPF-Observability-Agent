# Interview pack (this repo)

Four files: eBPF grill, **Rust grill (from zero, this repo)**, resume/story, spoken Q&A. **Expanded for rehearsal** — do not try to memorize every line. Memorize the scripts; use the grill as a lookup.

| File | Use | How to study |
| --- | --- | --- |
| [CONCEPTS_GRILL.md](CONCEPTS_GRILL.md) | Architecture, physics, **this** code path, traps, glossary | Whiteboard §0–§1 first; drill one concept per sitting |
| [RUST_GRILL.md](RUST_GRILL.md) | Rust from zero **as this repo uses it** (ownership, no_std, Arc/Mutex, unsafe, Tokio) | Five sittings in that file; then open the seven listed sources |
| [RESUME_AND_STORY.md](RESUME_AND_STORY.md) | One-liners, bullets by JD, STAR, never-claim, behavioral | Paste **one** bullet set; never mix Atlas numbers |
| [SCRIPT_AND_QNA.md](SCRIPT_AND_QNA.md) | 30s / 2–3 min / rounds / Q&A | Speak §1–§2 out loud; Q&A is open-book until the week of |

Do not mix numbers with other projects (e.g. Atlas). Build BPF only in WSL2.

**Do not memorize every line.** Speak SCRIPT §1–§2. Use CONCEPTS as lookup (ABI, FSM cases, OTLP). Use **RUST_GRILL** if they switch to language questions — you cannot survive a Rust round on eBPF physics alone. Use RESUME for paste + never-claim.

If Rust is new: RUST_GRILL sittings 1–5 **before** claiming a Rust JD. The spoken line is still “three crates / no_std ABI / Tokio drain,” not “I don’t know Rust.”

**Evidence spine (cite these, not memory):** [../testing/phase-5.tdd.md](../testing/phase-5.tdd.md) · [../handoff/SESSION-2026-08-15-kind-e2e.md](../handoff/SESSION-2026-08-15-kind-e2e.md) · [../architecture/correlation.md](../architecture/correlation.md)
