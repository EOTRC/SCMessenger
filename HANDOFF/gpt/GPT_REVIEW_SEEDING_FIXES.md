# GPT HANDOFF -- adversarial second-opinion review: ledger-seeding fixes

Status: AWAITING DIFF INSERTION (orchestrator inserts the fix diffs when
Wave 1b dispatches pass their compile gates)
Created: 2026-07-28
Executor: GPT-5.6 Sol session on the operator's MacBook
Estimated quota: ~5% of the weekly window

## Why this review exists

The Windows orchestrator (qwen3.8-max-preview) is both IMPLEMENTING and
self-reviewing the ledger-seeding security fixes. Vendor-independent
adversarial review catches same-model blind spots; the operator has
approved spending GPT quota on exactly this class of task.

## Background (read first)

- Verdict under remediation: HANDOFF/review/LEDGER_SEEDING_ADVERSARIAL_REVIEW_2026-07-25.md
  (Status was BLOCK: findings F1-F16).
- Operator decision: fix ALL open findings before tagging v0.4.0-alpha.1.
- HEAD-state analysis that scoped the fixes: the fixes target exactly
  these (orchestrator-verified at HEAD before dispatch):
  - F10 -- ledger_entry.rs: entries vec had NO cap/eviction/TTL;
    save_with_entries did a whole-file rewrite per mutation UNDER the
    mutex; mobile_bridge looped per entry. Fix: MAX_LEDGER_ENTRIES=1024
    with oldest-zero-success-first eviction, save after guard drop,
    annotate_identities_batch (one lock, one save), seed_addresses ordered
    by last_seen desc so dead seeds rotate out.
  - F7(a) -- swarm.rs: ConnectToSeedPeers dialed without
    dial_policy_manager.register_dial_attempt, bypassing backoff/dead
    policy. Fix: register gate per candidate, skip on false.
  - F7(b) -- swarm.rs: LedgerManager::record_failure had ZERO production
    callers. Fix: called at the OutgoingConnectionError arm (~:4841) and
    pending-dial sweep timeout (~:2847).
  - F13 -- swarm.rs: pending-dial resolution at ~:4550-4561 matched
    ConnectionEstablished remote addresses WITHOUT an endpoint.is_dialer()
    gate, so a simultaneous INBOUND connection could resolve a pending
    outbound dial Ok(()) with no NAT mapping created (false "connected"
    report). Fix: wrap in is_dialer(), mirroring the gated
    record_connection block at ~:4598.
  - NEW-6 -- swarm.rs: RelayAbuseGuardrails kept per-PeerId token buckets
    only; fresh Noise identities got fresh bursts (Sybil bypass). Fix:
    one global TokenBucketState (burst 20, refill 2/s) consumed alongside
    the per-peer token in the ledger-exchange request handler.

## The diff under review

[ORCHESTRATOR: insert `git diff` of the Wave 1b commits here -- files
core/src/store/ledger_entry.rs, core/src/mobile_bridge.rs,
core/src/transport/swarm.rs -- immediately before handing off.]

## Your task

1. Read the verdict file, then the diff, then the SURROUNDING code at the
   cited locations (clone: git clone
   https://github.com/Sovereign-Communication/SCMessenger.git; the fixes
   are on main at the commit named in your kickoff prompt).
2. For EACH of F10, F7(a), F7(b), F13, NEW-6, deliver a verdict:
   FIXED / PARTIALLY FIXED / NOT FIXED / REGRESSION / NEW ISSUE, with
   file:line evidence and a concrete failure scenario for anything short
   of FIXED. Probe specifically for: races (lock ordering, save-after-drop
   tearing, concurrent annotate vs exchange), desync (peer A's ledger
   diverging from peer B's under the new eviction/ordering), downgrade
   (does any path still accept DNS-form or SSRF addresses into the
   ledger?), framing/compat (old persisted ledger.json files vs the new
   cap -- load path behavior), and DoS (can any remaining unbounded
   structure still be driven by remote input?).
3. Also answer: does the F13 gate break any LEGITIMATE simultaneous-open
   scenario (two nodes dialing each other simultaneously -- which outcome
   SHOULD win)? Is the global bucket sized sanely for a 2-node alpha and a
   12-node farm?
4. Do NOT propose rewrites. One-paragraph max per finding. Adversarial
   posture: try to BREAK the fixes, default to skepticism.

## Output and delivery

Commit your report as HANDOFF/gpt/GPT_REVIEW_SEEDING_FIXES_VERDICT.md on
branch gpt/seeding-review (you are cleared to commit and push your own
branch and open a PR per the updated lane rules -- do NOT merge). Verdict
file format: one section per finding (verdict tag first line), then a
final SHIP / NO-SHIP line for the v0.4.0-alpha.1 tag with justification.
Also print the summary table in your session output for the operator.
