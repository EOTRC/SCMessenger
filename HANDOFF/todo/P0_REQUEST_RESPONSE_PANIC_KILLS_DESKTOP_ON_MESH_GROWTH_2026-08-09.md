# P0 -- desktop node dies from a libp2p-request-response assertion when the mesh grows

Status: Active
Severity: P0 (blocks the five-node run; the Windows node cannot survive the fleet assembling)
Discovered: 2026-08-09, live on the candidate during the post-UPnP soak
Observed on: `33c16712` (native behaviour identical to `bfc7cac9`)
Affects: desktop CLI nodes (Windows confirmed; macOS shares the code path)

## Summary

With UPnP removed, the Windows node ran **93 minutes clean with a single peer**.
Within **2 minutes of the mesh growing to four nodes** it panicked inside
`libp2p-request-response` and the swarm event loop died.

This is a **second, distinct desktop-killing panic**, unrelated to the UPnP one
that `HANDOFF/todo/P0_UPNP_PANIC_KILLS_DESKTOP_NODE_2026-08-08.md` covers. Fixing
UPnP did not fix this; it only removed the failure that was masking it.

```
thread 'tokio-rt-worker' (23688) panicked at
  libp2p-request-response-0.29.0\src\lib.rs:678:9:
assertion `left == right` failed
  left: false
 right: true
2026-08-09T12:22:49.824735Z ERROR scmessenger_cli: swarm_event_loop_died:
  the mesh is down but the process is still up; exiting so this node does not
  linger as a zombie
```

## This is the panic the candidate's own guard was written to prevent

`core/src/transport/swarm.rs`, above the `ConnectionClosed { num_established > 0 }`
arm, documents this exact signature:

```
//   libp2p-request-response-0.29.0/src/lib.rs:678
//   assertion `left == right` failed (left: false, right: true)
//
// Observed on the Windows node during 5-node run 1: three ...
```

The guard is present and correct in this build -- partial closes call
`remove_connection_by_id` and skip peer-level teardown. **It is not sufficient.**
The panic reproduced anyway once multiple peers with multiple paths were live.

## Timeline (UTC, 2026-08-09)

| Time | Event |
|---|---|
| 10:49:47 | node start, single peer (Pixel) |
| 10:49-12:20 | **93 minutes clean**, zero panics, bidirectional traffic |
| 12:20:49 | macOS node `12D3KooWNC5rEKFhuxDNDNsJ6Q58Ca75LnxfjUqspGzGRdYRUWyt` (192.168.0.136) identified as RELAY |
| 12:20:51 | disconnect from `12D3KooWJUJ1...` (192.168.0.142) |
| 12:20:52 | relay reservation accepted from `12D3KooWJUJ1...` |
| 12:21:52 | relay circuit reservation registered, `ListenerId(30)`; **nested multi-hop circuit listeners appear** |
| 12:21:53 | repeated dial-backoff resets against the macOS node |
| 12:22:36 | Pixel re-identified as RELAY |
| 12:22:49 | **panic, swarm loop dead, process exits** |

Uptime 93 minutes; time from four-node convergence to death **under 2 minutes**.

## Suspicious observation: self-referential multi-hop circuits

Immediately before the panic the node registered circuit listeners that route
**through a peer and back to itself**, two `p2p-circuit` hops deep:

```
/ip6/::1/tcp/8080/p2p/12D3KooWD6vZ<SELF>/p2p-circuit/p2p/12D3KooWJUJ1<PEER>/p2p-circuit/p2p/12D3KooWD6vZ<SELF>
/ip6/::1/tcp/9001/p2p/12D3KooWNC5r<MAC>/p2p-circuit/p2p/12D3KooWJUJ1<PEER>/p2p-circuit/p2p/12D3KooWD6vZ<SELF>
```

A circuit whose destination is this node, reached via this node, is pathological
on its face. Whether it is the direct trigger is unproven, but it is the most
anomalous state present at the time of the crash and it only appears once three
or more relay-capable peers are live. Every node advertises as a relay
(`scmessenger/0.4.0/full/relay/...`), so circuit topologies multiply quickly as
the fleet grows -- consistent with a failure that needs four nodes to show up.

## Why the earlier soak did not catch it

The 93-minute soak proved the UPnP fix and nothing more. It ran with **one
peer**. This panic requires multiple peers with multiple paths, which is exactly
what a five-node run creates deliberately.

**Soak methodology must change:** a single-peer soak is not evidence that a
desktop node survives a fleet. Any future soak must include at least three live
peers with relay circuits established.

## Impact on the five-node run

The Windows node would very likely die within minutes of the fleet assembling,
mid-run, producing what looks like transport, receipt or custody failures
downstream. This must be resolved or explicitly accepted before Run 1.

The CLI's own safety net worked correctly again: it detected the dead swarm loop
and exited rather than lingering as a zombie with no mesh.

## Evidence

Preserved on the Windows host (not committed, size):

- `soak1_stdout_DIED_1222.log` -- full node stdout for the 93-minute run
- `soak1_stderr_DIED_1222.log` -- the panic text (stderr only; the rolling
  `scm.log.*` file does NOT contain it, only the `swarm_event_loop_died`
  aftermath)

Note for anyone reproducing: **the panic never appears in the tracing log.**
Capture stderr separately or the root cause is invisible.

## Next steps

1. Node restarted 12:24:09Z with the full debug filter
   (`RUST_LOG=info,scmessenger_core::transport=debug,...`) to capture dial and
   connection-close detail if it recurs. Watch for a second occurrence to
   establish reproducibility, as was done for the UPnP panic.
2. Determine whether the self-referential circuit listeners are cause or
   symptom.
3. Establish whether the `ConnectionClosed` guard is incomplete, or whether the
   assertion is reached by a different path entirely (outbound request bookkeeping
   against a connection that closed underneath it is the obvious candidate).
4. Check whether upstream `libp2p-request-response` has a newer release that
   addresses this assertion, exactly as was done for `libp2p-upnp`.
5. Re-run the soak with three or more live peers before calling it clean.

## Acceptance criteria

1. A desktop node survives **at least 30 minutes with four or more live peers**
   and established relay circuits, with zero panics.
2. Reproduction attempted twice; both clean.
3. If the fix is upstream, the version bump is verified to contain it -- do not
   assume a version number fixes a panic without reading its changelog.

---

## ROOT CAUSE IDENTIFIED (2026-08-09, from the actual crate source)

Read directly from
`~/.cargo/registry/src/index.crates.io-*/libp2p-request-response-0.29.0/src/lib.rs`.
The assertion is **not** about pending requests. Line 678, inside
`on_connection_closed`:

```rust
debug_assert_eq!(connections.is_empty(), remaining_established == 0);
```

Our panic reported `left: false, right: true`, so:

- `connections.is_empty()` == **false** -- request-response still tracks one or
  more connections to that peer,
- `remaining_established == 0` == **true** -- the swarm says no connections to
  that peer remain.

**The behaviour's internal `connected` map has drifted out of sync with the
swarm's connection accounting.** It is holding at least one stale connection
entry for which it never received a `ConnectionClosed`. This is a
connection-bookkeeping drift, not an outbound-request leak.

Consequence in the same function: because `connections.is_empty()` is false, the
peer is **not** removed from `self.connected`, so the stale entry persists.

### This is a `debug_assert_eq!` -- profile changes the symptom

**Debug builds panic. Release builds do not.** In release the assertion is
compiled out and the node continues with drifted state: a stale connection entry
retained indefinitely, and a peer never cleared from `connected`.

The node that crashed was `target/debug`. So:

- A five-node run using **debug** desktop binaries will see nodes die.
- A five-node run using **release** desktop binaries will NOT crash, but will
  accumulate inconsistent request-response state, with unclear effects on
  response routing and memory over a long run.

Neither is "fine". Choosing release to make the crash disappear would be hiding
the defect, not fixing it, and it must not be presented as a pass.

### No upstream fix available

`libp2p-request-response` tops out at **0.29.0** on crates.io (0.27.0, 0.28.0,
0.28.1, 0.29.0). There is no newer release to bump to, unlike the `libp2p-upnp`
path. Any fix has to be ours: stop generating the drift, or work around it.

### Corrected investigation direction

The question is no longer "which pending request leaked". It is: **how does a
connection get registered in request-response and never produce a matching
`ConnectionClosed`?** Candidates, in order:

1. Multiple simultaneous connections to one peer (direct + relayed + circuit),
   where one path is torn down by a route other than a `ConnectionClosed` event
   the behaviour observes.
2. Circuit/relay listener teardown (`swarm.remove_listener` on relay reservation
   cleanup) removing a transport underneath an established connection.
3. The self-referential circuit listeners noted above producing a connection
   whose peer is this node, which request-response may account differently.

### Note on a delegated analysis that was wrong

A THINK-tier worker analysis claimed the assertion checks
`self.outbound.pending.is_empty()`, and asserted that no code path drops an
inbound channel without responding. **Both claims are false.** The assertion is
the connection/`remaining_established` comparison quoted above, and
`core/src/transport/swarm.rs:3540` and `:6731` both call `drop(channel)` on the
blocked-peer address-reflection path without sending a response. Its file:line
citations were also fabricated -- it cited `ConnectionClosed` at lines 2682-2724
when the real arms are at 4915 and 4930.

Recorded because it is a reusable lesson: a confident analysis with precise-looking
line numbers is not evidence. The crate source was on this machine the whole time
and settled the question in one read.
