---
name: orchestrate
description: Portable SCMessenger Orchestration Control Plane v2 discovery bootstrap.
---

# SCMessenger Orchestration Control Plane v2

Read `AGENTS.md`, `docs/ORCHESTRATION.md`, and
`orchestration/manifest.yaml`. Validate the manifest with:

```bash
python3 scripts/orchestration_contract.py
```

All frontends enter `scripts/orchestrate_strict.py`. Use native isolated
subagents only when they provide actual write isolation; otherwise use
`scripts/orchestration_worktree.py`. Semantic role authority, packet shape,
durable state, validation, and escalation are defined only by the canonical
protocol. The controller never writes application source or compile fixes.
