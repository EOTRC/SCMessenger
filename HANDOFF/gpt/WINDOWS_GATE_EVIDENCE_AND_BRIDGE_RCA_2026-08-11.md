# Windows lane -- gate evidence + inbox-bridge RCA (2026-08-11)

Status: Active
Last updated: 2026-08-11

Written for the GPT-MAC lane so both lanes hold the same record. Supplements
`WINDOWS_PR139_RESUME_STATE_2026-08-11B.md`; corrections to that document are
called out explicitly below.

## Headline corrections to the record

1. **The mesh-formation blocker is CLEARED.** `WINDOWS_PR139_RESUME_STATE_2026-08-11B.md`
   records Android at `0 peers (Core), 0 full, 0 headless`. A fresh logcat pull on
   2026-08-11T18:19:26Z shows all 20 `Mesh Stats` samples in the window reading
   **`3 peers (Core), 3 full, 0 headless`**. Android is forming p2p-circuit paths
   through both the Mac node (`12D3KooWNC5r...`) and the relay (`12D3KooWPJK6...`).
   The device is still dual-homed (`192.168.0.135` LAN plus `10.8.223.228`), so the
   VPN lead did **not** have to be resolved for peers to form. That lead is no longer
   the blocker and should be de-prioritised.

2. **`acked_without_receipt_protection` did NOT clear on `c93dfec5`.** Defect 3 of the
   resume doc stands. The device still holds a backlog logging
   `transport-acked message cannot be downgraded acked_count=1`, one message at
   `attempt=6`. Receiver-backed receipt evidence for Android is therefore still absent.

3. **Relay SSH details in the record are wrong.** The live host is
   `ec2-user@54.226.67.101` and docker requires `sudo`. The documented
   `ubuntu@54.242.56.150` times out on port 22, and `ubuntu@` is rejected on the live
   host.

4. **There is no `/version` endpoint.** `/api/version`, `/api/health` and `/api/info`
   reset the connection on the CLI control API; only `/api/identity`, `/api/peers`,
   `/api/history` and `/api/send` are served. Gate requests asking for `/version` on
   the Windows node cannot be satisfied as written. GPT-MAC has dispositioned this as
   an API gap, not as evidence against the node.

## Gate evidence (2026-08-11, no PASS claimed -- gate remains CLOSED)

### Windows node

| Field | Value |
|---|---|
| Binary | `%LOCALAPPDATA%\scmessenger\soak\bin\scmessenger-cli-053fd137.exe` (PID 25268) |
| SHA-256 | `e9501c5c67fe867fb466f5f5ca7dd5c24e64c008b3d794b059686961abc72f7f` |
| Repo head | `86fa1f7be0153e196350ca56164d44168d93f2a9` (`tracking/pre-v040-tag-work`) |
| Identity | `985a25f9505372de3eeea4fe6220784a956da88cf6681f57f9e5ffd92bf65826` |
| Peer id | `12D3KooWD6vZQrUqpyGaCqY3tNSK8p44BS78TvxpGpwhdPJ1T9mw` |

The SHA-256 matches the recorded pin exactly. The binary is built from `053fd137`,
which is **not** head. Per GPT-MAC: record `053fd137` as the deliberate Windows/AWS
parity pin, not as PR-head provenance.

### Android

| Field | Value |
|---|---|
| APK SHA-256 | `7c89f7a77177bd50eb95c9fae286dbf702e8d9029175befa0fa50f570bc45392` |
| Version | `versionName 0.4.0`, `versionCode 14`, minSdk 26, targetSdk 35 |
| Installed | 2026-08-11 00:43:13 device-local (HST) |
| Peer id | `12D3KooWNnPi9wqUJ7Jypj6g4jHmW2PUTmynUs9sJY1h6SQbjLrG` |

The hash was computed twice and agrees: device-side `toybox sha256sum`, and an
`adb pull`ed copy hashed on Windows.

**GAP -- build provenance.** The APK carries no recoverable commit SHA. Nothing in
1359 lines of app-scoped logcat emits a build or commit identifier, and `dumpsys`
exposes only `versionName`/`versionCode`. The APK **cannot** be bound to a commit from
the artifact itself, so "APK built at head" in the resume doc is UNVERIFIED. This is
`HANDOFF/todo/P1_STALE_BUILD_PROVENANCE_INVALIDATES_SHA_CLAIMS_2026-08-09.md`.

**GAP -- receiver-backed receipts.** Not available; see correction 2 above.

### AWS cloud node

| Field | Value |
|---|---|
| Container | `scm-relay`, image `testbotz/scmessenger:sha-053fd13`, Up 8 hours |
| Repo digest | `testbotz/scmessenger@sha256:7e9e3d75490d83f24a8b3b4f553362b5b68fff1682fddaa3eab048ebe8d61e16` |
| Local image id | `sha256:bddc67db4d801f631ac5ec8292a45ff3e2bfa123427f309f1165b6c68a1c2214` |
| Health | `/api/health` -> `{"status":"healthy"}` |
| Peers | 3 -- Windows, Mac, ChristyLove |

**FINDING -- relay reports no identity.** `/api/identity` on the relay returns
`initialized: false` with `device_id`, `identity_id`, `libp2p_peer_id` and
`public_key_hex` all `null`, nickname `scm-always-on-node`, while the container is
healthy and actively relaying. GPT-MAC has ruled that **relay-side receipt evidence
does not count toward the gate** until this identity is resolved or explicitly
dispositioned.

## RCA -- the inbox bridge silently dropped every GPT-MAC message

**Symptom.** GPT-MAC sent a gate re-request that produced no HANDOFF ticket, no ACK,
and no wake. It was found only by reading `/api/history` by hand.

**Root cause.** `%APPDATA%\scmessenger\inbox_bridge.json` carried a single
`allowed_peer_id`, the Android identity `a43772fe...`. GPT-MAC's identity
`3854e442...` was absent. Design rule 4 of `scripts/inbox_bridge.py` gives
non-allow-listed senders no ticket and no ACK by intent, so the drop was silent and
correct-by-design. The status file recorded `ignored_inbound_in_window: 50` against
`allowlisted_inbound_in_window: 1`.

**Contributing cause.** The running bridge (pid 10340, started 09:48:58Z) was an
orphan, not the supervisor's child: `soak/status.json` showed `bridge_pid: null`.
`soak_supervisor.py` launches a bridge under `--with-bridge`, but the single-instance
lock was already held by the orphan, so its child exited repeatedly and the supervisor
gave up after 3 attempts (`bridge_failures > 3`). A config fix would therefore never
have been picked up by a supervisor respawn -- there was none to respawn.

**Fix.** `allowed_peer_id` is now a list carrying both identities. `cmd_run` reads
config once at startup, so a restart was required.

**Restart, performed in a controlled window 2026-08-11T18:50:03Z.** The node soak was
deliberately NOT touched (generation 8, started 17:26:14Z, healthy throughout); only
the orphaned bridge was replaced. New bridge pid 25740.

Verification, from the bridge status file immediately after restart:

| Counter | Before | After |
|---|---|---|
| `ignored_inbound_in_window` | 50 | **0** |
| `allowlisted_inbound_in_window` | 1 | **50** |

The traffic the bridge was silently discarding is now accepted.

**Standing risk this exposes.** A single-identity allow-list plus silent-by-design
drops means any new lane joining the fleet is invisible to the bridge until someone
notices the absence of replies. Consider surfacing `ignored_inbound_in_window > 0` as
a health signal rather than a passive counter.

## Evidence artifacts

Held under `tmp/` on the Windows host and deliberately NOT committed -- `tmp/` is
gitignored, the raw captures are unredacted, and this repository is public.

| Artifact | SHA-256 |
|---|---|
| `tmp/gate_evidence_20260811T1848Z/bridge_status_prerestart.json` | `00fb3529421d036bb9a4f5480ec444692f910595aa33abd9a478eddedd04f5f6` |
| `tmp/gate_evidence_20260811T1848Z/bridge_config_prefix.json` | `6c03d31e1f970d31a2ce5b1bc9162b3e4dfc9deaf3930d90a41439a162e5ba84` |
| `tmp/gate_evidence_20260811T1848Z/bridge_config_current.json` | `5a4bbff4c08ba07b4197b3ac618a3279a6bf73639f6324373a2c88ea480a1b70` |
| `tmp/gate_evidence_20260811T1848Z/history_snapshot.json` | `0378d4f27ea972639e7519e3419670b3361cdf6c68a45243cdc4cfd9a1c40c73` |
| `tmp/androidlogs_20260811T181926Z/logcat_app.txt` | 304083 bytes, 1359 lines, span 18:03-18:18Z |

The full app-scoped logcat was transferred to GPT-MAC over the mesh in 87 chunks and
they confirmed receiver-side verification of all 87 plus the completion marker. The
2.8 MB full-system logcat is held on the Windows host and has not been sent.
