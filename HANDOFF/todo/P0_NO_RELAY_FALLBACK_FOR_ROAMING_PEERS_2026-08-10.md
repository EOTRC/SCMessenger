# P0 -- a roaming peer is unreachable: the node never falls back to the relay

Status: Open -- observed LIVE during the iteration-2 roaming condition
Filed: 2026-08-10 ~04:25Z, anchor `68fcc3f1`
Severity: P0. This is the core product promise. A device that leaves WiFi
becomes unreachable, and the relay that exists to solve that is never tried.

## The condition

iOS left the home LAN and is roaming on cellular. Android stayed on home WiFi.
Windows and the AWS relay stayed up. This is the exact scenario the product must
handle: one participant goes out, the others stay put.

Windows is **connected to the AWS relay** at the time of every failure below.

## What the node actually did

Address attempts for the roaming iPhone (`12D3KooWJUJ1ko...`) in one run:

| Address attempted | Count | Why it cannot work |
|---|---|---|
| `/ip6/::1/tcp/9001` | **13** | **loopback -- this node dials ITSELF** |
| `/ip4/192.168.0.142/tcp/443` | 9 | stale LAN address; the peer left the LAN |
| `/ip4/192.168.0.142/tcp/80` | 7 | stale LAN address |
| `/ip6/2600:381:...` (carrier) | 14 | behind carrier NAT, not dialable inbound |
| `/ip6/fd74:...` (ULA) | 7 | link-local scope, not routable off-LAN |

**Relay circuit attempts: ZERO.**

The resulting error, verbatim:

```
Outgoing connection error to 12D3KooWJUJ1koSWwSEAX32z6SGaepikyqpJawpojoy6gvQ8k688:
  Dial error: Unexpected peer ID 12D3KooWD6vZQrUqpyGaCqY3tNSK8p44BS78TvxpGpwhdPJ1T9mw
  at /ip6/::1/tcp/9001/p2p/12D3KooWJUJ1koSWwSEAX32z6SGaepikyqpJawpojoy6gvQ8k688
```

Read that carefully: **while trying to reach the iPhone, the node connected to
itself over loopback and discovered its own PeerId at the far end.** It did this
thirteen times, more than any other candidate.

## Three distinct defects, compounding

1. **No relay fallback.** The node holds an active circuit-relay connection to
   the AWS relay and never attempts a circuit to the unreachable peer. The
   candidate ladder appears to contain only directly-learned addresses. If direct
   candidates are exhausted without a relay attempt, a roaming peer is simply
   unreachable -- the relay provides no benefit at the one moment it exists for.

2. **Loopback addresses are dialled for REMOTE peers.** `/ip6/::1/...` can never
   reach another device. It is the single most-attempted candidate here, and it
   resolves to this node. This is the self-dial defect, but worse than previously
   filed: it is not merely wasteful, it is **outcompeting** viable candidates in
   the ladder.

3. **Stale addresses are never reaped.** The iPhone's LAN addresses persisted and
   were retried long after it left. Combined with defect 1, the node exhausts
   dead candidates and stops, rather than escalating to the relay.

## Ordering matters as much as membership

Every previous connectivity failure this session was a **dial-ordering** problem,
not a missing-address problem: Android reachable at `.111` while the node retried
`.141`; macOS reachable on plain TCP while `/ws` variants were tried. Same shape
here. The relay path may well be present in the ledger and simply never reached
because loopback and stale LAN entries are ranked ahead of it.

## Acceptance criteria

1. When direct candidates for a peer are exhausted or failing, the node MUST
   attempt a circuit through a connected relay before declaring the peer
   unreachable. Prove it with a log line naming the relay circuit attempted.
2. Loopback (`::1`, `127.0.0.0/8`) and link-local/ULA addresses must never be
   dialled for a REMOTE peer. Note the directional nuance in
   `ITERATION_2_NAT_TRAVERSAL_TEST_2026-08-10.md`: this must not be implemented
   as a blanket external-address block, which would break NAT traversal.
3. Stale addresses must be demoted or reaped once a peer is known to have moved.
4. Regression test: peer known at address A, peer moves, relay connected --
   assert a relay circuit is attempted. It must fail today.
5. Instrument the ladder. Log the ordered candidate list per dial. Every failure
   this session was invisible until the ordering was dumped.

## Related

- `P0_DUAL_BIND_TCP_AND_WS_ON_SAME_PORT_2026-08-10.md` -- wrong transport form
- `P0_NO_MOBILE_BOOTSTRAP_MEANS_NO_OFF_LAN_RENDEZVOUS_2026-08-10.md` -- no rendezvous
- `P1_NESTED_CIRCUIT_ADDRESSES_STILL_FORMED_2026-08-10.md` -- address bloat
- `ITERATION_2_NAT_TRAVERSAL_TEST_2026-08-10.md` -- DCUtR is the only route to a
  direct connection since UPnP was removed, so relay fallback is not optional

## Still to capture

iOS-side logs when the device returns: whether iOS attempted to reach US via the
relay, and whether it recorded the shared external address `147.81.41.188` as
the plan anticipates. This ticket covers only the Windows half.
