// Mesh Routing: Relay, Reputation, and Retry Logic (Phases 3-6)
//
// Implements the sovereign mesh routing system where:
// - Every node can relay messages for others (Phase 3)
// - Nodes track relay performance and reputation (Phase 5)
// - Message delivery uses multi-path retry with continuous adaptation (Phase 6)
// - Any node can bootstrap from any other node (Phase 4)

use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use web_time::{Duration, SystemTime, UNIX_EPOCH};

/// Route reason: direct-first policy candidate.
pub const ROUTE_REASON_DIRECT_FIRST: &str = "DIRECT_FIRST";
/// Route reason: relay chosen by recipient-recency and success score policy.
pub const ROUTE_REASON_RELAY_RECENCY_SUCCESS: &str = "RELAY_RECENCY_SUCCESS";
/// Route reason: relay chosen by success score when no recipient-recency signal exists.
pub const ROUTE_REASON_RELAY_SUCCESS_SCORE: &str = "RELAY_SUCCESS_SCORE";
/// Route reason: relay ordering required latest-success tie-break.
pub const ROUTE_REASON_RELAY_TIEBREAK_LAST_SUCCESS: &str = "RELAY_TIEBREAK_LAST_SUCCESS";
/// Route reason: relay ordering fell back to deterministic peer-id tie-break.
pub const ROUTE_REASON_RELAY_TIEBREAK_PEER_ID: &str = "RELAY_TIEBREAK_PEER_ID";

/// Ranked route candidate with deterministic metadata for trace logging.
#[derive(Debug, Clone)]
pub struct RankedRoute {
    pub path: Vec<PeerId>,
    pub reason_code: &'static str,
    pub recipient_recency: u64,
    pub relay_success_score: f64,
    pub latest_success_order: u64,
}

/// Output of advancing to the next route candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteCursorAdvance {
    pub next_index: usize,
    pub wrapped_pass: bool,
}

/// Advance to the next route in a pass; wraps to index 0 when exhausted.
pub fn advance_route_cursor(current_index: usize, candidate_count: usize) -> RouteCursorAdvance {
    if candidate_count == 0 {
        return RouteCursorAdvance {
            next_index: 0,
            wrapped_pass: false,
        };
    }

    let next_index = current_index.saturating_add(1);
    if next_index >= candidate_count {
        RouteCursorAdvance {
            next_index: 0,
            wrapped_pass: true,
        }
    } else {
        RouteCursorAdvance {
            next_index,
            wrapped_pass: false,
        }
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Tolerated clock skew when accepting a `last_seen`/`seen_at` recency signal
/// (review F12). Anything beyond `now + this` is clamped down to it.
pub const RECENCY_MAX_CLOCK_SKEW_SECS: u64 = 5 * 60;

/// Oldest recency signal a wire peer may assert, matching the 7-day horizon the
/// CLI ledger already prunes against (`cli/src/ledger.rs`).
pub const RECENCY_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// Upper bound on tracked `(relay, recipient)` recency pairs. The key is built
/// entirely from remote-supplied peer ids, so it needs a ceiling.
pub const RECENCY_MAX_TRACKED_ROUTES: usize = 4096;

// ============================================================================
// PHASE 3: RELAY CAPABILITY
// ============================================================================

/// Relay statistics for a peer
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelayStats {
    /// Total messages relayed through this peer
    pub messages_relayed: u64,
    /// Total bytes relayed
    pub bytes_relayed: u64,
    /// Messages successfully delivered
    pub successful_deliveries: u64,
    /// Messages that failed or timed out
    pub failed_deliveries: u64,
    /// Average latency in milliseconds
    pub avg_latency_ms: u64,
    /// When this peer was last used as a relay
    pub last_used: u64,
}

// ============================================================================
// PHASE 5: REPUTATION TRACKING
// ============================================================================

/// Reputation score for a relay peer
#[derive(Debug, Clone)]
pub struct RelayReputation {
    /// Peer ID
    pub peer_id: PeerId,
    /// Statistics
    pub stats: RelayStats,
    /// Calculated reputation score (0-100)
    pub score: f64,
    /// Is this peer currently considered reliable?
    pub is_reliable: bool,
}

impl RelayReputation {
    /// Calculate reputation score based on statistics
    pub fn calculate_score(&mut self) {
        if self.stats.messages_relayed == 0 {
            self.score = 50.0; // Neutral score for new peers
            self.is_reliable = true;
            return;
        }

        let success_rate =
            self.stats.successful_deliveries as f64 / self.stats.messages_relayed as f64;

        // Score factors:
        // - Success rate (70% weight)
        // - Latency (20% weight - lower is better)
        // - Recency (10% weight - recent usage preferred)

        let success_score = success_rate * 70.0;

        let latency_score = if self.stats.avg_latency_ms < 100 {
            20.0
        } else if self.stats.avg_latency_ms < 500 {
            15.0
        } else if self.stats.avg_latency_ms < 1000 {
            10.0
        } else {
            5.0
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs();
        let age_secs = now.saturating_sub(self.stats.last_used);
        let recency_score = if age_secs < 60 {
            10.0
        } else if age_secs < 300 {
            7.0
        } else if age_secs < 3600 {
            5.0
        } else {
            2.0
        };

        self.score = success_score + latency_score + recency_score;
        self.is_reliable = self.score >= 50.0;
    }
}

/// Tracks reputation of all known relay peers
#[derive(Debug, Clone)]
pub struct ReputationTracker {
    reputations: HashMap<PeerId, RelayReputation>,
}

impl Default for ReputationTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ReputationTracker {
    pub fn new() -> Self {
        Self {
            reputations: HashMap::new(),
        }
    }

    /// Record a relay attempt
    pub fn record_relay_attempt(
        &mut self,
        peer_id: PeerId,
        success: bool,
        latency_ms: u64,
        bytes: u64,
    ) {
        let rep = self.reputations.entry(peer_id).or_insert(RelayReputation {
            peer_id,
            stats: RelayStats::default(),
            score: 50.0,
            is_reliable: true,
        });

        rep.stats.messages_relayed += 1;
        rep.stats.bytes_relayed += bytes;

        if success {
            rep.stats.successful_deliveries += 1;
        } else {
            rep.stats.failed_deliveries += 1;
        }

        // Update average latency (moving average)
        rep.stats.avg_latency_ms = (rep.stats.avg_latency_ms + latency_ms) / 2;

        rep.stats.last_used = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs();

        rep.calculate_score();
    }

    /// Get best relay peers (sorted by reputation)
    pub fn best_relays(&self, count: usize) -> Vec<PeerId> {
        let mut peers: Vec<_> = self.reputations.values().collect();
        peers.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.peer_id.to_string().cmp(&b.peer_id.to_string()))
        });

        peers
            .into_iter()
            .filter(|r| r.is_reliable)
            .take(count)
            .map(|r| r.peer_id)
            .collect()
    }

    /// Add a peer as a potential relay (neutral reputation)
    pub fn add_relay(&mut self, peer_id: PeerId) {
        self.reputations
            .entry(peer_id)
            .or_insert_with(|| RelayReputation {
                peer_id,
                stats: RelayStats::default(),
                score: 50.0,
                is_reliable: true,
            });
    }

    /// Check if we have any known relays
    pub fn is_empty(&self) -> bool {
        self.reputations.is_empty()
    }

    /// Get all reputations
    pub fn all_reputations(&self) -> Vec<RelayReputation> {
        self.reputations.values().cloned().collect()
    }
}

// ============================================================================
// PHASE 6: CONTINUOUS RETRY LOGIC
// ============================================================================

/// Retry strategy for message delivery
#[derive(Debug, Clone)]
pub struct RetryStrategy {
    /// Optional maximum number of retry attempts.
    /// `None` means unbounded retries (default).
    pub max_attempts: Option<u32>,
    /// Initial retry delay
    pub initial_delay: Duration,
    /// Maximum retry delay
    pub max_delay: Duration,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
    /// Whether to use exponential backoff
    pub use_exponential_backoff: bool,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self {
            max_attempts: None, // WS1: no terminal retry cap
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 1.5,
            use_exponential_backoff: true,
        }
    }
}

impl RetryStrategy {
    /// Calculate delay for a given attempt number
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        if !self.use_exponential_backoff {
            return self.initial_delay;
        }

        let delay_ms =
            self.initial_delay.as_millis() as f64 * self.backoff_multiplier.powi(attempt as i32);

        let delay = Duration::from_millis(delay_ms as u64);

        delay.min(self.max_delay)
    }

    /// Should we retry after this many attempts?
    pub fn should_retry(&self, attempt: u32) -> bool {
        self.max_attempts.map(|max| attempt < max).unwrap_or(true)
    }
}

/// Tracks ongoing delivery attempts
#[derive(Debug, Clone)]
pub struct DeliveryAttempt {
    /// Message ID
    pub message_id: String,
    /// Target peer
    pub target_peer: PeerId,
    /// Attempt number (0-indexed)
    pub attempt: u32,
    /// Paths tried so far (direct or via relays)
    pub paths_tried: Vec<Vec<PeerId>>,
    /// Last attempt timestamp
    pub last_attempt: u64,
    /// Retry strategy
    pub strategy: RetryStrategy,
}

impl DeliveryAttempt {
    pub fn new(message_id: String, target_peer: PeerId) -> Self {
        Self {
            message_id,
            target_peer,
            attempt: 0,
            paths_tried: Vec::new(),
            last_attempt: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_secs(),
            strategy: RetryStrategy::default(),
        }
    }

    /// Get next retry delay
    pub fn next_retry_delay(&self) -> Duration {
        self.strategy.calculate_delay(self.attempt)
    }

    /// Should we retry?
    pub fn should_retry(&self) -> bool {
        self.strategy.should_retry(self.attempt)
    }

    /// Record a failed attempt via a specific path
    pub fn record_failure(&mut self, path: Vec<PeerId>) {
        self.paths_tried.push(path);
        self.attempt += 1;
        self.last_attempt = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs();
    }
}

/// Multi-path delivery manager
#[derive(Debug)]
pub struct MultiPathDelivery {
    /// Active delivery attempts
    attempts: HashMap<String, DeliveryAttempt>,
    /// Reputation tracker for selecting best paths
    reputation: ReputationTracker,
    /// Recipient-recency signals keyed by (relay, recipient)
    recipient_recency_by_route: HashMap<(PeerId, PeerId), u64>,
    /// Latest successful relay path order keyed by (relay, recipient)
    latest_success_by_route: HashMap<(PeerId, PeerId), u64>,
    /// Monotonic sequence for deterministic "latest successful path" tie-breaks
    success_sequence: u64,
}

impl Default for MultiPathDelivery {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiPathDelivery {
    pub fn new() -> Self {
        Self {
            attempts: HashMap::new(),
            reputation: ReputationTracker::new(),
            recipient_recency_by_route: HashMap::new(),
            latest_success_by_route: HashMap::new(),
            success_sequence: 0,
        }
    }

    /// Start a delivery attempt
    pub fn start_delivery(&mut self, message_id: String, target_peer: PeerId) {
        let attempt = DeliveryAttempt::new(message_id.clone(), target_peer);
        self.attempts.insert(message_id, attempt);
    }

    /// Register a potential relay node
    pub fn add_relay(&mut self, peer_id: PeerId) {
        self.reputation.add_relay(peer_id);
    }

    /// Record a recipient-recency signal for a relay candidate.
    ///
    /// `seen_at` must be a unix timestamp (seconds). Newer timestamps overwrite
    /// older values.
    ///
    /// FUTURE-CLAMP (review F12): `recipient_recency` is the PRIMARY descending
    /// sort key in [`Self::ranked_routes`], and the merge here is a monotone
    /// `max`, so a single `u64::MAX` would pin a route at the front of the
    /// ranking permanently -- it can never be lowered by the passage of time,
    /// by honest observation, or by a peer restart. Values beyond
    /// `now + RECENCY_MAX_CLOCK_SKEW_SECS` are clamped rather than rejected so
    /// an honest peer with a slightly fast clock still registers.
    pub fn record_recipient_seen_via_relay(
        &mut self,
        relay_peer: PeerId,
        recipient_peer: PeerId,
        seen_at: u64,
    ) {
        let ceiling = unix_now_secs().saturating_add(RECENCY_MAX_CLOCK_SKEW_SECS);
        let clamped = seen_at.min(ceiling);
        let key = (relay_peer, recipient_peer);
        let entry = self.recipient_recency_by_route.entry(key).or_insert(0);
        *entry = (*entry).max(clamped);
        self.prune_recipient_recency();
    }

    /// [`Self::record_recipient_seen_via_relay`] for a timestamp that arrived
    /// over the wire, i.e. one chosen by whoever sent it.
    ///
    /// Returns `true` if the signal was recorded. In addition to the
    /// future-clamp, this drops values older than [`RECENCY_MAX_AGE_SECS`] --
    /// the same 7-day horizon the CLI ledger uses -- and the `0` that an entry
    /// with no `last_seen` serialises to. A week-old sighting is not evidence
    /// about a route's current usefulness, and accepting it only lets a peer
    /// inject ranking weight for routes it has no live knowledge of.
    pub fn record_recipient_seen_via_relay_from_wire(
        &mut self,
        relay_peer: PeerId,
        recipient_peer: PeerId,
        seen_at: u64,
    ) -> bool {
        let now = unix_now_secs();
        if seen_at == 0 || seen_at.saturating_add(RECENCY_MAX_AGE_SECS) < now {
            return false;
        }
        self.record_recipient_seen_via_relay(relay_peer, recipient_peer, seen_at);
        true
    }

    /// Bound `recipient_recency_by_route`, which is keyed by a pair of
    /// remote-supplied peer ids and previously grew without limit. Evicts the
    /// oldest sightings first, which is also the least useful ranking data.
    fn prune_recipient_recency(&mut self) {
        if self.recipient_recency_by_route.len() <= RECENCY_MAX_TRACKED_ROUTES {
            return;
        }
        let mut by_age: Vec<((PeerId, PeerId), u64)> = self
            .recipient_recency_by_route
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        by_age.sort_by_key(|(_, seen_at)| *seen_at);
        let excess = by_age.len() - RECENCY_MAX_TRACKED_ROUTES;
        for (key, _) in by_age.into_iter().take(excess) {
            self.recipient_recency_by_route.remove(&key);
        }
    }

    /// Record a "seen now" recipient-recency signal.
    pub fn record_recipient_seen_now(&mut self, relay_peer: PeerId, recipient_peer: PeerId) {
        self.record_recipient_seen_via_relay(relay_peer, recipient_peer, unix_now_secs());
    }

    /// Deterministic ranked routes: direct-first, then relay ranking policy.
    pub fn ranked_routes(&self, target: &PeerId, count: usize) -> Vec<RankedRoute> {
        if count == 0 {
            return Vec::new();
        }

        let mut routes = Vec::with_capacity(count);
        routes.push(RankedRoute {
            path: vec![*target],
            reason_code: ROUTE_REASON_DIRECT_FIRST,
            recipient_recency: 0,
            relay_success_score: 0.0,
            latest_success_order: 0,
        });

        #[derive(Debug)]
        struct RelayCandidate {
            relay_peer: PeerId,
            relay_key: String,
            recipient_recency: u64,
            relay_success_score: f64,
            latest_success_order: u64,
        }

        let mut relays: Vec<RelayCandidate> = self
            .reputation
            .reputations
            .values()
            .filter(|rep| rep.is_reliable && rep.peer_id != *target)
            .map(|rep| {
                let relay_peer = rep.peer_id;
                RelayCandidate {
                    relay_peer,
                    relay_key: relay_peer.to_string(),
                    recipient_recency: self
                        .recipient_recency_by_route
                        .get(&(relay_peer, *target))
                        .copied()
                        .unwrap_or(0),
                    relay_success_score: rep.score,
                    latest_success_order: self
                        .latest_success_by_route
                        .get(&(relay_peer, *target))
                        .copied()
                        .unwrap_or(0),
                }
            })
            .collect();

        relays.sort_by(|a, b| {
            b.recipient_recency
                .cmp(&a.recipient_recency)
                .then_with(|| b.relay_success_score.total_cmp(&a.relay_success_score))
                .then_with(|| b.latest_success_order.cmp(&a.latest_success_order))
                .then_with(|| a.relay_key.cmp(&b.relay_key))
        });

        for relay in relays.into_iter().take(count.saturating_sub(1)) {
            let reason_code = if relay.recipient_recency > 0 {
                ROUTE_REASON_RELAY_RECENCY_SUCCESS
            } else if relay.latest_success_order > 0 {
                ROUTE_REASON_RELAY_TIEBREAK_LAST_SUCCESS
            } else if relay.relay_success_score > 0.0 {
                ROUTE_REASON_RELAY_SUCCESS_SCORE
            } else {
                ROUTE_REASON_RELAY_TIEBREAK_PEER_ID
            };

            routes.push(RankedRoute {
                path: vec![relay.relay_peer, *target],
                reason_code,
                recipient_recency: relay.recipient_recency,
                relay_success_score: relay.relay_success_score,
                latest_success_order: relay.latest_success_order,
            });
        }

        routes
    }

    /// Get best paths to try (direct + relay options)
    pub fn get_best_paths(&self, target: &PeerId, count: usize) -> Vec<Vec<PeerId>> {
        self.ranked_routes(target, count)
            .into_iter()
            .map(|route| route.path)
            .collect()
    }

    /// Record delivery success
    pub fn record_success(&mut self, message_id: &str, path: Vec<PeerId>, latency_ms: u64) {
        // Remove from active attempts
        self.attempts.remove(message_id);

        // Update reputation for relays in the path
        if path.len() > 1 {
            self.success_sequence = self.success_sequence.saturating_add(1);
            let latest_success_order = self.success_sequence;
            let target_peer = *path.last().unwrap_or(&path[0]);
            for relay in &path[..path.len() - 1] {
                self.reputation
                    .record_relay_attempt(*relay, true, latency_ms, 1024);
                self.latest_success_by_route
                    .insert((*relay, target_peer), latest_success_order);
            }
        }
    }

    /// Record delivery failure
    pub fn record_failure(&mut self, message_id: &str, path: Vec<PeerId>) {
        if let Some(attempt) = self.attempts.get_mut(message_id) {
            attempt.record_failure(path.clone());

            // Update reputation for relays that failed
            if path.len() > 1 {
                for relay in &path[..path.len() - 1] {
                    self.reputation
                        .record_relay_attempt(*relay, false, 10000, 0);
                }
            }
        }
    }

    /// Converge delivery state for a message once a final delivery marker is observed.
    ///
    /// Returns `true` when an active retry attempt was cleared.
    pub fn converge_delivery(&mut self, message_id: &str) -> bool {
        self.attempts.remove(message_id).is_some()
    }

    /// Get a specific pending delivery attempt by message id.
    pub fn delivery_attempt(&self, message_id: &str) -> Option<&DeliveryAttempt> {
        self.attempts.get(message_id)
    }

    /// Get pending delivery attempts
    pub fn pending_attempts(&self) -> Vec<&DeliveryAttempt> {
        self.attempts.values().collect()
    }

    /// Get reputation tracker
    pub fn reputation(&self) -> &ReputationTracker {
        &self.reputation
    }

    /// Get best relay peers (sorted by reputation)
    pub fn best_relays(&self, count: usize) -> Vec<PeerId> {
        self.reputation.best_relays(count)
    }
}

// ============================================================================
// PHASE 4: MESH-BASED DISCOVERY
// ============================================================================

/// Bootstrap capability - any node can help others join the network
#[derive(Debug, Clone)]
pub struct BootstrapCapability {
    /// Peers we know about (potential bootstrap candidates)
    pub known_peers: Vec<PeerId>,
    /// Last time we updated our peer list
    pub last_update: u64,
}

impl Default for BootstrapCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl BootstrapCapability {
    pub fn new() -> Self {
        Self {
            known_peers: Vec::new(),
            last_update: 0,
        }
    }

    /// Add a peer as a potential bootstrap node
    pub fn add_peer(&mut self, peer_id: PeerId) {
        if !self.known_peers.contains(&peer_id) {
            self.known_peers.push(peer_id);
            self.last_update = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_secs();
        }
    }

    /// Get bootstrap candidates (all stable peers)
    pub fn get_bootstrap_candidates(&self) -> &[PeerId] {
        &self.known_peers
    }

    /// Can this node help others bootstrap?
    pub fn can_bootstrap_others(&self) -> bool {
        !self.known_peers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reputation_calculation() {
        let mut rep = RelayReputation {
            peer_id: PeerId::random(),
            stats: RelayStats {
                messages_relayed: 100,
                successful_deliveries: 95,
                failed_deliveries: 5,
                avg_latency_ms: 50,
                ..Default::default()
            },
            score: 0.0,
            is_reliable: false,
        };

        rep.calculate_score();

        assert!(
            rep.score > 80.0,
            "High success rate should yield high score"
        );
        assert!(rep.is_reliable, "Should be marked as reliable");
    }

    #[test]
    fn test_retry_strategy() {
        let strategy = RetryStrategy::default();

        assert_eq!(strategy.calculate_delay(0), Duration::from_millis(100));
        assert!(strategy.calculate_delay(1) > Duration::from_millis(100));
        assert!(strategy.calculate_delay(5) < strategy.max_delay);

        assert!(strategy.should_retry(5));
        assert!(strategy.should_retry(100));

        let bounded = RetryStrategy {
            max_attempts: Some(3),
            ..RetryStrategy::default()
        };
        assert!(bounded.should_retry(2));
        assert!(!bounded.should_retry(3));
    }

    #[test]
    fn test_multi_path_delivery() {
        let mut delivery = MultiPathDelivery::new();
        let target = PeerId::random();
        let message_id = "test-message-123".to_string();

        delivery.start_delivery(message_id.clone(), target);

        let paths = delivery.get_best_paths(&target, 3);
        assert!(!paths.is_empty(), "Should provide at least direct path");
        assert_eq!(paths[0], vec![target], "First path should be direct");

        delivery.record_failure(&message_id, vec![target]);

        let pending = delivery.pending_attempts();
        assert_eq!(pending.len(), 1, "Should have one pending attempt");
    }

    #[test]
    fn test_converge_delivery_clears_pending_retry_attempt() {
        let mut delivery = MultiPathDelivery::new();
        let target = PeerId::random();
        let message_id = "converge-message-123".to_string();

        delivery.start_delivery(message_id.clone(), target);
        assert_eq!(delivery.pending_attempts().len(), 1);

        let cleared = delivery.converge_delivery(&message_id);
        assert!(cleared);
        assert!(delivery.delivery_attempt(&message_id).is_none());
        assert_eq!(delivery.pending_attempts().len(), 0);
    }

    // ------------------------------------------------------------------
    // F12 -- wire `last_seen` is attacker-controlled ranking input
    // ------------------------------------------------------------------

    /// Without the clamp, `u64::MAX` merged with a monotone `max` pins the
    /// attacker's route at the head of `ranked_routes` forever: it can never be
    /// lowered by time, honest observation or a peer restart.
    #[test]
    fn future_recency_cannot_pin_a_route_forever() {
        let mut delivery = MultiPathDelivery::new();
        let target = PeerId::random();
        let attacker_relay = PeerId::random();
        let honest_relay = PeerId::random();

        delivery.record_recipient_seen_via_relay_from_wire(attacker_relay, target, u64::MAX);
        delivery.record_recipient_seen_now(honest_relay, target);

        let attacker_recency = delivery
            .recipient_recency_by_route
            .get(&(attacker_relay, target))
            .copied()
            .unwrap_or(0);
        let honest_recency = delivery
            .recipient_recency_by_route
            .get(&(honest_relay, target))
            .copied()
            .unwrap_or(0);

        assert_ne!(
            attacker_recency,
            u64::MAX,
            "u64::MAX was stored verbatim as a recency score"
        );
        assert!(
            attacker_recency <= unix_now_secs() + RECENCY_MAX_CLOCK_SKEW_SECS,
            "recency {} exceeds now + max skew",
            attacker_recency
        );
        assert!(
            honest_recency + RECENCY_MAX_CLOCK_SKEW_SECS >= attacker_recency,
            "an honest live sighting is still hopelessly behind the clamped attacker value"
        );
    }

    /// A modest clock skew from an honest peer is tolerated, not discarded.
    #[test]
    fn small_clock_skew_is_clamped_not_dropped() {
        let mut delivery = MultiPathDelivery::new();
        let target = PeerId::random();
        let relay = PeerId::random();
        let slightly_ahead = unix_now_secs() + 30;

        assert!(delivery.record_recipient_seen_via_relay_from_wire(relay, target, slightly_ahead));
        let stored = delivery
            .recipient_recency_by_route
            .get(&(relay, target))
            .copied()
            .unwrap_or(0);
        assert_eq!(stored, slightly_ahead);
    }

    /// Anything older than the 7-day horizon (or the `0` an entry with no
    /// `last_seen` serialises to) carries no information about a route's
    /// current usefulness and must not enter the ranking at all.
    #[test]
    fn stale_wire_recency_is_rejected() {
        let mut delivery = MultiPathDelivery::new();
        let target = PeerId::random();
        let relay = PeerId::random();
        let ancient = unix_now_secs() - RECENCY_MAX_AGE_SECS - 60;

        assert!(!delivery.record_recipient_seen_via_relay_from_wire(relay, target, ancient));
        assert!(!delivery.record_recipient_seen_via_relay_from_wire(relay, target, 0));
        assert!(delivery.recipient_recency_by_route.is_empty());
    }

    /// The map is keyed entirely by remote-supplied peer ids, so it needs a
    /// ceiling.
    #[test]
    fn recipient_recency_map_is_bounded() {
        let mut delivery = MultiPathDelivery::new();
        let now = unix_now_secs();
        for i in 0..(RECENCY_MAX_TRACKED_ROUTES + 500) {
            delivery.record_recipient_seen_via_relay(
                PeerId::random(),
                PeerId::random(),
                now - (i as u64 % 1000),
            );
        }
        assert!(delivery.recipient_recency_by_route.len() <= RECENCY_MAX_TRACKED_ROUTES);
    }
}
