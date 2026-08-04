#!/usr/bin/env python3
"""
orchestrate_strict.py -- the pure-delegator orchestration loop, as a thin
COMPOSITION of already-tested pieces. This file intentionally contains
almost no decision logic of its own:

    dispatch_dial.py            WHAT effort level / scope this task needs
    lake_route.py                WHICH lake/model has quota right now
                                  (called BY dispatch_dial.py, not here)
    delegate_task.py             the actual API call + diff apply + verify
    parse_orchestration_footer.py  read the worker's structured report
    batch_handoff.py             move HANDOFF files + ONE commit + ledger

Revision note: an earlier version of this file (first pass at this
redesign) reimplemented routing and cooldown logic inline instead of
calling lake_route.py, and parsed worker responses with a fragile
LEDGER_JSON regex instead of the FILES/NOTES contract AGENTS.md already
defines. That duplicated, less-tested logic has been removed in favor of
the composition below -- see HANDOFF/ORCHESTRATION_TOKEN_STRATEGY.md Part 7
for why. scripts/supervisor.py from that same first pass is superseded by
batch_handoff.py (idempotent, tested, delegates ledger writes to
lake_route.py) and build_lock.py (serializes verification); it is kept in
the repo per the operator's do-not-delete convention but should not be
used for new work.

Orchestrator token budget per task, by design: read one queue entry
(~50 tokens) + one dispatch_dial.py call (near-zero, pure logic) + one
delegate_task.py invocation (the worker's tokens, not the orchestrator's)
+ one parse_orchestration_footer.py call (~50 tokens) = no per-task prompt
construction or response-grepping performed BY the orchestrator itself.

Usage:
    python scripts/orchestrate_strict.py \
        --queue scm_v1_farm_queue.jsonl \
        --max-tasks 5 \
        --provider qwenpaid

Each queue entry needs a pre-written dispatch prompt at
tmp/tasks/<ID>.dispatch.md (the task requirement + acceptance criteria --
this is normal HANDOFF/todo/<ID>_*.md content, not something this script
generates). If absent, the task is skipped with a clear message rather
than silently inventing prompt content.
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent


def read_next_task(queue_file, done_dir="HANDOFF/done", already_seen=None):
    """Read the queue (JSONL, one task dict per line -- matches the real
    scm_v1_farm_queue.jsonl format) and return the first task whose
    dependencies are satisfied and whose status is 'open'. Returns None if
    nothing is ready."""
    already_seen = already_seen or set()
    try:
        with open(queue_file, "r", encoding="utf-8") as f:
            lines = f.readlines()
    except OSError as e:
        print(f"[ERROR] cannot read queue {queue_file}: {e}", file=sys.stderr)
        return None

    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            task = json.loads(line)
        except json.JSONDecodeError:
            continue

        task_id = task.get("id")
        if task_id in already_seen:
            continue
        if task.get("status") != "open":
            continue

        depends = task.get("depends", [])
        deps_met = all(
            list(Path(done_dir).glob(f"{dep}_*.md")) or list(Path(done_dir).glob(f"{dep}.md"))
            for dep in depends
        ) if depends else True

        if deps_met:
            return task

    return None


def dial(task, lake_route_script="scripts/lake_route.py"):
    """Call dispatch_dial.py as a subprocess (not imported -- keeps this
    orchestrator usable by any language/model, matching the
    ORCHESTRATION.md doctrine that the canonical dispatch path is a
    script, not a Claude-only in-process import)."""
    cmd = [
        "python3", str(SCRIPT_DIR / "dispatch_dial.py"),
        "--tier", task.get("tier", "CODER"),
        "--description", task.get("description", ""),
        "--lake-route-script", lake_route_script,
    ]
    files = task.get("files", [])
    if files:
        cmd += ["--files"] + files

    result = subprocess.run(cmd, capture_output=True, text=True)
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return {
            "tier": task.get("tier", "CODER"), "lake": None, "model": None,
            "router_error": f"dispatch_dial.py produced unparseable output: {result.stdout!r} / {result.stderr!r}",
        }


def dispatch(task_id, spec, dispatch_prompt, verify_gate, max_rounds=3):
    """Run the actual worker call via delegate_task.py. Returns
    (exit_code, response_text_path)."""
    cmd = [
        "python3", str(SCRIPT_DIR / "delegate_task.py"),
        "--task", dispatch_prompt,
        "--provider", spec["lake"],
        "--model", spec["model"],
        "--files", *spec.get("files", []),
        "--apply",
        "--verify", verify_gate,
        "--mode", "diff",
        "--max-rounds", str(spec.get("max_rounds", max_rounds)),
    ]
    print(f"[DISPATCH] {task_id} -> {spec['lake']}/{spec['model']} (tier={spec['tier']}, thinking={spec['thinking']})")
    if spec.get("advisory"):
        for note in spec["advisory"]:
            print(f"  [ADVISORY] {note}")

    result = subprocess.run(cmd, capture_output=True, text=True)
    response_path = f"tmp/{Path(dispatch_prompt).stem}_response.md"
    return result.returncode, response_path, result.stdout, result.stderr


def parse_response(response_path):
    result = subprocess.run(
        ["python3", str(SCRIPT_DIR / "parse_orchestration_footer.py"), response_path],
        capture_output=True, text=True,
    )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return {"result": "UNKNOWN", "degraded": True, "files": [], "notes": []}


def main():
    parser = argparse.ArgumentParser(description="Pure-delegator orchestration loop (composition, not reimplementation)")
    parser.add_argument("--queue", default="scm_v1_farm_queue.jsonl")
    parser.add_argument("--max-tasks", type=int, default=5)
    parser.add_argument("--provider", help="Force a specific lake instead of letting dispatch_dial.py/lake_route.py choose")
    parser.add_argument("--lake-route-script", default="scripts/lake_route.py")
    parser.add_argument("--dry-run", action="store_true", help="Dial + print the plan, dispatch nothing")
    args = parser.parse_args()

    batch_tasks = []
    seen = set()

    for i in range(args.max_tasks):
        task = read_next_task(args.queue, already_seen=seen)
        if not task:
            print(f"[INFO] no more ready tasks (dispatched {i})")
            break
        seen.add(task["id"])

        spec = dial(task, args.lake_route_script)
        if args.provider:
            spec["lake"] = args.provider

        if not spec.get("lake"):
            print(f"[SKIP] {task['id']}: no lake available ({spec.get('router_error')})")
            continue

        prompt_file = f"tmp/tasks/{task['id']}.dispatch.md"
        if not os.path.exists(prompt_file):
            print(f"[SKIP] {task['id']}: no dispatch prompt at {prompt_file} -- write the task packet first")
            continue

        if args.dry_run:
            print(f"[PLAN] {task['id']}: {spec['lake']}/{spec['model']} tier={spec['tier']}")
            continue

        verify_gate = task.get("verify_gate", "cargo check --workspace")
        exit_code, response_path, stdout, stderr = dispatch(
            task["id"], spec, prompt_file, verify_gate, spec.get("max_rounds", 3)
        )

        report = parse_response(response_path) if os.path.exists(response_path) else {"result": "UNKNOWN", "degraded": True, "files": [], "notes": []}

        if report["degraded"]:
            print(f"[WARN] {task['id']}: no structured footer in response -- treating exit_code only (exit={exit_code})")

        destination = "done" if (exit_code == 0 and report["result"] in ("DONE", "UNKNOWN")) else "todo"
        batch_tasks.append({
            "id": task["id"],
            "destination": destination,
            "lake": spec["lake"],
            "model": spec["model"],
            "result": "ok" if destination == "done" else "error",
        })
        print(f"[{'OK' if destination == 'done' else 'REQUEUE'}] {task['id']}: exit={exit_code} result={report['result']} files={report['files']}")

    if args.dry_run or not batch_tasks:
        print("[INFO] nothing to commit (dry-run or empty batch)")
        return

    batch_file = "tmp/orchestrate_strict_batch.json"
    with open(batch_file, "w", encoding="utf-8") as f:
        json.dump({"tasks": batch_tasks}, f, indent=2)

    subprocess.run([
        "python3", str(SCRIPT_DIR / "batch_handoff.py"),
        "--batch-file", batch_file,
        "--provider", args.provider or "mixed",
        "--commit-message", f"batch of {len(batch_tasks)} task(s)",
    ])


if __name__ == "__main__":
    main()
