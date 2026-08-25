use scmessenger_core::IronCore;
use std::time::Duration;
use tempfile::tempdir;

const MAX_REOPEN_ATTEMPTS: u32 = 10;
const REOPEN_BACKOFF_BASE_MS: u64 = 50;

fn reopen_storage_with_retry(path: &str) -> IronCore {
    for attempt in 0..MAX_REOPEN_ATTEMPTS {
        // Sleep on Windows/macOS to give OS file lock time to release
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(
                REOPEN_BACKOFF_BASE_MS * attempt as u64,
            ));
        }
        let core = IronCore::with_storage(path.to_string());
        if core.get_identity_info().initialized {
            return core;
        }
    }
    IronCore::with_storage(path.to_string())
}

#[test]
fn test_identity_lookup_stable_across_invocations() {
    let dir = tempdir().expect("Failed to create tempdir");
    let storage_path = dir
        .path()
        .join("storage")
        .to_str()
        .expect("Valid path")
        .to_string();

    // 1. First invocation: initialize identity and set a nickname
    let id1;
    let pub1;
    let peer1;
    {
        let core1 = IronCore::with_storage(storage_path.clone());
        let info_before = core1.get_identity_info();
        assert!(
            !info_before.initialized,
            "Fresh storage should not be initialized"
        );
        assert!(info_before.identity_id.is_none());

        core1.grant_consent();
        core1
            .initialize_identity()
            .expect("First initialization must succeed");
        core1
            .set_nickname("TestNode".to_string())
            .expect("Setting nickname must succeed");

        let info1 = core1.get_identity_info();
        assert!(info1.initialized);
        id1 = info1.identity_id.expect("Identity ID must exist");
        pub1 = info1.public_key_hex.expect("Public key must exist");
        peer1 = info1.libp2p_peer_id.expect("Peer ID must exist");
        assert_eq!(info1.nickname, Some("TestNode".to_string()));
    }

    // 2. Second invocation: open storage again, look up identity WITHOUT re-initializing
    {
        let core2 = reopen_storage_with_retry(&storage_path);
        let info2 = core2.get_identity_info();
        assert!(
            info2.initialized,
            "Persisted identity must be automatically hydrated on open"
        );
        assert_eq!(
            info2.identity_id.as_deref(),
            Some(id1.as_str()),
            "Identity ID must match invocation 1"
        );
        assert_eq!(
            info2.public_key_hex.as_deref(),
            Some(pub1.as_str()),
            "Public key must match invocation 1"
        );
        assert_eq!(
            info2.libp2p_peer_id.as_deref(),
            Some(peer1.as_str()),
            "Peer ID must match invocation 1"
        );
        assert_eq!(
            info2.nickname.as_deref(),
            Some("TestNode"),
            "Nickname must match invocation 1"
        );
    }

    // 3. Third invocation: verify stability once more after calling initialize_identity (as cmd_identity does)
    {
        let core3 = reopen_storage_with_retry(&storage_path);
        core3.grant_consent();
        core3
            .initialize_identity()
            .expect("Loading existing identity must succeed");

        let info3 = core3.get_identity_info();
        assert!(info3.initialized);
        assert_eq!(
            info3.identity_id.as_deref(),
            Some(id1.as_str()),
            "Identity ID must remain identical on invocation 3"
        );
        assert_eq!(
            info3.public_key_hex.as_deref(),
            Some(pub1.as_str()),
            "Public key must remain identical on invocation 3"
        );
        assert_eq!(
            info3.libp2p_peer_id.as_deref(),
            Some(peer1.as_str()),
            "Peer ID must remain identical on invocation 3"
        );
        assert_eq!(
            info3.nickname.as_deref(),
            Some("TestNode"),
            "Nickname must remain identical on invocation 3"
        );
    }
}

#[test]
fn test_uninitialized_storage_does_not_claim_initialized() {
    let dir = tempdir().expect("Failed to create tempdir");
    let storage_path = dir
        .path()
        .join("storage")
        .to_str()
        .expect("Valid path")
        .to_string();

    let core = IronCore::with_storage(storage_path);
    let info = core.get_identity_info();
    assert!(!info.initialized);
    assert!(info.identity_id.is_none());
    assert!(info.public_key_hex.is_none());
    assert!(info.libp2p_peer_id.is_none());
}
