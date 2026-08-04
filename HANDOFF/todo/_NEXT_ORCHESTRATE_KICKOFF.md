# Next /orchestrate Kickoff -- v0.4.0 + v0.5.0 + 5-Node Test

Status: Ready to dispatch
Last updated: 2026-08-04

Paste the prompt in section 1 to start the next phase. Sections 2-4 are the
supporting detail that prompt refers to.

---

## 0-CRITICAL. BLOCKING IS BROKEN IN CORE -- fix before anything else

**Severity: CRITICAL. Security enforcement, production code, currently failing.**

`core/tests/integration_contact_block.rs` has three failing tests on macOS Native
Tests / the full CI suite:

- `test_blocked_message_persisted_but_hidden`
- `test_unblock_restores_hidden_message_visibility`
- `test_block_and_delete_purges_messages_and_drops_future_payloads`

**These are NOT broken tests. They are correctly detecting broken blocking.**

At `core/src/iron_core.rs:3188-3215` the inbound receive path does:

```rust
let is_blocked_and_deleted = self.blocked_manager.read()
    .is_blocked_and_deleted(&message.sender_id) ...
let is_blocked = self.blocked_manager.read()
    .is_blocked(&message.sender_id, sender_device_id.as_deref()) ...
```

After this branch's canonicalization, `message.sender_id` carries the **public
key**. But `block_peer()` stores the block under whatever identifier the caller
passed -- the **identity_id** in the tests, and in any block already on disk from
before this change. The comparison therefore misses.

**Consequence: a blocked peer's messages are no longer hidden, and a
blocked-and-deleted peer's messages are no longer dropped.** Blocking silently
stops working. Note the deliberate "FAIL CLOSED" comment sitting directly above
the broken check -- that path was carefully reasoned about for a different edge
case, and the identifier change went straight past it.

This is the THIRD instance of the identifier-confusion bug class found in this
one PR, and the first in production security enforcement rather than a UI
listing. The other two: the CLI message-request gate (fixed, commit b69b5eee /
ea0de26f) and the contacts migration tests (fixed, 79d51e17).

**Deliberately left unfixed at session end.** The fix is not hard -- resolve both
identifier flavors before the block check, using
`core::identity::identity_id_from_public_key_hex()` which this branch already
added for exactly this purpose. But it is security-critical core code,
`.claude/rules/security.md` requires adversarial review for it, no review
capacity was available (Anthropic session limit exhausted), and shipping a
silently-broken block gate is far worse than a red CI run. **The red CI is the
system correctly refusing to merge broken blocking. Leave it red until this is
fixed AND reviewed.**

Fix it, then re-run the FULL suite, not a scoped subset:

```
cargo test -p scmessenger-core --lib
cargo test -p scmessenger-core --test integration_contact_block
cargo test -p scmessenger-cli --test integration_message_requests
```

A scoped run is what let this reach CI in the first place: `--lib` passes 1286/1286
while the integration tests in `core/tests/` fail.

---

## 0-B. Another agent session is committing to this branch

Commits `1981f7b0` and `68875a7c` ("Qwen Code /orchestrate launcher", "FULL-QWEN
capability class") landed on `fix/identity-canonicalization-steps2-5` from a
different session while this one was working. Coordinate before assuming the
branch is yours alone -- check `git log` for unexpected commits before and after
any build, and re-read `.claude/rules/build.md` on never running two build tools
concurrently on this host.

---

## 0. READ FIRST -- branch state at session end (2026-08-04)

**PR #136 was RED at session end.** Commit `b69b5eee` introduced a compile error:
`cli/src/server.rs` referenced `blake3`, which is a dependency of `core` but NOT
of the `scmessenger-cli` crate. Four CI jobs failed on it (Lint, Rust Linting,
Docs, Test macos-latest) -- all the same root cause, since each compiles.

A correction was written but its local verification build had not finished when
the session ended. The correction:

- Adds `identity_id_from_public_key_hex()` to `core/src/identity/keys.rs` as the
  single source of truth for the public_key -> identity_id derivation, exported
  from `core/src/identity/mod.rs`.
- Changes `cli/src/server.rs` to import that instead of recomputing the hash
  locally. This is deliberately NOT "add blake3 to cli/Cargo.toml" -- duplicating
  the derivation across crates is exactly the drift that causes the identifier
  bug class this branch exists to fix.

**First action for the next session:**

```
git log --oneline -3
cargo test -j6 -p scmessenger-cli --test integration_message_requests
```

If the correction is uncommitted in the working tree, verify it compiles, then
commit and push it. If already committed, confirm CI went green. Either way, all
FOUR tests in that file must pass, not just the one that was originally failing.

**Useful discovery for Phase 1 (the identifier audit):** `core/src/identity/keys.rs`
already contains `PUBLIC_KEY_PREFIX`, `IDENTITY_ID_PREFIX`, and
`identify_key_type()`. The codebase already recognized that these two 64-hex
identifiers are dangerously confusable and built prefix constants for logging --
but they are not applied on the storage or comparison paths. The "add a prefix so
the flavors stop being indistinguishable" option in the audit is therefore
partially built already, not a from-scratch proposal.

---

## 1. The kickoff prompt

```
/orchestrate lanes

Read HANDOFF/SESSION_HANDOFF_2026-08-04_IDENTITY_AND_RUN2.md first -- it has the
current state, the open security debt, and the lane economics you must follow.

FREE LANES ONLY unless a task is a genuine audit gate or judgment call. The last
session exhausted the Anthropic quota by fanning out Claude subagents for bulk
code reading. Bulk reading, mechanical edits, and analysis go to qwenpaid, Groq,
or OpenRouter/FusionLite. Claude native is for verdicts.

Work these in order. Do not start a phase until the prior one is verified green.

PHASE 0 -- UNBLOCK PR #136 (critical path, nothing else moves until this lands)
  a. Verify all four tests pass:
     cargo test -j6 -p scmessenger-cli --test integration_message_requests
  b. ADVERSARIAL REVIEW of the message-request gating fix in cli/src/server.rs.
     REQUIRED before merge per .claude/rules/security.md -- it is a block/allow
     gate. Probe specifically: can a blocked peer reappear as a pending request
     under a different identifier flavor, and do accept/reject still agree on
     which flavor they surface versus consume.
  c. cargo fmt --all -- --check, scoped clippy, commit, push, CI to green, merge.

PHASE 1 -- IDENTIFIER UNIFICATION AUDIT (do this BEFORE the 5-node test)
  We have hit the identifier-confusion bug class twice in one PR. public_key_hex
  and identity_id are both 64-char lowercase hex and both bare Strings, so
  neither the compiler nor a reviewer can catch a mixup. Assume more sites exist.
  A ready-made workflow script is saved and re-runnable -- see section 3 below.
  Run it on free lanes. Deliver a confirmed-findings list plus a structural fix
  recommendation (newtype wrappers vs a single resolution choke point vs wire
  prefixes vs a CI lint).
  Fix every CRITICAL and HIGH finding before the 5-node test, or the test results
  cannot be trusted.

PHASE 2 -- 5-NODE RUN 2
  Nodes: Windows CLI, Android Pixel 6a, iOS, macOS CLI, AWS relay.
  All five must be freshly installed from post-merge main with wiped identity
  state. GPT owns iOS + macOS and is already armed -- see
  HANDOFF/gpt/GPT_WAIT_FOR_PR136_BEFORE_FINAL_IOS_MACOS_BUILD_2026-08-04.md.
  Relay is live at the address recorded in
  HANDOFF/audit/AWS_RELAY_REBUILD_2026-08-04.md -- read it fresh, never copy an
  IP out of a doc, there is no Elastic IP and the address drifts.
  Success = real bidirectional delivery with verified receipts across all five,
  evidenced by ConnectionEstablished plus receipt logs, not by dial-queue logs.

PHASE 3 -- v0.4.0 CLOSE-OUT
  Remaining blockers from HANDOFF/plans/MILESTONE_RELEASE_PLAN.md lines 76-172.
  See section 2 below for the list and which are already satisfied.

PHASE 4 -- v0.5.0 FARM SIMULATION
  Scope at MILESTONE_RELEASE_PLAN.md lines 173-253. Do not start until v0.4.0
  is tagged.

Standing rules: delegate everything implementable; commit after each verified
task; never push unless asked; treat every IP in every doc as ephemeral; run the
finalize-checklist skill before declaring done.
```

---

## 2. v0.4.0 remaining blockers (canonical list at MILESTONE_RELEASE_PLAN.md:81)

| # | Item | Status entering next session |
|---|---|---|
| 1 | Outbox Site-1 flush on reconnect | Reported DONE at HEAD (f521f142, 4 call sites) -- VERIFY by grep before dispatching |
| 2 | Receipt round-trip fix | Reported DONE (8f866bfc + iron_core.rs classify path) -- VERIFY |
| 3 | Android retry suppression | Kotlin side, confirm state |
| 4 | AWS relay live proof | **Relay is LIVE and verified 2026-08-04.** WAN end-to-end proof still owed |
| 5 | DNS-name-first hardening (IP-flip mandate) | OPEN, and now urgent -- the relay has no Elastic IP, so IP flip is a live failure mode. Note the unclosed `/dns4/` filter-bypass finding from the IP sweep |
| 6 | Bootstrap topology wiring | OPEN. Related: the JS clients still hardcode dead bootstrap IPs |
| 7 | Ledger convergence test + fix | Reported DONE 2026-07-23 -- VERIFY |
| 8 | Graceful dial policy | Reported COMPLETED per _QUEUE.md 2026-07-21 header -- VERIFY |

Human gates: H-04 (AWS relay activate) is now SATISFIED. Lucas port forwards
(tcp/443, tcp/80, udp/443) plus DDNS record still outstanding.

Do not trust a "DONE" marker without a grep-level confirmation at HEAD --
`_QUEUE.md` explicitly warns that several doc-claimed completions were stale.

---

## 3. Re-running the identifier audit

A complete workflow script already exists and is re-runnable:

```
C:\Users\SCM\.claude\projects\C--Users-SCM-Documents-GitHub-SCMessenger\09cba5c5-aa56-4e2b-b944-47e4c73a6010\workflows\scripts\identifier-unification-audit-wf_0634f975-9c1.js
```

It fans out five surface inventories (core identity/crypto, core store and
iron_core, core transport/routing/privacy, CLI plus WASM rpc, Android/iOS/UniFFI),
builds a producer-to-consumer matrix, adversarially refutes every candidate before
confirming it, then designs the structural fix.

**Its prompts are lane-agnostic and can be lifted wholesale into
`scripts/delegate_task.py` task files.** Do that rather than re-running it as a
Claude workflow -- that is what exhausted the quota. Each surface inventory is an
independent task file; run them against qwenpaid or OpenRouter, collect the five
outputs, then do the cross-reference and the confirm/refute pass.

Seed every dispatch with the central hazard, because it is the whole reason the
bug recurs:

> `public_key_hex` (the Ed25519 public key) and `identity_id`
> (`hex(blake3(public_key_bytes))`) are BOTH 64-character lowercase hex strings.
> Indistinguishable by length, regex, or eyeball. Both are bare `String`. The
> compiler cannot catch a mixup; it appears at runtime as a silent lookup miss or
> a "wrong key" decryption failure. Never assume a value's flavor from its
> variable name -- trace the actual value.

---

## 4. Known traps for the next orchestrator

- **Qwen diff headers are malformed.** Qwen emits `@@ def function_name(` instead
  of `@@ -1341,7 +1341,12 @@`. Every apply method rejects it as "only garbage was
  found in the patch input." The code is fine; the framing is not. Post-process
  the header or transcribe the body.
- **Do not read `$?` after a pipe.** `cargo fmt --check | head; echo $?` reports
  the status of `head`, always 0, so the gate silently cannot fail.
- **One build tool at a time on Windows.** Multiple agent sessions share this
  repo; Gradle can spawn cargo-ndk upstream. Check `tasklist` for
  cargo/rustc/gradle/java before any build.
- **Cargo is RAM-bound here:** 16 cores, ~11.8 GB RAM. Use `-j6`.
- **Disk was at 93% (18 GB free).** The build rules want 25 GB before a full
  five-gate sweep. Prefer scoped builds. Never `cargo clean --target <triple>` --
  despite the name it wipes all of `target/`.
- **Repository Hygiene CI fails on trailing whitespace,** and CRLF line endings
  present as trailing whitespace. Two CI cycles were burned on this.
- **No emojis anywhere** -- hook-enforced, and it scans the whole file, not just
  the changed lines.
