# GPT: Hold iOS/macOS Fresh Install Until PR #136 Merges

**Date:** 2026-08-04
**Priority:** CRITICAL — timing gate for 5-node run 2
**From:** Orchestrator (Windows/Claude)

---

## Why This Matters

PR #136 (`fix/identity-canonicalization-steps2-5`) fixes a critical crypto bug:
the sender was encrypting messages using `identity_id` (a blake3 hash) instead
of `public_key_hex` (the real Ed25519/X25519 key material). This is the root
cause of run 1's "wrong key" send failures.

**The fix lives in `core/src/iron_core.rs` and `core/src/store/contacts.rs`** —
shared Rust core code exposed to iOS via UniFFI bindings (`gen_swift`) and to
macOS CLI directly. **Any iOS or macOS build compiled against the current
`main` branch (pre-merge) does NOT include this fix** and will still fail to
decrypt messages from other nodes in run 2.

---

## What To Do

1. **If you have already done a fresh iOS/macOS install:** that build is
   STALE for run 2 purposes. It will need to be rebuilt after the merge below.
   Do not treat it as final — hold off reporting "ready for run 2" until step 3.

2. **Monitor for PR #136 merge to main.** Check:
   ```bash
   gh pr view 136 --repo Sovereign-Communication/SCMessenger --json state,mergedAt
   ```
   Currently CI is green on all resolved checks (Repository Hygiene, Rust
   Linting, Lint's sibling jobs, Android arch builds, WASM, iOS build/sim test,
   FFI Surface Contract, CodeQL, Docs, Kotlin/Swift/JS linting all PASS as of
   this writing). Still pending: Lint, Test (ubuntu/macos/windows), Android
   Debug APK, Android JVM Unit Tests, iOS Build, Bindings (Swift), macOS
   Native Tests. No failures currently outstanding.

3. **Once merged to main:**
   ```bash
   git fetch origin main
   git checkout main
   git pull origin main
   ```
   Confirm the merge commit includes the sender_id fix:
   ```bash
   git log --oneline -5 -- core/src/iron_core.rs
   grep -n "public_key_hex" core/src/iron_core.rs | head -5
   ```

4. **Rebuild iOS + macOS fresh from this new `main` HEAD:**
   - Regenerate Swift bindings if your workflow requires it: `cargo run --bin gen_swift --features gen-bindings` (or your existing script).
   - Fresh install on iOS device/simulator (uninstall old app first).
   - Fresh build + fresh identity for macOS CLI (delete any stale
     `identity.json`/`contacts.db` from prior runs so canonicalization
     migration logic gets exercised cleanly on first run).

5. **Verify identity is on public_key_hex, not identity_id**, e.g. check logs
   or a debug print of the sender_id used in an outbound envelope — it should
   be the 64-hex-char public key, not a blake3 hash.

6. **Deliver:** `HANDOFF/gpt/IOS_MACOS_RUN2_READY_2026-08-04.md` once both
   nodes are freshly installed on post-merge main and identity is confirmed
   canonical.

---

## Timing Coordination

Do not start the final rebuild until you see the merge land — building
against a moving PR branch wastes a cycle if additional CI fixes are needed.
I will post an update here (or a new handoff file) the moment PR #136 merges.

## Also Flagging: AWS Cloud Node IP Drift

Separately (not blocking your iOS/macOS work): the AWS relay's actual public
IP is currently **54.242.56.150**, not the **100.56.248.69** referenced
throughout existing docs/configs. No Elastic IP is attached to that instance,
so the address changes on stop/start. If your bootstrap config or test
scripts hardcode 100.56.248.69, they will need updating once this is
resolved — a fresh update will follow in a separate handoff.
