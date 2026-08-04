#!/usr/bin/env python3
"""
dispatch_dial.py -- deterministic task-properties-to-dispatch-settings
router. This is the "precision dial": given what the task IS, decide what
EFFORT LEVEL (tier / thinking) and SCOPE (which files, how much retry
budget) it gets, then hand off to the existing lake_route.py to decide
WHICH lake/model actually serves that tier right now.

Explicit division of labor (do not blur this line):
  dispatch_dial.py  -> WHAT effort level does this task need
                        (reads: queue entry, target files, description)
  lake_route.py      -> WHICH lake/model currently has quota for that tier
                        (reads: registry.json, ledger.jsonl, round_robin_state.json)
  delegate_task.py   -> HOW the API call itself is made
                        (reads: nothing decision-related, just executes)

dispatch_dial.py never makes network calls and never touches the ledger.
It is pure string/logic on task metadata already in hand, so calling it
costs the orchestrator effectively nothing.

Tier vocabulary matches ORCHESTRATION.md / registry.json: FLASH < CODER <
THINK < MAX. This is DIFFERENT from delegate_task.py's own --tier flag
(lowercase thinking/max/standard/plus/flash, which only resolves for
provider=="qwen" and produces model name aliases like "qwen-standard" that
do not exist in delegate_task.py's own MODEL_TOKEN_LIMITS table -- do not
use that flag; dispatch_dial.py always emits an explicit --model instead,
sourced from lake_route.py's output).

Gate rules mirror ORCHESTRATION.md Section 4 exactly (kept as data here so
both docs and code can be diffed against each other):
  - core/src/{crypto,transport,routing,privacy}/ diffs -> mandatory
    adversarial review at THINK+ before commit. This function does not
    perform that review; it raises the MINIMUM implementation tier to
    THINK so the first-pass implementation is already at the quality bar
    the review will demand, and it sets security_gate_required=True so the
    caller knows a separate adversarial dispatch is still owed regardless
    of which tier implemented it.
  - outbox/receipt/custody/retry ("WS-A delivery logic") diffs -> flags
    delivery_gate_required=True (3 distinct verifier dispatches or one
    Fusion Lite panel, per Section 4). Does not by itself change tier.

Usage (CLI, for shell-based orchestrators / smoke testing):
    python scripts/dispatch_dial.py \
        --tier CODER \
        --files core/src/transport/swarm.rs core/src/audit.rs \
        --description "Add observability hook to send path" \
        --retry-count 0

    -> prints one JSON line (a DispatchSpec) to stdout.

Usage (importable, for an orchestrator written in Python):
    from dispatch_dial import build_dispatch_spec
    spec = build_dispatch_spec(tier="CODER", files=[...], description="...")
"""

import argparse
import json
import subprocess
import sys

TIER_ORDER = ["FLASH", "CODER", "THINK", "MAX"]

# Mirrors ORCHESTRATION.md Section 4 row 1 exactly.
SECURITY_GATED_PREFIXES = (
    "core/src/crypto/",
    "core/src/transport/",
    "core/src/routing/",
    "core/src/privacy/",
)

# Mirrors ORCHESTRATION.md Section 4 row 2 ("WS-A delivery logic").
DELIVERY_GATE_KEYWORDS = ("outbox", "receipt", "custody", "retry")

# Mechanical-task heuristics: used ONLY to justify staying at FLASH when the
# queue already says FLASH, or to flag "this looks bigger than its stated
# tier" -- never used to silently downgrade an explicit THINK/MAX/CODER
# classification. The queue's own tier field is authoritative; this is an
# advisory cross-check, surfaced in the spec's 'advisory' field, not
# auto-applied.
MECHANICAL_DESCRIPTION_HINTS = (
    "unused import", "rename", "typo", "update doc", "add clippy",
    "regenerate", "bump version", "fix import", "formatting",
)


def _normalize(path):
    return path.replace("\\", "/")


def requires_security_gate(files):
    """True if ANY target file falls under a gated path prefix."""
    return any(
        _normalize(f).startswith(SECURITY_GATED_PREFIXES) for f in files
    )


def requires_delivery_gate(files, description):
    haystack = " ".join([description.lower()] + [f.lower() for f in files])
    return any(kw in haystack for kw in DELIVERY_GATE_KEYWORDS)


def looks_mechanical(description):
    desc = description.lower()
    return any(hint in desc for hint in MECHANICAL_DESCRIPTION_HINTS)


def escalate_tier(tier, levels=1):
    """Move up the TIER_ORDER ladder, clamped at MAX."""
    try:
        idx = TIER_ORDER.index(tier.upper())
    except ValueError:
        idx = 1  # unknown tier -> treat as CODER, escalate from there
    new_idx = min(idx + levels, len(TIER_ORDER) - 1)
    return TIER_ORDER[new_idx]


def resolve_lake_and_model(tier, lake_route_script="scripts/lake_route.py"):
    """Shell out to the existing, tested router. Never re-implement its
    quota/cooldown/round-robin logic here."""
    result = subprocess.run(
        ["python3", lake_route_script, "--tier", tier],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        return None, None, result.stderr.strip() or result.stdout.strip()
    parts = result.stdout.strip().split()
    if len(parts) != 2:
        return None, None, f"unexpected lake_route.py output: {result.stdout!r}"
    return parts[0], parts[1], None


def build_dispatch_spec(tier, files=None, description="", retry_count=0,
                          lake_route_script="scripts/lake_route.py",
                          resolve_lake=True):
    """Pure decision function (minus the optional lake_route.py subprocess
    call) -> dict DispatchSpec.

    Fields:
      tier                    effective tier after gate/retry escalation
      requested_tier          the tier the caller asked for, unescalated
      thinking                bool, enable_thinking flag for delegate_task.py
      security_gate_required  bool, per ORCHESTRATION.md Section 4 row 1
      delivery_gate_required  bool, per ORCHESTRATION.md Section 4 row 2
      max_rounds              int, retry budget for this dispatch
      lake / model            resolved via lake_route.py (None if
                               resolve_lake=False or router had nothing)
      router_error            str or None
      advisory                list[str], non-binding notes (e.g. "queue
                               says FLASH but touches gated path")
    """
    files = files or []
    requested_tier = tier.upper()
    effective_tier = requested_tier
    advisory = []

    sec_gate = requires_security_gate(files)
    delivery_gate = requires_delivery_gate(files, description)

    if sec_gate and TIER_ORDER.index(effective_tier) < TIER_ORDER.index("THINK"):
        advisory.append(
            f"escalated {effective_tier} -> THINK: touches gated path "
            f"({[f for f in files if _normalize(f).startswith(SECURITY_GATED_PREFIXES)]})"
        )
        effective_tier = "THINK"

    if retry_count >= 2:
        pre = effective_tier
        effective_tier = escalate_tier(effective_tier, levels=1)
        if effective_tier != pre:
            advisory.append(f"escalated {pre} -> {effective_tier}: retry_count={retry_count} (2+ failures)")

    if looks_mechanical(description) and requested_tier != "FLASH":
        advisory.append(f"note: description looks mechanical but queue tier is {requested_tier} -- not auto-downgraded")

    thinking = effective_tier in ("THINK", "MAX")

    # Retry budget: mechanical FLASH work fails fast; anything gated gets
    # the full budget since a wasted round there is expensive to re-review.
    if effective_tier == "FLASH" and not sec_gate:
        max_rounds = 2
    else:
        max_rounds = 3

    spec = {
        "requested_tier": requested_tier,
        "tier": effective_tier,
        "thinking": thinking,
        "security_gate_required": sec_gate,
        "delivery_gate_required": delivery_gate,
        "max_rounds": max_rounds,
        "files": files,
        "lake": None,
        "model": None,
        "router_error": None,
        "advisory": advisory,
    }

    if resolve_lake:
        lake, model, err = resolve_lake_and_model(effective_tier, lake_route_script)
        spec["lake"] = lake
        spec["model"] = model
        spec["router_error"] = err

    return spec


def main():
    parser = argparse.ArgumentParser(description="Task-properties-to-dispatch-settings dial")
    parser.add_argument("--tier", required=True, help="Queue-assigned tier: FLASH|CODER|THINK|MAX")
    parser.add_argument("--files", nargs="*", default=[], help="Target file paths")
    parser.add_argument("--description", default="", help="Task description text")
    parser.add_argument("--retry-count", type=int, default=0)
    parser.add_argument("--lake-route-script", default="scripts/lake_route.py")
    parser.add_argument("--no-resolve-lake", action="store_true", help="Skip the lake_route.py subprocess call (dry decision only)")
    args = parser.parse_args()

    spec = build_dispatch_spec(
        tier=args.tier,
        files=args.files,
        description=args.description,
        retry_count=args.retry_count,
        lake_route_script=args.lake_route_script,
        resolve_lake=not args.no_resolve_lake,
    )
    print(json.dumps(spec, indent=2))
    sys.exit(0 if (spec["lake"] or args.no_resolve_lake) else 1)


if __name__ == "__main__":
    main()
