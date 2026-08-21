#!/usr/bin/env bash
# gate.sh -- run a build/gate command with an unfaked exit status.
#
# WHY THIS EXISTS: reading a PIPELINE's exit code as the BUILD's is a
# documented failure mode of this repo (docs/rules/BUILD_AND_CI.md,
# "a piped gate can never fail"; it has been paid for repeatedly --
# files caps, commits caps, `head -8`, `[:6]`, and a coordinator reading
# `tail`'s exit 0 as cargo success twice in one session). A lesson stored
# as prose gets re-learned; this wrapper is the gate.
#
# Usage:
#   scripts/gate.sh <label> <command> [args...]
#
# Behavior:
#   - full output captured to tmp/gate-logs/<label>-<UTC stamp>.log
#   - prints the last 15 lines, the log path, and GATE_EXIT=<real code>
#   - exits with the command's REAL exit code (never the printer's)
#   - refuses labels containing '/' or '..' (log-path injection)
#
# Every cargo/gradle/test/gate invocation runs through this wrapper, or as
# a direct invocation whose exit code is echoed explicitly with full output
# saved. There is no third option.

set -u

if [ "$#" -lt 2 ]; then
    echo "[ERROR] usage: scripts/gate.sh <label> <command> [args...]" >&2
    exit 64
fi

label="$1"
shift

case "$label" in
    */*|*..*|"")
        echo "[ERROR] label must be a flat slug, got: '$label'" >&2
        exit 64
        ;;
esac

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

# Disk preflight: a build/test that starts with no headroom becomes an
# ENOSPC mid-write (2026-08-15: C: hit 100% full mid cargo test; rustc
# failed with "no space on device" and looked exactly like a test
# failure). Refuse build-bearing gates under MIN_FREE_GB and print the
# reclaimables. Non-build commands pass through.
MIN_FREE_GB=8
free_bytes_df="$(df -Pk "$repo_root" 2>/dev/null | awk 'NR==2 {print $4}')"
if [ -n "$free_bytes_df" ] && [ "$free_bytes_df" -lt $((MIN_FREE_GB * 1024 * 1024)) ]; then
    case "$*" in
        *cargo*|*gradle*|*rustc*|*mvn*|*node*test*)
            echo "[ERROR] disk preflight: less than ${MIN_FREE_GB} GB free ($(df -h "$repo_root" | awk 'NR==2 {print $4}'))." >&2
            echo "        Build-bearing gates refuse to start. Reclaim first:" >&2
            echo "        - scripts/clean_target.sh --all   (scoped, keeps generated-sources)" >&2
            echo "        - .scm-shared-target/* partials (shared build dir; only ones you own)" >&2
            echo "        - android/app/build (gradle buildDir, regenerable)" >&2
            exit 69
            ;;
    esac
fi

log_dir="$repo_root/tmp/gate-logs"
mkdir -p "$log_dir"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
log_file="$log_dir/${label}-${stamp}.log"

"$@" > "$log_file" 2>&1
real_exit=$?

echo "---- gate[$label] last 15 lines (full log: $log_file) ----"
tail -15 "$log_file"
echo "---- GATE_EXIT=$real_exit ----"

exit $real_exit
