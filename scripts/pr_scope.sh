#!/usr/bin/env bash
# pr_scope.sh -- answer "unless there's a reason not to?" for a pull request.
#
# WHY
# On 2026-08-15 PR #150 was described as "tooling-only, zero build risk" from
# memory of what had been authored in it. The operator said "merge it! (unless
# there's a reason not to?)". There were three:
#   - it was branched off `tracking` but aimed at `main`, so its diff was 100
#     commits / +17k lines -- effectively all of PR #139, which would have
#     merged sideways under a commit message about delegation scripts
#   - two required Android checks were FAILING
#   - it touched core/src/crypto and core/src/transport, which AGENTS.md rule 8
#     holds merge-blocked pending adversarial review
# All three were visible in one `gh pr view --json files`. None were visible in
# the author's recollection.
#
# So the question stopped being rhetorical and became this script. Run it before
# every merge. It prints reasons NOT to, or says there are none.
#
#   scripts/pr_scope.sh 150
#
# Exit 0 = no blockers found. Exit 1 = at least one blocker. Read-only.

set -uo pipefail
PR="${1:-}"
REPO="Sovereign-Communication/SCMessenger"

if [ -z "$PR" ]; then
  echo "usage: scripts/pr_scope.sh <pr-number>"
  exit 2
fi

J=$(gh pr view "$PR" --json title,state,baseRefName,headRefName,mergeable,mergeStateStatus,additions,deletions,files,commits 2>/dev/null)
if [ -z "$J" ]; then
  echo "[FAIL] could not read PR $PR"
  exit 2
fi

BLOCKERS=0
note() { echo "  [BLOCKER] $*"; BLOCKERS=$((BLOCKERS+1)); }
ok()   { echo "  [OK]      $*"; }

echo "=============================================================="
echo " PR #$PR  $(echo "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["title"])')"
echo "=============================================================="
# NOTE: no f-strings with quoted keys here. Inside a single-quoted shell string,
# escaped quotes inside an f-string become a SyntaxError. Cost two runs to learn
# (this script and scripts/agy_stream_watch.py). Use %-formatting.
echo "$J" | python3 -c '
import json, sys
d = json.load(sys.stdin)
print("  state       : %s  mergeable=%s  %s" % (d["state"], d["mergeable"], d["mergeStateStatus"]))
print("  base <- head: %s <- %s" % (d["baseRefName"], d["headRefName"]))
print("  size        : +%d/-%d across %d files, %d commits" % (
    d["additions"], d["deletions"], len(d["files"]), len(d["commits"])))
'
echo
echo "-- reasons not to merge --"

# 1. Scope: a PR far larger than its title suggests is usually mis-based.
FILES=$(echo "$J" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)["files"]))')
COMMITS=$(echo "$J" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)["commits"]))')
if [ "$COMMITS" -gt 20 ]; then
  note "$COMMITS commits. Is this branch based on the branch you are merging INTO?"
  note "  Check: git log --oneline <base>..<head> | wc -l"
else
  ok "$COMMITS commits, $FILES files -- scope is reviewable"
fi

# 2. Merge-blocked directories (AGENTS.md rule 8).
GATED=$(echo "$J" | python3 -c '
import json,sys,re
d=json.load(sys.stdin)
pat=re.compile(r"^core/src/(crypto|transport|routing|privacy)/")
hits=[f["path"] for f in d["files"] if pat.match(f["path"])]
print("\n".join(hits))
')
if [ -n "$GATED" ]; then
  note "touches merge-blocked directories (AGENTS.md rule 8):"
  echo "$GATED" | head -8 | sed 's/^/              /'
  note "  requires a crypto-security-auditor verdict before merge"
else
  ok "clear of core/src/{crypto,transport,routing,privacy}"
fi

# 3. Check state. FAILS CLOSED.
#
# The first version of this block wrote to /tmp and read it back with python3.
# Git Bash on Windows maps /tmp; python3 does not, so the read raised, the
# counts came back as empty strings, `${PENDING:-0}` defaulted to 0, and the
# whole thing fell through to "all checks green" while five of six checks were
# still IN_PROGRESS. A silent failure that produces a false PASS is worse than
# no check at all, in a script whose only job is to stop a bad merge.
# Two fixes: repo-local tmp/ per AGENTS.md rule 2, and no path where an error
# can be mistaken for success.
mkdir -p "$(git rev-parse --show-toplevel)/tmp"
CHECKS="$(git rev-parse --show-toplevel)/tmp/_prchecks_${PR}.json"
if ! gh pr checks "$PR" --json name,state > "$CHECKS" 2>/dev/null || [ ! -s "$CHECKS" ]; then
  note "could not read checks -- treating as a blocker, not as green"
else
  SUMMARY=$(python3 - "$CHECKS" <<'PYEOF' 2>&1
import json, sys
try:
    d = json.load(open(sys.argv[1], encoding="utf-8"))
except Exception as e:
    print("ERROR %s" % e)
    raise SystemExit(0)
failed = [c["name"] for c in d if c.get("state") == "FAILURE"]
busy = [c["name"] for c in d if c.get("state") in ("IN_PROGRESS", "QUEUED", "PENDING")]
if failed:
    print("FAILED %s" % ", ".join(failed[:6]))
elif busy:
    print("BUSY %d %s" % (len(busy), ", ".join(busy[:4])))
elif not d:
    print("ERROR no checks reported")
else:
    print("GREEN %d" % len(d))
PYEOF
)
  case "$SUMMARY" in
    GREEN*)  ok "all ${SUMMARY#GREEN } checks green" ;;
    BUSY*)   note "checks still running: ${SUMMARY#BUSY } -- not green YET" ;;
    FAILED*) note "failing checks: ${SUMMARY#FAILED }" ;;
    *)       note "check state unreadable ($SUMMARY) -- treating as a blocker" ;;
  esac
fi

# 4. Mergeability.
MG=$(echo "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["mergeable"])')
[ "$MG" = "MERGEABLE" ] && ok "no conflicts" || note "mergeable=$MG"

echo
if [ "$BLOCKERS" -eq 0 ]; then
  echo "[OK] no reasons not to merge were found."
  exit 0
fi
echo "[STOP] $BLOCKERS reason(s) not to merge. Resolve or get an explicit operator"
echo "       decision naming each one. A 'yes' given before these were surfaced"
echo "       was not informed consent."
exit 1
