use crate::transport::addr_filter::{is_dialable_multiaddr, NetworkMode};
use libp2p::Multiaddr;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

fn current_timestamp() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ============================================================================
// CONNECTION LEDGER
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub multiaddr: String,
    pub peer_id: Option<String>,
    pub public_key: Option<String>,
    pub nickname: Option<String>,
    pub success_count: u32,
    pub failure_count: u32,
    pub last_seen: Option<u64>,
    pub topics: Vec<String>,
}

/// Maximum number of [`SeedLedgerEntry`] records an invite may carry, and the
/// hard cap [`LedgerManager::import_seed_entries`] enforces on any caller.
///
/// Sized against the QR byte-mode budget: see
/// `crate::relay::invite::QR_BYTE_BUDGET` and the
/// `seed_ledger_full_invite_fits_qr_budget` test in `relay/invite.rs`.
pub const MAX_SEED_LEDGER_ENTRIES: usize = 16;

/// A routing-only peer record carried inside an invite (item 1 of the v0.4.0
/// ledger seeding work).
///
/// ROUTING ONLY -- NO IDENTITY (operator directive 2026-07-25). This type has
/// exactly one field and must keep exactly one field. `peer_id`, `public_key`,
/// `nickname`, `topics`, `success_count`, `failure_count` and `last_seen` are
/// all deliberately absent: every one of them is identity or behavioural
/// metadata about a third party who never consented to being listed in someone
/// else's invite. An invite says *where to knock*, not *who lives there*.
///
/// The invitee dials the bare address, completes the Noise handshake and learns
/// the peer identity from Identify at connect time, then attaches it locally via
/// [`LedgerManager::annotate_identity`]. Dropping `peer_id` forgoes dial-time
/// identity pinning, which is an availability property; message confidentiality
/// is per-contact X25519 / XChaCha20-Poly1305 established out of band and is
/// unaffected by which node answers at a given address.
///
/// `relay/invite.rs` has a leak-regression test that asserts no peer id, public
/// key or nickname appears in the serialised invite bytes. If you add a field
/// here, that test is what will stop you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct SeedLedgerEntry {
    /// Peer-id-stripped dialable multiaddr, e.g. `/ip4/A.B.C.D/tcp/9001`.
    pub multiaddr: String,
}

/// Strip the endpoint `/p2p/<peer-id>` component(s) from a multiaddr string.
///
/// Matches the CLI ledger's key convention (`cli/src/ledger.rs::strip_peer_id`,
/// which now delegates to the same implementation) so the two ledgers dedupe on
/// identical keys.
///
/// This used to be `multiaddr.find("/p2p/")` + truncate, which collapsed
/// `/ip4/A/tcp/443/p2p/QmRelay/p2p-circuit/p2p/QmTarget` to the RELAY's bare
/// address while [`ledger_entry_to_shared`] kept `last_peer_id = QmTarget` --
/// a wire record asserting "QmTarget is directly reachable at the relay's IP"
/// that recipients feed into `kademlia.add_address()`. See review F8 and
/// [`crate::transport::addr_filter::strip_peer_id_multiaddr`].
fn strip_peer_id_component(multiaddr: &str) -> String {
    crate::transport::addr_filter::strip_peer_id(multiaddr)
}

fn is_dns_multiaddr(addr_str: &str) -> bool {
    addr_str.contains("/dns/")
        || addr_str.contains("/dns4/")
        || addr_str.contains("/dns6/")
        || addr_str.contains("/dnsaddr/")
}

fn get_multiaddr_port(addr_str: &str) -> Option<u16> {
    if let Ok(addr) = addr_str.parse::<Multiaddr>() {
        for proto in addr.iter() {
            match proto {
                libp2p::multiaddr::Protocol::Tcp(port) => return Some(port),
                libp2p::multiaddr::Protocol::Udp(port) => return Some(port),
                _ => {}
            }
        }
    }
    None
}

#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct LedgerManager {
    /// `None` means "hold the ledger in memory only, never touch the disk".
    ///
    /// Added for review F11: `IronCore::new()` (the storage-less constructor)
    /// used to point its `LedgerManager` at `std::env::temp_dir()`, writing the
    /// node's whole peer topology into a world-readable directory on desktop.
    /// An in-memory core must have an in-memory ledger.
    storage_path: Option<std::path::PathBuf>,
    entries: Arc<Mutex<Vec<LedgerEntry>>>,
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl LedgerManager {
    #[uniffi::constructor]
    pub fn new(storage_path: String) -> Self {
        Self {
            storage_path: Some(std::path::PathBuf::from(storage_path)),
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn load(&self) -> Result<(), crate::IronCoreError> {
        let Some(storage_path) = self.storage_path.as_ref() else {
            return Ok(());
        };
        let ledger_file = storage_path.join("ledger.json");
        if ledger_file.exists() {
            let data = std::fs::read_to_string(&ledger_file)
                .map_err(|_| crate::IronCoreError::StorageError)?;
            let entries: Vec<LedgerEntry> =
                serde_json::from_str(&data).map_err(|_| crate::IronCoreError::Internal)?;
            *self.entries.lock() = entries;
        }
        Ok(())
    }

    fn save_with_entries(&self, entries: &[LedgerEntry]) -> Result<(), crate::IronCoreError> {
        let Some(storage_path) = self.storage_path.as_ref() else {
            return Ok(());
        };
        std::fs::create_dir_all(storage_path).map_err(|_| crate::IronCoreError::StorageError)?;

        let ledger_file = storage_path.join("ledger.json");
        let data =
            serde_json::to_string_pretty(entries).map_err(|_| crate::IronCoreError::Internal)?;
        std::fs::write(&ledger_file, data).map_err(|_| crate::IronCoreError::StorageError)?;

        Ok(())
    }

    pub fn save(&self) -> Result<(), crate::IronCoreError> {
        let entries = self.entries.lock();
        self.save_with_entries(&entries)
    }

    pub fn record_connection(&self, multiaddr: String, peer_id: String) {
        let mut entries = self.entries.lock();
        let target_port = get_multiaddr_port(&multiaddr);
        let mut found_dns_idx = None;
        for (idx, entry) in entries.iter().enumerate() {
            if entry.peer_id.as_deref() == Some(&peer_id)
                && is_dns_multiaddr(&entry.multiaddr)
                && (target_port.is_none() || get_multiaddr_port(&entry.multiaddr) == target_port)
            {
                found_dns_idx = Some(idx);
                break;
            }
        }

        if let Some(idx) = found_dns_idx {
            let entry = &mut entries[idx];
            entry.success_count += 1;
            entry.last_seen = Some(current_timestamp());
        } else if let Some(entry) = entries.iter_mut().find(|e| e.multiaddr == multiaddr) {
            entry.success_count += 1;
            entry.peer_id = Some(peer_id);
            entry.last_seen = Some(current_timestamp());
        } else {
            entries.push(LedgerEntry {
                multiaddr,
                peer_id: Some(peer_id),
                public_key: None,
                nickname: None,
                success_count: 1,
                failure_count: 0,
                last_seen: Some(current_timestamp()),
                topics: Vec::new(),
            });
        }
        let _ = self.save_with_entries(&entries);
    }

    pub fn record_failure(&self, multiaddr: String) {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.iter_mut().find(|e| e.multiaddr == multiaddr) {
            entry.failure_count += 1;
        }
        let _ = self.save_with_entries(&entries);
    }

    pub fn annotate_identity(
        &self,
        multiaddr: String,
        peer_id: String,
        public_key: Option<String>,
        nickname: Option<String>,
    ) {
        let normalized_public_key = public_key.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        let normalized_nickname = nickname.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

        let mut entries = self.entries.lock();
        let target_port = get_multiaddr_port(&multiaddr);
        let mut found_dns_idx = None;
        for (idx, entry) in entries.iter().enumerate() {
            if entry.peer_id.as_deref() == Some(&peer_id)
                && is_dns_multiaddr(&entry.multiaddr)
                && (target_port.is_none() || get_multiaddr_port(&entry.multiaddr) == target_port)
            {
                found_dns_idx = Some(idx);
                break;
            }
        }

        let is_new = if let Some(idx) = found_dns_idx {
            let entry = &mut entries[idx];
            if normalized_public_key.is_some() {
                entry.public_key = normalized_public_key;
            }
            if normalized_nickname.is_some() {
                entry.nickname = normalized_nickname;
            }
            entry.last_seen = Some(current_timestamp());
            false
        } else if let Some(entry) = entries.iter_mut().find(|e| e.multiaddr == multiaddr) {
            entry.peer_id = Some(peer_id);
            if normalized_public_key.is_some() {
                entry.public_key = normalized_public_key;
            }
            if normalized_nickname.is_some() {
                entry.nickname = normalized_nickname;
            }
            entry.last_seen = Some(current_timestamp());
            false
        } else {
            entries.push(LedgerEntry {
                multiaddr,
                peer_id: Some(peer_id),
                public_key: normalized_public_key,
                nickname: normalized_nickname,
                success_count: 0,
                failure_count: 0,
                last_seen: Some(current_timestamp()),
                topics: Vec::new(),
            });
            true
        };
        let _ = self.save_with_entries(&entries);
        let _ = is_new;
    }

    pub fn dialable_addresses(&self) -> Vec<LedgerEntry> {
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|e| e.success_count > 0 && e.failure_count < 5)
            .cloned()
            .collect()
    }

    /// Addresses known only from an invite/QR seed: recorded, syntactically
    /// valid, but never yet successfully dialed by us.
    ///
    /// WHY A SEPARATE ACCESSOR rather than relaxing
    /// [`Self::dialable_addresses`]: that filter (`success_count > 0 &&
    /// failure_count < 5`) means "addresses we have actually reached", and the
    /// CLI depends on exactly that meaning -- its startup `DialScheduler` sweep,
    /// its relay ranking and its ledger display all read it. Folding unproven,
    /// attacker-suppliable seed addresses into it would silently change what
    /// every existing caller believes it is getting. Seeds are a strictly
    /// lower-confidence tier, so they get their own accessor and callers opt in
    /// by name: sweep the proven set first, then this one. A first successful
    /// connection promotes a seed into the proven set via
    /// [`Self::record_connection`] with no special casing.
    ///
    /// `limit` bounds the returned Vec (review F4). The seed tier is the
    /// attacker-suppliable tier and this used to clone the ENTIRE unproven set
    /// on every `ConnectToSeedPeers`, synchronously on the swarm event-loop
    /// thread. `0` means "no entries", not "unlimited".
    pub fn seed_addresses(&self, limit: u32) -> Vec<LedgerEntry> {
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|e| e.success_count == 0 && e.failure_count < 5)
            .take(limit as usize)
            .cloned()
            .collect()
    }

    /// Export our best-known peers as routing-only seed entries for an invite.
    ///
    /// Ordered by [`Self::get_preferred_relays`] ranking (proven peers, most
    /// recently seen first). The caller is responsible for prepending its own
    /// dialable address -- see `crate::relay::invite::build_seed_ledger`.
    ///
    /// Everything except the multiaddr is dropped here, including the peer id:
    /// see the type-level note on [`SeedLedgerEntry`]. This is the only export
    /// path for invites, so it is also the choke point that keeps third-party
    /// identity out of them.
    pub fn export_seed_entries(&self, limit: u32) -> Vec<SeedLedgerEntry> {
        self.get_preferred_relays(limit)
            .into_iter()
            .map(|entry| SeedLedgerEntry {
                multiaddr: strip_peer_id_component(&entry.multiaddr),
            })
            .collect()
    }

    /// Merge seed entries learned out-of-band (invite / QR) into the ledger.
    /// Returns the number of entries that were newly added.
    ///
    /// MERGE POLICY (deliberate -- seed data is attacker-suppliable):
    /// - Dedupe key is the `/p2p/`-stripped multiaddr, matching the CLI
    ///   ledger's key convention (`cli/src/ledger.rs::strip_peer_id`).
    /// - A seed carries no identity and no counters, so there is nothing to
    ///   merge into an existing entry: a known address is left completely
    ///   untouched. `success_count`, `failure_count`, `last_seen`, `peer_id`,
    ///   `public_key` and `nickname` all keep their current values. An invite
    ///   is not evidence that a peer was reachable at any particular time, and
    ///   it is certainly not evidence about who is listening there.
    /// - New entries are added with `success_count = 0` and no identity fields.
    ///   That means they are deliberately NOT returned by
    ///   [`Self::dialable_addresses`] (which requires `success_count > 0`) nor
    ///   by [`Self::get_preferred_relays`]: an unproven address handed to us by
    ///   whoever held the invite must not masquerade as an address we have
    ///   actually reached. They surface through [`Self::seed_addresses`]
    ///   instead -- see the reasoning on that method. The first successful
    ///   connection promotes the entry via [`Self::record_connection`], and
    ///   [`Self::annotate_identity`] attaches the identity learned from
    ///   Identify at that point.
    /// - Entries whose multiaddr does not parse, is empty, carries no transport
    ///   component, or is not routable
    ///   ([`crate::transport::addr_filter::is_dialable_multiaddr`]) are
    ///   dropped, and the whole batch is capped at
    ///   [`MAX_SEED_LEDGER_ENTRIES`].
    ///
    /// Uses [`NetworkMode::Local`], i.e. RFC1918 peers stay importable: an
    /// invite is the LAN/mesh cold-start path and a node has no reliable way to
    /// know its own network context from inside the store layer. Callers that
    /// do know (a cellular-only node) should use
    /// [`Self::import_seed_entries_with_mode`].
    pub fn import_seed_entries(&self, entries: Vec<SeedLedgerEntry>) -> u32 {
        self.import_seed_entries_with_mode(entries, NetworkMode::Local)
    }

    pub fn get_preferred_relays(&self, limit: u32) -> Vec<LedgerEntry> {
        let entries = self.entries.lock();
        let mut preferred: Vec<LedgerEntry> = entries
            .iter()
            .filter(|e| e.success_count > 0)
            .cloned() // Clone now so we can sort
            .collect();
        // Sort by last_seen descending
        preferred.sort_by_key(|b| std::cmp::Reverse(b.last_seen.unwrap_or(0)));
        preferred.truncate(limit as usize);
        preferred
    }

    pub fn all_known_topics(&self) -> Vec<String> {
        let entries = self.entries.lock();
        let mut topics: Vec<String> = entries.iter().flat_map(|e| e.topics.clone()).collect();
        topics.sort();
        topics.dedup();
        topics
    }

    pub fn summary(&self) -> String {
        let entries = self.entries.lock();
        format!("Ledger contains {} peer entries", entries.len())
    }
}

/// Rust-only surface. Deliberately NOT `uniffi::export`ed: these methods take
/// [`NetworkMode`] or exist purely to keep the swarm event loop bounded, and
/// neither concept belongs in the mobile binding.
impl LedgerManager {
    /// A ledger that lives entirely in memory and never touches the disk.
    ///
    /// Review F11: `IronCore::new()` -- the storage-less constructor -- used to
    /// build its `LedgerManager` over `std::env::temp_dir()`, i.e. it wrote the
    /// node's peer topology (who we talk to, at which addresses, how often)
    /// into a world-readable directory on every desktop platform. An in-memory
    /// core gets an in-memory ledger.
    pub fn ephemeral() -> Self {
        Self {
            storage_path: None,
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// [`Self::import_seed_entries`] with an explicit network mode.
    ///
    /// Every rejection reason is deliberate; see review F3 and F9:
    /// - `stripped.is_empty()`: `"".parse::<Multiaddr>()` returns
    ///   `Ok(<empty>)`, so a seed of `"/p2p/QmX"` stripped to `""` and was
    ///   stored, then gossiped onward as an empty record.
    /// - not dialable: loopback / unspecified / link-local (including
    ///   `169.254.169.254`) / multicast / broadcast / RFC1918-in-`Public`.
    ///   Without this an invite holder could load a victim's dial set with
    ///   internal host:port pairs and read open/closed off the dial timing.
    pub fn import_seed_entries_with_mode(
        &self,
        entries: Vec<SeedLedgerEntry>,
        mode: NetworkMode,
    ) -> u32 {
        let mut ledger = self.entries.lock();
        let mut added = 0u32;

        if entries.len() > MAX_SEED_LEDGER_ENTRIES {
            tracing::warn!(
                "Seed import capped: {} entries offered, {} accepted",
                entries.len(),
                MAX_SEED_LEDGER_ENTRIES
            );
        }

        for seed in entries.into_iter().take(MAX_SEED_LEDGER_ENTRIES) {
            let stripped = strip_peer_id_component(&seed.multiaddr);
            if stripped.is_empty() {
                tracing::debug!("Dropping seed multiaddr with no transport component");
                continue;
            }
            if !is_dialable_multiaddr(&stripped, mode) {
                tracing::debug!("Dropping non-routable seed multiaddr: {}", stripped);
                continue;
            }

            let already_known = ledger
                .iter()
                .any(|e| strip_peer_id_component(&e.multiaddr) == stripped);

            // A known address is left exactly as it is. A seed has no field
            // that could improve it, and none that we would trust if it did.
            if !already_known {
                ledger.push(LedgerEntry {
                    multiaddr: stripped,
                    peer_id: None,
                    public_key: None,
                    nickname: None,
                    success_count: 0,
                    failure_count: 0,
                    last_seen: None,
                    topics: Vec::new(),
                });
                added += 1;
            }
        }

        let _ = self.save_with_entries(&ledger);
        added
    }

    /// Build the peer list for a `/sc/ledger-exchange/1.0.0` RESPONSE.
    ///
    /// This is the single choke point for review F6: the response goes to any
    /// peer that completed a Noise handshake, with no app-layer opt-in, so
    /// every restriction has to live here rather than at the call site.
    ///
    /// - `limit` is applied BEFORE cloning, so a large ledger cannot make the
    ///   swarm event loop allocate a large vector per request.
    /// - Entries are filtered through the same routability gate as dial
    ///   candidates, so we never disclose our RFC1918-in-`Public` neighbours,
    ///   loopback services, or link-local addresses to an internet peer.
    /// - `known_topics` is dropped unconditionally. Gossipsub topic names are
    ///   group-membership / social-graph data about THIRD PARTIES who never
    ///   consented to appearing in our answer to a stranger, and disclosing
    ///   them directly contradicts the "where to knock, not who lives there"
    ///   principle this feature is documented on (see [`SeedLedgerEntry`]).
    /// - The requester is never echoed back to itself.
    pub fn exchange_response_entries(
        &self,
        limit: usize,
        mode: NetworkMode,
        requester_peer_id: &str,
    ) -> Vec<SharedPeerEntry> {
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|e| e.success_count > 0 && e.failure_count < 5)
            .filter(|e| e.peer_id.as_deref() != Some(requester_peer_id))
            .filter(|e| is_dialable_multiaddr(&strip_peer_id_component(&e.multiaddr), mode))
            .take(limit)
            .map(ledger_entry_to_shared_routing_only)
            .collect()
    }
}

/// A shared peer entry for ledger exchange.
/// Stripped-down version of ledger data suitable for wire transfer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SharedPeerEntry {
    /// The multiaddr (transport address only, no /p2p/ suffix)
    pub multiaddr: String,
    /// Last known PeerID at this address (if any)
    pub last_peer_id: Option<String>,
    /// Unix timestamp of last successful connection
    pub last_seen: u64,
    /// Gossipsub topics this peer was subscribed to
    pub known_topics: Vec<String>,
}

/// Convert a stored [`LedgerEntry`] into the wire form used by
/// `/sc/ledger-exchange/1.0.0`.
///
/// UNIT CONVERSION, do not "simplify" this away: [`LedgerEntry::last_seen`] is
/// stored in **milliseconds** (see `current_timestamp` at the top of this
/// file), while [`SharedPeerEntry::last_seen`] is a Unix timestamp in
/// **seconds** -- that is what the CLI ledger emits and what
/// `MultiPathDelivery::record_recipient_seen_via_relay` compares against
/// `unix_now_secs()`. Shipping milliseconds on the wire makes every shared
/// peer look ~1000x more recent than it is and corrupts relay ranking.
pub fn ledger_entry_to_shared(entry: &LedgerEntry) -> SharedPeerEntry {
    SharedPeerEntry {
        multiaddr: strip_peer_id_component(&entry.multiaddr),
        last_peer_id: entry.peer_id.clone(),
        last_seen: entry.last_seen.unwrap_or(0) / 1000,
        known_topics: entry.topics.clone(),
    }
}

/// [`ledger_entry_to_shared`] with `known_topics` forced empty.
///
/// Review F6: the ledger-exchange response is readable by any peer that
/// completes a Noise handshake. Topic names are third-party group membership,
/// not routing information, and have no business in an unauthenticated reply.
/// Kept as a separate function (rather than a parameter) so the wire shape used
/// by the disclosure path is greppable and testable on its own.
pub fn ledger_entry_to_shared_routing_only(entry: &LedgerEntry) -> SharedPeerEntry {
    SharedPeerEntry {
        known_topics: Vec::new(),
        ..ledger_entry_to_shared(entry)
    }
}

fn default_version() -> u8 {
    1
}

/// Ledger exchange request — sent automatically on new connection.
/// "Here are all the peers I know about. Tell me yours."
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LedgerExchangeRequest {
    /// Explicit version tag for bincode wire format
    #[serde(default = "default_version")]
    pub version_tag: u8,
    /// Our known peers (shared generously)
    pub peers: Vec<SharedPeerEntry>,
    /// Our own PeerID (so the remote can record us)
    pub sender_peer_id: String,
    /// Protocol version for forward compatibility
    pub version: u32,
}

/// Ledger exchange response — reciprocal sharing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LedgerExchangeResponse {
    /// Explicit version tag for bincode wire format
    #[serde(default = "default_version")]
    pub version_tag: u8,
    /// Their known peers (shared back)
    pub peers: Vec<SharedPeerEntry>,
    /// Number of new peers they learned from our request
    pub new_peers_learned: u32,
    /// Protocol version
    pub version: u32,
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> (tempfile::TempDir, LedgerManager) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_string_lossy().to_string();
        (dir, LedgerManager::new(path))
    }

    fn seed(addr: &str) -> SeedLedgerEntry {
        SeedLedgerEntry {
            multiaddr: addr.to_string(),
        }
    }

    /// A syntactically valid peer id. The old string-truncating
    /// `strip_peer_id_component` accepted any junk after `/p2p/`; the
    /// protocol-iterating replacement (review F8) requires the whole multiaddr
    /// to parse, so the fixtures have to be real.
    fn peer() -> String {
        libp2p::PeerId::random().to_string()
    }

    #[test]
    fn strip_peer_id_component_matches_cli_convention() {
        let pid = peer();
        assert_eq!(
            strip_peer_id_component(&format!("/ip4/1.2.3.4/tcp/9001/p2p/{pid}")),
            "/ip4/1.2.3.4/tcp/9001"
        );
        assert_eq!(
            strip_peer_id_component("/ip4/1.2.3.4/tcp/9001"),
            "/ip4/1.2.3.4/tcp/9001"
        );
    }

    /// F8 regression. Before the fix this returned `/ip4/1.2.3.4/tcp/443` --
    /// the RELAY's address -- and the caller kept the TARGET's peer id, so the
    /// wire record claimed the target was directly reachable at the relay's
    /// IP:port. Recipients feed that into `kademlia.add_address()`.
    #[test]
    fn strip_peer_id_component_does_not_collapse_circuit_to_relay_address() {
        let relay = peer();
        let target = peer();
        let circuit = format!("/ip4/1.2.3.4/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}");

        let stripped = strip_peer_id_component(&circuit);

        assert_ne!(
            stripped, "/ip4/1.2.3.4/tcp/443",
            "circuit address collapsed to the bare relay address"
        );
        assert_eq!(
            stripped,
            format!("/ip4/1.2.3.4/tcp/443/p2p/{relay}/p2p-circuit")
        );
        assert!(!stripped.contains(&target), "target peer id must be gone");
        assert!(
            stripped.contains(&relay),
            "relay peer id is part of the address and must survive"
        );
    }

    /// F8 regression at the wire boundary: the record we ship must never say
    /// "<target> is at <relay ip>:<relay port>".
    #[test]
    fn ledger_entry_to_shared_never_binds_target_peer_id_to_relay_address() {
        let relay = peer();
        let target = peer();
        let entry = LedgerEntry {
            multiaddr: format!("/ip4/1.2.3.4/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}"),
            peer_id: Some(target.clone()),
            public_key: None,
            nickname: None,
            success_count: 1,
            failure_count: 0,
            last_seen: Some(1_700_000_000_000),
            topics: Vec::new(),
        };

        let shared = ledger_entry_to_shared(&entry);

        assert_eq!(shared.last_peer_id.as_deref(), Some(target.as_str()));
        assert_ne!(
            shared.multiaddr, "/ip4/1.2.3.4/tcp/443",
            "shared record binds the target peer id to the relay's direct address"
        );
        assert!(
            shared.multiaddr.contains("/p2p-circuit"),
            "the circuit hop must remain visible so recipients treat it as relayed, got {}",
            shared.multiaddr
        );
    }

    #[test]
    fn import_seed_entries_adds_unproven_entries() {
        let (_dir, mgr) = manager();
        let added = mgr.import_seed_entries(vec![
            seed("/ip4/10.0.0.1/tcp/9001"),
            seed("/ip4/10.0.0.2/tcp/9001"),
        ]);
        assert_eq!(added, 2);

        // Seeds are unproven: they must NOT appear as dialable/preferred.
        assert!(mgr.dialable_addresses().is_empty());
        assert!(mgr.get_preferred_relays(10).is_empty());

        // ...but they must be reachable through the seed accessor, and they
        // must carry no identity whatsoever.
        let seeds = mgr.seed_addresses(64);
        assert_eq!(seeds.len(), 2);
        assert!(seeds.iter().all(|e| e.success_count == 0));
        assert!(
            seeds
                .iter()
                .all(|e| e.peer_id.is_none() && e.public_key.is_none() && e.nickname.is_none()),
            "seed import must not populate identity fields"
        );
    }

    #[test]
    fn import_seed_entries_dedupes_on_stripped_multiaddr() {
        let (_dir, mgr) = manager();
        assert_eq!(
            mgr.import_seed_entries(vec![seed("/ip4/10.0.0.1/tcp/9001")]),
            1
        );
        // Same address, /p2p/ suffix attached -- must dedupe, not duplicate.
        assert_eq!(
            mgr.import_seed_entries(vec![seed(&format!(
                "/ip4/10.0.0.1/tcp/9001/p2p/{}",
                peer()
            ))]),
            0
        );
        let seeds = mgr.seed_addresses(64);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].multiaddr, "/ip4/10.0.0.1/tcp/9001");
        // A peer id smuggled inside the multiaddr string must not survive.
        assert!(seeds[0].peer_id.is_none());
    }

    #[test]
    fn import_seed_entries_never_clobbers_proven_entry() {
        let (_dir, mgr) = manager();
        mgr.record_connection("/ip4/10.0.0.1/tcp/9001".to_string(), "realpeer".to_string());
        mgr.record_connection("/ip4/10.0.0.1/tcp/9001".to_string(), "realpeer".to_string());
        mgr.record_failure("/ip4/10.0.0.1/tcp/9001".to_string());
        let before = mgr.dialable_addresses();
        assert_eq!(before.len(), 1);
        let (succ, fail, last_seen) = (
            before[0].success_count,
            before[0].failure_count,
            before[0].last_seen,
        );

        // An invite lists an address we already have a history with.
        let added = mgr.import_seed_entries(vec![seed("/ip4/10.0.0.1/tcp/9001")]);
        assert_eq!(added, 0);

        let after = mgr.dialable_addresses();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].success_count, succ, "success_count was clobbered");
        assert_eq!(after[0].failure_count, fail, "failure_count was clobbered");
        assert_eq!(after[0].last_seen, last_seen, "last_seen was clobbered");
        assert_eq!(
            after[0].peer_id.as_deref(),
            Some("realpeer"),
            "known peer_id was disturbed by seed data"
        );
    }

    #[test]
    fn import_seed_entries_rejects_garbage_and_caps_batch() {
        let (_dir, mgr) = manager();
        assert_eq!(mgr.import_seed_entries(vec![seed("not-a-multiaddr")]), 0);

        let batch: Vec<SeedLedgerEntry> = (0..MAX_SEED_LEDGER_ENTRIES + 8)
            .map(|i| seed(&format!("/ip4/10.0.1.{}/tcp/9001", i)))
            .collect();
        assert_eq!(
            mgr.import_seed_entries(batch),
            MAX_SEED_LEDGER_ENTRIES as u32
        );
    }

    #[test]
    fn import_seed_entries_survives_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_string_lossy().to_string();
        let mgr = LedgerManager::new(path.clone());
        assert_eq!(
            mgr.import_seed_entries(vec![seed("/ip4/10.0.0.7/tcp/9001")]),
            1
        );

        let reloaded = LedgerManager::new(path);
        reloaded.load().expect("load");
        let seeds = reloaded.seed_addresses(64);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].multiaddr, "/ip4/10.0.0.7/tcp/9001");
    }

    #[test]
    fn export_seed_entries_only_exports_proven_peers_without_identity() {
        let (_dir, mgr) = manager();
        let proven = peer();
        let proven_addr = format!("/ip4/10.0.0.1/tcp/9001/p2p/{proven}");
        mgr.import_seed_entries(vec![seed("/ip4/10.0.0.9/tcp/9001")]);
        mgr.record_connection(proven_addr.clone(), proven.clone());
        mgr.annotate_identity(
            proven_addr,
            proven,
            Some("deadbeef".to_string()),
            Some("alice-laptop".to_string()),
        );

        let exported = mgr.export_seed_entries(16);
        assert_eq!(exported.len(), 1);
        // Peer-id-stripped, and the struct has no room for identity at all.
        assert_eq!(exported[0].multiaddr, "/ip4/10.0.0.1/tcp/9001");
    }

    #[test]
    fn ledger_entry_to_shared_converts_millis_to_seconds() {
        let pid = peer();
        let entry = LedgerEntry {
            multiaddr: format!("/ip4/10.0.0.1/tcp/9001/p2p/{pid}"),
            peer_id: Some(pid),
            public_key: None,
            nickname: None,
            success_count: 3,
            failure_count: 0,
            last_seen: Some(1_700_000_000_123),
            topics: vec!["sc-mesh".to_string()],
        };
        let shared = ledger_entry_to_shared(&entry);
        assert_eq!(shared.last_seen, 1_700_000_000);
        assert_eq!(shared.multiaddr, "/ip4/10.0.0.1/tcp/9001");
        assert_eq!(shared.known_topics, vec!["sc-mesh".to_string()]);
    }

    // ------------------------------------------------------------------
    // F3 -- SSRF / internal-probing addresses must never enter the ledger
    // ------------------------------------------------------------------

    #[test]
    fn import_seed_entries_rejects_ssrf_and_non_routable_addresses() {
        let (_dir, mgr) = manager();
        let hostile = vec![
            // Cloud metadata service.
            seed("/ip4/169.254.169.254/tcp/80"),
            // Loopback -- services bound to the victim's own host.
            seed("/ip4/127.0.0.1/tcp/8080"),
            seed("/ip6/::1/tcp/8080"),
            // IPv4-mapped IPv6 form of the same thing.
            seed("/ip6/::ffff:127.0.0.1/tcp/8080"),
            // Unspecified / multicast / broadcast.
            seed("/ip4/0.0.0.0/tcp/9001"),
            seed("/ip4/224.0.0.1/tcp/9001"),
            seed("/ip4/255.255.255.255/tcp/9001"),
        ];
        let hostile_len = hostile.len();

        assert_eq!(
            mgr.import_seed_entries(hostile),
            0,
            "non-routable seeds were accepted into the ledger"
        );
        assert!(
            mgr.seed_addresses(64).is_empty(),
            "non-routable seeds became dial candidates"
        );
        assert!(mgr.dialable_addresses().is_empty());
        assert_eq!(hostile_len, 7);
    }

    #[test]
    fn import_seed_entries_honours_network_mode_for_rfc1918() {
        let (_dir, local_mgr) = manager();
        // Local mesh: an RFC1918 peer is exactly what invites are for.
        assert_eq!(
            local_mgr.import_seed_entries_with_mode(
                vec![seed("/ip4/192.168.1.1/tcp/443")],
                NetworkMode::Local
            ),
            1
        );

        // Public-only node: it has no route to anyone's LAN, and dialing one is
        // an internal probe.
        let (_dir2, public_mgr) = manager();
        assert_eq!(
            public_mgr.import_seed_entries_with_mode(
                vec![
                    seed("/ip4/192.168.1.1/tcp/443"),
                    seed("/ip4/10.1.2.3/tcp/443"),
                    seed("/ip4/172.20.0.1/tcp/443"),
                ],
                NetworkMode::Public
            ),
            0,
            "RFC1918 seeds accepted on a public-only node"
        );
        assert!(public_mgr.seed_addresses(64).is_empty());
    }

    // ------------------------------------------------------------------
    // F9 -- "" parses as a valid Multiaddr
    // ------------------------------------------------------------------

    #[test]
    fn import_seed_entries_rejects_entries_with_no_transport_component() {
        let (_dir, mgr) = manager();
        // Both of these previously stripped to something that `parse()`
        // accepted ("" and "/p2p-circuit") and were stored and re-gossiped.
        assert_eq!(
            mgr.import_seed_entries(vec![
                seed(&format!("/p2p/{}", peer())),
                seed(&format!("/p2p-circuit/p2p/{}", peer())),
                seed(""),
            ]),
            0
        );
        assert!(mgr.seed_addresses(64).is_empty());
    }

    // ------------------------------------------------------------------
    // F4 -- the seed tier must be bounded before it reaches the event loop
    // ------------------------------------------------------------------

    #[test]
    fn seed_addresses_is_bounded_by_limit() {
        let (_dir, mgr) = manager();
        // Import in MAX_SEED_LEDGER_ENTRIES-sized batches to build a ledger
        // larger than any single caller's cap.
        for batch in 0..8u32 {
            let entries: Vec<SeedLedgerEntry> = (0..MAX_SEED_LEDGER_ENTRIES)
                .map(|i| seed(&format!("/ip4/10.{}.{}.1/tcp/9001", batch, i)))
                .collect();
            mgr.import_seed_entries(entries);
        }
        let total = mgr.seed_addresses(u32::MAX).len();
        assert!(total >= 100, "expected a large ledger, got {total}");

        assert_eq!(mgr.seed_addresses(8).len(), 8);
        assert_eq!(mgr.seed_addresses(1).len(), 1);
        assert_eq!(mgr.seed_addresses(0).len(), 0);
    }

    // ------------------------------------------------------------------
    // F6 -- the ledger-exchange response is an unauthenticated disclosure
    // ------------------------------------------------------------------

    #[test]
    fn exchange_response_entries_caps_filters_and_drops_topics() {
        let (_dir, mgr) = manager();

        // 40 proven, routable peers, each with topic subscriptions.
        for i in 0..40u32 {
            let addr = format!("/ip4/198.51.100.{}/tcp/9001", i + 1);
            let pid = peer();
            mgr.record_connection(addr.clone(), pid.clone());
            mgr.annotate_identity(addr.clone(), pid, None, None);
        }
        {
            let mut entries = mgr.entries.lock();
            for entry in entries.iter_mut() {
                entry.topics = vec!["sc-family-chat".to_string(), "sc-activists".to_string()];
            }
        }
        // A proven but non-routable peer -- we can reach it, a stranger cannot,
        // and telling them about it maps our internal network.
        mgr.record_connection("/ip4/192.168.7.7/tcp/9001".to_string(), peer());
        let requester = peer();
        mgr.record_connection("/ip4/203.0.113.9/tcp/9001".to_string(), requester.clone());

        let response = mgr.exchange_response_entries(16, NetworkMode::Public, &requester);

        assert_eq!(response.len(), 16, "response cap not applied");
        assert!(
            response.iter().all(|e| e.known_topics.is_empty()),
            "known_topics leaked group membership into an unauthenticated response"
        );
        assert!(
            !response
                .iter()
                .any(|e| e.multiaddr.starts_with("/ip4/192.168.")),
            "RFC1918 neighbour disclosed to a public peer"
        );
        assert!(
            !response
                .iter()
                .any(|e| e.last_peer_id.as_deref() == Some(requester.as_str())),
            "requester echoed back to itself"
        );
    }

    // ------------------------------------------------------------------
    // F11 -- an in-memory core must not write topology to a temp dir
    // ------------------------------------------------------------------

    #[test]
    fn ephemeral_ledger_never_touches_the_filesystem() {
        let before: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();

        let mgr = LedgerManager::ephemeral();
        mgr.record_connection("/ip4/198.51.100.5/tcp/9001".to_string(), peer());
        assert_eq!(mgr.dialable_addresses().len(), 1);
        mgr.save().expect("ephemeral save is a no-op, not an error");
        mgr.load().expect("ephemeral load is a no-op, not an error");
        // The entry survives in memory across a load() (which must not clear).
        assert_eq!(mgr.dialable_addresses().len(), 1);

        let after: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(
            !after.iter().any(|n| n == "ledger.json") || before.iter().any(|n| n == "ledger.json"),
            "ephemeral ledger wrote ledger.json into the shared temp directory"
        );
    }
}
