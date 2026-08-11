#!/usr/bin/env python3
"""Safe, repo-owned writer-worktree lifecycle for the orchestration kernel."""

import argparse
import json
import re
import subprocess
from pathlib import Path


TASK_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")


def _run(args, cwd=None):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True)


def repo_root(cwd=None):
    result = _run(["git", "rev-parse", "--show-toplevel"], cwd=cwd)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "not inside a git worktree")
    return Path(result.stdout.strip()).resolve()


def plan(task_id, base_sha=None, root=None, attempt=1):
    if not TASK_ID_RE.fullmatch(task_id):
        raise ValueError("task id must contain only letters, numbers, dot, underscore, and dash")
    root = Path(root or repo_root()).resolve()
    base = _run(["git", "rev-parse", base_sha or "HEAD"], cwd=root)
    if base.returncode:
        raise RuntimeError(base.stderr.strip() or "cannot resolve base SHA")
    worktree_root = root / "tmp" / "orchestration" / "worktrees"
    if attempt < 1:
        raise ValueError("worktree attempt must be at least one")
    name = task_id if attempt == 1 else f"{task_id}-attempt-{attempt}"
    path = (worktree_root / name).resolve()
    if worktree_root not in path.parents:
        raise ValueError("worktree path escapes orchestration workspace")
    return {
        "task_id": task_id, "base_sha": base.stdout.strip(), "attempt": attempt,
        "path": str(path), "isolation_id": f"writer:{task_id}:attempt:{attempt}",
    }


def create(task_id, base_sha=None, root=None, attempt=1):
    item = plan(task_id, base_sha, root, attempt)
    path = Path(item["path"])
    if path.exists():
        raise RuntimeError(f"writer worktree already exists: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    result = _run(["git", "worktree", "add", "--detach", str(path), item["base_sha"]], cwd=repo_root(root))
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "git worktree add failed")
    return item


def remove(task_id, root=None, attempt=1):
    item = plan(task_id, root=root, attempt=attempt)
    path = Path(item["path"])
    if not path.exists():
        return item
    result = _run(["git", "worktree", "remove", "--force", str(path)], cwd=repo_root(root))
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "git worktree remove failed")
    return item


def main():
    parser = argparse.ArgumentParser(description="Manage isolated orchestration writer worktrees")
    parser.add_argument("--task", required=True)
    parser.add_argument("--base-sha")
    parser.add_argument("--attempt", type=int, default=1)
    parser.add_argument("--create", action="store_true")
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if args.create and args.remove:
        parser.error("choose only one of --create or --remove")
    try:
        if args.remove:
            value = plan(args.task, root=repo_root(), attempt=args.attempt) if args.dry_run else remove(args.task, attempt=args.attempt)
        else:
            value = plan(args.task, args.base_sha, attempt=args.attempt)
            if args.create and not args.dry_run:
                value = create(args.task, args.base_sha, attempt=args.attempt)
    except (RuntimeError, ValueError) as exc:
        print(json.dumps({"ok": False, "error": str(exc)}))
        return 1
    value["ok"] = True
    print(json.dumps(value))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
