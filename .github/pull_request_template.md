## Summary
Briefly describe the change.

## Why
What problem does this PR solve? Link the issue, or explain why no issue exists.

## Release Scope
- [ ] Fix or improvement on the current `v0.3.5` alpha baseline
- [ ] `v1.0.0` Phase 1 scope (Windows/Android transport parity)
- [ ] `v1.0.0` Phase 2 scope (everything else)
- [ ] Repo-governance / documentation / tooling work

## Documentation Impact
- [ ] Canonical docs updated
- [ ] Supporting docs updated
- [ ] No docs update needed, because:

## Validation
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace -- -D warnings -A clippy::empty_line_after_doc_comments`
- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `./scripts/docs_sync_check.sh`
- [ ] Targeted platform/manual validation:

## Risk / Security Notes
- [ ] No new security-sensitive behavior introduced
- [ ] Risk notes documented below

## Checklist
- [ ] Changes are focused and minimal
- [ ] Tests were added or updated when needed
- [ ] Existing behavior was revalidated for the changed area
- [ ] Docs/reporting surfaces stay aligned with the change
