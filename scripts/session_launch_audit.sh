#!/usr/bin/env bash
# session_launch_audit.sh -- Fast pre-dispatch sanity gate for active sessions.
#
# Audits the local environment prior to work dispatch:
#   1. Disk headroom on C: (RED < 5 GB, AMBER < 15 GB)
#   2. Concurrent build tools running (cargo.exe, gradle, java.exe)
#   3. Git worktree count and dirty worktrees
#   4. Shared checkout divergence from origin/main (commits behind)
#   5. Lane health via scripts/lane_probe.py (RED on EMPTY content or probe failure)
#   6. GitHub CLI auth status and open PR count
#
# Prints a structured table and exits non-zero if ANY check is RED.
# Every RED check outputs an actionable remedy on the same line.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

RED_COUNT=0
AMBER_COUNT=0

# Formatted table line helper
# Args: <CHECK_NAME> <STATUS> <DETAILS>
print_table_row() {
    local check_name="$1"
    local status="$2"
    local details="$3"
    printf "%-26s | %-9s | %s\n" "$check_name" "$status" "$details"
}

echo "=========================================================================================="
echo "                           SCM SESSION LAUNCH AUDIT GATE                                  "
echo "=========================================================================================="
printf "%-26s | %-9s | %s\n" "CHECK" "STATUS" "DETAILS / REMEDY"
echo "---------------------------+-----------+--------------------------------------------------"

# -----------------------------------------------------------------------------
# Check 1: Disk Headroom on C:
# -----------------------------------------------------------------------------
FREE_KB=$(df -k /c 2>/dev/null | awk 'NR==2 {print $4}' || echo "0")
if [ -z "$FREE_KB" ] || [ "$FREE_KB" -eq 0 ]; then
    # Fallback to repo root filesystem if /c is not directly mounted
    FREE_KB=$(df -k "$REPO_ROOT" 2>/dev/null | awk 'NR==2 {print $4}' || echo "0")
fi

FREE_GB_INT=$((FREE_KB / 1024 / 1024))
FREE_GB_FLOAT=$(awk "BEGIN {printf \"%.1f\", $FREE_KB / 1024 / 1024}" 2>/dev/null || echo "${FREE_GB_INT}.0")

if [ "$FREE_GB_INT" -lt 5 ]; then
    RED_COUNT=$((RED_COUNT + 1))
    print_table_row "Disk Headroom (C:)" "[FAIL]" "${FREE_GB_FLOAT} GB free (< 5 GB limit) -- Remedy: run 'cargo clean' or remove stale target/tmp files on C:"
elif [ "$FREE_GB_INT" -lt 15 ]; then
    AMBER_COUNT=$((AMBER_COUNT + 1))
    print_table_row "Disk Headroom (C:)" "[WARNING]" "${FREE_GB_FLOAT} GB free (tight headroom < 15 GB) -- Caution: gate sweeps grow target/ by ~40 GB"
else
    print_table_row "Disk Headroom (C:)" "[OK]" "${FREE_GB_FLOAT} GB free (sufficient headroom)"
fi

# -----------------------------------------------------------------------------
# Check 2: Concurrent Build Tools
# -----------------------------------------------------------------------------
RUNNING_BUILDS=""
TASKLIST_OUT=$(tasklist.exe 2>/dev/null || echo "")
if [ -n "$TASKLIST_OUT" ]; then
    MATCHES=$(echo "$TASKLIST_OUT" | grep -iE '(^|[[:space:]])(cargo\.exe|gradle|java\.exe|rustc\.exe)($|[[:space:]])' | awk '{print $1}' | sort -u | tr '\n' ' ' | sed 's/[[:space:]]*$//')
    if [ -n "$MATCHES" ]; then
        RUNNING_BUILDS="$MATCHES"
    fi
fi

if [ -n "$RUNNING_BUILDS" ]; then
    RED_COUNT=$((RED_COUNT + 1))
    print_table_row "Concurrent Build Tools" "[FAIL]" "Active build processes: ${RUNNING_BUILDS} -- Remedy: wait for builds to finish or run 'taskkill //F //IM <name>'"
else
    print_table_row "Concurrent Build Tools" "[OK]" "No concurrent cargo, gradle, or java processes running"
fi

# -----------------------------------------------------------------------------
# Check 3: Git Worktree Count & Dirty Worktrees
# -----------------------------------------------------------------------------
WORKTREE_OUTPUT=$(git worktree list 2>/dev/null || echo "")
WT_TOTAL=0
DIRTY_WTS=()

if [ -n "$WORKTREE_OUTPUT" ]; then
    while IFS= read -r wt_line; do
        [ -z "$wt_line" ] && continue
        WT_TOTAL=$((WT_TOTAL + 1))
        wt_path=$(echo "$wt_line" | awk '{print $1}')
        if [ -d "$wt_path" ]; then
            dirty_count=$(git -C "$wt_path" status --porcelain 2>/dev/null | wc -l)
            if [ "$dirty_count" -gt 0 ]; then
                wt_name="$(basename "$wt_path")"
                DIRTY_WTS+=("${wt_name} (${dirty_count} files)")
            fi
        fi
    done <<< "$WORKTREE_OUTPUT"
fi

if [ "${#DIRTY_WTS[@]}" -gt 0 ]; then
    AMBER_COUNT=$((AMBER_COUNT + 1))
    DIRTY_STR="${DIRTY_WTS[*]}"
    print_table_row "Git Worktrees" "[WARNING]" "${WT_TOTAL} total worktrees; ${#DIRTY_WTS[@]} dirty: ${DIRTY_STR}"
else
    print_table_row "Git Worktrees" "[OK]" "${WT_TOTAL} total worktrees, all clean"
fi

# -----------------------------------------------------------------------------
# Check 4: Shared Checkout Divergence vs origin/main
# -----------------------------------------------------------------------------
SHARED_PATH=$(git worktree list 2>/dev/null | head -n 1 | awk '{print $1}')
BEHIND_COUNT=0
SHARED_VALID=0

if [ -n "$SHARED_PATH" ] && [ -d "$SHARED_PATH" ]; then
    SHARED_HEAD=$(git -C "$SHARED_PATH" rev-parse HEAD 2>/dev/null || echo "")
    ORIGIN_MAIN=$(git rev-parse origin/main 2>/dev/null || echo "")
    if [ -n "$SHARED_HEAD" ] && [ -n "$ORIGIN_MAIN" ]; then
        SHARED_VALID=1
        BEHIND_COUNT=$(git -C "$SHARED_PATH" rev-list --count "${SHARED_HEAD}..origin/main" 2>/dev/null || echo "0")
    fi
fi

if [ "$SHARED_VALID" -eq 0 ]; then
    RED_COUNT=$((RED_COUNT + 1))
    print_table_row "Shared Checkout Sync" "[FAIL]" "Unable to inspect shared checkout at '${SHARED_PATH}' -- Remedy: check repository worktree structure"
elif [ "$BEHIND_COUNT" -gt 0 ]; then
    RED_COUNT=$((RED_COUNT + 1))
    print_table_row "Shared Checkout Sync" "[FAIL]" "Shared checkout is ${BEHIND_COUNT} commit(s) behind origin/main -- Remedy: run 'git -C \"${SHARED_PATH}\" pull --ff-only origin main'"
else
    print_table_row "Shared Checkout Sync" "[OK]" "Shared checkout is up to date with origin/main"
fi

# -----------------------------------------------------------------------------
# Check 5: Lane Health via scripts/lane_probe.py
# -----------------------------------------------------------------------------
LANE_PROBE_SCRIPT="$REPO_ROOT/scripts/lane_probe.py"
if [ ! -f "$LANE_PROBE_SCRIPT" ]; then
    RED_COUNT=$((RED_COUNT + 1))
    print_table_row "Lane Health Probe" "[FAIL]" "scripts/lane_probe.py not found -- Remedy: restore scripts/lane_probe.py"
else
    LANE_OUT_TMP="$(mktemp "$REPO_ROOT/tmp/lane_probe_XXXXXX.out" 2>/dev/null || echo "$REPO_ROOT/tmp/lane_probe_$$.out")"
    mkdir -p "$(dirname "$LANE_OUT_TMP")"
    python3 "$LANE_PROBE_SCRIPT" > "$LANE_OUT_TMP" 2>&1
    LANE_STATUS=$?

    LANE_OUT="$(cat "$LANE_OUT_TMP" 2>/dev/null || echo "")"
    rm -f "$LANE_OUT_TMP"

    EMPTY_LANES=()
    LIVE_COUNT=0

    while IFS= read -r line; do
        if echo "$line" | grep -q "[[:space:]]EMPTY[[:space:]]"; then
            lane_id=$(echo "$line" | awk '{print $1}')
            EMPTY_LANES+=("$lane_id")
        elif echo "$line" | grep -q "HTTP lanes live"; then
            LIVE_COUNT=$(echo "$line" | awk '{print $1}')
        fi
    done <<< "$LANE_OUT"

    if [ "${#EMPTY_LANES[@]}" -gt 0 ]; then
        RED_COUNT=$((RED_COUNT + 1))
        EMPTY_STR="${EMPTY_LANES[*]}"
        print_table_row "Lane Health Probe" "[FAIL]" "Lane(s) returned EMPTY content: ${EMPTY_STR} -- Remedy: adjust reasoning knob in scripts/lanes.json or disable thinking"
    elif [ "$LANE_STATUS" -ne 0 ]; then
        RED_COUNT=$((RED_COUNT + 1))
        print_table_row "Lane Health Probe" "[FAIL]" "Lane probe failed with exit code ${LANE_STATUS} -- Remedy: verify API keys and network connectivity"
    elif [ "$LIVE_COUNT" -eq 0 ]; then
        RED_COUNT=$((RED_COUNT + 1))
        print_table_row "Lane Health Probe" "[FAIL]" "0 HTTP lanes reported live -- Remedy: check API keys in ~/.config/scmorc/"
    else
        print_table_row "Lane Health Probe" "[OK]" "${LIVE_COUNT} HTTP lanes live, zero empty responses"
    fi
fi

# -----------------------------------------------------------------------------
# Check 6: GitHub CLI Auth & Open PRs
# -----------------------------------------------------------------------------
GH_AUTH_OK=0
if gh auth status >/dev/null 2>&1; then
    GH_AUTH_OK=1
fi

if [ "$GH_AUTH_OK" -eq 0 ]; then
    RED_COUNT=$((RED_COUNT + 1))
    print_table_row "GitHub CLI & PRs" "[FAIL]" "gh auth status failed or unreachable -- Remedy: run 'gh auth login' or refresh GitHub credentials"
else
    OPEN_PR_COUNT=$(gh pr list --state open --limit 100 --json number --jq 'length' 2>/dev/null || echo "?")
    print_table_row "GitHub CLI & PRs" "[OK]" "Authenticated to GitHub; ${OPEN_PR_COUNT} open PR(s)"
fi

echo "=========================================================================================="

if [ "$RED_COUNT" -gt 0 ]; then
    echo "[FAIL] Launch audit failed: ${RED_COUNT} RED check(s), ${AMBER_COUNT} WARNING(s)."
    echo "       Resolve blocking issues before dispatching work."
    exit 1
elif [ "$AMBER_COUNT" -gt 0 ]; then
    echo "[WARNING] Launch audit passed with ${AMBER_COUNT} WARNING(s). Ready for dispatch."
    exit 0
else
    echo "[OK] Launch audit passed completely. Ready for dispatch."
    exit 0
fi
