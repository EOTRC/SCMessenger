# Tracking: Pre-v0.4.0 Tag Work

**Branch:** `tracking/pre-v040-tag-work`
**Created:** 2026-08-05
**Purpose:** Track all remaining work that must land before the v0.4.0 tag, per operator directive and five-node gate plan.

---

## Status Overview

All main branch workflows are **GREEN** at commit `a53dc099` (PRs #136, #137, #138 merged).
Five-node fleet rollout in progress:
- [OK] Windows CLI: rebuilt at main tip (6b2573fa)
- [OK] Android: fresh APK installed in-place on Pixel 6a
- [IN PROGRESS] AWS: rebuilt, new IP `54.226.67.101` (Docker Publish green)
- [IN PROGRESS] iOS/macOS: GPT lane GO packet dispatched, in-place update test
- [OK] iOS Build & Test: green on main

---

## Remaining Pre-v0.4.0-Tag Work (from five-node gate plan)

### 1. Identifier-Gate Follow-ups (P1/P3/P4)
**Source:** `HANDOFF/todo/IDENTIFIER_GATE_FOLLOWUPS_2026-08-04.md`
**Origin:** Phase 0b adversarial review of PR #136 block gate
**Deadline:** Before v0.4.0 tag
**Tier:** THINK/MAX via qwenpaid

| Task | Priority | Description |
|------|----------|-------------|
| **T1 (P1)** | HIGH | Mixed-fleet block bypass: block stored under public key + inbound sender_id as identity_id (old build) misses both candidates. Fix: store BOTH identifier flavors at block-write time or add reverse index. Requires design note + adversarial review (core store change). |
| **T2 (P3)** | MEDIUM | `identity_id_from_public_key_hex` must return `None` unless input passes `is_valid_public_key` (Ed25519 curve point). Mechanical change in `core/src/identity/keys.rs`. |
| **T3 (P4)** | HIGH | `GetPendingMessageRequests` shows request when sender cannot be proven known-or-blocked. Change filter so UNRESOLVABLE sender is suppressed or flagged. CLI-only (`cli/src/server.rs`). |
| **T4 (P5)** | MEDIUM | Centralize flavor resolution into `BlockedManager.is_blocked_resolved` for single policy. Refactor after T1 lands. |

---

### 2. Ledger Visibility Gap (Phase 2)
**Source:** `HANDOFF/todo/LEDGER_SHARING_ANDROID_NODE_VISIBILITY_2026-08-05.md`
**Audit:** `HANDOFF/review/LEDGER_VISIBILITY_AUDIT_QWENPAID_2026-08-05.md`
**Design:** `HANDOFF/plans/TRUST_SCOPED_LAN_DISCLOSURE_DESIGN_2026-08-05.md`
**Priority:** HIGH (field-observed parity gap)
**Class:** AUDIT-GATE (core/src/{routing,transport} + app sync path)

**Observation:** iOS sees 4 nodes + 1 headless; Android sees only Christy on same network.
**Root Cause (ranked):**
1. **Most Likely:** `exchange_response_entries` filters by `success_count > 0` AND applies `is_disclosable_multiaddr` which strips RFC1918. iOS redacts LAN neighbors per NEW-2 security doctrine. Trust escalation for paired peers missing.
2. **Contributing:** Fresh Android has `success_count == 0` for all entries; ledger exchange requires proven peers.
3. **Possible:** Identity canonicalization drops entries during ingest.
4. **UI Mismatch:** Android may read from Contacts store vs Ledger store.

**Fix Direction (from design doc):** Gate RFC1918/topic disclosure on `LedgerEntry::AnnotateIdentity` trust level (`TrustLevel::Trusted`) in `exchange_response_entries()`. Default-deny preserved. Mixed-version compatible via sender-side filtering.

**Blocked On:** **Operator decision on disclosure policy** (AGENTS.md rule 9 - security trade-off).

---

### 3. Transport BLE/LAN Hiccup Verification
**Source:** `HANDOFF/todo/TRANSPORT_BLE_LAN_HICCUP_VERIFICATION_2026-08-05.md`
**Priority:** HIGH
**Class:** AUDIT-GATE (core/src/transport/)

**Observation:** Mid-session messages stopped between Android (Pixel 6a) and iOS (Christy). Restarting iOS app resumed flow. Theory: only BLE carrying traffic; LAN/WiFi never established or failed silently.

**Required Verification:**
1. Transport-level evidence: which transport carried each message
2. Confirm LAN/WiFi establishes at all (test with BLE disabled)
3. Failure mode: when one transport dies, does node detect and failover?
4. Locate keep-alive/reconnect gap (stale socket, missed heartbeat, dial-queue not retrying)

**Acceptance Criteria:**
- Captured evidence of LAN/WiFi transport delivering messages
- Killing BLE mid-session → traffic resumes over surviving transport (no app restart)
- Same test in reverse (kill LAN path → BLE carries)

**Note:** PR #137 (transport liveness: zombie transport reconciliation + auto-reconnect) should address this. Need field verification.

---

### 4. Five-Node Gate Execution (Twice Reproducible)
**Source:** `HANDOFF/plans/V040_V050_FIVE_NODE_GATE_PLAN_2026-08-05.md` (Section "THE FIVE-NODE GATE")

| Gate | Criterion | Status |
|------|-----------|--------|
| **G1** | Pairwise bidirectional: every reachable pair exchanges messages both directions with receipts | Pending |
| **G2** | Transport coverage: LAN, BLE, internet relay (with custody proof) all deliver | Pending |
| **G3** | Delivery truth: statuses reflect receipts E2E, no false failures, outbox flushes on reconnect | Pending |
| **G4** | Fleet convergence: every node lists full fleet within bounded window; restart re-converges without re-pair | **Blocked on ledger Phase 2 decision** |
| **G5** | Liveness: disrupt network → peers auto-reconnect without app restart (fleet proof of PR #137) | Pending |
| **G6** | Provenance: all five nodes report same git stamp | Pending |

**Must run twice, reproducibly, before tag.**

---

### 5. Dependabot Vulnerabilities (Queued Cleanup)
**Status:** 7 vulnerabilities on main (3 high)
**Action:** Queued cleanup wave post-v0.4.0 tag

---

## Open Operator Decisions

| # | Decision | Blocks |
|---|----------|--------|
| **D1** | Ledger Phase 2 disclosure policy (trust-scoped LAN disclosure design) | G4 fleet convergence criterion |
| **D2** | v0.4.0 tag flavor (alpha.N vs stable) after gate passes twice | release artifacts |
| **D3** | Accept AWS IP drift (EIP denied by IAM) or widen policy / DDNS | unattended reconnect to cloud node |

---

## Files to Watch / Modify

### Core Changes Required
- `core/src/store/ledger_entry.rs` — `exchange_response_entries()` trust-gated disclosure
- `core/src/transport/addr_filter.rs` — `is_disclosable_multiaddr` (already strict, correct)
- `core/src/identity/keys.rs` — `identity_id_from_public_key_hex` validation (T2)
- `core/src/transport/swarm.rs` — ledger exchange handler trust context extraction
- `cli/src/server.rs` — `GetPendingMessageRequests` filter (T3)
- `core/src/store/blocked.rs` — `BlockedManager.is_blocked_resolved` (T1, T4)

### Design / Audit Docs
- `HANDOFF/review/LEDGER_VISIBILITY_AUDIT_QWENPAID_2026-08-05.md`
- `HANDOFF/plans/TRUST_SCOPED_LAN_DISCLOSURE_DESIGN_2026-08-05.md`
- `HANDOFF/todo/IDENTIFIER_GATE_FOLLOWUPS_2026-08-04.md`
- `HANDOFF/todo/LEDGER_SHARING_ANDROID_NODE_VISIBILITY_2026-08-05.md`
- `HANDOFF/todo/TRANSPORT_BLE_LAN_HICCUP_VERIFICATION_2026-08-05.md`

### Evidence / Rollout Docs
- `HANDOFF/gpt/AWS_RELAY_CURRENT_ADDRESS.md` — current AWS IP: `54.226.67.101`
- `HANDOFF/gpt/GPT_GO_IOS_MACOS_POST_PR138_2026-08-05.md` — iOS/macOS in-place update packet
- `HANDOFF/plans/V040_V050_FIVE_NODE_GATE_PLAN_2026-08-05.md` — gate criteria
- `HANDOFF/gpt/IOS_MACOS_POST138_READY_2026-08-05.md` — GPT report (when filed)

---

## Dispatch Order (per _QUEUE.md)

1. **Identifier-gate T1 (P1)** — design note + implementation (qwenpaid, adversarial review)
2. **Identifier-gate T2 (P3)** — mechanical fix `keys.rs` (qwenpaid, diff mode)
3. **Identifier-gate T3 (P4)** — CLI filter fix `server.rs` (qwenpaid, diff mode)
4. **Ledger Phase 2** — **BLOCKED ON OPERATOR D1** — implement trust-scoped LAN disclosure
5. **Transport BLE/LAN verification** — field test with transport evidence capture
6. **Five-node gate execution** — twice, with recorded evidence

---

## Notes for Next Session

- All main CI green; no PRs open except dependabot (7 vulns, 3 high)
- Android fresh install successful; iOS/macOS in GPT lane
- Windows node at main tip; AWS node rebuilt with new IP
- Next actionable: wait for operator decision D1, then dispatch T1/T2/T3 via qwenpaid
- Five-node gate runs after T1-T3 + ledger Phase 2 land