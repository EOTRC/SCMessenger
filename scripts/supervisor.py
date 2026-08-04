#!/usr/bin/env python3
"""
supervisor.py -- Verification and state management (delegated from orchestrator)

STATUS NOTE (2026-08-03): kept per the repo's do-not-delete-superseded-work
convention, but scripts/batch_handoff.py + scripts/build_lock.py now cover
this file's job with more testing (idempotent re-run, single-commit batch,
ledger writes delegated to lake_route.py --record instead of duplicated
here, verification serialized against concurrent Windows builds). Prefer
those two for new work. See HANDOFF/ORCHESTRATION_TOKEN_STRATEGY.md Part 7
for the reasoning. This file is unchanged below and still works standalone
for a single task.

The supervisor's job is to:
1. Run verification commands (orchestrator shouldn't do this)
2. Parse worker evidence
3. Update HANDOFF state (move files)
4. Record ledger entries

This is a SEPARATE AGENT from orchestrator, keeping orchestrator token cost low.
Can be same model as orchestrator, or a cheaper validator (e.g., Groq Flash).

Usage:
  python scripts/supervisor.py \
    --task E-04 \
    --verify "cargo check --workspace" \
    --touched-files core/src/transport/swarm.rs \
    --ledger-entry '{"ts":"...", "lake":"qwenpaid", "result":"ok"}'
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

def run_verification(command):
    """Run verification shell command, return (pass: bool, output: str)."""
    print(f"[VERIFY] Running: {command}")

    result = subprocess.run(
        command,
        shell=True,
        capture_output=True,
        text=True,
        timeout=300,  # 5 min timeout
    )

    output = result.stdout + result.stderr
    return result.returncode == 0, output

def move_handoff_file(task_id, destination):
    """Move task file from HANDOFF/todo to HANDOFF/destination."""
    todo_glob = list(Path("HANDOFF/todo").glob(f"{task_id}_*.md"))
    if not todo_glob:
        print(f"[WARN] No todo file found for {task_id}")
        return False

    source = todo_glob[0]
    dest_dir = Path("HANDOFF") / destination
    dest_dir.mkdir(parents=True, exist_ok=True)

    dest_file = dest_dir / source.name
    source.rename(dest_file)
    print(f"[OK] Moved {source.name} -> {destination}/")
    return True

def update_ledger(task_id, ledger_entry, ledger_file):
    """Append ledger entry."""
    os.makedirs("tmp/lakes", exist_ok=True)

    with open(ledger_file, "a", encoding="utf-8") as f:
        f.write(json.dumps(ledger_entry) + "\n")

    print(f"[OK] Ledger updated (task {task_id})")

def commit_changes(message):
    """Git commit HANDOFF changes."""
    result = subprocess.run(
        ["git", "add", "-A", "HANDOFF/"],
        capture_output=True,
        text=True,
    )

    if result.returncode != 0:
        print(f"[WARN] git add failed: {result.stderr}")
        return False

    result = subprocess.run(
        ["git", "commit", "-m", message],
        capture_output=True,
        text=True,
    )

    if result.returncode == 0:
        print(f"[OK] Commit: {message}")
        return True
    elif "nothing to commit" in result.stdout:
        print(f"[INFO] No changes to commit")
        return True
    else:
        print(f"[WARN] git commit failed: {result.stderr}")
        return False

def main():
    parser = argparse.ArgumentParser(description="Verification and state supervisor")
    parser.add_argument("--task", required=True, help="Task ID (e.g., E-04)")
    parser.add_argument("--verify", required=True, help="Verification command")
    parser.add_argument("--touched-files", nargs="*", default=[], help="Files touched by worker")
    parser.add_argument("--ledger-entry", required=True, help="Ledger JSON entry (string)")
    parser.add_argument("--ledger", default="tmp/lakes/ledger.jsonl", help="Ledger file")
    parser.add_argument("--provider", default="native", help="Provider (for commit message)")

    args = parser.parse_args()

    # Parse ledger entry
    try:
        ledger_entry = json.loads(args.ledger_entry)
    except json.JSONDecodeError:
        print(f"[ERROR] Invalid ledger JSON: {args.ledger_entry}")
        sys.exit(1)

    # RUN VERIFICATION
    verify_pass, verify_output = run_verification(args.verify)

    if verify_pass:
        print(f"[PASS] Verification succeeded for {args.task}")
        ledger_entry["verification_result"] = "pass"

        # MOVE TO DONE
        if not move_handoff_file(args.task, "done"):
            print(f"[ERROR] Could not move {args.task} to done/")
            sys.exit(1)

        # UPDATE LEDGER
        update_ledger(args.task, ledger_entry, args.ledger)

        # COMMIT
        commit_msg = f"{args.provider}: completed {args.task}"
        commit_changes(commit_msg)

        print(f"\n[OK] Task {args.task} complete and verified")
        sys.exit(0)
    else:
        print(f"[FAIL] Verification failed for {args.task}")
        print(f"[OUTPUT]\n{verify_output[:500]}")
        ledger_entry["verification_result"] = "fail"

        # REQUEUE (move back to IN_PROGRESS or stay in todo for retry)
        print(f"[REQUEUE] {args.task} (verification failed)")

        # Update ledger with failure
        update_ledger(args.task, ledger_entry, args.ledger)

        # No commit (file stays in todo for retry)
        sys.exit(1)

if __name__ == "__main__":
    main()
