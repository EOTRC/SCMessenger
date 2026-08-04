#!/usr/bin/env python3
"""
batch_handoff.py -- move a batch of HANDOFF task files in one pass and make
ONE git commit, instead of the orchestrator doing N serial Edit+commit
cycles. Delegates ledger writes to lake_route.py --record (the existing,
tested single writer of the ledger/cooldown format) instead of duplicating
that logic.

This script does NOT decide pass/fail. It is told the outcome by the
caller (the orchestrator, after consulting parse_orchestration_footer.py
and running the real verification gate). It only performs the mechanical,
idempotent file-move + commit + ledger-record steps.

Idempotency: safe to re-run. If a task's todo/ file is already gone and its
destination file already exists, that task is skipped with a NOTICE, not
an error -- reflects that HANDOFF/ already IS in the target state.

Windows-build-lock aware: does NOT run any build/verify commands itself
(that stays the caller's job, see build_lock.py), so it never needs the
lock. Pure filesystem + git operations only.

Usage:
    python scripts/batch_handoff.py \
      --batch-file tmp/batch_result.json \
      --provider qwenpaid \
      --commit-message "P1-P5 backoff+outbox+receipt work"

Where tmp/batch_result.json is:
{
  "tasks": [
    {"id": "P1", "destination": "done", "lake": "qwenpaid",
     "model": "qwen3-coder-plus", "result": "ok",
     "in_tokens": 6120, "out_tokens": 1480},
    {"id": "P2", "destination": "review", "lake": "qwenpaid",
     "model": "qwen3-coder-plus", "result": "ok"}
  ]
}

destination is one of: done, review, IN_PROGRESS, todo (todo = requeue,
file is left in place / not moved, ledger is still written so the retry
is on record).

--dry-run prints the plan without touching the filesystem, git, or ledger.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

HANDOFF_ROOT = Path("HANDOFF")
VALID_DESTINATIONS = {"done", "review", "IN_PROGRESS", "todo"}


def find_source(task_id):
    """Locate a task's current file under HANDOFF/todo/ (or IN_PROGRESS/,
    review/ -- a task being re-batched after a prior partial run may live
    there). Returns None if not found anywhere (already terminal, or the
    id is wrong)."""
    for bucket in ("todo", "IN_PROGRESS", "review"):
        matches = list((HANDOFF_ROOT / bucket).glob(f"{task_id}*.md"))
        if matches:
            return matches[0]
    return None


def already_at_destination(task_id, destination):
    dest_dir = HANDOFF_ROOT / destination
    if not dest_dir.exists():
        return False
    return len(list(dest_dir.glob(f"{task_id}*.md"))) > 0


def plan_moves(tasks):
    """Pure function: given task list, return (moves, skips, requeues) with
    no side effects. moves/skips/requeues are lists of dicts for reporting
    and for the actual execute step to consume."""
    moves, skips, requeues = [], [], []

    for task in tasks:
        task_id = task["id"]
        destination = task.get("destination", "done")

        if destination not in VALID_DESTINATIONS:
            skips.append({"id": task_id, "reason": f"invalid destination '{destination}'"})
            continue

        if destination == "todo":
            requeues.append({"id": task_id, "reason": "requeued, file left in place"})
            continue

        if already_at_destination(task_id, destination):
            skips.append({"id": task_id, "reason": f"already in {destination}/ (idempotent no-op)"})
            continue

        source = find_source(task_id)
        if source is None:
            skips.append({"id": task_id, "reason": "no source file found in todo/IN_PROGRESS/review"})
            continue

        moves.append({
            "id": task_id,
            "source": str(source),
            "dest_dir": str(HANDOFF_ROOT / destination),
            "dest_file": str(HANDOFF_ROOT / destination / source.name),
        })

    return moves, skips, requeues


def execute_moves(moves):
    """Side-effecting: actually rename files. Returns count moved."""
    moved = 0
    for m in moves:
        source = Path(m["source"])
        dest_dir = Path(m["dest_dir"])
        dest_dir.mkdir(parents=True, exist_ok=True)
        source.rename(Path(m["dest_file"]))
        print(f"[OK] {m['id']}: {m['source']} -> {m['dest_file']}")
        moved += 1
    return moved


def record_ledger(tasks, ledger_script="scripts/lake_route.py"):
    """Delegate to the existing lake_route.py --record for every task that
    carries lake/model info, so cooldown/ledger format has exactly one
    writer in the whole codebase. Tasks without a 'lake' field (e.g. a
    manual human-side task) are skipped silently -- not every HANDOFF
    entry is a lake dispatch."""
    recorded = 0
    for task in tasks:
        if "lake" not in task or "model" not in task or "result" not in task:
            continue
        cmd = [
            "python3", ledger_script, "--record",
            "--lake", task["lake"],
            "--model", task["model"],
            "--task", task["id"],
            "--result", task["result"],
        ]
        if "in_tokens" in task:
            cmd += ["--in-tokens", str(task["in_tokens"])]
        if "out_tokens" in task:
            cmd += ["--out-tokens", str(task["out_tokens"])]

        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode == 0:
            recorded += 1
        else:
            print(f"[WARN] ledger record failed for {task['id']}: {result.stderr.strip()}", file=sys.stderr)
    return recorded


def git_commit_all(message):
    add = subprocess.run(["git", "add", "-A", "HANDOFF/"], capture_output=True, text=True)
    if add.returncode != 0:
        print(f"[ERROR] git add failed: {add.stderr}", file=sys.stderr)
        return False

    # Check staged state directly instead of string-matching git's human
    # message: "nothing to commit, working tree clean" is only one of
    # several phrasings git uses (e.g. "nothing added to commit but
    # untracked files present" when unrelated untracked files exist
    # elsewhere in the repo, such as scratch files under tmp/) --
    # verified empirically, see ORCHESTRATION_TOKEN_STRATEGY.md Part 7.
    staged = subprocess.run(["git", "diff", "--cached", "--quiet", "--", "HANDOFF/"])
    if staged.returncode == 0:
        print("[INFO] nothing to commit (all moves were no-ops)")
        return True

    commit = subprocess.run(["git", "commit", "-m", message], capture_output=True, text=True)
    if commit.returncode == 0:
        print(f"[OK] commit: {message}")
        return True
    print(f"[ERROR] git commit failed: {commit.stderr or commit.stdout}", file=sys.stderr)
    return False


def main():
    parser = argparse.ArgumentParser(description="Batch HANDOFF file mover + single-commit + ledger recorder")
    parser.add_argument("--batch-file", required=True, help="JSON file: {'tasks': [...]}")
    parser.add_argument("--provider", default="native", help="Provider name for the commit message prefix")
    parser.add_argument("--commit-message", help="Commit message suffix (default: auto-generated from task ids)")
    parser.add_argument("--dry-run", action="store_true", help="Print the plan, touch nothing")
    parser.add_argument("--no-commit", action="store_true", help="Move files and record ledger but skip git commit")
    parser.add_argument("--ledger-script", default="scripts/lake_route.py")
    args = parser.parse_args()

    try:
        with open(args.batch_file, "r", encoding="utf-8") as f:
            batch = json.load(f)
    except (OSError, json.JSONDecodeError) as e:
        print(f"[ERROR] could not read --batch-file {args.batch_file}: {e}", file=sys.stderr)
        sys.exit(1)

    tasks = batch.get("tasks", [])
    if not tasks:
        print("[INFO] no tasks in batch file, nothing to do")
        sys.exit(0)

    moves, skips, requeues = plan_moves(tasks)

    print(f"[PLAN] {len(moves)} move(s), {len(skips)} skip(s), {len(requeues)} requeue(s)")
    for s in skips:
        print(f"  [SKIP] {s['id']}: {s['reason']}")
    for r in requeues:
        print(f"  [REQUEUE] {r['id']}: {r['reason']}")
    for m in moves:
        print(f"  [MOVE] {m['id']}: {m['source']} -> {m['dest_file']}")

    if args.dry_run:
        print("[DRY-RUN] no files touched, no commit made, no ledger written")
        sys.exit(0)

    moved_count = execute_moves(moves)
    recorded_count = record_ledger(tasks, args.ledger_script)
    print(f"[SUMMARY] moved={moved_count} ledger_recorded={recorded_count}")

    if args.no_commit:
        print("[INFO] --no-commit set, skipping git commit")
        sys.exit(0)

    ids = ", ".join(t["id"] for t in tasks)
    message = f"{args.provider}: completed batch ({ids})"
    if args.commit_message:
        message = f"{args.provider}: {args.commit_message}"

    ok = git_commit_all(message)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
