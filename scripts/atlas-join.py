#!/usr/bin/env python3
"""Join obsagent gen_ai spans to Atlas gateway step rows and report the error distribution.

usage: atlas-join.py <atlas.sqlite> <otlp-sink.log> [--window-s 1.0]

Atlas side: steps(ts, completion_ms, ttft_ms, prompt_tokens, completion_tokens, run_id, step), ts = commit time.
Agent side: OTLP JSON lines "<path> <body>", spans named "chat" with gen_ai.usage.* attributes.
Join: one-to-one, nearest end time (agent span end vs Atlas ts) within the window; the two clocks are the
same host's wall clock. Every unmatched row/span is reported.
"""

from __future__ import annotations

import json
import sqlite3
import statistics
import sys


def agent_spans(path: str) -> list[dict]:
    out = []
    for line in open(path, "rb"):
        p, _, body = line.partition(b" ")
        if p != b"/v1/traces":
            continue
        for rs in json.loads(body)["resourceSpans"]:
            for ss in rs["scopeSpans"]:
                for s in ss["spans"]:
                    if s["name"] != "chat":
                        continue
                    a = {x["key"]: next(iter(x["value"].values())) for x in s["attributes"]}
                    out.append(
                        {
                            "end": int(s["endTimeUnixNano"]) / 1e9,
                            "total_ms": (int(s["endTimeUnixNano"]) - int(s["startTimeUnixNano"])) / 1e6,
                            "ttfb_ms": int(a["obsagent.llm.ttfb_ns"]) / 1e6,
                            "in": int(a["gen_ai.usage.input_tokens"]) if "gen_ai.usage.input_tokens" in a else None,
                            "out": int(a["gen_ai.usage.output_tokens"]) if "gen_ai.usage.output_tokens" in a else None,
                            "complete": a.get("obsagent.llm.complete"),
                        }
                    )
    return sorted(out, key=lambda x: x["end"])


def atlas_rows(path: str) -> list[dict]:
    db = sqlite3.connect(path)
    cols = "run_id, step, ts, is_stream, outcome, fault, ttft_ms, completion_ms, prompt_tokens, completion_tokens"
    return [dict(zip(cols.split(", "), r)) for r in db.execute(f"SELECT {cols} FROM steps ORDER BY ts")]


def pct(v: list[float], q: float) -> float:
    v = sorted(v)
    return v[min(len(v) - 1, int(q * len(v)))]


def dist(name: str, errs: list[float], unit: str) -> None:
    if not errs:
        print(f"  {name}: no pairs")
        return
    a = [abs(e) for e in errs]
    print(
        f"  {name}: n={len(errs)} signed median={statistics.median(errs):+.2f}{unit} "
        f"|err| p50={pct(a, .5):.2f} p90={pct(a, .9):.2f} max={max(a):.2f}{unit}"
    )


def main() -> None:
    db, sink = sys.argv[1], sys.argv[2]
    win = float(sys.argv[sys.argv.index("--window-s") + 1]) if "--window-s" in sys.argv else 1.0
    rows, spans = atlas_rows(db), agent_spans(sink)
    llm_rows = [r for r in rows if r["is_stream"] and r["fault"] is None and r["outcome"] == "ok"]
    other = [r for r in rows if r not in llm_rows]
    print(f"atlas steps={len(rows)} (streaming ok={len(llm_rows)}, other={len(other)}) agent chat spans={len(spans)}")

    used: set[int] = set()
    pairs, missed_rows = [], []
    for r in llm_rows:
        best = min(
            (i for i in range(len(spans)) if i not in used),
            key=lambda i: abs(spans[i]["end"] - r["ts"]),
            default=None,
        )
        if best is not None and abs(spans[best]["end"] - r["ts"]) <= win:
            used.add(best)
            pairs.append((r, spans[best]))
        else:
            missed_rows.append(r)
    extra = [s for i, s in enumerate(spans) if i not in used]

    print(f"joined={len(pairs)} atlas_rows_without_span={len(missed_rows)} spans_without_row={len(extra)}")
    dist("total_ms  (agent total - atlas completion_ms)", [s["total_ms"] - r["completion_ms"] for r, s in pairs if r["completion_ms"] is not None], "ms")
    dist("ttfb vs ttft (agent headers - atlas first content)", [s["ttfb_ms"] - r["ttft_ms"] for r, s in pairs if r["ttft_ms"] is not None], "ms")
    dist("end-time skew (agent end - atlas ts)", [(s["end"] - r["ts"]) * 1000 for r, s in pairs], "ms")
    for name, t in (
        ("input_tokens", [(s["in"], r["prompt_tokens"]) for r, s in pairs]),
        ("output_tokens", [(s["out"], r["completion_tokens"]) for r, s in pairs]),
    ):
        exact = sum(1 for a, b in t if a == b)
        none = sum(1 for a, _ in t if a is None)
        wrong = [(a, b) for a, b in t if a is not None and a != b]
        print(f"  {name}: exact={exact}/{len(t)} agent_none={none} wrong={wrong[:5]}")
    for r in missed_rows:
        print(f"  MISS row run={r['run_id']} step={r['step']} ts={r['ts']:.3f}: no span within {win}s")
    for r in other:
        print(f"  NOT-COMPARABLE row run={r['run_id']} step={r['step']} is_stream={r['is_stream']} outcome={r['outcome']} fault={r['fault']}")
    for s in extra[:10]:
        print(f"  EXTRA span end={s['end']:.3f} total_ms={s['total_ms']:.1f}")


if __name__ == "__main__":
    main()
