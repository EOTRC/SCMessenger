#!/usr/bin/env python3
"""
build_lock.py -- serialize verification runs so a strict-token orchestrator
that dispatches a whole batch before checking in cannot accidentally start
two concurrent cargo/gradle builds.

This closes a real gap in the token-reduction redesign: batching dispatch
(read once, fire N tasks, verify N results) is exactly the pattern that
risks two verification subprocesses landing at the same time if the
orchestrator (or several agent instances) parallelize the "run the gate"
step. build.md's Windows-Specific Rules are explicit and paid-for-in-a-
bad-build: "Never run two build-tool invocations concurrently... Gradle can
spawn cargo-ndk upstream" and ORCHESTRATION.md Section 9.5: "Never run two
concurrent delegate_task.py --verify jobs (2 concurrent cargo/gradle builds
risk rlib lock corruption)."

This is a simple advisory lockfile, not a kernel-level lock -- it protects
cooperating callers (this repo's own scripts) from each other. It does NOT
protect against a human running cargo by hand in another terminal at the
same time; nothing in-process can.

Usage as a CLI wrapper (recommended -- runs the command for you, holds the
lock for exactly its duration, always releases even on failure/Ctrl-C):
    python scripts/build_lock.py --run "cargo check --workspace"
    python scripts/build_lock.py --run "cd android && ./gradlew assembleDebug -x lint --quiet"

Usage as acquire/release primitives (if a caller needs the lock held across
several of its own steps):
    python scripts/build_lock.py --acquire   # exit 0 = acquired, 1 = busy
    ... caller runs its own verification ...
    python scripts/build_lock.py --release

Stale-lock recovery: a lock older than --stale-after seconds (default 1800
= 30 min, generous for a full workspace test compile) is treated as
abandoned (e.g. the process that held it was killed) and force-acquired,
with a loud warning -- never a silent steal.
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

LOCK_PATH = Path("tmp/.build.lock")
DEFAULT_STALE_AFTER = 1800  # seconds


def _read_lock():
    try:
        with open(LOCK_PATH, "r", encoding="utf-8") as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return None


def _write_lock(payload):
    LOCK_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(LOCK_PATH, "w", encoding="utf-8") as f:
        json.dump(payload, f)


def acquire(holder="unknown", stale_after=DEFAULT_STALE_AFTER):
    """Returns (True, None) if acquired, (False, existing_lock_dict) if busy."""
    existing = _read_lock()
    if existing is not None:
        age = time.time() - existing.get("acquired_at", 0)
        if age < stale_after:
            return False, existing
        print(
            f"[WARN] stale lock from '{existing.get('holder')}' "
            f"(pid {existing.get('pid')}, age {int(age)}s > {stale_after}s) -- "
            f"force-acquiring. If that process is still legitimately running, "
            f"this WILL cause a concurrent-build conflict.",
            file=sys.stderr,
        )

    _write_lock({
        "holder": holder,
        "pid": os.getpid(),
        "acquired_at": time.time(),
    })
    # Re-read to guard the (small, non-atomic-write) race between two
    # callers acquiring near-simultaneously: last writer wins the file, so
    # confirm we are still the writer we just wrote.
    confirm = _read_lock()
    if confirm and confirm.get("pid") == os.getpid() and confirm.get("holder") == holder:
        return True, None
    return False, confirm


def release(holder="unknown"):
    """Ownership is by HOLDER NAME, not pid. This is intentional: --acquire
    and --release are meant to be usable as two separate CLI invocations
    (e.g. from a shell script wrapping several of its own build steps
    between them), which are two different processes/pids by construction.
    pid is retained in the lock file purely as diagnostic/staleness
    metadata (see acquire()'s stale-lock message), never as the
    authorization check -- that was tried and breaks the primary CLI use
    case (caught by testing, see ORCHESTRATION_TOKEN_STRATEGY.md Part 7)."""
    existing = _read_lock()
    if existing is None:
        return True  # nothing to release
    if existing.get("holder") != holder:
        print(
            f"[WARN] refusing to release lock held by '{existing.get('holder')}' "
            f"(pid {existing.get('pid')}) -- you passed --holder '{holder}'",
            file=sys.stderr,
        )
        return False
    try:
        LOCK_PATH.unlink()
    except OSError:
        pass
    return True


def run_locked(command, holder="build_lock_run", stale_after=DEFAULT_STALE_AFTER,
                wait_seconds=0, poll_interval=5):
    """Acquire, run `command` via shell, always release, return exit code.

    wait_seconds > 0: instead of failing immediately when busy, poll until
    the lock frees up or wait_seconds elapses (useful for a batch
    orchestrator that queued several verify steps and is fine waiting its
    turn rather than treating "someone else is building" as an error).
    """
    deadline = time.time() + wait_seconds
    while True:
        ok, existing = acquire(holder=holder, stale_after=stale_after)
        if ok:
            break
        if time.time() >= deadline:
            print(
                f"[BUSY] build lock held by '{existing.get('holder') if existing else '?'}' "
                f"(pid {existing.get('pid') if existing else '?'}); not acquired within "
                f"{wait_seconds}s, refusing to run concurrently.",
                file=sys.stderr,
            )
            return 3  # distinct exit code: could not acquire, command NOT run
        time.sleep(poll_interval)

    try:
        print(f"[LOCK] acquired by '{holder}' (pid {os.getpid()}); running: {command}")
        result = subprocess.run(command, shell=True)
        return result.returncode
    finally:
        release(holder=holder)
        print(f"[LOCK] released by '{holder}'")


def main():
    parser = argparse.ArgumentParser(description="Serialize build/verify commands (Windows rlib-lock safety)")
    parser.add_argument("--run", help="Command to run while holding the lock")
    parser.add_argument("--acquire", action="store_true", help="Acquire only, exit 0/1")
    parser.add_argument("--release", action="store_true", help="Release only")
    parser.add_argument("--holder", default=os.environ.get("USER", "unknown"), help="Identifier recorded in the lock file")
    parser.add_argument("--stale-after", type=int, default=DEFAULT_STALE_AFTER)
    parser.add_argument("--wait-seconds", type=int, default=0, help="Poll up to N seconds for the lock instead of failing immediately (--run only)")
    args = parser.parse_args()

    if args.run:
        sys.exit(run_locked(args.run, holder=args.holder, stale_after=args.stale_after, wait_seconds=args.wait_seconds))

    if args.acquire:
        ok, existing = acquire(holder=args.holder, stale_after=args.stale_after)
        if ok:
            print(f"[OK] lock acquired by '{args.holder}'")
            sys.exit(0)
        print(f"[BUSY] held by '{existing.get('holder')}' (pid {existing.get('pid')})", file=sys.stderr)
        sys.exit(1)

    if args.release:
        ok = release(holder=args.holder)
        sys.exit(0 if ok else 1)

    parser.print_help()
    sys.exit(1)


if __name__ == "__main__":
    main()
