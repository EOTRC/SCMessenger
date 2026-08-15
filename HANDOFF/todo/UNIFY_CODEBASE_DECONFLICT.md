# Unify the codebase — de-conflict scripts, docs, and agent contracts

Status: Active
Created: 2026-08-15
Owner: CTO session (dispatches), operator (approves deletions)
Cadence: **filler work.** Run this while blocked on CI, never instead of the
critical path. If SHIP_PLAN D1-D5 can move, move that first.

A unified codebase is not tidiness. Today an orchestrator test failed because
two delegation scripts exist and the SOP points at the deprecated one; the model
picked the documented-but-wrong path and did exactly what it was told. Ambiguity
in the repo becomes wrong behaviour in every agent that reads it.

---

## THE STANDARD FLOW (use this shape for every de-confliction round)

**Phase 1 — CALL OUT ONLY.** Produce a list of conflicts. Change nothing, delete
nothing, merge nothing. Deliverable is a table: what conflicts, where, and what
depends on each side. A phase-1 pass that edits a file has failed.

**Phase 2 — SAFE PRUNE + PLAN.** For each conflict, classify:
  - `SAFE`      — provably unreferenced, or an exact duplicate. Prune.
  - `PLAN`      — needs a migration (deprecate, redirect, merge content).
  - `CALLOUT`   — cannot be resolved without a judgement call.
  Evidence required per row: the grep/command that proves nothing depends on it.

**Phase 3 — SURFACE TO CTO.** Every `CALLOUT` comes to the CTO session with the
evidence attached. CTO decides.

**Phase 4 — ESCALATE AT <99% CONFIDENCE.** If the CTO is not ≥99% confident, it
goes to the CEO session ("SCMessenger CEO strategy") for confirmation. No
consensus → TABLE it, record it here, move on. Never block on an unanswered
question; there is always another conflict to work.

**Deletion rule:** nothing is deleted in the same round it is identified. A round
that both finds and removes has skipped the review that makes it safe.

---

## PHASE 1 FINDINGS — 2026-08-15 (first pass, nothing changed)

### C1. Two delegation scripts, SOP points at the deprecated one — CONFIRMED HARM
- `scripts/delegate_task.py` (mainline, 41.6 KB) vs `scripts/delegate.py`
  (on `chore/delegation-lane-routing`, PR #150).
- `docs/rules/DELEGATION.md` on mainline has a full `## scripts/delegate_task.py`
  section and **zero** mentions of `delegate.py`.
- Caused a real failure today: an orchestrator read the SOP, used
  `delegate_task.py`, and dispatched to lanes that are dead (qwen/dashscope
  return HTTP 401).
- Disposition: **PLAN.** Merge PR #150, then make `delegate_task.py` a thin
  deprecation pointer. Do not delete it in the same change.

### C2. Eight `orchestrat*` scripts with unclear division of labour
`orchestrate_strict.py`, `orchestration_contract.py`, `orchestration_worktree.py`,
`orchestrator_activate.sh`, `orchestrator_guard.py`, `parse_orchestration_footer.py`,
`test_orchestration.sh`, `test_orchestration_v2.py`
- `test_orchestration.sh` vs `test_orchestration_v2.py` is a versioned pair —
  one is probably dead.
- Disposition: **CALLOUT.** Needs someone to say which is the live entry point.

### C3. Seventeen `verify_*` scripts
Likely mostly legitimate (different subsystems), but no index says which are
current. `verify_all.ps1` vs `verify_all_builds.sh` overlap by name.
- Disposition: **PLAN.** Produce an index first; prune only what the index proves dead.

### C4. Three agent contract files at root
`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`. CLAUDE.md declares itself the Claude
superset of AGENTS.md; GEMINI.md's relationship is undeclared.
- Disposition: **CALLOUT.** Cross-agent contract — CEO-level.

### C5. Seven agent harness config directories, 148 tracked files
`.claude` (81), `.kiro` (32), `.codex` (16), `.mimocode` (7), `.bob` (6),
`.agents` (5), `.qwen` (1).
- Several correspond to harnesses not in current use.
- Disposition: **CALLOUT.** Deleting a harness config strands whoever uses it.
  Needs the operator to say which lanes are live.

### C6. Six documents claiming planning authority
`SHIP_PLAN.md` ("the **only** execution queue"), `REMAINING_WORK_TRACKING.md`,
`docs/CURRENT_STATE.md`, `HANDOFF/todo/_QUEUE.md`,
`HANDOFF/V1_0_0_EXECUTION_PLAN.md`, `DOCUMENTATION.md`.
- SHIP_PLAN.md is the current truth and says so; the other five do not defer to it.
- Disposition: **PLAN.** Add a one-line "superseded by SHIP_PLAN.md until the
  v0.4.0 tag" header to the other five. Cheap, reversible, removes the ambiguity
  without deleting history.

### C7. Duplicate-by-platform pairs
`OllamaQuotaScraper.ps1` / `.sh`, `verify_all.ps1` / `verify_all_builds.sh`.
- Disposition: **SAFE-ish** but verify both are still called before touching either.

---

## NEXT ROUND

Phase 2 on C1, C6, C7 only — the three with a clear migration and no judgement
call. C2, C4, C5 stay parked as CALLOUTs until the CTO or operator rules.

Do not start a round while a required CI check is red.
