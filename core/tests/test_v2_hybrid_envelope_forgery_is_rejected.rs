//! Integration test verifying that V2 hybrid envelope forgery without the sender's private key
//! is rejected (Leg 1 KDF sender authentication and Leg 2 ingress signature verification),
//! while honest senders succeed.
//!
//! Run with:
//!   cargo test -p scmessenger-core --test test_v2_hybrid_envelope_forgery_is_rejected

use scmessenger_core::crypto::negotiation::negotiate_suite;
use scmessenger_core::crypto::{
    decrypt_with_ratchet_fallback, RatchetSession, RatchetSessionManager,
};
use scmessenger_core::drift::{DriftEnvelope, EnvelopeType, DRIFT_VERSION};
use scmessenger_core::identity::{sign_bundle, IdentityKeys};
use scmessenger_core::message::codec::{decode_message, encode_message};
use scmessenger_core::message::{EnvelopeV2, Message, MessageType, WireEnvelope};
use scmessenger_core::IronCore;

#[test]
fn test_v2_hybrid_envelope_forgery_is_rejected() {
    // =========================================================================
    // STEP 1: Honest Alice and Honest Bob Generate Key Bundles
    // =========================================================================
    let alice_keys = IdentityKeys::generate();
    let alice_public_bundle = sign_bundle(&alice_keys).expect("Alice bundle signing must succeed");
    let alice_pubkey_hex = alice_keys.public_key_hex();

    let bob_keys = IdentityKeys::generate();
    let bob_public_bundle = sign_bundle(&bob_keys).expect("Bob bundle signing must succeed");

    // Mallory only has access to Alice's and Bob's public bundles.
    let mallory_known_alice_bundle = alice_public_bundle.clone();
    let mallory_known_bob_bundle = bob_public_bundle.clone();

    // =========================================================================
    // STEP 2: Mallory Attempts to Forge an Envelope Pretending to be Alice
    // =========================================================================
    let (suite, transcript_hash) = negotiate_suite(
        &mallory_known_alice_bundle.supported_suites,
        &mallory_known_bob_bundle.supported_suites,
        &mallory_known_alice_bundle.ed25519_public,
        &mallory_known_bob_bundle.ed25519_public,
    )
    .expect("Suite negotiation must succeed");
    // Both 0x02 (original hybrid) and 0x03 (current hybrid with sender-auth DH)
    // are valid PQ-hybrid suites. The negotiated suite depends on what both peers
    // advertise in their bundles. Current nodes advertise [0x01, 0x03], so two
    // current nodes negotiate 0x03. The test logic works identically for either
    // suite since both use the same hybrid encryption primitives -- only the
    // initial root key derivation differs (see RatchetSession::is_pq_hybrid).
    assert!(
        matches!(suite, 0x02 | 0x03),
        "Expected suite 0x02 or 0x03 (PQ-hybrid), got 0x{:02x}",
        suite
    );

    // Mallory generates her own throwaway signing key because she does not possess Alice's.
    let mut mallory_secret = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut mallory_secret);
    let mallory_dummy_signing_key = ed25519_dalek::SigningKey::from_bytes(&mallory_secret);
    // Mallory also has no access to Alice's dedicated X25519 encryption secret -- she can
    // only use her own throwaway one.
    let mallory_dummy_x25519_secret =
        scmessenger_core::crypto::encrypt::ed25519_to_x25519_secret(&mallory_dummy_signing_key);

    // Mallory initializes a sender hybrid session with her dummy key
    let mut mallory_session = RatchetSession::init_as_sender_hybrid(
        &mallory_dummy_x25519_secret,
        &mallory_known_bob_bundle,
        transcript_hash,
    )
    .expect("Sender hybrid session init must succeed for Mallory");

    let forged_payload_text = "MALLORY_FORGED_IMPERSONATION_PAYLOAD";
    let forged_message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender_id: alice_pubkey_hex.clone(), // Mallory attributes sender to Alice
        recipient_id: bob_keys.public_key_hex(),
        message_type: MessageType::Text,
        payload: forged_payload_text.as_bytes().to_vec(),
        timestamp: scmessenger_core::util::unix_time_secs(),
    };
    let forged_message_bytes =
        encode_message(&forged_message).expect("Message encoding must succeed");

    // Mallory encrypts the message using her session
    let encrypt_result = mallory_session
        .encrypt(
            &forged_message_bytes,
            &mallory_known_alice_bundle.ed25519_public,
        )
        .expect("Ratchet encrypt must succeed");

    let bootstrap_hct = mallory_session
        .bootstrap_hct
        .as_ref()
        .expect("bootstrap_hct must exist");
    let pq_our_keypair = mallory_session
        .pq_our_keypair
        .as_ref()
        .expect("pq_our_keypair must exist");

    // Construct forged WireEnvelope::V2 directly
    let forged_wire_v2 = WireEnvelope::V2(EnvelopeV2 {
        suite,
        sender_public_key: mallory_known_alice_bundle.ed25519_public.to_vec(),
        ephemeral_public_key: bootstrap_hct.x25519_ephemeral_public.to_vec(),
        nonce: encrypt_result.nonce.clone(),
        ciphertext: encrypt_result.ciphertext.clone(),
        ratchet_dh_public: Some(encrypt_result.our_dh_public.to_vec()),
        ratchet_message_number: Some(encrypt_result.message_number),
        pq_kem_ciphertext: Some(bootstrap_hct.mlkem_ciphertext.clone()),
        pq_encaps_key: Some(pq_our_keypair.public_key().to_vec()),
        transcript_hash: Some(transcript_hash.to_vec()),
    });

    // Construct forged DriftEnvelope with dummy/zero signature
    let forged_drift_envelope = DriftEnvelope {
        version: DRIFT_VERSION,
        envelope_type: EnvelopeType::EncryptedMessage,
        compressed: false,
        message_id: *uuid::Uuid::parse_str(&forged_message.id)
            .unwrap()
            .as_bytes(),
        recipient_hint: DriftEnvelope::hint_from_public_key(
            &mallory_known_bob_bundle.ed25519_public,
        ),
        created_at: scmessenger_core::util::unix_time_secs() as u32,
        ttl_expiry: 0,
        hop_count: 0,
        priority: 128,
        sender_public_key: mallory_known_alice_bundle.ed25519_public,
        ephemeral_public_key: bootstrap_hct.x25519_ephemeral_public,
        nonce: encrypt_result
            .nonce
            .clone()
            .try_into()
            .expect("24-byte nonce"),
        signature: [0u8; 64], // Dummy signature: Mallory cannot sign for Alice!
        ciphertext: encrypt_result.ciphertext,
        ratchet_dh_public: Some(encrypt_result.our_dh_public),
        ratchet_message_number: Some(encrypt_result.message_number),
        suite: Some(suite),
        pq_kem_ciphertext: Some(bootstrap_hct.mlkem_ciphertext.clone()),
        pq_encaps_key: Some(pq_our_keypair.public_key().to_vec()),
        transcript_hash: Some(transcript_hash.to_vec()),
    };
    let forged_drift_wire_bytes = forged_drift_envelope
        .to_bytes()
        .expect("Drift envelope serialization must succeed");

    // =========================================================================
    // STEP 3: Bob Attempts to Decrypt / Receive the Inbound Forged Message
    // =========================================================================

    // --- LEVEL 1a: Primitive-level test (`decrypt_with_ratchet_fallback` direct) ---
    // Leg 1 ensures KDF derives mismatched root key because Mallory lacks Alice's static private key.
    let mut bob_session_manager_direct = RatchetSessionManager::new();
    let direct_decrypt_result = decrypt_with_ratchet_fallback(
        &bob_keys.signing_key,
        Some(&bob_keys.x25519_encryption_secret),
        &forged_wire_v2,
        Some(&mut bob_session_manager_direct),
        Some(&bob_keys.mlkem_keypair),
        Some(&bob_public_bundle),
        Some(&alice_public_bundle),
    );
    assert!(
        direct_decrypt_result.is_err(),
        "Direct WireEnvelope::V2 primitive decryption MUST FAIL on forged envelope without Alice's static key"
    );

    // --- LEVEL 1b: Codec decode from Drift wire bytes ---
    // Leg 2 ensures Drift wire decoding rejects unverifiable signature.
    let decoded_wire_result =
        scmessenger_core::message::codec::decode_wire_envelope(&forged_drift_wire_bytes);
    assert!(
        decoded_wire_result.is_err(),
        "Drift wire decoding MUST FAIL on envelope with dummy/invalid signature"
    );

    // --- LEVEL 2: High-level full system test (`IronCore::receive_message`) ---
    let bob_node = IronCore::new();
    bob_node.grant_consent();
    bob_node
        .initialize_identity()
        .expect("Bob node initialization must succeed");

    // Bob imports Alice's published public bundle into his contacts store
    let bob_node_keys = bob_node
        .get_identity_keys()
        .expect("Bob node must have keys");
    bob_node
        .contacts_store_manager()
        .save_contact_bundle(&alice_pubkey_hex, &alice_public_bundle)
        .expect("Saving Alice's public bundle in Bob's contact store must succeed");

    // Re-forge the envelope specifically addressed to bob_node's actual initialized identity
    let bob_node_bundle =
        sign_bundle(&bob_node_keys).expect("Signing bob_node's bundle must succeed");
    let (node_suite, node_transcript_hash) = negotiate_suite(
        &mallory_known_alice_bundle.supported_suites,
        &bob_node_bundle.supported_suites,
        &mallory_known_alice_bundle.ed25519_public,
        &bob_node_bundle.ed25519_public,
    )
    .expect("Suite negotiation with bob_node must succeed");
    assert!(
        matches!(node_suite, 0x02 | 0x03),
        "Expected suite 0x02 or 0x03 (PQ-hybrid), got 0x{:02x}",
        node_suite
    );

    let mut mallory_session_for_node = RatchetSession::init_as_sender_hybrid(
        &mallory_dummy_x25519_secret,
        &bob_node_bundle,
        node_transcript_hash,
    )
    .expect("Sender hybrid session init for bob_node must succeed");

    let node_forged_message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender_id: alice_pubkey_hex.clone(),
        recipient_id: bob_node.get_identity_info().public_key_hex.unwrap(),
        message_type: MessageType::Text,
        payload: forged_payload_text.as_bytes().to_vec(),
        timestamp: scmessenger_core::util::unix_time_secs(),
    };
    let node_forged_bytes =
        encode_message(&node_forged_message).expect("Encoding node forged message");

    let node_encrypt_result = mallory_session_for_node
        .encrypt(
            &node_forged_bytes,
            &mallory_known_alice_bundle.ed25519_public,
        )
        .expect("Ratchet encrypt for node");

    let node_bootstrap_hct = mallory_session_for_node.bootstrap_hct.as_ref().unwrap();
    let node_pq_keypair = mallory_session_for_node.pq_our_keypair.as_ref().unwrap();

    let node_forged_drift_envelope = DriftEnvelope {
        version: DRIFT_VERSION,
        envelope_type: EnvelopeType::EncryptedMessage,
        compressed: false,
        message_id: *uuid::Uuid::parse_str(&node_forged_message.id)
            .unwrap()
            .as_bytes(),
        recipient_hint: DriftEnvelope::hint_from_public_key(&bob_node_bundle.ed25519_public),
        created_at: scmessenger_core::util::unix_time_secs() as u32,
        ttl_expiry: 0,
        hop_count: 0,
        priority: 128,
        sender_public_key: mallory_known_alice_bundle.ed25519_public,
        ephemeral_public_key: node_bootstrap_hct.x25519_ephemeral_public,
        nonce: node_encrypt_result
            .nonce
            .clone()
            .try_into()
            .expect("24-byte nonce"),
        signature: [0u8; 64], // Forged / zero signature
        ciphertext: node_encrypt_result.ciphertext,
        ratchet_dh_public: Some(node_encrypt_result.our_dh_public),
        ratchet_message_number: Some(node_encrypt_result.message_number),
        suite: Some(node_suite),
        pq_kem_ciphertext: Some(node_bootstrap_hct.mlkem_ciphertext.clone()),
        pq_encaps_key: Some(node_pq_keypair.public_key().to_vec()),
        transcript_hash: Some(node_transcript_hash.to_vec()),
    };
    let node_forged_drift_bytes = node_forged_drift_envelope
        .to_bytes()
        .expect("Serialize node forged drift envelope");

    // Feed the forged drift envelope directly into Bob's IronCore::receive_message
    let receive_result = bob_node.receive_message(node_forged_drift_bytes);

    // =========================================================================
    // STEP 4: Assertions on the Outcome
    // =========================================================================
    assert!(
        receive_result.is_err(),
        "IronCore::receive_message MUST REJECT forged envelope at ingress"
    );
}

#[test]
fn test_v2_hybrid_honest_sender_succeeds() {
    // =========================================================================
    // STEP 1: Honest Alice and Honest Bob Setup
    // =========================================================================
    let alice_keys = IdentityKeys::generate();
    let alice_public_bundle = sign_bundle(&alice_keys).expect("Alice bundle signing must succeed");
    let alice_pubkey_hex = alice_keys.public_key_hex();

    let bob_keys = IdentityKeys::generate();
    let bob_public_bundle = sign_bundle(&bob_keys).expect("Bob bundle signing must succeed");

    // =========================================================================
    // STEP 2: Alice Encrypts with Her Real Private Keys
    // =========================================================================
    let (suite, transcript_hash) = negotiate_suite(
        &alice_public_bundle.supported_suites,
        &bob_public_bundle.supported_suites,
        &alice_public_bundle.ed25519_public,
        &bob_public_bundle.ed25519_public,
    )
    .expect("Suite negotiation must succeed");
    assert!(
        matches!(suite, 0x02 | 0x03),
        "Expected suite 0x02 or 0x03 (PQ-hybrid), got 0x{:02x}",
        suite
    );

    let mut alice_session = RatchetSession::init_as_sender_hybrid(
        &alice_keys.x25519_encryption_secret,
        &bob_public_bundle,
        transcript_hash,
    )
    .expect("Sender hybrid session init must succeed for Alice");

    let honest_payload_text = "HONEST_AUTHENTICATED_HELLO_FROM_ALICE";
    let honest_message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender_id: alice_pubkey_hex.clone(),
        recipient_id: bob_keys.public_key_hex(),
        message_type: MessageType::Text,
        payload: honest_payload_text.as_bytes().to_vec(),
        timestamp: scmessenger_core::util::unix_time_secs(),
    };
    let honest_message_bytes =
        encode_message(&honest_message).expect("Message encoding must succeed");

    let encrypt_result = alice_session
        .encrypt(&honest_message_bytes, &alice_public_bundle.ed25519_public)
        .expect("Ratchet encrypt must succeed");

    let bootstrap_hct = alice_session
        .bootstrap_hct
        .as_ref()
        .expect("bootstrap_hct must exist");
    let pq_our_keypair = alice_session
        .pq_our_keypair
        .as_ref()
        .expect("pq_our_keypair must exist");

    let honest_wire_v2 = WireEnvelope::V2(EnvelopeV2 {
        suite,
        sender_public_key: alice_public_bundle.ed25519_public.to_vec(),
        ephemeral_public_key: bootstrap_hct.x25519_ephemeral_public.to_vec(),
        nonce: encrypt_result.nonce.clone(),
        ciphertext: encrypt_result.ciphertext.clone(),
        ratchet_dh_public: Some(encrypt_result.our_dh_public.to_vec()),
        ratchet_message_number: Some(encrypt_result.message_number),
        pq_kem_ciphertext: Some(bootstrap_hct.mlkem_ciphertext.clone()),
        pq_encaps_key: Some(pq_our_keypair.public_key().to_vec()),
        transcript_hash: Some(transcript_hash.to_vec()),
    });

    let honest_drift_envelope = DriftEnvelope::from_v2_envelope(
        match &honest_wire_v2 {
            WireEnvelope::V2(env2) => env2.clone(),
            _ => unreachable!(),
        },
        honest_message.id.clone(),
        bob_public_bundle.ed25519_public,
        &alice_keys.signing_key,
    )
    .expect("Drift envelope creation from V2 must succeed");

    let honest_drift_wire_bytes = honest_drift_envelope
        .to_bytes()
        .expect("Drift serialization must succeed");

    // =========================================================================
    // STEP 3: Bob Decrypts Honest Message
    // =========================================================================

    // --- LEVEL 1a: Direct primitive decrypt ---
    let mut bob_session_manager_direct = RatchetSessionManager::new();
    let direct_decrypt_result = decrypt_with_ratchet_fallback(
        &bob_keys.signing_key,
        Some(&bob_keys.x25519_encryption_secret),
        &honest_wire_v2,
        Some(&mut bob_session_manager_direct),
        Some(&bob_keys.mlkem_keypair),
        Some(&bob_public_bundle),
        Some(&alice_public_bundle),
    );
    assert!(
        direct_decrypt_result.is_ok(),
        "Direct WireEnvelope::V2 primitive decryption failed for honest sender: {:?}",
        direct_decrypt_result.err()
    );

    // --- LEVEL 1b: Drift decode and decrypt ---
    let decoded_wire =
        scmessenger_core::message::codec::decode_wire_envelope(&honest_drift_wire_bytes)
            .expect("Drift wire decoding must succeed for valid signed envelope");

    let mut bob_session_manager = RatchetSessionManager::new();
    let primitive_decrypt_result = decrypt_with_ratchet_fallback(
        &bob_keys.signing_key,
        Some(&bob_keys.x25519_encryption_secret),
        &decoded_wire,
        Some(&mut bob_session_manager),
        Some(&bob_keys.mlkem_keypair),
        Some(&bob_public_bundle),
        Some(&alice_public_bundle),
    );
    assert!(
        primitive_decrypt_result.is_ok(),
        "Primitive decryption failed for honest sender: {:?}",
        primitive_decrypt_result.err()
    );
    let recovered_msg =
        decode_message(&primitive_decrypt_result.unwrap()).expect("Decoded message");
    assert_eq!(recovered_msg.sender_id, alice_pubkey_hex);
    assert_eq!(
        recovered_msg.text_content().expect("text"),
        honest_payload_text
    );

    // --- LEVEL 2: High-level IronCore::receive_message test ---
    let bob_node = IronCore::new();
    bob_node.grant_consent();
    bob_node
        .initialize_identity()
        .expect("Bob node initialization must succeed");

    let bob_node_keys = bob_node
        .get_identity_keys()
        .expect("Bob node must have keys");
    bob_node
        .contacts_store_manager()
        .save_contact_bundle(&alice_pubkey_hex, &alice_public_bundle)
        .expect("Saving Alice's public bundle in Bob's contact store must succeed");

    let bob_node_bundle =
        sign_bundle(&bob_node_keys).expect("Signing bob_node's bundle must succeed");
    let (node_suite, node_transcript_hash) = negotiate_suite(
        &alice_public_bundle.supported_suites,
        &bob_node_bundle.supported_suites,
        &alice_public_bundle.ed25519_public,
        &bob_node_bundle.ed25519_public,
    )
    .expect("Suite negotiation with bob_node must succeed");
    assert!(
        matches!(node_suite, 0x02 | 0x03),
        "Expected suite 0x02 or 0x03 (PQ-hybrid), got 0x{:02x}",
        node_suite
    );

    let mut alice_session_for_node = RatchetSession::init_as_sender_hybrid(
        &alice_keys.x25519_encryption_secret,
        &bob_node_bundle,
        node_transcript_hash,
    )
    .expect("Sender hybrid session init for bob_node must succeed");

    let node_honest_message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender_id: alice_pubkey_hex.clone(),
        recipient_id: bob_node.get_identity_info().public_key_hex.unwrap(),
        message_type: MessageType::Text,
        payload: honest_payload_text.as_bytes().to_vec(),
        timestamp: scmessenger_core::util::unix_time_secs(),
    };
    let node_honest_bytes = encode_message(&node_honest_message).expect("Encoding node message");

    let node_encrypt_result = alice_session_for_node
        .encrypt(&node_honest_bytes, &alice_public_bundle.ed25519_public)
        .expect("Ratchet encrypt for node");

    let node_bootstrap_hct = alice_session_for_node.bootstrap_hct.as_ref().unwrap();
    let node_pq_keypair = alice_session_for_node.pq_our_keypair.as_ref().unwrap();

    let node_honest_drift_envelope = DriftEnvelope::from_v2_envelope(
        EnvelopeV2 {
            suite: node_suite,
            sender_public_key: alice_public_bundle.ed25519_public.to_vec(),
            ephemeral_public_key: node_bootstrap_hct.x25519_ephemeral_public.to_vec(),
            nonce: node_encrypt_result.nonce.clone(),
            ciphertext: node_encrypt_result.ciphertext,
            ratchet_dh_public: Some(node_encrypt_result.our_dh_public.to_vec()),
            ratchet_message_number: Some(node_encrypt_result.message_number),
            pq_kem_ciphertext: Some(node_bootstrap_hct.mlkem_ciphertext.clone()),
            pq_encaps_key: Some(node_pq_keypair.public_key().to_vec()),
            transcript_hash: Some(node_transcript_hash.to_vec()),
        },
        node_honest_message.id.clone(),
        bob_node_bundle.ed25519_public,
        &alice_keys.signing_key,
    )
    .expect("Drift envelope creation must succeed");

    let node_honest_drift_bytes = node_honest_drift_envelope
        .to_bytes()
        .expect("Serialize node honest drift envelope");

    let receive_result = bob_node.receive_message(node_honest_drift_bytes);
    assert!(
        receive_result.is_ok(),
        "IronCore::receive_message failed for honest sender: {:?}",
        receive_result.err()
    );

    let received_message = receive_result.unwrap();
    assert_eq!(
        received_message.sender_id, alice_pubkey_hex,
        "Honest message sender_id must be Alice's public key"
    );
    assert_eq!(
        received_message.text_content().expect("Text payload"),
        honest_payload_text,
        "Decrypted plaintext must match honest payload"
    );
}
