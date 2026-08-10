#!/usr/bin/env bash
# pr_comment_watch.sh -- poll a GitHub PR for new comments and append them to a
# log, so a lane that is busy building or soaking does not miss cross-lane
# traffic. Read-only: it never posts, never merges, never mutates the PR.
#
# Usage: scripts/pr_comment_watch.sh <pr-number> [interval-seconds] [log-path]
#
# State: <log-path>.seen holds the createdAt of the newest comment already
# recorded, so a restart does not re-log the whole thread (PR 139 has 149).

set -uo pipefail

PR="${1:?usage: pr_comment_watch.sh <pr-number> [interval] [log]}"
INTERVAL="${2:-120}"
LOG="${3:-tmp/logs/pr${PR}_comments.log}"
SEEN="${LOG}.seen"

mkdir -p "$(dirname "$LOG")"
touch "$LOG"
[ -f "$SEEN" ] || echo "1970-01-01T00:00:00Z" > "$SEEN"

echo "[INFO] watching PR #${PR} every ${INTERVAL}s -> ${LOG}" | tee -a "$LOG"

while true; do
    last_seen="$(cat "$SEEN" 2>/dev/null || echo '1970-01-01T00:00:00Z')"

    # One API call per cycle. jq filters server-side via --jq so only genuinely
    # new comments cross the boundary.
    new_json="$(gh pr view "$PR" --json comments \
        --jq "[.comments[] | select(.createdAt > \"${last_seen}\")]" 2>/dev/null)"

    if [ -n "$new_json" ] && [ "$new_json" != "[]" ]; then
        count="$(echo "$new_json" | jq 'length' 2>/dev/null || echo 0)"
        if [ "${count:-0}" -gt 0 ]; then
            echo "$new_json" | jq -r '.[] | "=====[" + .createdAt + "] " + .author.login + "=====\n" + .body' >> "$LOG"
            newest="$(echo "$new_json" | jq -r 'sort_by(.createdAt) | last | .createdAt')"
            echo "$newest" > "$SEEN"
            echo "[NEW] ${count} comment(s) on PR #${PR}, newest ${newest}"
        fi
    fi

    sleep "$INTERVAL"
done
