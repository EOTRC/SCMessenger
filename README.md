# SCMessenger

**Status**: Active
**Last updated**: 2026-07-24
**Version**: v0.3.5 (alpha, driving to v1.0.0)

[![CI](https://github.com/Sovereign-Communication/SCMessenger/actions/workflows/ci.yml/badge.svg)](https://github.com/Sovereign-Communication/SCMessenger/actions/workflows/ci.yml)
[![License: Unlicense](https://img.shields.io/badge/license-Unlicense-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](rust-toolchain.toml)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

**Messaging that works when the internet does not.**

SCMessenger is a sovereign, end-to-end encrypted, decentralized messaging
mesh. No servers. No accounts. No phone numbers. Your identity is a keypair
you generate on-device; your messages travel over whatever path physically
exists between you and your peer -- Bluetooth in a crowd, WiFi on a plane,
LAN at home, or a relay across the internet.

## Why it exists

Every mainstream messenger dies with its servers: censored, subpoenaed,
rate-limited, or simply offline. SCMessenger assumes the worst-case network
from the start -- no internet, no WiFi, a stranger's phone passing by on
BLE -- and treats the happy path as a bonus. If any radio on your device can
reach any radio on theirs, directly or through intermediate custody, the
message gets through.

## How it works

### Transports

SCMessenger implements several transports and picks between them using a
health- and score-based policy, not a fixed fallback chain:

| Transport | Notes |
|---|---|
| BLE | Bluetooth LE, for proximity with no network at all |
| WiFi Aware / WiFi Direct | Android; peer-to-peer without an access point |
| Multipeer | iOS client-side transport |
| LAN | TCP, QUIC-v1, and WebSocket listeners; peers found via mDNS (Android uses `NsdManager` instead) |
| Relay | QUIC/TCP relay plus store-and-forward custody for offline peers |

Transport selection is driven by an escalation policy
(`core/src/transport/escalation.rs`) with modes `Balanced` (default),
`PreferHighBandwidth`, `PreferLowLatency`, and `PreferLowPower`. Per-peer
scoring lives in `core/src/transport/manager.rs`. Android additionally races
candidate transports in parallel after trying the health-preferred one.

- **Adaptive ports**: if the default port is firewalled, listeners and
  dialers ladder through 443, 80, 8080, and 9090, plus an optional random
  ephemeral port -- whatever lands traffic on that network is the right port.
  Last-good `{transport, port}` pairs are remembered per peer per network
  fingerprint (`core/src/store/transport_memory.rs`) and tried first on the
  next dial.
- **Relay custody**: offline peers do not lose messages. Relays hold
  encrypted envelopes until receipt confirmation, then release custody.
- **Store**: local sled database on native platforms, IndexedDB in the
  browser. Nothing leaves the device unencrypted.

### Cryptography

- Identity: Ed25519 signing keys, generated and held on-device.
- Sessions: X25519 ECDH with a double ratchet; XChaCha20-Poly1305
  authenticated encryption (24-byte nonce) for every message.
- **Post-quantum**: hybrid X25519 + ML-KEM-768 key agreement (via
  `libcrux-ml-kem`) with a domain-separated transcript, cipher-suite
  negotiation, versioned wire envelopes carrying a `suite` byte, and a
  PQ-augmented ratchet that re-encapsulates every 100 messages. This is
  compiled unconditionally -- there is no opt-in feature flag -- and ML-DSA-65
  signatures are also implemented. Old data stays decryptable; new sessions
  negotiate the strongest suite both ends advertise. A second adversarial
  review pass is still outstanding before v1.0.0 (see
  [docs/V1_KNOWN_LIMITATIONS.md](docs/V1_KNOWN_LIMITATIONS.md)).
- Privacy layer: onion routing (up to 5 hops) and cover traffic for
  metadata-resistant delivery, toggled via
  `scm config privacy --onion --cover-traffic --padding`.
- Verification: property-based tests (proptest), Kani formal proofs on crypto
  paths behind the `kani-proofs` feature (Linux/macOS only), and a standing
  adversarial-review gate on every change to crypto, transport, routing, or
  privacy code.

## Platforms

| Platform | Client | State |
|---|---|---|
| Windows / Linux / macOS | `scmessenger-cli` headless daemon + local web UI | Active; Windows CLI <-> Android validated end-to-end across LAN/TCP/relay (Phase 1 exit, 2026-07-10) |
| Android | Kotlin / Jetpack Compose app | Active; full transport stack incl. BLE + WiFi Aware/Direct |
| iOS | SwiftUI app | Feature-parity codebase (BLE, Multipeer, LAN, relay); bindings regen pending |
| Browser | WASM thin client over local JSON-RPC WebSocket | Active |
| Linux desktop (KMP) | Compose Multiplatform | Planned (v1.0 scope) |

One Rust core (`scmessenger-core`) drives all of them via UniFFI bindings
(Android/iOS) and JSON-RPC 2.0 (browser).

## Quick start

```bash
# Prerequisites: Rust stable (rustup), Git
git clone https://github.com/Sovereign-Communication/SCMessenger.git
cd SCMessenger
```

Build and test the core:

```bash
cargo build --workspace
cargo test --workspace
```

Run a node:

```bash
cargo run --release --bin scmessenger-cli -- start
```

Run a headless relay node:

```bash
cargo run --release --bin scmessenger-cli -- relay --listen /ip4/0.0.0.0/tcp/0
```

The binary is `scmessenger-cli`, but it presents itself as `scm` in its own
help output and examples (`scm start`, `scm status`, `scm stop`).

**Default ports** (with `--http-port 9000`):

| Port | Bind | Purpose |
|---|---|---|
| 9000 | 127.0.0.1 | HTTP web UI, and the browser client's JSON-RPC WebSocket at `/ws` |
| 9001 | 0.0.0.0 | libp2p P2P (derived as HTTP port + 1) |
| 9002 | 0.0.0.0 | libp2p WebSocket transport |
| 9876 | 127.0.0.1 | Local control API |

The web UI and control API bind to loopback only; `--http-bind` is an
explicit opt-in to change that. The P2P and WebSocket transport listeners are
data-plane and bind all interfaces by design.

Linux builds need `libdbus-1-dev` and `pkg-config` -- these are required to
build the CLI crate at all, not only for BLE.

Platform guides: [Android](docs/platform/ANDROID_SETUP.md) |
[iOS](docs/platform/IOS_SETUP.md) | [WASM](docs/platform/WASM_SETUP.md) |
[CLI](docs/platform/CLI_SETUP.md)

## Workspace layout

```
core/            scmessenger-core: identity, crypto, transport, store, routing, relay, privacy
cli/             scmessenger-cli: headless daemon + embedded web server
mobile/          scmessenger-mobile: UniFFI bridge crate (Android/iOS bindings)
wasm/            scmessenger-wasm: browser thin-client (JSON-RPC over WebSocket)
desktop_bridge/  scmessenger-desktop-bridge
android/         Kotlin/Compose app
iOS/             SwiftUI app
docs/            canonical documentation
HANDOFF/         live task backlog
```

`wasm/` is built separately with `--target wasm32-unknown-unknown` rather
than as part of a plain workspace build.

## Documentation

- [DOCUMENTATION.md](DOCUMENTATION.md) -- docs hub and navigation
- [Architecture](docs/ARCHITECTURE.md) -- system design
- [Current State](docs/CURRENT_STATE.md) -- verified implementation status
- [Known Limitations](docs/V1_KNOWN_LIMITATIONS.md) -- what does not work yet
- [v1.0.0 Execution Plan](HANDOFF/V1_0_0_EXECUTION_PLAN.md) -- the road to 1.0
- [Testing Guide](docs/TESTING_GUIDE.md) -- gates and test inventory
- [Protocol](docs/PROTOCOL.md) -- wire contract

## Contributing

Contributions welcome -- see [CONTRIBUTING.md](CONTRIBUTING.md). The short
version: fork, branch, `cargo test --workspace`, `cargo fmt` +
`cargo clippy`, conventional commits, PR. Changes to `core/src/{crypto,
transport,routing,privacy}` require adversarial security review before
merge (see [SECURITY.md](SECURITY.md)).

## Security

Do not open public issues for vulnerabilities. Report privately via GitHub
Security Advisories. Policy: [SECURITY.md](SECURITY.md).

## License

Public domain under [The Unlicense](LICENSE). Take it, fork it, ship it --
sovereignty includes the code.
