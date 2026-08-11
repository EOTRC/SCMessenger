---
description: SCMessenger Orchestration Control Plane v2 adapter for Qwen Code.
---

# SCMessenger orchestration adapter: Qwen

This is a thin Qwen bootstrap. Read `AGENTS.md`, `docs/ORCHESTRATION.md`, and
validate `orchestration/manifest.yaml` with `python3 scripts/orchestration_contract.py`.
Use `scripts/orchestrate_strict.py` as the common kernel and record the actual
provider/model it chooses. Qwen-native subagents may be used only as fresh,
scoped workers with real isolation; otherwise use the repo-owned worktree.

The controller has no source-authoring or compile-fix exception. Map provider
tiers onto the manifest's semantic roles; unavailable models change provider,
not authority. Follow the durable lifecycle and fail closed on missing or
`UNKNOWN` worker metadata.
