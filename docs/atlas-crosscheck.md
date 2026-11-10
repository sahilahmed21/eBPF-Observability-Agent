# Cross-check against the Atlas gateway's own records

Question: how close are the agent's `gen_ai` spans to an independent record kept by an application that sits on the LLM call path? The independent record is the Atlas gateway (separate repo, Python, `httpx`), which stores per-step `ttft_ms`, `completion_ms` and token counts in SQLite.

Everything below is from `scripts/atlas-crosscheck.sh` and `scripts/atlas-join.py`. The "provider" is `testdata/mock-llm-server.py`, a deterministic stand-in with a known schedule, not a real LLM API. Numbers are single runs on WSL2 with the k3s node running alongside.

## Feasibility (what was checked)

| Question | Answer |
|---|---|
| Is the provider call HTTP/1.1? | Yes. `httpx` defaults to HTTP/1.1 (the gateway's `OpenAIWorkerClient` never enables HTTP/2). |
| Does the path match the classifier? | Yes. The client posts to `<base_url>/chat/completions`; with a `/v1` base URL that is `/v1/chat/completions`, one of the three paths the tracker accepts. |
| Can the uprobes attach to the libssl Python uses? | Attach: yes on Ubuntu-packaged Python (3.14 here links `/lib/x86_64-linux-gnu/libssl.so.3`; a `uv`-managed Python would link OpenSSL statically and could not be hooked). **Capture: no, for the gateway's TLS client.** See below. |
| What does Atlas's default config use? | Cleartext: its worker `base_url`s are `http://127.0.0.1:800x/v1` (local vLLM). The join below uses a cleartext upstream, which the agent captures through the syscall probes. |

**TLS upstream is not captured for this client.** With the upstream on TLS, the agent recorded 12 `TlsIo` events for 6 calls, all of them from the *mock server's* process (reading the request, writing the response; the agent ignores TLS server-side pairing on purpose). The gateway's own TLS client I/O produced no usable event; the only trace is `tls_unmapped=1`. The likely cause is that asyncio TLS uses a memory BIO, so the SSL object has no socket fd to attribute plaintext to. I did not isolate that further. Making it work would need a way to attribute memory-BIO SSL objects to a connection (for example by matching the SSL call to the nearby socket syscall on the same thread), which is a new capture feature and was out of scope.

## How to run the gateway in WSL2

```bash
# once
sudo apt-get install -y python3-venv python3-pip
python3 -m venv ~/atlas-venv && ~/atlas-venv/bin/pip install fastapi "uvicorn[standard]" httpx pyyaml \
  prometheus-client opentelemetry-api opentelemetry-sdk opentelemetry-exporter-otlp-proto-http python-multipart
./scripts/wsl-run.sh build                     # release obsagent

# each run (root, because of BPF); steps and first-token delay are optional
sudo ATLAS=/mnt/c/projects/Atlas ./scripts/atlas-crosscheck.sh 30 0
```

Atlas pins Python below 3.14 in `pyproject.toml`; this ran on the distro's 3.14 with unpinned dependency versions, and the gateway started and recorded all steps. The script generates a tenant with `record_allowed: true`, a worker pointing at the mock, starts the agent (`OBSAGENT_SAMPLE_N=1`, OTLP to a local sink), the gateway (`uvicorn app:create_app_from_env --factory`, `ATLAS_RECORDER_DB=...`) and a client that sends sequential streaming steps with `x-atlas-mode: record`, then joins spans to rows. `UP_TLS=1` switches the upstream to TLS (not captured, see above).

## Results (30 streamed steps, cleartext upstream, `drops=0`, `sample_n=1`)

Join: one-to-one on end time within 0.4 s; where several agent spans are in the window the longest wins.

| | Result |
|---|---|
| Atlas rows joined to an agent span | 30 of 30 |
| Total time, agent minus Atlas `completion_ms` | median -71 ms, p90 -93 ms, max -126 ms |
| Headers (agent TTFB) minus Atlas TTFT | median -36 ms, p90 -48 ms |
| End-time skew, agent end minus Atlas commit time | median -36 ms, p90 -46 ms, max -49 ms |
| Input tokens | 25 exact, 0 wrong, 5 absent |
| Output tokens | 25 exact, 0 wrong, 5 absent |

**Who is right about time?** The mock's schedule is known: headers at 300 ms, stream end at about 503 ms. The agent reports 300.5-300.7 ms and 502.7-504 ms. Atlas reports about 336 ms and 573 ms. So the gap is about +36 ms (TTFT) and +71 ms (completion) of in-process time on the Atlas side that the wire does not show; I did not instrument Atlas to say which part of that is connection handling, event-loop scheduling or parsing.

**TTFB is not TTFT.** The agent's TTFB ends at response headers; Atlas's TTFT ends at the first content chunk. With the mock holding the first token 200 ms after the headers (12 steps, `OAI_FIRST_TOKEN_MS=200`), the difference moved to a median of -239 ms: about -200 ms of definition plus the same -36 ms. Servers that send headers only with the first token (as the mock does by default) hide this.

## Misses and their causes

| Observation | Count (30-step run) | Cause |
|---|---|---|
| Atlas row with no agent span | 0 | n/a. In an earlier clean run one call looked missed; it was in fact observed (502.7 ms) but reported `complete=false`, see next row. |
| Joined span without a stop marker (`complete=false`, no tokens) | 1 | The stop marker (`[DONE]`) was not seen. The agent only sees the first 256 B of each read, and when the last events arrive coalesced in one read the marker can lie past byte 256. Also happens when a read event is lost to ring drops. |
| Joined span with no token counts | 4 of 29 complete spans | The usage chunk arrived coalesced with the preceding chunk in one read (a 350 B read in the recording), so `usage` lay past the 256 B prefix. The agent reports `None`, not 0. |
| Agent spans with no Atlas row | 30 | The upstream server's own view of the same call. On one host both ends of the connection are observed (cleartext pairing accepts client and server order); the server-side view ends at about 300 ms because its next write ends the stream. Not an error; it would not exist across hosts. |

With ring drops the picture degrades: in one 12-step run with `drops=2247` the `[DONE]` read of one call was absent from the recording, so that call was only closed by the client's next request; in a 12-step run with `drops=3635` all 12 rows still joined but 3 had no tokens. Per-call counts are a lower bound whenever `drops` is non-zero (see README, Limitations).

## Two defects this cross-check found in the agent

1. **Span wall-clock position was anchored at report time.** A call reported late (a lost stop marker is only noticed at the client's next request) was placed up to a step later than it happened, which made the first join pair neighbouring calls and give input-token counts off by one or two. `build_llm_span` now positions the span from the call's own monotonic timestamps (`start_ts_ns + total_ns`); covered by `llm_span_is_placed_by_event_time_not_report_time`.
2. **Sticky sampling is per connection.** The gateway reuses one upstream connection for every call, so an auto-raised `sample_n` can drop all of its calls at once. The script pins `OBSAGENT_SAMPLE_N=1` to rule that out; the effect itself was not measured separately.

## Not done

- No real provider: TLS to a hosted API would hit the memory-BIO gap above for this client.
- Non-streaming Atlas steps are not comparable (the tracker only follows SSE).
- One 30-step run for the headline table; the 200 ms first-token run has 12 steps. No repeat for variance.
