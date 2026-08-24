use std::collections::HashSet;

/// Suite IDs whose sessions use PQ-hybrid (ML-KEM-768 + X25519) ratchet
/// establishment and mixing, as opposed to suite 0x01 (classical per-message
/// ECDH, no PQ).
///
/// 0x02 is the ORIGINAL hybrid derivation ("iron-core session-root v2
/// 2026-07", root key derived from the ML-KEM/X25519 hybrid shared secret and
/// the negotiation transcript hash only -- no sender-authentication DH term).
/// 0x03 is the CURRENT hybrid derivation ("iron-core session-root v3
/// 2026-08"), which additionally folds a static-static DH term (keyed on the
/// sender's dedicated X25519 encryption key) into the root key so the
/// derivation can no longer be computed from public material alone.
///
/// These two MUST stay on separate, never-merged code paths
/// (`RatchetSession::init_as_sender_hybrid_suite02` /
/// `init_as_receiver_hybrid_suite02` for 0x02;
/// `RatchetSession::init_as_sender_hybrid` / `init_as_receiver_hybrid` for
/// 0x03) -- once a suite ID ships, its derivation is frozen. A new derivation
/// gets a new suite ID, never a silent in-place redefinition of an existing
/// one (that class of bug is what made 0x02 and 0x03 diverge into two
/// definitions in the first place).
///
/// Everything AFTER session establishment (ratchet stepping, PQ ratchet-step
/// cadence, PQ-stripping validation) is suite-agnostic between 0x02 and 0x03
/// -- both negotiate identical ongoing hybrid mixing behavior via
/// `RatchetSession::is_pq_hybrid()`. Only the initial root-key derivation
/// differs.
pub const HYBRID_SUITE_IDS: [u8; 2] = [0x02, 0x03];

/// Negotiates a cryptographic suite and generates a transcript hash bound to the negotiation.
///
/// Returns `(negotiated_suite, transcript_hash)`.
///
/// The transcript hash MUST be computed by the INITIATOR with `our_` prefix referring to the initiator
/// and `their_` prefix referring to the responder. The responder recomputes this from their perspective
/// by calling this function where `our_` refers to the INITIATOR and `their_` refers to the RESPONDER.
/// This means the responder calls this function passing the initiator's properties as `our_...` to match.
pub fn negotiate_suite(
    our_suites: &[u8],
    their_suites: &[u8],
    our_ed25519_pub: &[u8; 32],
    their_ed25519_pub: &[u8; 32],
) -> Result<(u8, [u8; 32]), crate::IronCoreError> {
    let our_set: HashSet<u8> = our_suites.iter().cloned().collect();
    let their_set: HashSet<u8> = their_suites.iter().cloned().collect();
    let intersection: Vec<u8> = our_set.intersection(&their_set).cloned().collect();

    // An empty intersection means no mutually supported suite; surface it as a
    // recoverable negotiation failure rather than panicking on `max()`.
    let negotiated_suite = match intersection.iter().max() {
        Some(&suite) => suite,
        None => return Err(crate::IronCoreError::CryptoError),
    };

    let mut material = Vec::new();
    material.extend_from_slice(our_suites);
    material.push(0xFF);
    material.extend_from_slice(their_suites);
    material.push(0xFF);
    material.push(negotiated_suite);
    material.extend_from_slice(our_ed25519_pub);
    material.extend_from_slice(their_ed25519_pub);

    let transcript_hash = blake3::derive_key("iron-core suite-transcript v1", &material);

    Ok((negotiated_suite, transcript_hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_negotiate_suite_empty_intersection() {
        let our_suites = [0x01, 0x02];
        let their_suites = [0x03, 0x04];
        let our_pub = [0u8; 32];
        let their_pub = [1u8; 32];

        let result = negotiate_suite(&our_suites, &their_suites, &our_pub, &their_pub);
        assert!(result.is_err());
    }

    #[test]
    fn test_negotiate_suite_singleton() {
        let our_suites = [0x01];
        let their_suites = [0x01, 0x02];
        let our_pub = [0u8; 32];
        let their_pub = [1u8; 32];

        let (suite, _) = negotiate_suite(&our_suites, &their_suites, &our_pub, &their_pub).unwrap();
        assert_eq!(suite, 0x01);
    }

    #[test]
    fn test_negotiate_suite_future_suites() {
        let our_suites = [0x01, 0x02, 0xFF];
        let their_suites = [0x01, 0x02, 0xFF, 0xFE];
        let our_pub = [0u8; 32];
        let their_pub = [1u8; 32];

        let (suite, _) = negotiate_suite(&our_suites, &their_suites, &our_pub, &their_pub).unwrap();
        assert_eq!(suite, 0xFF);
    }

    #[test]
    fn test_negotiate_suite_symmetry() {
        let our_suites = [0x01, 0x02, 0x03];
        let their_suites = [0x01, 0x02, 0x03];
        let our_pub = [0u8; 32];
        let their_pub = [1u8; 32];

        let (suite_1, hash_1) =
            negotiate_suite(&our_suites, &their_suites, &our_pub, &their_pub).unwrap();
        // The responder calls it passing the INITIATOR's stuff as "our" to ensure identical material order
        let (suite_2, hash_2) =
            negotiate_suite(&our_suites, &their_suites, &our_pub, &their_pub).unwrap();

        assert_eq!(suite_1, suite_2);
        assert_eq!(hash_1, hash_2);

        // If responder accidentally inverted the args (used their own suites as `our`), the hash MUST mismatch!
        let (_, hash_inverted) =
            negotiate_suite(&their_suites, &our_suites, &their_pub, &our_pub).unwrap();
        assert_ne!(hash_1, hash_inverted);
    }

    /// Regression test for the suite 0x02 silent-redefinition bug: a node
    /// advertising the CURRENT suite set `[0x01, 0x03]` meeting a peer that
    /// only ever advertised the OLD suite set `[0x01, 0x02]` (i.e. a peer that
    /// predates the 0x03 suite and has no way to know it exists) must
    /// negotiate DOWN to 0x01 -- the one suite ID whose derivation never
    /// changed -- rather than colliding on a redefined 0x02 and silently
    /// deriving mismatched, non-interoperable root keys on each side.
    #[test]
    fn test_negotiate_suite_new_node_meets_old_peer_falls_back_to_v1() {
        let new_node_suites = [0x01, 0x03];
        let old_peer_suites = [0x01, 0x02];
        let our_pub = [0u8; 32];
        let their_pub = [1u8; 32];

        let (suite, _) =
            negotiate_suite(&new_node_suites, &old_peer_suites, &our_pub, &their_pub).unwrap();
        assert_eq!(
            suite, 0x01,
            "a node offering only 0x03 hybrid meeting a peer offering only 0x02 hybrid \
             must fall back to 0x01 (classical), never silently agree on a suite ID \
             whose derivation the two sides disagree about"
        );

        // Symmetric from the old peer's perspective too.
        let (suite_from_old_side, _) =
            negotiate_suite(&old_peer_suites, &new_node_suites, &their_pub, &our_pub).unwrap();
        assert_eq!(suite_from_old_side, 0x01);
    }

    /// Two current nodes (both advertising `[0x01, 0x03]`) negotiate the
    /// current hybrid suite, not the old one -- 0x02 is never chosen when
    /// both sides are up to date.
    #[test]
    fn test_negotiate_suite_two_current_nodes_pick_0x03() {
        let our_suites = [0x01, 0x03];
        let their_suites = [0x01, 0x03];
        let our_pub = [0u8; 32];
        let their_pub = [1u8; 32];

        let (suite, _) = negotiate_suite(&our_suites, &their_suites, &our_pub, &their_pub).unwrap();
        assert_eq!(suite, 0x03);
    }
}
