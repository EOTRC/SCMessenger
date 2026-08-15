#!/usr/bin/env python3
"""Read agy --output-format stream-json on stdin, print live progress.

Separate file rather than `python -c` inside the shell wrapper: the inline form
needs escaped quotes inside f-strings, which bash's single-quoting mangles into
a SyntaxError. Cost 1 failed run to learn.

Exits 0 on SUCCESS, 1 on failure, stall, or a stream that ends with no result.
"""
import json
import sys
import time

STALL_SECONDS = 120


def main():
    start = time.time()
    last = start
    steps = 0

    for line in sys.stdin:
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            ev = json.loads(line)
        except Exception:
            continue

        now = time.time()
        kind = ev.get("event")

        if kind == "init":
            model = (ev.get("init") or {}).get("model", "?")
            print("[%6.1fs] init   model=%s" % (now - start, model), flush=True)

        elif kind == "step_update":
            s = ev.get("step_update") or {}
            if s.get("state") != "DONE":
                continue
            steps += 1
            dur = s.get("duration_seconds") or 0.0
            usage = s.get("usage") or {}
            text = (s.get("text_delta") or "").strip().replace("\n", " ")[:64]
            gap = now - last
            last = now
            flag = "  <-- STALL" if gap > STALL_SECONDS else ""
            print(
                "[%6.1fs] step %2d %-16s %5.1fs in=%6d out=%5d%s%s"
                % (
                    now - start,
                    steps,
                    s.get("step_type", "?"),
                    dur,
                    usage.get("input_tokens", 0),
                    usage.get("output_tokens", 0),
                    ("  " + text) if text else "",
                    flag,
                ),
                flush=True,
            )

        elif kind == "result":
            r = ev.get("result") or {}
            u = r.get("usage") or {}
            print(
                "\n[RESULT] %s in %.1fs  turns=%s  tokens in=%d out=%d"
                % (
                    r.get("status"),
                    r.get("duration_seconds", 0.0),
                    r.get("num_turns"),
                    u.get("input_tokens", 0),
                    u.get("output_tokens", 0),
                ),
                flush=True,
            )
            print("-" * 70, flush=True)
            print((r.get("response") or "").strip(), flush=True)
            return 0 if r.get("status") == "SUCCESS" else 1

    print("\n[FAIL] stream ended with no result event -- timed out or died.", flush=True)
    print("       Resume with --continue; a fresh dispatch discards the work", flush=True)
    print("       already paid for.", flush=True)
    return 1


if __name__ == "__main__":
    sys.exit(main())
