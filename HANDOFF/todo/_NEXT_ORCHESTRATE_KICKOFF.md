# Next /orchestrate Kickoff -- Post-PR-136 Wave (Field Parity + v0.4.0 Gates)

Status: Active -- Windows lane orchestrator session running
Last updated: 2026-08-09 (Windows lane resumed; see session block below)

## SESSION 2026-08-09 (Windows lane) -- live state, read this first

ANCHOR: re-anchoring this lane on **`49bc3f56`** (PR 139 head; runtime code
`acda09df`). Announced on PR 139 so both lanes produce comparable evidence.
Do not move the anchor without saying so on the PR first.

TRAP THAT NEARLY COST A RUN: this checkout was sitting 14 commits behind at
`cfd3624a`, which predates BOTH `4083e59b` (five-node runtime gate, per-peer
connection cap 4 -> 2) and `acda09df` (malformed contact key). Always check
`git rev-list --count HEAD..origin/<branch>` before building soak evidence.

TWO DISTINCT PANIC CLASSES -- do not conflate them:
- Class A `libp2p-upnp behaviour.rs:497` "mapping should exist". Windows runs
  on `6cb7033a`: 5m20s and 16m42s, 2 of 2. Fixed by `21382b8a` (upnp source
  removed); the fix IS in the current anchor.
- Class B `libp2p request-response` during multi-path convergence. Addressed
  by `4083e59b` (`with_max_established_per_peer` 4 -> 2). Ticket:
  `HANDOFF/todo/PANIC_libp2p_request_response_duplicate_disconnect.md`.
The Mac lane has been asked to grep the panic SITE in its 13m23s capture to
say which class it was.

FLEET FACTS verified this session (not inferred):
- AWS relay LIVE: `http://54.226.67.101:9876/health` -> `{"status":"healthy"}`.
  Relay PeerId `12D3KooWPJK6KgKsafefLWeGs4kVbj7wBnU67yKe88ni3FHZ3Hr2`,
  bootstrap `/ip4/54.226.67.101/tcp/9001/p2p/<that id>`.
- Windows node PeerId `12D3KooWD6vZQrUqpyGaCqY3tNSK8p44BS78TvxpGpwhdPJ1T9mw`,
  data dir `%LOCALAPPDATA%\scmessenger\` survives rebuilds (storage/db,
  ledger.json, peers.json ~920 KB, relay_custody/).
- FIXED the dead bootstrap the P0 ticket flagged: config
  `%APPDATA%\scmessenger\config.json` had ONLY
  `/ip4/127.0.0.1/tcp/19001/...` which nothing serves. Replaced with the live
  relay multiaddr; timestamped `.bak` written alongside.
- macOS node `12D3KooWNC5rEK...` IS present in our ledger, reachable via
  `/p2p-circuit` through the relay -- relevant to PR 139 question 1.
- LAN fleet seen in ledger: 192.168.0.139/140/141/142 plus .111/.131.

HOOKS RUNNING: `scripts/pr_comment_watch.sh 139` (read-only, 120s) appending
to `tmp/logs/pr139_comments.log`.

DISK: was 98% / 6.6 GB free and blocking build gates. Reclaimed 15.2 GB by
deleting `scm-winlane/target` (operator-authorised; worktree was clean, no
work product lost) and pruned two dead worktree registrations. Now 22 GB
free. NOTE: the P1 prune ticket's premise is measurably wrong -- agent data
is only ~2.3 GB total; the real bloat is stale `target/` in sibling
worktrees. `scm-review-8621a4b5/target` (2.7 GB) is still there and still
clean if more space is needed.

OPEN / NEXT: build at the anchor, start the CLI node (inbound `/api/*` on
127.0.0.1), soak it, and report uptime + panic SITE + peak concurrent
connections per peer. A single clean run is NOT proof (`d48558a8` ran 9h32m
and still told us little).

## Original 2026-08-05 kickoff follows

## What changed since the previous kickoff (verified, not doc-claimed)

- PHASE 0 CLOSED: PR #136 (identity canonicalization + block gate) merged
  green at 68ef6256; post-merge CI failure (release-signing gate regex
  false-positive on gradle test tasks) fixed by b8500a42 + 50d20011;
  integration_contact_block tests pass in the green wave. The previous
  kickoff's 0-CRITICAL "blocking is broken" item was resolved BY that PR.
- Main CI fully green (waves through 718fc53a).
- orch/qwen-takeover-setup-2026-08-04 merged (ae43b8a4; merge/unify plan
  step 2). SECURITY.md restored. Stranded security pins 4df163a1/81797a40
  cherry-picked to main (6e6f9c59/1cd073ea).
- Fresh APK (Mobile run 30985808228, code 50d20011) installed on the
  operator's Pixel 6a. Operator CONFIRMED bidirectional Android<->iOS
  messaging with Christy's iPhone -- PHASE 2 partial (Android + iOS legs
  proven; Windows CLI, macOS CLI, AWS relay legs still owe fresh-install
  proof).

## Wave directives (operator 2026-08-05, standing)

1. PR-FIRST: all functional work lands via branch + PR; the PR must show
   green CI before merge. No direct-to-main pushes for functional changes
   (orchestrator state files -- HANDOFF tickets/queue/kickoff -- are the
   exception). "Ensure we don't regress, only safely advance."
2. Two FIELD FINDINGS top the queue (both AUDIT-GATE, core transport/
   routing territory -- adversarial review required before any fix merges):
   - HANDOFF/todo/LEDGER_SHARING_ANDROID_NODE_VISIBILITY_2026-08-05.md
   - HANDOFF/todo/TRANSPORT_BLE_LAN_HICCUP_VERIFICATION_2026-08-05.md
3. Then the v0.4.0 gates: T1 block-flavor fix (design ACCEPTED:
   HANDOFF/plans/T1_BLOCK_FLAVOR_FIX_DESIGN_2026-08-05.md, implementation
   pending + adversarial review), T3/P3 follow-ups
   (HANDOFF/todo/IDENTIFIER_GATE_FOLLOWUPS_2026-08-04.md), Dependabot batch
   (PR_MERGE_UNIFY_PLAN_2026-08-04.md step 3; GitHub reports 7 vulns,
   3 high), then remaining PHASE 2 five-node parity legs, then PHASE 3
   close-out and tag per MILESTONE_RELEASE_PLAN.md.

## Lanes and economics (unchanged)

FREE lanes first, always; qwenpaid is the primary paid lane (operator
directive 2026-07-28, qwenpaid-first for ALL dispatches); Claude Code
sessions remain LOCKED OUT
(HANDOFF/todo/CLAUDE_CODE_SONNET_LOCKOUT_2026-08-04.md). Dispatch via
`python scripts/delegate_task.py --provider qwenpaid ...`. Adversarial
reviews: qwen3.8 MAX-tier. Delegate everything implementable; the
orchestrator context does dispatch, verification, gates, commits, device
ops. BUDGET LESSON 2026-08-05: the previous session ran ZERO delegation --
everything inline in one context -- and burned its quota fast. Do not
repeat: sweeps, audits, and implementations go to delegates.

## Traps carried from the previous kickoff (all still live)

- Qwen diff headers are malformed (`@@ def function_name(` instead of
  `@@ -1341,7 +1341,12 @@`) -- post-process the header or transcribe the
  body before applying.
- Do not read `$?` after a pipe (it reports the last segment's status).
- One build tool at a time on Windows: wrap verify commands in
  `scripts/build_lock.py --run`; check tasklist for cargo/rustc/gradle/java.
- Cargo is RAM-bound (16 cores, ~11.8 GB): use `-j6`.
- Disk was at 93% (~18 GB free): prefer scoped builds; never
  `cargo clean --target <triple>` (it wipes all of target/).
- Repository Hygiene CI fails on trailing whitespace; CRLF presents as
  trailing whitespace.
- No emojis anywhere (hook-enforced, whole-file scan).
- Treat every IP in every doc as ephemeral (AWS relay has no Elastic IP).
