#!/usr/bin/env python3
"""
parse_orchestration_footer.py -- extract the structured report footer from a
worker response so the orchestrator never has to grep prose.

Field names and RESULT/VERIFICATION values are IDENTICAL to AGENTS.md's
existing REMOTE SANDBOX / FOREIGN WORKER report contract:

    RESULT: DONE|BLOCKED|FAILED
    VERIFICATION: NONE|CONTAINER(<what ran, exact commands>)
    FILES: <paths touched>
    NOTES: <max 8 lines: decisions made, risks, what the verifier must run>

This is deliberate: AGENTS.md is the canonical, model-agnostic contract for
ANY agent in this repo (see AGENTS.md line 6-11), so a worker dispatched via
delegate_task.py should never be asked to learn a second, contradictory
vocabulary. What this script adds is purely a PARSING upgrade, not a new
contract: FILES/NOTES may be a JSON list (machine-friendly) or plain
AGENTS.md-style free text (comma-separated paths / prose lines) -- both
parse into the same structured output.

The one real addition beyond AGENTS.md is the delimiter. AGENTS.md's
REMOTE/FOREIGN formats assume the report IS the entire message (a worker
that only edits files and reports back). delegate_task.py's workers return
a diff/full-file payload as the MAIN content (parsed separately by
extract_diff_blocks / extract_file_blocks) and this footer is a SUPPLEMENT
appended at the end, so it needs a delimiter to be found reliably inside a
much longer response:

    ---ORCHESTRATION_METADATA---
    RESULT: DONE
    VERIFICATION: CONTAINER(cargo check --workspace)
    FILES: ["core/src/transport/swarm.rs", "core/src/audit.rs"]
    NOTES: ["line 42: added observability_event() call", "48 tests pass"]
    ---END---

Design constraints (all learned from real failure modes recorded in
ORCHESTRATION.md Section 9 -- models hallucinate paths, wrap things in code
fences despite instructions, emit invalid JSON with trailing commas or
single quotes):
  - Never raises. Always returns a dict with a 'degraded' key.
  - Tolerates the footer being wrapped in a ``` fence.
  - Tolerates a JSON list, a near-JSON list (single quotes / trailing
    commas), or bare AGENTS.md-style comma-separated free text for
    FILES/NOTES -- in that preference order.
  - Missing footer is NOT an error by itself -- it is reported as
    degraded=True with result='UNKNOWN' so the caller MUST fall back to
    the pre-existing grep/diff-block extraction path, never silently
    treated as success.

Usage:
    python scripts/parse_orchestration_footer.py <response_file>
    python scripts/parse_orchestration_footer.py --stdin < response.md
    (importable: from parse_orchestration_footer import parse_footer)
"""

import json
import re
import sys

FOOTER_RE = re.compile(
    r"-{3,}ORCHESTRATION_METADATA-{3,}\s*\n(.*?)\n-{3,}END-{3,}",
    re.DOTALL,
)

# Strips a wrapping ```...``` fence if the model added one despite instructions.
FENCE_RE = re.compile(r"^```[a-zA-Z]*\n(.*)\n```$", re.DOTALL)

LIST_FIELD_BRACKET_RE = re.compile(r"\[(.*?)\]", re.DOTALL)
QUOTED_ITEM_RE = re.compile(r"""["']([^"']+)["']""")

VALID_RESULTS = {"DONE", "BLOCKED", "FAILED"}


def _strip_fence(block: str) -> str:
    m = FENCE_RE.match(block.strip())
    return m.group(1) if m else block


def _parse_list_field(raw: str):
    """Best-effort parse of FILES:/NOTES: value. Returns [] on empty input,
    never raises. Order of attempts:
      1. Strict JSON list (preferred, what dispatch prompts should ask for).
      2. Near-JSON: scrape quoted items out of a [...] bracket (handles
         single quotes / trailing commas).
      3. Bare AGENTS.md-style free text: comma-separated, no brackets at
         all (what a worker that only knows AGENTS.md's contract writes).
    """
    raw = raw.strip()
    if not raw:
        return []

    try:
        val = json.loads(raw)
        if isinstance(val, list):
            return [str(x) for x in val]
    except (json.JSONDecodeError, ValueError):
        pass

    m = LIST_FIELD_BRACKET_RE.search(raw)
    if m:
        items = QUOTED_ITEM_RE.findall(m.group(1))
        if items:
            return items
        # brackets present but nothing quoted inside -- fall through to
        # treating the bracket contents as bare comma-separated text
        raw = m.group(1)

    # Bare free text (AGENTS.md style): "core/src/a.rs, core/src/b.rs"
    return [item.strip() for item in raw.split(",") if item.strip()]


def parse_footer(response_text: str) -> dict:
    """Extract the metadata footer from a worker response.

    Returns a dict that ALWAYS has these keys, so callers never need
    defensive .get() chains:
      result:         'DONE' | 'BLOCKED' | 'FAILED' | 'UNKNOWN'
      verification:   str (raw VERIFICATION value, '' if absent)
      files:          list[str]  (AGENTS.md's FILES field)
      notes:          list[str]  (AGENTS.md's NOTES field)
      degraded:       bool -- True if footer missing/unparseable; caller
                      MUST fall back to legacy grep/diff-block extraction,
                      never treat degraded=True as success.
      raw_footer:     str or None -- the matched block, for audit trail
    """
    result = {
        "result": "UNKNOWN",
        "verification": "",
        "files": [],
        "notes": [],
        "degraded": True,
        "raw_footer": None,
    }

    if not response_text or not response_text.strip():
        return result

    match = FOOTER_RE.search(response_text)
    if not match:
        return result

    footer = _strip_fence(match.group(1))
    result["raw_footer"] = footer
    result["degraded"] = False  # footer found; may still be partially malformed

    for line in footer.splitlines():
        line = line.strip()
        if not line or ":" not in line:
            continue
        key, _, value = line.partition(":")
        key = key.strip().upper()
        value = value.strip()

        if key == "RESULT":
            candidate = value.upper()
            result["result"] = candidate if candidate in VALID_RESULTS else "UNKNOWN"
        elif key == "VERIFICATION":
            result["verification"] = value
        elif key == "FILES":
            result["files"] = _parse_list_field(value)
        elif key == "NOTES":
            result["notes"] = _parse_list_field(value)

    # A footer that parsed but never set RESULT to a valid value is still
    # unsafe to treat as success -- keep degraded=False (structure WAS
    # found) but 'UNKNOWN' already fails closed for any caller checking
    # result == 'DONE' explicitly.
    return result


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--stdin":
        text = sys.stdin.read()
    elif len(sys.argv) >= 2:
        with open(sys.argv[1], "r", encoding="utf-8") as f:
            text = f.read()
    else:
        print("Usage: parse_orchestration_footer.py <response_file> | --stdin", file=sys.stderr)
        sys.exit(1)

    parsed = parse_footer(text)
    print(json.dumps(parsed, indent=2))
    # Exit code mirrors result so shell callers can branch without
    # re-parsing JSON: 0=DONE, 1=BLOCKED/FAILED, 2=UNKNOWN/degraded.
    if parsed["degraded"] or parsed["result"] == "UNKNOWN":
        sys.exit(2)
    sys.exit(0 if parsed["result"] == "DONE" else 1)


if __name__ == "__main__":
    main()
