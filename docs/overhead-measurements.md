# Overhead measurements

Record agent cost under a **fixed** synthetic load at every phase milestone.
Same load script → comparable numbers → honest resume metric.

## Method

1. Start target workload (`demos/http-server` or equivalent).
2. Run `./scripts/loadgen.sh` (fixed RPS / concurrency — pin versions in the script).
3. Measure baseline (no agent) then with agent attached.
4. Capture agent CPU% and RSS via `/proc/<pid>/stat` sampling and/or `perf stat`.
5. Log results below.

## Results

| Date | Phase / commit | Load profile | Baseline CPU (host) | Agent CPU% | Agent RSS | Notes |
|------|----------------|--------------|---------------------|------------|-----------|-------|
| — | — | — | — | — | — | *(fill at Milestone 1)* |

## Target

- Headline: **&lt; 2% CPU** for the agent under the documented load profile on the reference machine.
- If a phase exceeds budget: note the hot probe, consider filtering, sampling, or smaller prefixes before adding features.
