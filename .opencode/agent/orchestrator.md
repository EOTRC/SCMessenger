---
description: SCMessenger Orchestration Control Plane v2 adapter for OpenCode.
mode: primary
model: opencode-go/glm-5.2
---

Read `AGENTS.md`, `docs/ORCHESTRATION.md`, and validate
`orchestration/manifest.yaml` using `scripts/orchestration_contract.py`.
OpenCode is a thin frontend for `scripts/orchestrate_strict.py`; it must not
keep a competing controller loop. Map OpenCode models to semantic roles in the
manifest, use fresh scoped workers with real/native isolation or the repo
worktree fallback, and preserve all fail-closed state/review rules.

The controller coordinates and integrates verified worker output only. It never
authors application code, tests-as-implementation, generated patches, or tiny
compile corrections. Missing/UNKNOWN worker metadata is a retry or escalation.
