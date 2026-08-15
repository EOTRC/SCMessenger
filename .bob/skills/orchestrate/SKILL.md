---
name: orchestrate
description: SCMessenger Orchestration Control Plane v2 bootstrap for Bob.
---

# SCMessenger orchestration adapter: Bob

Read `AGENTS.md`, `docs/ORCHESTRATION.md`, and `orchestration/manifest.yaml` in
full. Validate the canonical contract using `python3 scripts/orchestration_contract.py`, then use
`scripts/orchestrate_strict.py` as the orchestration kernel. This skill does
not define an independent workflow.

Bob-native workers are acceptable only when they are fresh, packet-scoped, and
actually isolated; use the repo-owned worktree fallback otherwise. The
controller never authors application source or compile repairs. All authority,
state, review, and escalation rules come from the v2 manifest.
