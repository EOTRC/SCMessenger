# PR #180 Adversarial Validation Report

**Date:** 2026-08-18
**Validator:** CRITICAL_VALIDATOR (gemini-3.1-pro-high)
**Target:** PR #180 (`fix/dualbind-advertise-only-bound` into `origin/main` b4ccd30a)

## Subject

PR #180, "eliminate dual-binding of TCP and WS on same port".
Commits `cdf8c0fc`, `0d533dbc`, `4e67f750`. Base `origin/main` `b4ccd30a`.
Touches `core/src/transport/multiport.rs` and `core/tests/test_multiport.rs`.

## Pass 1 -- model gemini-3.1-pro-high -- RESULT: BLOCK

One HIGH finding, which the CTO verified directly against the source and ACCEPTED rather than overrode:
- `core/src/transport/swarm.rs:2760-2770` unconditionally binds `/ip4/0.0.0.0/tcp/9002/ws` ("Always expose a WebSocket listener for WASM bridge on 9002"), while `multiport.rs` `EXCLUDED_PORTS` contained only 9876. A node configured with port 9002 would therefore advertise `/tcp/9002` from the multiport generator AND bind `/tcp/9002/ws` from `swarm.rs` -- recreating the exact dual-bind defect the PR exists to remove, reachable by configuration.

This pass also FALSIFIED the CTO's own stated claim that the change set "emits TCP only". Record that plainly; it is the reason the pass had value.

## Remediation -- commit 4e67f750

Added 9002 to `EXCLUDED_PORTS` in `core/src/transport/multiport.rs` with a comment naming `swarm.rs`'s hardcoded listener, plus unit test `test_excluded_port_9002_wasm_bridge_never_emitted`. The `swarm.rs` WebSocket listener was deliberately NOT removed -- removing it is an architecture decision reserved for the operator, and the WASM bridge depends on it.

## Pass 2 -- model gemini-3.1-pro-high, independent re-review -- RESULT: APPROVE

PRIOR_BLOCK_CLEARED: YES. CRYPTO_TOUCHED: NO. FINDINGS: NONE.

- **R1:** 9002 is excluded on the `preferred_port`, `common_ports` and `additional_ports` paths via `EXCLUDED_PORTS.contains(&port)`; the random-port path emits `/tcp/0` and cannot emit 9002 (`multiport.rs:81`).
- **R2:** The new test is non-vacuous: it feeds 9002 through BOTH `additional_ports` and `preferred_port` and asserts absence across IPv4/IPv6 permutations (`multiport.rs:399-440`).
- **R3:** `EXCLUDED_PORTS` is actively consulted, not dead configuration -- the `add_port` closure returns early on a match (`multiport.rs:81`).
- **R4:** No other hardcoded fixed-port listener exists in `core/src/transport/`. Port 0 is used for dynamic/QUIC; 9876 is bound in the CLI and already excluded.
- **R5:** The change set satisfies its goal: a node no longer advertises a TCP address that collides with the WS bind.

## Test-contract change, reviewed explicitly

`core/tests/test_multiport.rs` `test_custom_ports_only` previously asserted `addresses.len() == 6` for three custom ports -- two addresses per port, which ENCODED the dual-bind behaviour being removed. Pass 1 was asked specifically whether commit `0d533dbc` weakened the test to make CI green, and answered that it did not: the test was TIGHTENED, asserting `/tcp/` present and `/ws` absent, with no test deleted and no `#[ignore]` added.

## Gates run by the CTO (not by a worker)

```
cargo fmt --all --check                                          exit 0
cargo test -p scmessenger-core --test test_multiport             12 passed, 0 failed, 1 ignored
cargo clippy -p scmessenger-core --all-features -- -D warnings   exit 0
cargo clippy --workspace --all-features -- -D warnings           exit 0
```

## Outstanding, NOT cleared by this review

The CI "Lint" lane is RED, but NOT because of this PR: it is red on every open PR in the repo, including ones touching only markdown. Cause identified from the CI log: `cargo deny check` reports `advisories FAILED` for RUSTSEC-2026-0258 (`h2` 0.4.15, "unbounded empty DATA frames", low severity, patched in 0.4.16). Tracked by PR #186. #180 must not merge until that is green.

Note: pass 2 speculated the Lint cause was a Python E741 warning in `scripts/watch_handoff_lanes.py`. That guess is WRONG -- the CI log names `cargo deny`. Recorded here because a validator's speculation is not evidence.

---

## Conclusion

Verdict: **APPROVE**
PRIOR_BLOCK_CLEARED: YES
CRYPTO_TOUCHED: NO
REGRESSION RISK: NONE
