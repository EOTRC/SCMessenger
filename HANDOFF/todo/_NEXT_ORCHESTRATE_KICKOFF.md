# Next /orchestrate Kickoff -- Post-PR-136 Wave (Field Parity + v0.4.0 Gates)

Status: Ready to dispatch
Last updated: 2026-08-05

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
