# P1 -- async delivery receipts never converge; sender stays `pending` forever

Status: Open -- queued BEHIND the five-node anchor rollout
Filed: 2026-08-10 ~01:40Z (Windows lane)
Ties into: the existing async receipt-convergence effort (`sc-receipt-convergence`)

## This is NOT "async is slow". Async would converge. This does not.

The design is asynchronous by intent -- `/api/send` accepts a message into the
outbox and the retry machinery delivers later. That is correct and is not the
defect. The defect is that the sender's view **never catches up with reality**,
even after the receiver has the message and an ACK has been exchanged.

## Evidence, both directions

**Windows -> macOS.** Message `21831e84-cd6c-463f-86c9-85ea677aaa88` sent
01:08:41Z. The macOS lane confirmed it in Mac history with `delivered=true`,
content matching exactly. More than ten minutes later the Windows sender still
reports:

```
{"message_id":"21831e84-...","status":"pending","delivered":false}
```

Same for the follow-up `ef9f0318-3ef5-4d4a-91d4-522628ac2728`.

**macOS -> Windows, symmetric.** Their probes `778f9437-...` and `2d7867de-...`
were received by Windows and ACKed:

```
01:08:10.288Z  inbox_receive  message_id=778f9437-...  -> Sending delivery ACK to 12D3KooWP1hv...
01:08:44.557Z  inbox_receive  message_id=2d7867de-...  -> Sending delivery ACK to 12D3KooWP1hv...
```

Their sender-side status stayed "accepted but still pending" for both.

So **both lanes independently show the same failure**: delivery succeeds, the
receipt does not make it back into the sender's status.

## Why it matters beyond cosmetics

1. **It makes the five-node run unscoreable from the sender side.** Scoring has
   to fall back to receiver-side `inbox_receive` plus the ACK, which means every
   delivery claim requires access to the *receiving* node. That is workable for
   two lanes and impractical for five.
2. **It is indistinguishable from real failure.** An operator watching
   `/api/send/:id` sees a message that never delivers. There is no way to tell a
   stuck message from a delivered one.
3. **It probably drives redundant retries.** The outbox logged
   `outbox_retry_attempt (attempt #1/12)` for a message the peer already had.
   Retrying delivered messages wastes dial budget and feeds the concurrent-
   connection storms behind the P0.

## The existing effort this attaches to

The node already subscribes to a delivery-convergence gossip topic:

```
16:05:45  Subscribed to delivery convergence topic: sc-receipt-convergence
16:05:49  Peer 12D3KooWP1hv... subscribed to topic: sc-receipt-convergence
```

So the async receipt-convergence mechanism EXISTS and both peers are subscribed
to it. This ticket is not a request to design one -- it is that the existing one
is not closing the loop. Start there rather than building a parallel path.

Prior related work worth reading first:
- `HANDOFF/done/CRITICAL_ANDROID_FALSE_DELIVERY_FAILURE_NO_RECEIPT_ACK.md`
- `HANDOFF/done/P1_CORE_004_Mobile_Receipt_Wiring.md`

## Strong lead: the identifiers do not match

One exchange used three different identifier forms:

| Where | Value |
|---|---|
| addressed by the caller | `12D3KooWP1hvZbqCCPMMfrZbW16EHy7wXp41pDPWtHzdn3MbwG5e` |
| outbox retry target | `c40fa8137108c523541739f1384a63df93f1f038c7208f3db7d14449a3d71239` |
| inbound `sender_id` | `7dad8fdf5dfce395a15ef88ac88870554fa580a38a57fb5cdf49ff109851ce17` |

Neither 64-hex value is the peer's PeerId or its published public key
(`a185af9484e8f42ef5eeea4f431371ec89895ef24adb0991a17625663b941d0c`), and
neither is a plain SHA-256 of the PeerId string, the public-key hex string, or
the public-key bytes -- all three were checked and ruled out.

**Hypothesis:** an ACK arrives keyed by one identifier form while the outbox
entry is keyed by another, so the receipt never matches an outstanding message
and the status is never updated. That would explain the symmetry across lanes
exactly.

This is also the identifier-unification concern raised by the operator: the
forms may all be legitimate and necessary, but they must map to each other
losslessly and be resolvable in both directions.

## Acceptance criteria

1. Enumerate every identifier form in the messaging path with its derivation,
   cited by `file:line`: PeerId, public key, `sender_id`, outbox `peer_id`, and
   any contact/device id. Produce the mapping table. Unification is optional;
   **a documented, tested, bidirectional mapping is not**.
2. Identify where the ACK is matched to an outbox entry and prove which
   identifier each side uses.
3. After a confirmed delivery, `/api/send/:id` transitions to delivered within a
   bounded, documented time.
4. The outbox stops retrying a message once its receipt is recorded. Assert the
   retry count stops climbing.
5. Regression test: enqueue, deliver, ACK, assert sender status converges. It
   must fail without the fix.
6. Verify across a real two-node pair, not only in-process -- this failed
   identically on two different platforms, so an in-process test may pass while
   the real path stays broken.

## Sequencing

**Queued behind the five-node anchor rollout** per operator direction. Do not
start this while nodes are being re-anchored to `68fcc3f1`; a moving target
during a receipt investigation would waste the run.
