# Claim-lock hygiene (review fixes) TDD

**Source:** Core review blockers (2026-08-24) — fail-closed e2e image/restore + stronger claim checks.  
**Runner:** `bash scripts/test-claim-lock-hygiene.sh` (strip CRLF on Windows/WSL).  
**Commits:** skipped (user rule: commit only when asked).

## User journeys

1. As a reader of README/resume, I must not see affirmative unproven VISION-95 claims.
2. As a claim-lock maintainer, bare `**PASS**` on any row 1–10 is illegal without a qualifier; row 5 PASS requires `docs/handoff/artifacts/e2e-k3s.pass`.
3. As CI/`e2e-k3s`, missing/failed agent image import fails; privileged restore failure fails the script.

## RED → GREEN

| Stage | Result |
|---|---|
| RED | `FAIL: … no RESTORE_FAILED…`; `FAIL: … image missing…`; `FAIL: … k3s ctr import…` |
| GREEN | `PASS: claim-lock hygiene` |

## Guarantees

| # | What is guaranteed | Command | Result |
|---|---|---|---|
| 1 | README/resume reject affirmative unproven VISION-95 claims | `test-claim-lock-hygiene.sh` | PASS |
| 2 | No bare `**PASS**` on claim rows 1–10; row 5 PASS needs artifact | same | PASS |
| 3 | `e2e-k3s` fail-closed image import + `RESTORE_FAILED` restore path | same (static) | PASS |

## Coverage / gaps

- Static script checks only (no live k3s in this gate).
- Skipped full YAML claim-lock schema; add when greppable markdown stops being enough.
- Image digest pinning vs `:latest` still open (rollout proves presence, not bit-identity to HEAD).
