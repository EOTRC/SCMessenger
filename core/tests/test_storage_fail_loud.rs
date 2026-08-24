// Test suite verifying that IronCore fails loud and never silently degrades
// persistent storage to RAM, preventing identity churn and fail-open security bypass.

use scmessenger_core::IronCore;
use tempfile::tempdir;

#[test]
fn test_storage_lock_contention_does_not_silently_mint_memory_identity() {
    let dir = tempdir().expect("tempdir");
    let path = dir
        .path()
        .join("storage_db")
        .to_str()
        .expect("valid path")
        .to_string();

    // 1. Open sled directly to hold the database lock (simulating a running relay).
    let _held_sled = sled::Config::default()
        .path(&path)
        .mode(sled::Mode::LowSpace)
        .use_compression(false)
        .open()
        .expect("held sled open");

    // 2. Attempt to construct IronCore with the same path while the lock is held.
    let core = IronCore::with_storage(path.clone());

    // 3. Assert that IronCore knows its storage is degraded.
    assert!(
        core.is_storage_degraded(),
        "IronCore must mark storage as degraded when sled lock cannot be acquired"
    );
    assert!(
        !core.is_storage_healthy(),
        "IronCore must report storage as not healthy"
    );
    assert!(
        core.storage_error().is_some(),
        "IronCore must provide a storage error string"
    );

    // 4. Assert that try_with_storage returns Err on lock contention.
    let try_result = IronCore::try_with_storage(path.clone());
    assert!(
        try_result.is_err(),
        "try_with_storage must return Err when database is locked"
    );

    // 5. Assert that attempting to initialize identity fails loud and does NOT mint a fresh identity in RAM.
    core.grant_consent();
    let init_result = core.initialize_identity();
    assert!(
        init_result.is_err(),
        "initialize_identity must fail when storage is degraded rather than minting a disposable identity"
    );

    let info = core.get_identity_info();
    assert!(
        !info.initialized,
        "Identity must not be initialized on degraded storage"
    );
    assert!(
        info.identity_id.is_none(),
        "No identity ID should be generated on degraded storage"
    );
    assert!(
        info.public_key_hex.is_none(),
        "No public key should be generated on degraded storage"
    );
}

#[test]
fn test_degraded_storage_fails_closed_on_blocked_peer_checks() {
    let dir = tempdir().expect("tempdir");
    let path = dir
        .path()
        .join("storage_db")
        .to_str()
        .expect("valid path")
        .to_string();

    // 1. Initialize a valid storage first, block a peer, then drop the core.
    let target_peer = "pk:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    {
        let core = IronCore::with_storage(path.clone());
        assert!(!core.is_storage_degraded());
        core.block_peer(target_peer.to_string(), None, None)
            .expect("block peer");
    }

    // 2. Now hold the sled lock directly to simulate contention / lock failure on reopen.
    let _held_sled = sled::Config::default()
        .path(&path)
        .mode(sled::Mode::LowSpace)
        .use_compression(false)
        .open()
        .expect("held sled open");

    // 3. Reopen IronCore with storage while locked -> degraded state.
    let degraded_core = IronCore::with_storage(path.clone());
    assert!(
        degraded_core.is_storage_degraded(),
        "Storage must be marked degraded when lock is held"
    );

    // 4. Verify low-level block manager read returns Err (StorageError), not Ok(false)
    let block_res = degraded_core.is_peer_blocked(target_peer.to_string(), None);
    assert!(
        block_res.is_err(),
        "is_peer_blocked must return Err(StorageError) on degraded storage instead of Ok(false)"
    );

    // 5. Verify notify_peer_discovered suppresses notification (fails closed).
    // notify_peer_discovered silently suppresses when block lookup fails (fail-closed).
    // We verify this does not panic.
    degraded_core.notify_peer_discovered(target_peer.to_string());
}

#[test]
fn test_try_with_storage_and_logs_fails_on_locked_storage() {
    let dir = tempdir().expect("tempdir");
    let path = dir
        .path()
        .join("storage_db")
        .to_str()
        .expect("valid path")
        .to_string();
    let log_dir = dir
        .path()
        .join("logs")
        .to_str()
        .expect("valid path")
        .to_string();

    let _held_sled = sled::Config::default()
        .path(&path)
        .mode(sled::Mode::LowSpace)
        .use_compression(false)
        .open()
        .expect("held sled open");

    let result = IronCore::try_with_storage_and_logs(path, log_dir);
    assert!(
        result.is_err(),
        "try_with_storage_and_logs must return Err when storage cannot be opened"
    );
}

#[test]
fn test_mesh_service_start_refuses_degraded_storage_and_succeeds_when_healthy() {
    use scmessenger_core::mobile_bridge::{MeshService, MeshServiceConfig};
    use scmessenger_core::IronCoreError;
    use std::sync::Arc;

    let dir = tempdir().expect("tempdir");
    let path = dir
        .path()
        .join("storage_db")
        .to_str()
        .expect("valid path")
        .to_string();

    let config = MeshServiceConfig {
        discovery_interval_ms: 5000,
        battery_floor_pct: 20,
    };

    // 1. Locked storage: hold sled lock, verify start() fails loud with StorageError before starting core
    {
        let _held_sled = sled::Config::default()
            .path(&path)
            .mode(sled::Mode::LowSpace)
            .use_compression(false)
            .open()
            .expect("held sled open");

        let service = Arc::new(MeshService::with_storage(config.clone(), path.clone()));
        let result = service.start();
        assert!(
            matches!(result, Err(IronCoreError::StorageError)),
            "MeshService::start must return Err(IronCoreError::StorageError) on locked storage, got {:?}",
            result
        );
    }

    // 2. Healthy storage: lock released, verify start() succeeds
    {
        let service = Arc::new(MeshService::with_storage(config, path));
        let result = service.clone().start();
        assert!(
            result.is_ok(),
            "MeshService::start must return Ok(()) on healthy storage, got {:?}",
            result
        );
        service.stop();
    }
}

#[test]
fn test_mesh_service_start_with_logs_refuses_degraded_storage_and_succeeds_when_healthy() {
    use scmessenger_core::mobile_bridge::{MeshService, MeshServiceConfig};
    use scmessenger_core::IronCoreError;
    use std::sync::Arc;

    let dir = tempdir().expect("tempdir");
    let path = dir
        .path()
        .join("storage_db")
        .to_str()
        .expect("valid path")
        .to_string();
    let log_dir = dir
        .path()
        .join("logs")
        .to_str()
        .expect("valid path")
        .to_string();

    let config = MeshServiceConfig {
        discovery_interval_ms: 5000,
        battery_floor_pct: 20,
    };

    // 1. Locked storage: hold sled lock, verify start() fails loud with StorageError before starting core
    {
        let _held_sled = sled::Config::default()
            .path(&path)
            .mode(sled::Mode::LowSpace)
            .use_compression(false)
            .open()
            .expect("held sled open");

        let service = Arc::new(MeshService::with_storage_and_logs(
            config.clone(),
            path.clone(),
            log_dir.clone(),
        ));
        let result = service.start();
        assert!(
            matches!(result, Err(IronCoreError::StorageError)),
            "MeshService::start with logs must return Err(IronCoreError::StorageError) on locked storage, got {:?}",
            result
        );
    }

    // 2. Healthy storage: lock released, verify start() succeeds
    {
        let service = Arc::new(MeshService::with_storage_and_logs(config, path, log_dir));
        let result = service.clone().start();
        assert!(
            result.is_ok(),
            "MeshService::start with logs must return Ok(()) on healthy storage, got {:?}",
            result
        );
        service.stop();
    }
}
