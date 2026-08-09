# P1 -- CLI can receive and decrypt from a peer it cannot reply to

Status: Active
Severity: P1 (blocks the reply leg of any five-node gate unless worked around)
Discovered: 2026-08-09, live on the Windows node during the candidate soak
Affects: `scmessenger-cli` (Windows, macOS, Linux desktop nodes)
Candidate observed on: `33c16712`

## Summary

The CLI accepts, decrypts, and durably stores a message from a peer that is not
in its contacts store -- but it cannot send anything back to that peer, because
`/api/send` resolves the recipient **only** against the contacts list. The reply
fails with `Contact not found` and a `404`.

Android does not behave this way: it auto-creates a contact on discovery
("Auto-created/updated contact for discovered peer", observed in
`docs/fieldtest/PR139_WINDOWS_LANE_CORRELATION_2026-08-09.md`). The two
platforms disagree about whether an inbound peer becomes addressable.

## Reproduction (actually observed, not theoretical)

1. Start the Windows CLI on a clean-ish contacts store (`Claude-Windows-Driver`,
   `12D3KooWD6vZQrUqpyGaCqY3tNSK8p44BS78TvxpGpwhdPJ1T9mw`).
2. From the Pixel (`Lucaso`, identity `a43772fe...`, libp2p
   `12D3KooWNnPi9wqUJ7Jypj6g4jHmW2PUTmynUs9sJY1h6SQbjLrG`), send a message.
3. The CLI **receives and decrypts it** -- confirmed in durable history via
   `POST /api/history`, `direction: "received"`, timestamp `1786272628`
   (`2026-08-09T10:50:28Z`), full sender identity block intact.
4. Attempt to reply. All three of these return `404 Contact not found`:
   - `{"recipient":"a43772fe...”}` (sender identity id)
   - `{"recipient":"12D3KooWNnPi9wqUJ7..."}` (libp2p peer id)
   - `{"recipient":"Lucaso"}` (nickname carried in the received payload)

## Root cause

`cli/src/api.rs:568-572`, in `handle_send_message`:

```rust
let list = contacts.list().unwrap_or_default();
let contact = list
    .into_iter()
    .find(|c| c.peer_id == request.recipient || c.nickname.as_ref() == Some(&request.recipient))
    .ok_or_else(|| (StatusCode::NOT_FOUND, "Contact not found".to_string()))?;
```

Recipient resolution is a linear scan of the contacts store on `peer_id` or
`nickname` only. There is no fallback to:

- the identity id of a peer already in durable history,
- the live peer table (`/api/peers` listed the peer as connected at the time),
- the sender identity block that arrived inside the message itself, which
  already carries `identity_id`, `public_key`, `device_id`, `nickname`, and
  `libp2p_peer_id`.

Every field needed to address a reply was already in hand and on disk. The send
path simply does not look anywhere except contacts.

## Why this matters for the five-node gate

The G-gates require **both directions** of an exchange with receipts. As it
stands, a desktop node that is messaged first by a mobile node cannot answer
until a contact is manually created. That would present during a run as "the
Windows node never replied" and be misread as a transport, receipt, or custody
failure, when it is a recipient-resolution bug in the local API.

**Workaround for Run 1 (do this if the fix does not land first):** pre-seed
contacts on every desktop node before the run, via
`POST /api/contacts {"peer_id": "<libp2p peer id>", "public_key": "<hex>",
"name": "<nickname>"}`. Confirmed working -- after adding the Pixel this way,
the reply sent and delivered direct in 81 ms (`10:53:51Z`,
`route=direct`, `policy_reason=DIRECT_FROM_ROUTING_ENGINE`).

## Suggested fix

Extend recipient resolution in `handle_send_message` to fall back, in order:

1. contacts by `peer_id` or `nickname` (current behaviour),
2. contacts by identity id / public key,
3. a peer present in durable history, using the stored sender identity block,
4. a currently connected peer from the live peer table.

Decide explicitly whether the CLI should auto-create a contact on first inbound
message, as Android does. Consistency between platforms is the real fix; the
resolution fallback is the minimum.

**Security note:** this is a gated path in spirit even though it is not under
`core/src/{crypto,transport,routing,privacy}/`. Auto-creating contacts from
inbound traffic means an unsolicited peer can insert itself into the local
contacts store. Android already does this, so the behaviour exists in the
product, but the CLI change should be reviewed with that in mind rather than
copied uncritically -- and it must interact correctly with the blocked-peer
gates added in this PR.

## Acceptance criteria

1. A desktop node messaged first by an unknown peer can reply without manual
   contact creation.
2. Blocked peers remain unreplyable -- the fallback must not bypass
   `is_peer_blocked`.
3. Regression test covering reply-to-unsaved-peer and reply-to-blocked-peer.
4. Platform behaviour documented: whether CLI auto-creates contacts, and if it
   diverges from Android, why.
