# Ledger binds peer identities to addresses that are not theirs -- self, NAT-shared, loopback, Docker, VPC-internal

Status: Active
Severity: P1 systemic (best current explanation for "LAN nodes never detected")
Discovered: 2026-08-09, Windows lane, live soak at anchor `49bc3f56`
Gate: `core/src/transport/` -- MANDATORY adversarial review before any fix

## Summary

Four separately-reported symptoms are one defect: **the ledger treats
`ip:port` as a stable peer identity, and it is not.** Peer IDs are being bound
to addresses that provably cannot belong to them, including this node's own
address.

## Evidence, one 50-minute run

### 1. Identity churn on a shared NAT address -- 9 events on ONE address

`PeerID changed` fired 18 times. Distribution:

| Count | Address | What it is |
|---|---|---|
| 9 | `147.81.41.188:6891` | the household's **shared public NAT address** |
| 3 | `192.168.0.121:443` / `:8080` | **THIS NODE'S OWN LAN ADDRESS** |
| 2 | `54.226.67.101:9001` | the AWS relay |
| 2 | `192.168.0.111:443` | Android |
| 1 | `192.168.0.136:9090` | macOS |

Every node in this fleet sits behind one NAT, so they all egress from
`147.81.41.188`. The ledger sees one address whose PeerId keeps changing and
rewrites the binding each time. Behind a shared NAT, `ip:port` cannot identify
a peer -- but the ledger is treating it as if it can.

### 2. It bound another peer's identity to OUR OWN address

```
[WARNING] PeerID changed at 192.168.0.121:8080: 12D3KooWJUJ1koSW... (iOS)
[WARNING] PeerID changed at 192.168.0.121:443:  12D3KooWJUJ1koSW... (iOS)
```

`192.168.0.121` is this node's own LAN address (confirmed against
`/api/listeners`). The ledger recorded the iOS peer as living at our address.
Any dial to "iOS" using that entry targets ourselves.

### 3. Android does the same thing to the relay, 336 times

From the Pixel 6a's own log:

```
Dial error: Unexpected peer ID <Android's OWN peerid>
            at /ip6/::1/tcp/9001/p2p/<relay peerid>
```

Android holds a ledger entry claiming the AWS relay is at `::1` -- its own
loopback. It dials, reaches itself, and rejects on PeerId mismatch. 336
occurrences. **This is why Android is not connected to the relay**, which is
in turn why relayed probes to Android do not land.

### 4. Windows accepted a peer at a Docker bridge address

```
Connected to <macOS peer> via /ip4/172.17.0.1/tcp/443
             (promiscuous mode - any PeerID accepted)
```

Filed separately as `PROMISCUOUS_ACCEPT_UNROUTABLE_ADDR_2026-08-09.md`; it is
the same root disease. Plus 68 dial failures to private ranges carrying
`/p2p-circuit`, including the relay's VPC-internal `172.31.19.216`.

## Why this is likely THE explanation for the field report

The standing field complaint is "the other nodes on the same LAN were never
detected". With addresses misattributed like this:

- dials go to the wrong host (or to self) and fail,
- failures increment `failure_count` against the WRONG peer,
- entries hit `LEDGER_DEAD_FAILURE_THRESHOLD` and get marked dead,
- and `success_count` never rises -- which then hides the peer from the
  Android render path entirely
  (`ANDROID_LEDGER_VISIBILITY_ROOT_CAUSE_2026-08-09.md`).

That chain converts an address-attribution bug into a permanent
invisible-peer condition, which matches the symptom better than anything
proposed so far.

## What must be established (do NOT jump to a fix)

1. Where does the ledger key an entry -- by address, by `(peer_id, address)`,
   or by peer with an address list? Cite the code.
2. What does `PeerID changed` do on fire: rewrite the binding, or add? If it
   rewrites, two nodes behind one NAT will fight over one row indefinitely.
3. Should an address ever be recorded from a promiscuous accept?
4. Should this node's OWN listen addresses be excluded from every peer's
   candidate set? That check appears to be missing or not reached.
5. Does `addr_filter.rs` have a self-address / RFC1918-from-remote rule, and
   does it run on the ledger-ingest path?

## Falsification

If this is right, a node whose ledger is pruned of (a) its own addresses,
(b) any address learned from a remote peer that is in a private range not on
the local subnet, and (c) shared-NAT external addresses used as identity keys,
should show a sharp drop in failed dials and should start seeing LAN peers.
Test on one node before proposing a core change.

## Related tickets

- `PROMISCUOUS_ACCEPT_UNROUTABLE_ADDR_2026-08-09.md` (accept side)
- `ANDROID_LEDGER_VISIBILITY_ROOT_CAUSE_2026-08-09.md` (render consequence)
- `ANDROID_INBOUND_CRYPTOERROR_2026-08-09.md` (Android/relay disconnection)
