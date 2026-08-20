#!/usr/bin/env python3
"""generate_wiring_burndown.py -- Generates FFI_WIRING_BURNDOWN.md from unwired_functions.json.

Reads the metrics outputs under log-visualizer/public/data/ (written by
build_wiring_graph.py). Generated markdown must pass scripts/rules_check.py:
no emoji, no trailing whitespace.
"""

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
UNWIRED_JSON = REPO_ROOT / "log-visualizer" / "public" / "data" / "unwired_functions.json"
WIRING_JSON = REPO_ROOT / "log-visualizer" / "public" / "data" / "wiring_graph.json"  # sibling, informational
OUT_MD = REPO_ROOT / "FFI_WIRING_BURNDOWN.md"

def main():
    if not UNWIRED_JSON.exists():
        print(f"Error: {UNWIRED_JSON} not found.")
        sys.exit(1)

    with open(UNWIRED_JSON, "r", encoding="utf-8") as f:
        data = json.load(f)

    functions = data.get("functions", [])

    # Ghost re-verification: even though build_wiring_graph.py filters
    # records for removed paths, this committed JSON can be older than the
    # tree. Drop entries whose file is gone and PRINT the dropped count and
    # list (reduced confidence is expressed by printing MORE, never less).
    ghosts = [fn for fn in functions if not (REPO_ROOT / fn.get("file", "")).exists()]
    if ghosts:
        ghost_files = sorted(set(g["file"] for g in ghosts))
        print(f"[GHOST-FILTER] dropped {len(ghosts)} unwired entries in "
              f"{len(ghost_files)} files that no longer exist in this tree:")
        for gf in ghost_files:
            n = sum(1 for g in ghosts if g["file"] == gf)
            print(f"  [GHOST-DROPPED] {gf} ({n} entries)")
    functions = [fn for fn in functions if fn not in ghosts]

    # Categorize functions
    stubs = [fn for fn in functions if fn.get("is_stub")]
    unwired = [fn for fn in functions if not fn.get("is_stub")]

    # Group by file/module
    by_module = {}
    for fn in functions:
        mod = fn.get("file", "unknown")
        by_module.setdefault(mod, []).append(fn)

    # Sort modules by count descending
    sorted_mods = sorted(by_module.items(), key=lambda x: len(x[1]), reverse=True)

    meta = data.get("meta", {})
    lines = []
    lines.append("# SCMessenger FFI & Function Wiring Burndown Matrix")
    lines.append("")
    lines.append(f"**Generated**: {meta.get('generated_at', 'unknown')}")
    lines.append(f"**Total Unwired/Stub Functions**: {len(functions)} (Unwired: {len(unwired)}, Stubs: {len(stubs)})")
    lines.append(f"**Corpus**: {', '.join(meta.get('inputs', ['HANDOFF_AUDIT/REPO_MAP.jsonl', 'HANDOFF/discovery/REPO_MAP.jsonl']))}")
    lines.append(f"**Ghost entries filtered**: {meta.get('ghost_files_dropped', 'unknown')} corpus files"
                 f" ({meta.get('ghost_functions_dropped', 'unknown')} functions) removed from the tree"
                 f" by build_wiring_graph.py; {len(ghosts)} additional stale entries dropped at"
                 f" generation time")
    lines.append("")
    lines.append("## Overview & Burndown Priorities")
    lines.append("")
    lines.append("This document tracks unwired and stubbed interface functions across **Rust Core**, **Mobile UniFFI**, **Android Kotlin**, and **iOS Swift**.")
    lines.append("")

    lines.append("### High-Priority Stub Implementations (Must be implemented for Phase 4)")
    lines.append("| Function | Location | Line | Target Integration Layer |")
    lines.append("| :--- | :--- | :---: | :--- |")
    if not stubs:
        lines.append("| (none -- no stubs flagged by the discovery overlay in the surviving corpus) | -- | -- | -- |")
    for fn in stubs[:25]:
        target = "Android/iOS Mobile Bridge" if "mobile" in fn["file"].lower() or "android" in fn["file"].lower() or "ios" in fn["file"].lower() else "Rust Core"
        lines.append(f"| `{fn['name']}` | `{fn['file']}` | {fn.get('line', 0)} | {target} |")

    lines.append("")
    lines.append("### Module Breakdown (Top Modules by Unwired Count)")
    lines.append("| Module / File | Total Unwired | Stubs | Status |")
    lines.append("| :--- | :---: | :---: | :--- |")
    for mod, fns in sorted_mods[:20]:
        stub_cnt = sum(1 for f in fns if f.get("is_stub"))
        lines.append(f"| `{mod}` | {len(fns)} | {stub_cnt} | Pending Audit |")

    lines.append("")
    lines.append("## Action Plan for Burndown")
    lines.append("1. **Mobile UniFFI Surface**: Wire core transport stubs (`MobileBridge`, `CoreBridge.swift`) to active Kotlin/Swift view models.")
    lines.append("2. **Observed Stubs**: Replace simulated mock channels with production libp2p and sled store calls.")
    lines.append("3. **Dead Code Clearance**: Remove unreferenced diagnostic helpers that are obsolete.")

    text = "\n".join(lines)
    if re.search(r"[ \t]+$", text, re.M):
        print("[ERROR] internal: generated markdown would contain trailing whitespace")
        sys.exit(1)
    with open(OUT_MD, "w", encoding="utf-8", newline="\n") as f:
        f.write(text + "\n")

    print(f"Generated {OUT_MD} with {len(functions)} functions cataloged "
          f"across {len(sorted_mods)} modules "
          f"({len(ghosts)} ghost entries dropped, printed above).")

if __name__ == "__main__":
    main()
