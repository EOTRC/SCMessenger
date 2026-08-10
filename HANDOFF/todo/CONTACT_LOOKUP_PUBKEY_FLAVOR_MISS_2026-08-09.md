# Contact lookup misses the public-key flavor -- every send to a known contact takes the unknown-peer path

Status: Active
Severity: P1 (delivery-truth; silently downgrades contact validation on every send)
Discovered: 2026-08-09, Windows lane, live node run at anchor `49bc3f56`
Discovered by: running the five-node link, not by code reading
Gate: touches `core/src/` delivery path -- adversarial review required before merge

## Summary

`IronCore`'s send path looks a contact up by **public key**. The contact store
is keyed by **libp2p PeerId**, with a fallback that resolves an **identity_id**.
There is no public-key index. So a send to a contact that demonstrably exists
logs `[WARN] sending to a recipient with no contact record; proceeding
(unknown-peer sends remain allowed)` and proceeds down the unknown-peer branch.

This is the same identifier-flavor class that `T1` fixed for BLOCKS
(`core/src/store/blocked.rs` dual-flavor write). Contacts never got the
equivalent treatment.

## Evidence (live, this run -- not inferred)

Contact exists. `GET /api/contacts` returns:

```json
{"peer_id":"12D3KooWNC5rEKFhuxDNDNsJ6Q58Ca75LnxfjUqspGzGRdYRUWyt",
 "public_key":"b7dc9198306d41952c49410f63cfd19f231536f37e886767753cbccc78616e0f",
 "name":"MacLane-GPT"}
```

That endpoint reads the SAME store as the send path --
`cli/src/api.rs:849-852` calls `core.contacts_store_manager()`, and
`core/src/iron_core.rs:3530-3532` returns `self.contact_manager.read().clone()`.
So this is not a two-store split; it is one store answering `list()` and
missing on `get()`.

Send to that exact public key, same process, seconds later:

```
2026-08-10T06:35:50.192968Z WARN scmessenger_core::iron_core:
  [WARN] sending to a recipient with no contact record; proceeding
```

## Mechanism (file:line)

- Write path keys by PeerId: `core/src/store/contacts.rs:221`
  `let key = contact_key(&contact.peer_id);`
- Read path: `core/src/store/contacts.rs:241` `get(peer_id: String)` builds
  `contact_key(&peer_id)`, and on miss (`:252-253`) tries
  `resolve_identity_id(&peer_id)`.
- Indexed flavors: **peer_id** and **identity_id**. NOT public_key.
- Caller passes a public key: `core/src/iron_core.rs:779-785`
  `self.contact_manager.read().get(recipient_id.to_string())`, where
  `recipient_id` is the recipient public key (the `/api/send` `recipient`
  field is a 64-char public key hex).

`b7dc9198...` is a public key, so the primary key misses and the identity_id
fallback also misses -- identity_id is `blake3(public_key_bytes)`, a different
value. Result: `None`.

## Why this matters beyond a noisy log line

The miss silently routes every send to a KNOWN contact through the
unknown-peer branch, so contact-record-based checks do not run on the path
that carries real traffic. It also defeats the loud, deliberate guard
immediately below it (`iron_core.rs:797-806`), which is designed to catch the
hash/pubkey confusion by scanning contacts -- that scan compares
`blake3(contact.public_key)` against the recipient, so it only fires for an
identity_id-valued recipient, never for this case.

Note this is NOT the same defect the Mac lane hit earlier (a contact whose
`public_key` field literally contained a PeerId string). That one is repaired;
this one is a missing index and is still live.

## Fix sketch (do not implement without the adversarial gate)

Mirror the T1 dual-flavor approach in `core/src/store/contacts.rs`:

1. On write (`:221`), in addition to the PeerId-keyed entry, maintain a
   `public_key -> peer_id` index alongside the existing
   `identity_id -> public_key` index (`:227-230`).
2. In `get()` (`:241`), after the PeerId key and the identity_id fallback both
   miss, resolve via the public-key index.
3. Keep `list()` deduped so one contact does not surface three times.

Alternative considered: normalise the caller to look up by PeerId. Rejected as
the primary fix -- the send path legitimately holds only a public key at that
point, and the other two flavors are already indexed, so the store is the
right place to close the gap.

## Acceptance

- Unit test: store a contact keyed by PeerId, then `get(public_key_hex)`
  returns it.
- Unit test: `list()` still returns exactly one entry for that contact.
- Live: repeat the send that produced the warning above and confirm the
  `no contact record` WARN no longer fires.

## Verification not yet done

The observed behaviour is confirmed; the FIX is not written and not verified.
No code was changed by the session that filed this.
