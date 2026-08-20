# SCMessenger FFI & Function Wiring Burndown Matrix

**Generated**: 2026-08-20T08:26:19.171948+00:00
**Total Unwired/Stub Functions**: 835 (Unwired: 835, Stubs: 0)
**Corpus**: HANDOFF_AUDIT/REPO_MAP.jsonl, HANDOFF/discovery/REPO_MAP.jsonl
**Ghost entries filtered**: 14 corpus files (88 functions) removed from the tree by build_wiring_graph.py; 0 additional stale entries dropped at generation time

## Overview & Burndown Priorities

This document tracks unwired and stubbed interface functions across **Rust Core**, **Mobile UniFFI**, **Android Kotlin**, and **iOS Swift**.

### High-Priority Stub Implementations (Must be implemented for Phase 4)
| Function | Location | Line | Target Integration Layer |
| :--- | :--- | :---: | :--- |
| (none -- no stubs flagged by the discovery overlay in the surviving corpus) | -- | -- | -- |

### Module Breakdown (Top Modules by Unwired Count)
| Module / File | Total Unwired | Stubs | Status |
| :--- | :---: | :---: | :--- |
| `core/src/iron_core.rs` | 72 | 0 | Pending Audit |
| `wasm/src/lib.rs` | 58 | 0 | Pending Audit |
| `core/src/transport/swarm.rs` | 36 | 0 | Pending Audit |
| `core/src/mobile_bridge.rs` | 30 | 0 | Pending Audit |
| `AgentSwarmCline/scmessenger_swarm/observability_tests.rs` | 28 | 0 | Pending Audit |
| `wasm/src/daemon_bridge.rs` | 25 | 0 | Pending Audit |
| `core/src/privacy/padding.rs` | 24 | 0 | Pending Audit |
| `core/src/crypto/encrypt.rs` | 21 | 0 | Pending Audit |
| `core/src/abuse/reputation.rs` | 16 | 0 | Pending Audit |
| `cli/src/cli.rs` | 14 | 0 | Pending Audit |
| `core/src/routing/smart_retry.rs` | 14 | 0 | Pending Audit |
| `android/app/src/main/java/com/scmessenger/android/data/MeshRepository.kt` | 13 | 0 | Pending Audit |
| `core/src/routing/engine.rs` | 13 | 0 | Pending Audit |
| `core/src/transport/manager.rs` | 13 | 0 | Pending Audit |
| `core/src/transport/health.rs` | 12 | 0 | Pending Audit |
| `core/src/drift/relay.rs` | 11 | 0 | Pending Audit |
| `core/src/routing/resume_prefetch.rs` | 11 | 0 | Pending Audit |
| `core/src/wasm_support/rpc.rs` | 11 | 0 | Pending Audit |
| `core/src/routing/global.rs` | 10 | 0 | Pending Audit |
| `core/src/transport/discovery.rs` | 10 | 0 | Pending Audit |

## Action Plan for Burndown
1. **Mobile UniFFI Surface**: Wire core transport stubs (`MobileBridge`, `CoreBridge.swift`) to active Kotlin/Swift view models.
2. **Observed Stubs**: Replace simulated mock channels with production libp2p and sled store calls.
3. **Dead Code Clearance**: Remove unreferenced diagnostic helpers that are obsolete.
