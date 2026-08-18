# Field evidence -- v0.4.0 rollout to Android + Windows, 2026-08-18

Status: Active
Build under test: `b4ccd30a` (main, post-#139 trunk merge)
Targets: Pixel 6a (bluejay, Android) and Windows desktop node. AWS/iOS/macOS
explicitly out of scope for this run, per operator.

This records what a real two-node rollout on a normal home LAN actually did. It
exists because two defects that CI calls green are, in the field, product-fatal.

---

## 1. What the rollout proved works

| Item | Evidence |
|---|---|
| Windows node runs from main | binary self-reports `Core Provenance: 0.4.0 (b4ccd30a:HEAD)` |
| Android builds and installs from main | `BUILD SUCCESSFUL`, fresh install, `firstInstallTime=2026-08-18 13:02:40` |
| App starts and the mesh comes up | pid alive, empty crash buffer, `MeshRepository` emitting stats, listening on `/ip4/192.168.0.129/tcp/9001` |
| **PR #176 manifest restore works on hardware** | `pm query-activities -a android.intent.action.VIEW -d scmessenger://invite` resolves to `com.scmessenger.android/.ui.MainActivity` |

The deep-link entry point restored in #176 is confirmed live on a device. That
path was dead before today.

---

## 2. DUAL_BIND is not a theoretical D4 risk. It is breaking LAN messaging now.

`HANDOFF/todo/P0_DUAL_BIND_TCP_AND_WS_ON_SAME_PORT_2026-08-10.md` was
dispositioned on 2026-08-17 as D4 work on the reasoning that a phone *might*
dial the wrong socket. The field is worse than that reasoning.

**Observed on the Windows node in a single session:**

```
High rate of incoming negotiation failures from /ip6/::1/tcp/80 -> /ip6/::1/tcp/9090:
  Listen error: Failed to negotiate transport protocol(s)
[DIAL-BACKOFF] Peer marked as dead after 3 failed attempts
```

| Measure | Value |
|---|---|
| `Failed to negotiate transport protocol(s)` occurrences | **14,496** |
| references to port 80 in the storm | 11,635 |
| unique peers marked dead by dial-backoff | **13** |
| peers the phone discovered | **0** |
| Windows node outcome | exited |

**Mechanism, confirmed rather than inferred.** The node's own listeners include
`/ip6/::1/tcp/9001/ws`, `/ip6/::1/tcp/443/ws`, `/ip6/::1/tcp/80/ws` -- WebSocket
on those ports -- while `core/src/transport/multiport.rs:73-80` also advertises
plain TCP on the *same* ports. Every failing pair in the log is `::1 -> ::1`:
**the node dials itself across its own port matrix and fails negotiation each
time.** Real peers get swept into the same dial-backoff and marked dead.

The phone independently advertises the same colliding pattern:

```
/ip4/192.168.0.129/tcp/9001/ws
/ip4/192.168.0.129/tcp/9001
```

repeated for 9090, 8080, 80 and 443.

**Consequence:** two nodes on the same LAN, both healthy, both listening, cannot
complete a handshake. An in-app message between them stays queued. This is the
product's core function failing on the simplest possible topology.

**Severity: raised from "D4 work" to tag-blocking.** The 2026-08-17 disposition
underestimated it; this supersedes that assessment.

Fix direction remains an operator decision per AGENTS.md rule 9. CTO
recommendation is unchanged and strengthened by this evidence: **bind one
transport per port and advertise only what actually bound.** It is the only
option that does not change the addressing model, and it directly removes the
self-dial storm.

---

## 3. A second regression of the same class: features wired out, not deleted

The operator reported QR-based APK sharing missing, along with the diagnostics
and logs reachable from it. Root cause found:

`ui/dialogs/ApkShareDialog.kt` exists and implements both native system share
and **local-node QR-hosted sideloading (an ephemeral HTTP server on the local
Wi-Fi)**. It has exactly ONE reference in the entire tree -- its own
declaration at `:36`. **Zero callers.**

`ebf5411b^:ui/screens/SettingsScreen.kt` had all three wiring points:

```
:79   var showApkShareDialog by remember { mutableStateOf(false) }
:255  onShareApkClick = { showApkShareDialog = true }
:447  ApkShareDialog(onDismiss = ...)
```

All three are absent from the current file. The implementation was restored
after `ebf5411b`; **the call sites were not.**

This is the identical shape as the manifest defect fixed in #176: live code, no
entry point, compiles clean, passes lint, dead at runtime. It also directly
damages **D2** -- QR sideloading is how a friends-and-family build gets
distributed.

`CTO-ANDROID-WIRING-AUDIT` is dispatched to find every remaining instance.

---

## 4. Process findings from this run

- **A missing core native library did not fail the build.** An APK built with a
  non-default `CARGO_TARGET_DIR` packaged `libjnidispatch.so` but not
  `libscmessenger_core.so`, because `app/build.gradle:227` sources jniLibs from
  `../../core/target/android-libs` while cargo-ndk had written elsewhere.
  Gradle reported `BUILD SUCCESSFUL in 48m24s`; the app died instantly with
  `UnsatisfiedLinkError`. **A build that cannot start should not pass.** Same
  vacuous-success class as the `ffi_surface.sh` trap in `clean_target.sh`.
- **Android crashes do not appear in the main logcat buffer.** Diagnosing from
  `adb logcat` alone produced a confident wrong answer (memory pressure);
  `adb logcat -b crash` had the real `UnsatisfiedLinkError` immediately. Check
  the crash buffer first.
- The device did show genuine memory pressure (swap 368 kB free of 3.1 GB) and
  the LMK was killing Google's own services. It was real, and it was not the
  cause. Concurrent real symptoms make a wrong diagnosis look supported.
