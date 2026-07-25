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

/// Strip any trailing `/p2p/<peer-id>` component from a multiaddr string.
///
/// Matches the CLI ledger's key convention (`cli/src/ledger.rs::strip_peer_id`)
/// so the two ledgers dedupe on identical keys.
fn strip_peer_id_component(multiaddr: &str) -> String {
    match multiaddr.find("/p2p/") {
        Some(idx) => multiaddr[..idx].to_string(),
        None => multiaddr.to_string(),
    }
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
    storage_path: std::path::PathBuf,
    entries: Arc<Mutex<Vec<LedgerEntry>>>,
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl LedgerManager {
    #[uniffi::constructor]
    pub fn new(storage_path: String) -> Self {
        Self {
            storage_path: std::path::PathBuf::from(storage_path),
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn load(&self) -> Result<(), crate::IronCoreError> {
        let ledger_file = self.storage_path.join("ledger.json");
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
        std::fs::create_dir_all(&self.storage_path)
            .map_err(|_| crate::IronCoreError::StorageError)?;

        let ledger_file = self.storage_path.join("ledger.json");
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
    pub fn seed_addresses(&self) -> Vec<LedgerEntry> {
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|e| e.success_count == 0 && e.failure_count < 5)
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
    /// - Entries whose multiaddr does not parse are dropped, and the whole
    ///   batch is capped at [`MAX_SEED_LEDGER_ENTRIES`].
    pub fn import_seed_entries(&self, entries: Vec<SeedLedgerEntry>) -> u32 {
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
            if stripped.parse::<Multiaddr>().is_err() {
                tracing::debug!("Dropping unparseable seed multiaddr: {}", seed.multiaddr);
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

    #[test]
    fn strip_peer_id_component_matches_cli_convention() {
        assert_eq!(
            strip_peer_id_component("/ip4/1.2.3.4/tcp/9001/p2p/12D3KooWabc"),
            "/ip4/1.2.3.4/tcp/9001"
        );
        assert_eq!(
            strip_peer_id_component("/ip4/1.2.3.4/tcp/9001"),
            "/ip4/1.2.3.4/tcp/9001"
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
        let seeds = mgr.seed_addresses();
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
            mgr.import_seed_entries(vec![seed("/ip4/10.0.0.1/tcp/9001/p2p/12D3KooWaaa")]),
            0
        );
        let seeds = mgr.seed_addresses();
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
        let seeds = reloaded.seed_addresses();
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].multiaddr, "/ip4/10.0.0.7/tcp/9001");
    }

    #[test]
    fn export_seed_entries_only_exports_proven_peers_without_identity() {
        let (_dir, mgr) = manager();
        mgr.import_seed_entries(vec![seed("/ip4/10.0.0.9/tcp/9001")]);
        mgr.record_connection(
            "/ip4/10.0.0.1/tcp/9001/p2p/proven".to_string(),
            "proven".to_string(),
        );
        mgr.annotate_identity(
            "/ip4/10.0.0.1/tcp/9001/p2p/proven".to_string(),
            "proven".to_string(),
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
        let entry = LedgerEntry {
            multiaddr: "/ip4/10.0.0.1/tcp/9001/p2p/12D3KooWaaa".to_string(),
            peer_id: Some("12D3KooWaaa".to_string()),
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
}
