# Build & CI Rules

Re-injected into agent context on every turn.

## Build Verification (Mandatory)

Scoped to what changed, before finalizing any run (prefer the `build-verify` skill):
1. Rust edits: `cargo build --workspace` (record output in HANDOFF notes).
2. Android edits: `cd android && ./gradlew assembleDebug -x lint --quiet`.
3. WASM edits: `cargo build -p scmessenger-wasm --target wasm32-unknown-unknown`.
4. Format: `cargo fmt --all -- --check`.
5. Lint: `cargo clippy --workspace -- -D warnings -A clippy::empty_line_after_doc_comments`.

Compile gate: `cargo test --workspace --no-run` must pass before any task is
considered complete.

## Disk Space Preflight (Windows & macOS)

Before running a full gate sweep, verify free disk space via `scripts/preflight_disk.sh`
(or `.ps1` on Windows). This check is not CI-enforced but is mandatory for local and
agent-driven builds to prevent OOM/disk-full crashes.

**Constraint:** A full five-gate sweep (fmt, clippy default, clippy --all-features,
`cargo test --workspace --no-run`, wasm release) regrows `target/` to ~40-47 GB.
Measured evidence: three consecutive runs today reclaimed 42.7 GB, 35.7 GB, and 47.2 GB.
Threshold: 25 GB minimum free space.

**Critical traps:**
1. `cargo clean --target <triple>` does NOT scope to a single target — it wipes ALL
   of `target/`. Verified today: intended to reclaim ~4 GB, deleted 44.7 GB.
2. `scripts/ffi_surface.sh` silently depends on `core/target/generated-sources/`.
   Note the path: that directory lives under `core/target/`, which is SEPARATE from
   the workspace `target/`. Measured 2026-07-27: a plain `cargo clean` from the
   workspace root removed 22,557 files / 47.1 GiB from `target/` and left
   `core/target/generated-sources/` intact, so a root clean does NOT require
   regenerating bindings.
   What DOES destroy it: `cargo clean` run from inside `core/`, `cargo clean
   --target <triple>` (see trap 1 -- it wipes everything), or deleting `core/target`
   directly. After any of those, regenerate bindings (`gen_swift`, `gen_kotlin`) and
   verify the files exist before running `ffi_surface.sh --update` -- skipping that
   check produced a vacuous "Updated Swift snapshot" with exit 0 and no bindings,
   twice.
   Cheap insurance before any clean: `cp -r core/target/generated-sources <tmp>`
   (1.2 MB).

## Docs Sync

Run `./scripts/docs_sync_check.sh` (or the `.ps1`) after any documentation
change; resolve failures before finalizing.

## Path Conventions (CI Enforced)

Enforced by the `Repository Hygiene` workflow (`.github/workflows/hygiene.yml`):

- `iOS/` uppercase-I in ALL path references; XCFramework at `iOS/SCMessengerCore.xcframework/`
  (step: `Verify path governance rules`).
- No `.py` in repo root (use `scripts/`); no build artifacts committed
  (`git ls-files "*.log" "*.pid" "*.logcat"` must be empty) (step:
  `Verify root directory layout`).
- Keep the repo root minimal. Documentation belongs under `docs/` (with
  historical material in `docs/historical/`), executable scripts under
  `scripts/`. Only tooling-mandated files and GitHub community-health files
  belong at the root.

## Windows

- Incremental compilation disabled (`.cargo/config.toml`); also
  `export CARGO_INCREMENTAL=0` in the shell before cargo commands.
- Never run two build-tool invocations concurrently (see CLAUDE.md
  Windows-Specific Rules -- Gradle can spawn cargo-ndk upstream).
- Shell scripts need Git Bash/WSL; CI is ubuntu/macos only -- Windows builds
  verified locally.

## Model Availability Check (the `swarm` backend ONLY)

Only when using the `swarm` backend (ollama pool): verify the target ollama model
via `bash .claude/model_validation_template.sh` or `https://ollama.com/api/tags`.
Not applicable to the `lanes`, `native`, or `agent` backends -- for `native` the
model truth is `claude --help` aliases; for `lanes` it is the lake registry
`docs/orchestration/SCM_UNIFIED_LAKE_ORCHESTRATION.md`.
