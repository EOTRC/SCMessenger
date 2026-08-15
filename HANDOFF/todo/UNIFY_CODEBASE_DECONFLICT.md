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

### C4 + C5. Agent contracts and harness config — RESOLVED, operator ruling 2026-08-15

Operator: "unify and rip/replace — we need a cohesive strategy that will work
for any tool."

**Correction to the first pass:** GEMINI.md's relationship was recorded as
"undeclared". That was wrong — it was not read before being classified. It is
505 bytes and already says "Read `AGENTS.md` ... Gemini is an adapter to
`scripts/orchestrate_strict.py`, not an alternate controller." It is the
reference implementation of the pattern below, not a conflict.

Sizes: `AGENTS.md` 9,904 B · `CLAUDE.md` 3,629 B · `GEMINI.md` 505 B.

#### The strategy: one contract, thin adapters, executable-only config dirs

1. **`AGENTS.md` is THE contract.** Model-agnostic, harness-agnostic, one copy.
   Every rule that is true regardless of which tool is running lives here or in
   `docs/rules/`. This already matches the emerging cross-vendor convention, so
   a new tool arrives already knowing where to look.

2. **Each harness gets exactly ONE thin adapter file**, at whatever path that
   tool loads by convention (`CLAUDE.md`, `GEMINI.md`, `.codex/AGENTS.md`, ...).
   Its only job: point at `AGENTS.md`, then state the handful of mechanics
   unique to that harness. `GEMINI.md` at 505 B is the size target. `CLAUDE.md`
   at 3.6 KB is at its cap and must not grow — it is re-paid uncached on every
   subagent spawn.

3. **A harness config directory holds ONLY what that tool's runtime loads
   automatically.** Hooks, commands, skills, settings. Nothing else.

   The test, and it is the whole rule: **"does this tool's runtime read this
   file by itself, without a human naming it?"** If no, it is content, and
   content does not live in a harness directory.

4. **Shared prose moves to `docs/rules/`.** One copy, referenced by every
   adapter. This is already the instinct behind `.claude/rules/*.md` being
   stubs — that directory auto-loads into every spawn, so detail there is paid
   for repeatedly. Generalise that to all harnesses.

5. **Specs and plans move to `docs/` or `HANDOFF/`.** They are project content
   that happens to sit under a vendor directory.

#### What that means per directory (evidence: `git ls-files` by subdir)

| Dir | Contents | Verdict |
|---|---|---|
| `.claude` | scripts 23, skills 11, archive 7, prompts 6, rules 5, hooks 5 | MIXED — keep hooks/skills/commands; `archive/` and `prompts/` are content, `rules/` are already stubs |
| `.kiro` | specs 32 | **CONTENT** — no executable config at all. Move to `docs/specs/`, then the dir goes |
| `.codex` | agents 13, hooks 2, hooks.json | executable — keep, add adapter pointer |
| `.mimocode` | plans 5, config 1, doc 1 | **CONTENT** — plans move to `HANDOFF/`; keep only the config |
| `.bob` | skills 6 | executable — keep, add adapter pointer |
| `.agents` | skills 4, rules 1 | executable — keep; the `rules` file folds into `docs/rules/` |
| `.qwen` | commands 1 | executable — keep |

Nothing here requires knowing which harnesses are "live", which is why this
stopped being a CALLOUT: the rule is about what a runtime loads, not about who
is using it. A dormant harness with a valid adapter costs one small file.

- Disposition: **PLAN.** Sequence: (a) move `.kiro/specs` and `.mimocode/plans`
  out; (b) add adapter pointers for `.codex`/`.bob`/`.agents`/`.qwen`;
  (c) fold `.agents/rules` into `docs/rules/`; (d) only then remove emptied
  directories. Per the deletion rule, no directory is removed in the round that
  empties it.

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

Phase 2 on C1, C6, C7 and the C4+C5 plan. C2 (orchestrat* scripts) and C3
(verify_* index) remain, C2 still a CALLOUT.
call. C2, C4, C5 stay parked as CALLOUTs until the CTO or operator rules.

Do not start a round while a required CI check is red.
