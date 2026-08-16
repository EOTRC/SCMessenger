#!/usr/bin/env bash
# agy_run.sh -- dispatch to agy with LIVE progress and stall detection.
#
# WHY
# agy's default `-p` print mode buffers everything until the run ends. A long
# task therefore looks identical to a hung one: an empty output file, for
# fifteen minutes. On 2026-08-15 two orchestrator tests were scored as
# capability failures on exactly that evidence. Both were wrong -- the lane was
# healthy (a trivial prompt answered in 3.9s) and the real cause was a total
# --print-timeout too short for a six-step chain.
#
# agy already emits structured events; nothing was watching them. This is that
# watcher. It is a wrapper, not an agent -- spending model tokens to watch a
# process that already prints its own progress would be silly.
#
#   scripts/agy_run.sh <model> <timeout> <prompt-file> [log-dir]
#   scripts/agy_run.sh gemini-3.7-flash-low 90m tmp/orch_test/ORCH_PROMPT.txt
#
# Prints one line per step as it happens, then a summary with token usage.
# Exits 0 on SUCCESS, 1 on failure/stall.

set -uo pipefail
REPO="$(git rev-parse --show-toplevel)"
AGY="${AGY:-${LOCALAPPDATA}/agy/bin/agy.exe}"

MODEL="${1:-}"; TIMEOUT="${2:-90m}"; PROMPT_FILE="${3:-}"; LOGDIR="${4:-$REPO/tmp/agy}"
if [ -z "$MODEL" ] || [ -z "$PROMPT_FILE" ] || [ ! -f "$PROMPT_FILE" ]; then
  echo "usage: $0 <model> <timeout> <prompt-file> [log-dir]"
  echo "models: run '$AGY models' (exact names only -- shorthand silently substitutes)"
  exit 2
fi

parse_duration_seconds() {
  local dur="$1"
  local total=0
  if [[ "$dur" =~ ^[0-9]+$ ]]; then
    echo "$dur"
    return
  fi
  local rem="$dur"
  if [[ "$rem" =~ ([0-9]+)h ]]; then
    local hours="${BASH_REMATCH[1]}"
    total=$((total + hours * 3600))
    rem="${rem/${hours}h/}"
  fi
  if [[ "$rem" =~ ([0-9]+)m ]]; then
    local mins="${BASH_REMATCH[1]}"
    total=$((total + mins * 60))
    rem="${rem/${mins}m/}"
  fi
  if [[ "$rem" =~ ([0-9]+)s ]]; then
    local secs="${BASH_REMATCH[1]}"
    total=$((total + secs))
    rem="${rem/${secs}s/}"
  fi
  echo "$total"
}

# Warn if timeout is below 90m floor for build-bearing tasks
DUR_SECS=$(parse_duration_seconds "$TIMEOUT")
if [ "$DUR_SECS" -gt 0 ] && [ "$DUR_SECS" -lt 5400 ]; then
  if grep -qE "cargo test|cargo build|cargo clippy|gradlew|assembleDebug" "$PROMPT_FILE"; then
    echo "[WARNING] timeout $TIMEOUT is below the 90m floor for build-bearing tasks" >&2
  fi
fi

mkdir -p "$LOGDIR"
STAMP=$(git rev-parse --short HEAD)
RAW="$LOGDIR/agy_${MODEL}_${STAMP}.jsonl"

echo "[INFO] model=$MODEL timeout=$TIMEOUT prompt=$PROMPT_FILE"
echo "[INFO] raw event log: $RAW"
echo

# --add-dir is mandatory: without it agy re-discovers the repo every dispatch
# and frequently bails, which reads as a random timeout.
"$AGY" --add-dir "$REPO" \
       --model "$MODEL" \
       --dangerously-skip-permissions \
       --print-timeout "$TIMEOUT" \
       --output-format stream-json \
       -p "$(cat "$PROMPT_FILE")" 2>&1 | tee "$RAW" | python3 -u "$REPO/scripts/agy_stream_watch.py"
