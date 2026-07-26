// Regression tests for the ledger-seeding adversarial review of 2026-07-25
// (HANDOFF/review/LEDGER_SEEDING_ADVERSARIAL_REVIEW_2026-07-25.md).
//
// Covers, end to end rather than at the unit boundary:
//   F11 -- IronCore is the ledger owner: the constructor hydrates it from disk,
//          and a real connection populates `dialable_addresses()` with nobody
//          calling `record_connection` by hand. The pre-existing
//          `integration_ledger_convergence.rs` only passed because its test
//          body seeded the ledger itself, so it proved nothing about
//          production.
//   F6  -- the `/sc/ledger-exchange/1.0.0` RESPONSE is capped, address-filtered
//          and carries no `known_topics`.
//   F3  -- an SSRF/internal address sitting in our ledger is never disclosed to
//          a peer and never becomes a dial candidate.
//
// Networked cases are #[ignore] by default, matching the rest of the suite:
//   cargo test -p scmessenger-core --test integration_ledger_seeding_hardening \
//       -- --include-ignored

use libp2p::identity::Keypair;
use libp2p::Multiaddr;
use scmessenger_core::store::ledger_entry::{LedgerEntry, SharedPeerEntry};
use scmessenger_core::transport::swarm::{start_swarm, SwarmEvent2, SwarmHandle};
use scmessenger_core::IronCore;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::mpsc;

/// F11, disk half: `IronCore`'s persistent constructors must call
/// `LedgerManager::load()`.
///
/// Nothing in `core/src` ever did, so every restart began with an empty ledger:
/// `success_count` was always 0, `dialable_addresses()` was permanently empty,
/// and both the seed-dial proven tier and the ledger-exchange response shipped
/// nothing. Needs no networking.
#[test]
fn iron_core_constructor_hydrates_the_ledger_from_disk() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().to_string_lossy().to_string();

    {
        let core = IronCore::with_storage(path.clone());
        core.ledger_manager.record_connection(
            "/ip4/198.51.100.42/tcp/9001".to_string(),
            libp2p::PeerId::random().to_string(),
        );
        assert_eq!(core.ledger_manager.dialable_addresses().len(), 1);
    }

    let restarted = IronCore::with_storage(path);
    let dialable = restarted.ledger_manager.dialable_addresses();
    assert_eq!(
        dialable.len(),
        1,
        "IronCore did not load the persisted ledger; every restart starts blind"
    );
    assert_eq!(dialable[0].multiaddr, "/ip4/198.51.100.42/tcp/9001");
    assert!(dialable[0].success_count > 0);
}

/// F11, world-readable-temp-dir half: the storage-less constructor must not
/// write peer topology into `std::env::temp_dir()`.
#[test]
fn in_memory_core_has_no_on_disk_ledger() {
    let core = IronCore::new();
    let temp_ledger = std::env::temp_dir().join("ledger.json");
    let before = std::fs::metadata(&temp_ledger).ok().map(|m| m.len());

    core.ledger_manager.record_connection(
        "/ip4/198.51.100.77/tcp/9001".to_string(),
        libp2p::PeerId::random().to_string(),
    );

    assert_eq!(core.ledger_manager.dialable_addresses().len(), 1);
    let after = std::fs::metadata(&temp_ledger).ok().map(|m| m.len());
    assert_eq!(
        before, after,
        "in-memory IronCore wrote its ledger into the shared temp directory"
    );
}

/// F11, production-path half: after a real dial, the dialer's ledger must show
/// a proven entry, with the test body never calling `record_connection`.
///
/// Before the fix `record_connection` had ZERO callers in `core/src`, so this
/// assertion was unsatisfiable in production no matter how long you waited.
#[tokio::test]
#[ignore = "requires real networking; run with --include-ignored"]
async fn dialing_a_peer_populates_the_dialer_ledger_without_manual_seeding() {
    let dir1 = TempDir::new().expect("tempdir 1");
    let dir2 = TempDir::new().expect("tempdir 2");
    let core1 = Arc::new(IronCore::with_storage(
        dir1.path().to_string_lossy().to_string(),
    ));
    let core2 = Arc::new(IronCore::with_storage(
        dir2.path().to_string_lossy().to_string(),
    ));

    assert!(
        core2.ledger_manager.dialable_addresses().is_empty(),
        "precondition: node 2 starts with a cold ledger"
    );

    let keypair1 = Keypair::generate_ed25519();
    let peer_id1 = libp2p::PeerId::from(keypair1.public());
    let keypair2 = Keypair::generate_ed25519();

    let (event_tx1, mut event_rx1) = mpsc::channel(256);
    let (event_tx2, mut event_rx2) = mpsc::channel(256);

    let swarm1: SwarmHandle = start_swarm(
        keypair1,
        None,
        event_tx1,
        Some(Arc::downgrade(&core1)),
        false,
        None,
        scmessenger_core::transport::default_routing_engine_handle(),
    )
    .await
    .expect("Failed to start swarm1");

    let node1_addr = first_loopback_tcp(&mut event_rx1).await;

    let swarm2: SwarmHandle = start_swarm(
        keypair2,
        None,
        event_tx2,
        Some(Arc::downgrade(&core2)),
        false,
        None,
        scmessenger_core::transport::default_routing_engine_handle(),
    )
    .await
    .expect("Failed to start swarm2");

    // Keep both event channels drained so the bounded mpsc never backpressures
    // the swarm task.
    tokio::spawn(async move { while event_rx1.recv().await.is_some() {} });
    tokio::spawn(async move { while event_rx2.recv().await.is_some() {} });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut dial_addr = node1_addr.clone();
    dial_addr.push(libp2p::multiaddr::Protocol::P2p(peer_id1));
    dial_or_already_connected(&swarm2, dial_addr).await;

    // Poll rather than sleep-and-hope.
    let mut dialable = Vec::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        dialable = core2.ledger_manager.dialable_addresses();
        if !dialable.is_empty() {
            break;
        }
    }

    assert!(
        !dialable.is_empty(),
        "node 2 dialed node 1 successfully but its ledger stayed empty -- \
         nothing in production calls record_connection"
    );
    assert!(
        dialable
            .iter()
            .any(|e| e.peer_id.as_deref() == Some(&peer_id1.to_string())),
        "the proven entry does not carry the peer id we actually reached: {:?}",
        dialable
    );
    assert!(
        dialable.iter().all(|e| !e.multiaddr.contains("/p2p/")),
        "ledger keys must be peer-id-stripped: {:?}",
        dialable
    );

    let _ = swarm1.shutdown().await;
    let _ = swarm2.shutdown().await;
}

/// F6 + F3: the unauthenticated ledger-exchange RESPONSE must be capped, must
/// not carry `known_topics`, and must not disclose non-routable addresses.
#[tokio::test]
#[ignore = "requires real networking; run with --include-ignored"]
async fn ledger_exchange_response_is_capped_topic_free_and_address_filtered() {
    const HOSTILE: &[&str] = &[
        "/ip4/169.254.169.254/tcp/80",
        "/ip4/127.0.0.1/tcp/8080",
        "/ip6/::1/tcp/8080",
    ];

    let dir1 = TempDir::new().expect("tempdir 1");
    let dir2 = TempDir::new().expect("tempdir 2");

    // Seed node 1's ledger on disk so the topics are populated (nothing in core
    // writes `LedgerEntry::topics` today, and an empty-topics assertion against
    // an always-empty field would be vacuous). Constructing IronCore
    // afterwards also exercises the F11 `load()` wiring.
    let mut seeded: Vec<LedgerEntry> = Vec::new();
    // 100 proven, routable peers -- more than the 64 response cap.
    for i in 0..100u32 {
        seeded.push(LedgerEntry {
            multiaddr: format!("/ip4/198.51.{}.{}/tcp/9001", i / 256, i % 256),
            peer_id: Some(libp2p::PeerId::random().to_string()),
            public_key: None,
            nickname: None,
            success_count: 3,
            failure_count: 0,
            last_seen: Some(1_700_000_000_000),
            topics: vec!["sc-family-chat".to_string(), "sc-activists".to_string()],
        });
    }
    // Proven, but nothing a stranger should ever hear about.
    for addr in HOSTILE {
        seeded.push(LedgerEntry {
            multiaddr: addr.to_string(),
            peer_id: Some(libp2p::PeerId::random().to_string()),
            public_key: None,
            nickname: None,
            success_count: 3,
            failure_count: 0,
            last_seen: Some(1_700_000_000_000),
            topics: vec!["sc-family-chat".to_string()],
        });
    }
    std::fs::create_dir_all(dir1.path()).expect("create ledger dir");
    std::fs::write(
        dir1.path().join("ledger.json"),
        serde_json::to_string_pretty(&seeded).expect("serialize seeded ledger"),
    )
    .expect("write seeded ledger");

    let core1 = Arc::new(IronCore::with_storage(
        dir1.path().to_string_lossy().to_string(),
    ));
    let core2 = Arc::new(IronCore::with_storage(
        dir2.path().to_string_lossy().to_string(),
    ));
    assert_eq!(
        core1.ledger_manager.dialable_addresses().len(),
        seeded.len(),
        "IronCore did not hydrate the seeded ledger"
    );

    let keypair1 = Keypair::generate_ed25519();
    let peer_id1 = libp2p::PeerId::from(keypair1.public());
    let keypair2 = Keypair::generate_ed25519();

    let (event_tx1, mut event_rx1) = mpsc::channel(256);
    let (event_tx2, mut event_rx2) = mpsc::channel(256);

    let swarm1: SwarmHandle = start_swarm(
        keypair1,
        None,
        event_tx1,
        Some(Arc::downgrade(&core1)),
        false,
        None,
        scmessenger_core::transport::default_routing_engine_handle(),
    )
    .await
    .expect("Failed to start swarm1");

    let node1_addr = first_loopback_tcp(&mut event_rx1).await;
    tokio::spawn(async move { while event_rx1.recv().await.is_some() {} });

    let swarm2: SwarmHandle = start_swarm(
        keypair2,
        None,
        event_tx2,
        Some(Arc::downgrade(&core2)),
        false,
        None,
        scmessenger_core::transport::default_routing_engine_handle(),
    )
    .await
    .expect("Failed to start swarm2");

    let responses: Arc<parking_lot::Mutex<Vec<Vec<SharedPeerEntry>>>> =
        Arc::new(parking_lot::Mutex::new(Vec::new()));
    let responses_task = responses.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx2.recv().await {
            if let SwarmEvent2::LedgerReceived { entries, .. } = event {
                responses_task.lock().push(entries);
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut dial_addr = node1_addr.clone();
    dial_addr.push(libp2p::multiaddr::Protocol::P2p(peer_id1));
    dial_or_already_connected(&swarm2, dial_addr).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Node 2 initiates; node 1's application layer never calls share_ledger.
    swarm2
        .share_ledger(peer_id1, Vec::new())
        .await
        .expect("Failed to share ledger");

    let mut received: Vec<SharedPeerEntry> = Vec::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        received = responses.lock().iter().flatten().cloned().collect();
        if !received.is_empty() {
            break;
        }
    }

    assert!(
        !received.is_empty(),
        "node 2 received no reciprocal ledger at all"
    );
    assert!(
        received.len() <= 64,
        "response exceeded the 64-record cap: {}",
        received.len()
    );
    assert!(
        received.iter().all(|e| e.known_topics.is_empty()),
        "known_topics leaked group membership to an unauthenticated peer"
    );
    for needle in ["127.0.0.1", "169.254.169.254", "/ip6/::1/"] {
        assert!(
            !received.iter().any(|e| e.multiaddr.contains(needle)),
            "non-routable address containing {} was disclosed over the wire: {:?}",
            needle,
            received
                .iter()
                .map(|e| e.multiaddr.as_str())
                .collect::<Vec<_>>()
        );
    }

    let _ = swarm1.shutdown().await;
    let _ = swarm2.shutdown().await;
}

async fn first_loopback_tcp(rx: &mut mpsc::Receiver<SwarmEvent2>) -> Multiaddr {
    let mut all_addrs: Vec<Multiaddr> = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = rx.recv().await {
            if let SwarmEvent2::ListeningOn(addr) = event {
                all_addrs.push(addr);
                if select_dialable_tcp_loopback(&all_addrs).is_some() {
                    break;
                }
            }
        }
    })
    .await
    .ok();

    select_dialable_tcp_loopback(&all_addrs)
        .expect("no plain TCP loopback listen address was reported")
}

/// Dial `addr`, tolerating the case where mDNS already connected the two nodes.
async fn dial_or_already_connected(swarm: &SwarmHandle, addr: Multiaddr) {
    if let Err(e) = swarm.dial(addr).await {
        let msg = e.to_string();
        let already_connected =
            msg.contains("already connected") || msg.contains("dial is in progress");
        assert!(already_connected, "Failed to dial: {}", msg);
        eprintln!("[INFO] Explicit dial skipped, peers already connected: {msg}");
    }
}

/// Pick a plain TCP loopback listener, avoiding the fixed WS port 9002.
fn select_dialable_tcp_loopback(addrs: &[Multiaddr]) -> Option<Multiaddr> {
    let mut loopback_ephemeral: Option<Multiaddr> = None;
    let mut loopback_any_tcp: Option<Multiaddr> = None;

    for addr in addrs {
        let s = addr.to_string();
        if s.contains("/ws") || s.contains("/quic") || s.contains("/p2p-circuit") {
            continue;
        }

        let mut has_tcp = false;
        let mut tcp_port: u16 = 0;
        let mut is_loopback = false;
        for proto in addr.iter() {
            match proto {
                libp2p::multiaddr::Protocol::Ip4(ip) => {
                    if ip == std::net::Ipv4Addr::LOCALHOST {
                        is_loopback = true;
                    }
                }
                libp2p::multiaddr::Protocol::Tcp(p) => {
                    has_tcp = true;
                    tcp_port = p;
                }
                _ => {}
            }
        }

        if !has_tcp || !is_loopback {
            continue;
        }
        if loopback_any_tcp.is_none() {
            loopback_any_tcp = Some(addr.clone());
        }
        if tcp_port != 9002 && loopback_ephemeral.is_none() {
            loopback_ephemeral = Some(addr.clone());
        }
    }

    loopback_ephemeral.or(loopback_any_tcp)
}
