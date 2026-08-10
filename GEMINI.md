# Gemini adapter

Status: Active
Last updated: 2026-08-10

Read `AGENTS.md`, `docs/ORCHESTRATION.md`, and validate
`orchestration/manifest.yaml` with `scripts/orchestration_contract.py`.
Gemini is an adapter to `scripts/orchestrate_strict.py`, not an alternate
controller. Use fresh isolated workers, semantic roles from the manifest, and
the durable lifecycle. Never grant the controller source-authoring, compile-fix,
planner, validator, or operator authority because a model is unavailable.
