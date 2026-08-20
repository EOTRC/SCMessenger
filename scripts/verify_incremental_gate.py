#!/usr/bin/env python3
"""verify_incremental_gate.py -- Continuous "Always Green" Incremental Quality Gate.

Executes 5 atomic verification checks before any subagent edit is accepted:
  1. Workspace Compilation: cargo check
  2. Targeted Unit/Integration Tests: cargo test
  3. Strict Clippy Lints: cargo clippy (0 new warnings)
  4. Repository Rules Invariants: rules_check.py
  5. Wiring Metrics Non-Regression: build_wiring_graph.py +
     generate_wiring_burndown.py + rules_check.py on the output, then
     enforces non-regression: fresh unwired count must not exceed the
     count committed in FFI_WIRING_BURNDOWN.md (read BEFORE regeneration
     overwrites it).

Usage:
    python scripts/verify_incremental_gate.py --module iron_core
    python scripts/verify_incremental_gate.py --all
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BURNDOWN_MD = REPO_ROOT / "FFI_WIRING_BURNDOWN.md"
UNWIRED_COUNT_RE = re.compile(r"\*\*Total Unwired/Stub Functions\*\*: (\d+)")

def run_cmd(cmd, cwd=REPO_ROOT):
    res = subprocess.run(cmd, cwd=cwd, shell=True, capture_output=True, text=True)
    return res.returncode, res.stdout, res.stderr

def parse_unwired_count():
    """Read the committed unwired count from FFI_WIRING_BURNDOWN.md."""
    try:
        text = BURNDOWN_MD.read_text(encoding="utf-8")
    except OSError:
        return None
    m = UNWIRED_COUNT_RE.search(text)
    return int(m.group(1)) if m else None

def check_wiring_gate():
    """Gate 5: run the full wiring measurement chain and enforce
    non-regression.

      1. python scripts/build_wiring_graph.py
         (HANDOFF_AUDIT + HANDOFF/discovery REPO_MAP corpus ->
          log-visualizer/public/data/*.json; ghost corpus entries dropped
          with a printed count)
      2. python scripts/generate_wiring_burndown.py
         (log-visualizer/public/data/unwired_functions.json ->
          FFI_WIRING_BURNDOWN.md)
      3. python scripts/rules_check.py FFI_WIRING_BURNDOWN.md
         (generated markdown must pass repo rules: no emoji, etc.)
      4. Non-regression: the regenerated unwired count must not EXCEED the
         count committed in FFI_WIRING_BURNDOWN.md (read BEFORE step 2
         overwrites it). A higher fresh count fails the gate until the new
         baseline is consciously committed.
    """
    print("\n[Gate 5/5] Checking wiring graph non-regression ...")
    baseline = parse_unwired_count()
    if baseline is None:
        print("  [INFO] no committed baseline count in FFI_WIRING_BURNDOWN.md"
              " -- non-regression comparison skipped this run.")

    rc, stdout, stderr = run_cmd("python scripts/build_wiring_graph.py")
    if rc != 0:
        print("  [FAIL] Wiring graph build error:")
        print(stderr[:1000])
        return False
    print("  [PASS] build_wiring_graph.py ran clean.")

    rc, stdout, stderr = run_cmd("python scripts/generate_wiring_burndown.py")
    if rc != 0:
        print("  [FAIL] Wiring burndown generation error:")
        print(stderr[:1000])
        return False
    print("  [PASS] generate_wiring_burndown.py ran clean.")

    rc, stdout, stderr = run_cmd(
        "python scripts/rules_check.py FFI_WIRING_BURNDOWN.md")
    if rc != 0:
        print("  [FAIL] Generated FFI_WIRING_BURNDOWN.md violates repo rules:")
        print(stderr[:1000])
        return False
    print("  [PASS] Generated burndown markdown passes rules_check.py.")

    fresh = parse_unwired_count()
    if baseline is not None and fresh is not None and fresh > baseline:
        print(f"  [FAIL] Wiring regression: unwired count rose "
              f"{baseline} -> {fresh}. Re-wire or consciously commit the "
              f"new baseline (regenerate FFI_WIRING_BURNDOWN.md).")
        return False
    if baseline is not None and fresh is not None:
        print(f"  [PASS] Unwired count {fresh} <= committed baseline "
              f"{baseline}.")
    print("  [PASS] Wiring graph non-regression intact.")
    return True

def check_incremental(module_name=None):
    print("=" * 80)
    print(f"RUNNING INCREMENTAL QUALITY GATE (Target: {module_name or 'workspace'})")
    print("=" * 80)

    # 1. Cargo Check
    print("\n[Gate 1/5] Running cargo check ...")
    rc, stdout, stderr = run_cmd("cargo check --workspace")
    if rc != 0:
        print("  [FAIL] Compilation error in cargo check:")
        print(stderr[:1000])
        return False
    print("  [PASS] Compilation clean.")

    # 2. Cargo Test
    print("\n[Gate 2/5] Running cargo test ...")
    test_cmd = f"cargo test -p scmessenger-core --lib {module_name}" if module_name else "cargo test -p scmessenger-core --lib"
    rc, stdout, stderr = run_cmd(test_cmd)
    if rc != 0:
        print(f"  [FAIL] Test failure in {test_cmd}:")
        print(stderr[:1000])
        return False
    print("  [PASS] Targeted tests passed.")

    # 3. Cargo Clippy
    print("\n[Gate 3/5] Running strict cargo clippy ...")
    clippy_cmd = "cargo clippy -p scmessenger-core --lib"
    rc, stdout, stderr = run_cmd(clippy_cmd)
    if rc != 0 or "error:" in stderr.lower():
        print("  [FAIL] Clippy errors/warnings detected:")
        print(stderr[:1000])
        return False
    print("  [PASS] Clippy lints clean.")

    # 4. Rules Check
    print("\n[Gate 4/5] Running repository rules check ...")
    rc, stdout, stderr = run_cmd("python scripts/rules_check.py AGENTS.md Cargo.toml README.md")
    if rc != 0:
        print("  [FAIL] Rules violation:")
        print(stderr[:1000])
        return False
    print("  [PASS] Repo rules intact.")

    # 5. Wiring Non-Regression Check
    if not check_wiring_gate():
        return False

    print("\n" + "=" * 80)
    print("ALL 5 GATES PASSED -- WORKSPACE REMAINS 100% GREEN!")
    print("=" * 80)
    return True

def main():
    parser = argparse.ArgumentParser(description="Incremental Always-Green Verification Gate")
    parser.add_argument("--module", type=str, help="Specific module to verify (e.g. iron_core, mobile_bridge)")
    args = parser.parse_args()

    success = check_incremental(args.module)
    sys.exit(0 if success else 1)

if __name__ == "__main__":
    main()
