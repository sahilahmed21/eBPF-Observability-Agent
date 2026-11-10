# Observing LLM calls and agent behaviour from the kernel, with no SDK

This is a write-up of what I added to a small Rust + Aya eBPF agent, what I measured, and what did not work. Every number below is from a script in this repository; the limits are stated next to the numbers.

## The starting point

The agent attaches syscall tracepoints and OpenSSL uprobes, reconstructs HTTP/1.1 and HTTP/2 exchanges in userspace and exports latency histograms, a service map and OTLP data. In August I recorded "~87% of one core" for 500 HTTP plus 200 gRPC requests per second and never explained it. That number, and a pipeline that could only be tested against a live kernel, were the two things I wanted to fix before adding features.

## 1. Make the userspace pipeline replayable

The kernel side filters, samples and gates events in a closed loop, so a log cannot reproduce it. The userspace side (reassemble, correlate, parse) is almost pure, so I made it fully pure: one function, `ingest_pure`, takes an event and the pipeline state and returns what happened, with a virtual clock derived from event timestamps.

`OBSAGENT_RECORD=<path>` writes the events the agent keeps to a log (mode 0600, header values for `authorization`, `cookie`, `set-cookie` and `api-key` overwritten before they hit disk). `obsagent replay <file>` runs the log through the same function without root. A real 50 ms `/slow` capture is checked in as a test fixture: the replay gives 15 exchanges with p50 50.5 ms, identical to what the live agent reported on the same run, and twenty replays are identical. Dropping 0, 10 and 50% of records recovers exactly the exchanges whose request and response records both survive. HTTP/2 header blocks are not redacted in the log; that is a known gap.

## 2. Streamed LLM calls without changing the kernel ABI

Latency that ends at response headers is useless for a streamed completion. The kernel gates body reads with a map (`INFLIGHT`) that userspace clears after each message. For an SSE response the userspace tracker simply does not clear it until the stream ends, so the body keeps flowing to the existing read path. The tracker records time to first byte, total time and token usage (`message_start` input tokens, cumulative `message_delta` output tokens, or `prompt_tokens`/`completion_tokens` for OpenAI-compatible servers). Only the first 256 bytes of each read are visible, so a usage object cut at that boundary yields `None`, never `0`.

I checked it against a mock TLS server whose client records ground-truth timing: 30 calls, worst error 0.14–0.25% on first-byte time and 0.06–0.08% on total time, token counts exact. Finished calls are also exported as OTLP `chat` client spans with `gen_ai.usage.input_tokens` and `gen_ai.usage.output_tokens`; I confirmed all 30 reach a collector over HTTP with the expected attributes. Limits: OpenSSL only, HTTP/1.1 chunked SSE only, and the provider name is guessed from the URL path.

One failure worth reporting: on one run with the agent started on a busy k3s node, the ring buffer dropped events at start-up, sticky sampling engaged, and only 13 of 30 calls were seen. The drop counters showed it. The agent reports loss; it does not hide it.

## 3. A detection-only policy audit

`openat` and `execve` tracepoints feed a small rule set (`deny_open <prefix>`, `deny_exec <basename>`). In five labelled scenarios against harmless canary files all five violations were detected, with zero false positives across five benign scenarios. It only detects; it does not block, and rules are not scoped to a process tree.

The cost is real. On an open-heavy loop (about 1.2 million `open()` calls in one second) the probe adds roughly 1.8 µs per call and the workload ran 51% longer than with the agent alone; the agent itself went from ~0.7% to ~5.7% of one core. That is a worst case, not a typical service, but it is why the probes are opt-in (`OBSAGENT_FILEOPS=1`) and should be scoped with `OBSAGENT_COMM_ALLOW`.

## 4. The 87% that did not reproduce

On plain WSL2 the same 500 + 200 rps load used 14–21% of one core. On the k3s node with the k3s defaults, HTTP alone at 500 rps used about 10.6%. Two things stand out. First, most of that is not the parsing pipeline: replaying recorded events costs about 2,600 instructions per record, around 15% of the per-request cost measured live. Second, in-kernel BPF time (about 16 µs per `write`) is comparable to the agent's own CPU and is charged to the application, so a `perf stat -p <agent>` number understates total cost. I also tried shortening the per-event prefix copy in the kernel; it changed nothing, and I reverted it.

I still cannot explain August. The remaining suspects are gRPC/HTTP/2 decoding and OTLP export under load, neither of which I could drive on this cluster (`frontend-grpc` is crash-looping). The README says "high and environment-dependent", not "under 2%".

## What I would do next

Parse SSE over HTTP/2, add Go and rustls TLS capture or defer to OpenTelemetry's eBPF Instrumentation, link spans with trace propagation, and measure the policy probes with a per-process scope. The repository's CI runs the unit tests (22 common, 190 agent) on every push; the correctness gates need root and a BPF-capable host.
