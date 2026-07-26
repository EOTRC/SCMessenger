// Regression tests for the ledger-seeding RE-REVIEW of 2026-07-25, routing half
// (HANDOFF/review/LEDGER_SEEDING_ADVERSARIAL_REVIEW_2026-07-25.md):
//
//   NEW-3 -- the F12 pruner reintroduced the F4 event-loop DoS: it collected and
//            SORTED the whole recency map on every insert past the ceiling, to
//            evict exactly one entry, on the `select!` thread that also owns the
//            swarm poll and the dial sweep.
//   NEW-4 -- eviction was steerable: the sort key was `seen_at`, which arrives
//            over the wire, so a flood at `now + RECENCY_MAX_CLOCK_SKEW_SECS`
//            evicted every honest route and handed the attacker the primary
//            ranking key in `ranked_routes`.
//
// `core/src/transport/mesh_routing.rs` holds the unit tests; this file exercises
// the same invariants through the public API only, plus a property test (routing
// changes require one per `.claude/rules/rust.md`).

use libp2p::PeerId;
use proptest::prelude::*;
use scmessenger_core::transport::mesh_routing::{
    MultiPathDelivery, RECENCY_MAX_CLOCK_SKEW_SECS, RECENCY_MAX_ROUTES_PER_RELAY,
    RECENCY_MAX_TRACKED_ROUTES, RECENCY_PRUNE_TARGET_ROUTES,
};
use web_time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// NEW-3: one ledger-exchange message used to be able to stall the swarm.
///
/// The handler now caps a message at 64 entries, but the pruner still has to be
/// amortised, because a peer can keep sending. This drives the recorder with
/// two orders of magnitude more inserts than the map can hold and requires it to
/// stay comfortably interactive.
#[test]
fn sustained_wire_flood_keeps_the_recency_map_bounded_and_cheap() {
    let mut delivery = MultiPathDelivery::new();
    let now = now_secs();
    let inserts = RECENCY_MAX_TRACKED_ROUTES * 16;

    let started = std::time::Instant::now();
    for i in 0..inserts {
        delivery.record_recipient_seen_via_relay_from_wire(
            PeerId::random(),
            PeerId::random(),
            now - (i as u64 % 3600),
        );
    }
    let elapsed = started.elapsed();

    assert!(
        delivery.tracked_recency_routes() <= RECENCY_MAX_TRACKED_ROUTES,
        "map grew past its ceiling: {}",
        delivery.tracked_recency_routes()
    );
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "{inserts} recency inserts took {elapsed:?}; this runs inline on the swarm \
         event-loop thread, which also owns the swarm poll and the dial sweep"
    );
}

/// NEW-3, hysteresis: crossing the ceiling must drop to the low-water mark, so
/// the following inserts do no pruning work at all. A pruner that trims back to
/// exactly the ceiling pays the full eviction cost on every single insert
/// forever after -- which is precisely the shape of the bug.
#[test]
fn pruning_uses_a_hysteresis_band() {
    let mut delivery = MultiPathDelivery::new();
    let now = now_secs();

    for _ in 0..=RECENCY_MAX_TRACKED_ROUTES {
        delivery.record_recipient_seen_via_relay(PeerId::random(), PeerId::random(), now);
    }

    assert_eq!(
        delivery.tracked_recency_routes(),
        RECENCY_PRUNE_TARGET_ROUTES,
        "pruner did not drop to the low-water mark; there is no amortisation"
    );
    assert!(RECENCY_PRUNE_TARGET_ROUTES < RECENCY_MAX_TRACKED_ROUTES);
}

/// NEW-4: a bounded set of attacker identities cannot flush honest routes out,
/// however many future-dated sightings they assert.
///
/// A relay peer id is not free -- it costs a Noise handshake -- so the per-relay
/// quota converts "unlimited messages" into "at most 64 slots per identity".
#[test]
fn future_dated_flood_cannot_take_over_the_ranking_key() {
    let mut delivery = MultiPathDelivery::new();
    let now = now_secs();

    // Honest routes recorded FIRST, so an insertion-ordered pruner would reach
    // them first: this is not passing because of lucky ordering.
    let honest: Vec<(PeerId, PeerId)> = (0..32)
        .map(|_| (PeerId::random(), PeerId::random()))
        .collect();
    for (relay, target) in &honest {
        delivery.record_recipient_seen_now(*relay, *target);
    }

    let attackers: Vec<PeerId> = (0..8).map(|_| PeerId::random()).collect();
    let future = now + RECENCY_MAX_CLOCK_SKEW_SECS;
    for i in 0..(RECENCY_MAX_TRACKED_ROUTES * 8) {
        delivery.record_recipient_seen_via_relay_from_wire(
            attackers[i % attackers.len()],
            PeerId::random(),
            future,
        );
    }

    for (relay, target) in &honest {
        assert!(
            delivery.recipient_recency(relay, target).is_some(),
            "an honest route was evicted by a future-dated flood; \
             `recipient_recency_by_route` is the primary descending sort key in \
             `ranked_routes`"
        );
    }
    for attacker in &attackers {
        assert!(
            delivery.tracked_recency_routes_for_relay(attacker) <= RECENCY_MAX_ROUTES_PER_RELAY,
            "one identity holds {} route slots",
            delivery.tracked_recency_routes_for_relay(attacker)
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// PROPERTY (NEW-4): which routes survive eviction is a function of the KEY
    /// SEQUENCE ALONE and is completely independent of the `seen_at` values,
    /// which are attacker-supplied.
    ///
    /// Replay the identical sequence of `(relay, recipient)` keys twice -- once
    /// with honest timestamps, once with every value pushed to the maximum the
    /// clamp permits -- and require the surviving key set to be identical. The
    /// old pruner sorted by `seen_at`, so the two runs kept different routes;
    /// that difference IS the steerability the finding describes.
    #[test]
    fn surviving_routes_do_not_depend_on_wire_timestamps(
        relay_slots in prop::collection::vec(0usize..24, 6000..7000),
        offsets in prop::collection::vec(0u64..3600, 6000..7000),
    ) {
        let now = now_secs();
        let relays: Vec<PeerId> = (0..24).map(|_| PeerId::random()).collect();
        let recipients: Vec<PeerId> = (0..relay_slots.len()).map(|_| PeerId::random()).collect();

        let mut honest = MultiPathDelivery::new();
        let mut hostile = MultiPathDelivery::new();

        for (i, slot) in relay_slots.iter().enumerate() {
            let relay = relays[*slot];
            let recipient = recipients[i];
            // Honest node: a real, slightly-in-the-past observation.
            honest.record_recipient_seen_via_relay(
                relay,
                recipient,
                now - offsets[i % offsets.len()],
            );
            // Attacker: everything as far in the future as the clamp allows,
            // which is what the old pruner sorted to the back of the queue.
            hostile.record_recipient_seen_via_relay(
                relay,
                recipient,
                now + RECENCY_MAX_CLOCK_SKEW_SECS,
            );
        }

        prop_assert_eq!(
            honest.tracked_recency_routes(),
            hostile.tracked_recency_routes(),
            "timestamp choice changed how many routes survived"
        );
        for (i, slot) in relay_slots.iter().enumerate() {
            let key = (relays[*slot], recipients[i]);
            prop_assert_eq!(
                honest.recipient_recency(&key.0, &key.1).is_some(),
                hostile.recipient_recency(&key.0, &key.1).is_some(),
                "timestamp choice changed WHICH route survived -- eviction is \
                 steerable by wire data"
            );
        }

        // Bounds hold for both runs regardless of the values supplied.
        for delivery in [&honest, &hostile] {
            prop_assert!(delivery.tracked_recency_routes() <= RECENCY_MAX_TRACKED_ROUTES);
            for relay in &relays {
                prop_assert!(
                    delivery.tracked_recency_routes_for_relay(relay)
                        <= RECENCY_MAX_ROUTES_PER_RELAY
                );
            }
        }
    }
}
