# SCMessenger -- Agent Instructions

The only always-loaded instructions. Everything else loads on demand via the
table below. **Keep this under 3.5 KB** -- it is re-paid uncached by every
subagent and every `claude -p` spawn. Detail belongs in `docs/rules/`.
(Currently ~3.1 KB. If an addition would breach the cap, something else must
move to `docs/rules/` in the same change.)

## Invariants

No hook, CI job, or compiler catches these.

**Builds** (Windows, 16 logical cores / 11.8 GB RAM -- RAM-bound, not core-bound)

- `cargo -j12` default; `-j6` if RAM is contended; `-j4` cold post-clean.
  Keep `CARGO_INCREMENTAL=0`.
- Never run two build tools at once -- sessions share this repo, and Gradle
  spawns cargo-ndk.
- Never `cargo clean --target <triple>`: it wipes ALL of `target/`, not one
  triple (44.7 GB lost). Use `scripts/clean_target.sh`.
- A rustc crash (`STATUS_STACK_BUFFER_OVERRUN`, "can't find crate", stuck macro
  resolution) is resource exhaustion -- not corruption, not a code bug. Retry at
  lower `-j`; check `df -h /c`.
- Never read `$?` after a pipe: it reports the last command, so a piped gate can
  never fail.

**Code**

- No emojis anywhere. Use `[OK]`, `[FAIL]`, `[WARNING]`, `[INFO]`. Hook-enforced.
- Rust: state behind `Arc<RwLock<..>>` (parking_lot); `IronCore` is the only
  entry point; no sled outside `store/`; Ed25519 signs, X25519 encrypts,
  XChaCha20-Poly1305 seals; never `unwrap()` in production paths.
- `core/src/{crypto,transport,routing,privacy}/` is merge-blocked until
  adversarial review signs off.

**Compile gate:** `cargo test --workspace --no-run` must pass before any task is
complete.

## Routing table

Answers "does documentation exist for this" without a lookup.

| About to... | Read/run first | Else |
|---|---|---|
| Clean or delete build artifacts | `scripts/clean_target.sh --dry-run` | 44.7 GB wiped, bindings destroyed |
| Dispatch a worker (agy, delegate_task, Qwen) | `docs/rules/DELEGATION.md` | timeouts, silent truncation, quota burn |
| Run a build or gate | deconflict, `df -h /c`, `build-verify` skill | concurrent builds corrupt `target/` |
| Edit crypto/transport/routing/privacy | `docs/rules/SECURITY_PROTOCOL.md`, `crypto-security-auditor` | merge blocked; race and timing defects ship |
| Edit other Rust | `docs/rules/RUST_CONVENTIONS.md` | module-boundary violation caught late |
| Android work | `docs/rules/ANDROID.md`, `android-qa` agent | hardcoded strings, missing FGS channel |
| Change documentation | `docs-sync` skill | mandatory sync check fails at finalize |
| Start an orchestration run | `docs/ORCHESTRATION.md`, **in full** | fragments led to the wrong primary lane |
| Finalize or commit | `finalize-checklist` skill | secrets, unverified build, stale docs |
| Anything not listed | `docs/DOCUMENT_STATUS_INDEX.md` | 30 KB -- do not load reflexively |

## Keeping this file small

Tier 0 is this file. Tier 1 is `docs/rules/` and skills, loaded on demand.
`.claude/rules/*.md` are stubs so older cross-references still resolve -- never
re-inline detail there, that directory auto-loads into every spawn.

Before adding a rule here, ask whether violating it already fails loudly. If a
hook, CI job, or compiler catches it, it belongs in `docs/rules/`.
