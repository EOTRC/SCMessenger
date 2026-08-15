#!/usr/bin/env bash
# triage_lane.sh -- first moves on a red CI lane, in the order that costs least.
#
# WHY THIS EXISTS
# On 2026-08-15 a diagnosis burned a large share of a session because it started
# from a plausible hypothesis (task ordering) instead of from lane history. One
# `gh run list` would have shown the lane passed on 08-13 and failed on 08-14 --
# a regression with a three-commit blast radius -- before any code was written.
# Two wrong fixes were pushed first.
#
# The lesson is not "remember to check history". Prose lessons decay; this repo
# has ~222k lines of them and still repeats mistakes. So the lesson is a script,
# and the rule is: run this BEFORE forming a hypothesis about why a lane is red.
#
#   scripts/triage_lane.sh mobile.yml
#   scripts/triage_lane.sh ci.yml 20
#
# Read-only. Runs no builds, changes nothing.

set -uo pipefail

WF="${1:-}"
LIMIT="${2:-14}"
REPO="Sovereign-Communication/SCMessenger"

if [ -z "$WF" ]; then
  echo "usage: scripts/triage_lane.sh <workflow-file> [limit]"
  echo "workflows:"
  ls .github/workflows/*.yml 2>/dev/null | xargs -n1 basename | sed 's/^/  /'
  exit 2
fi

echo "=============================================================="
echo " 1. LANE HISTORY -- did this EVER pass? when did it break?"
echo "=============================================================="
# Never read \$? after a pipe; capture to a file and test that instead.
gh run list --workflow="$WF" -L "$LIMIT" \
   --json conclusion,headBranch,headSha,createdAt \
   --jq '.[] | select(.conclusion != "") | "\(.conclusion)\t\(.createdAt[0:16])\t\(.headSha[0:9])\t\(.headBranch)"' \
   > /tmp/_lane_hist.txt 2>/dev/null
if [ ! -s /tmp/_lane_hist.txt ]; then
  echo "  (no completed runs found -- check the workflow filename)"
  exit 1
fi
cat /tmp/_lane_hist.txt

# Find the most recent success and the oldest failure newer than it. Rows are
# newest-first, so walk down to the first success and take the row above it.
LAST_PASS=$(awk -F'\t' '$1=="success"{print $3; exit}' /tmp/_lane_hist.txt)
FIRST_FAIL=$(awk -F'\t' '$1=="success"{exit} $1=="failure"{sha=$3} END{print sha}' /tmp/_lane_hist.txt)

echo
echo "=============================================================="
echo " 2. REGRESSION WINDOW"
echo "=============================================================="
if [ -z "${LAST_PASS:-}" ]; then
  echo "  NEVER PASSED in the last $LIMIT runs."
  echo "  -> This is not a regression. Do not go looking for a breaking commit."
  echo "  -> Treat it as never-worked: read the job log directly (step 4)."
elif [ -z "${FIRST_FAIL:-}" ]; then
  echo "  No failure newer than the last success ($LAST_PASS). Lane looks healthy."
  exit 0
else
  echo "  last PASS : $LAST_PASS"
  echo "  first FAIL: $FIRST_FAIL"
  echo
  echo "  -- commits in the window --"
  git log --oneline "${LAST_PASS}..${FIRST_FAIL}" 2>/dev/null | sed 's/^/    /' \
    || echo "    (SHAs not fetched locally: git fetch --all)"
  echo
  echo "  -- files changed in the window --"
  git diff --stat "${LAST_PASS}..${FIRST_FAIL}" 2>/dev/null | tail -25 | sed 's/^/    /' \
    || echo "    (SHAs not fetched locally)"
  echo
  echo "  -> Your hypothesis MUST explain something in this window."
  echo "  -> If it explains a condition that predates ${LAST_PASS}, it is wrong,"
  echo "     because the lane was green under that same condition."
fi

echo
echo "=============================================================="
echo " 3. WHAT IS RED RIGHT NOW"
echo "=============================================================="
gh run list --workflow="$WF" -L 1 --json databaseId --jq '.[0].databaseId' > /tmp/_lane_run.txt 2>/dev/null
RUN_ID=$(cat /tmp/_lane_run.txt 2>/dev/null)
if [ -n "${RUN_ID:-}" ]; then
  gh run view "$RUN_ID" --json jobs \
     --jq '.jobs[] | select(.conclusion=="failure") | "  FAILED  \(.name)\n          job \(.databaseId)"' 2>/dev/null
  echo
  echo "  4. THEN read the log -- and find the FIRST failing task, not the loudest:"
  echo "       gh api repos/$REPO/actions/jobs/<job-id>/logs > /tmp/j.txt"
  echo "       grep -nE '^e: |error\[|FAILED|What went wrong' /tmp/j.txt | head -30"
  echo
  echo "  A later error is often downstream of an earlier one. KSP reporting"
  echo "  'error.NonExistentClass' means a type was not produced -- that is a"
  echo "  symptom, and the cause is above it or in the generating task."
fi
