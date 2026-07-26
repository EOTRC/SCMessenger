//! Shared multiaddr validation: "is this address worth dialing, and is it
//! safe to dial or to disclose?"
//!
//! WHY THIS MODULE EXISTS (adversarial review F3, 2026-07-25): the CLI had
//! `is_dialable_multiaddr` / `is_self_address` in `cli/src/ledger.rs`, but core
//! had no equivalent. Core's ledger-seed import, its seed-dial candidate build
//! and its ledger-exchange response all accepted any string that merely
//! *parsed* as a `Multiaddr`. An attacker could therefore push
//! `/ip4/169.254.169.254/tcp/80` (cloud metadata), `/ip4/127.0.0.1/tcp/8080`
//! or arbitrary RFC1918 host:port pairs into a victim's dial set and read the
//! result off the dial-outcome timing (refused resolves in milliseconds,
//! filtered hangs to the sweep timeout) -- an SSRF/internal-port-scan oracle.
//!
//! The CLI now re-exports these functions so there is exactly ONE definition
//! of "dialable" in the workspace.
//!
//! This module is deliberately free of any I/O, any lock and any platform
//! `cfg` so it compiles identically on wasm32, Android and desktop.

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Network context for address filtering.
///
/// `Local` (WiFi/LAN/mesh) keeps private/LAN ranges dialable for local mesh
/// discovery; `Public` (cellular / public-only) additionally drops private
/// ranges since a public-only node cannot reach anyone's LAN.
///
/// Defaults to the conservative-for-connectivity `Local`, matching the CLI's
/// pre-existing behaviour. Do NOT hardcode `Public`: the entire BLE/WiFi-first
/// transport priority order depends on RFC1918 peers staying dialable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NetworkMode {
    #[default]
    Local,
    Public,
}

/// Whether a DNS-form multiaddr (`/dns/`, `/dns4/`, `/dns6/`, `/dnsaddr/`) may
/// be accepted.
///
/// WHY THIS EXISTS (re-review NEW-1, 2026-07-25): the first remediation pass
/// validated every `Ip4`/`Ip6` component and then set `has_transport = true`
/// for DNS components while validating NOTHING about them. A name resolves to
/// whatever its owner's zone says, so `/dns4/evil.example/tcp/80` skipped every
/// rule below: publish `A evil.example -> 169.254.169.254`, put that string in
/// a ledger-exchange entry or an invite `seed_ledger`, and the desktop swarm --
/// which wires a real resolver -- resolves and dials it. That restores the full
/// SSRF/internal-port-scan oracle F3 was filed for, and it is re-pointable per
/// probe (change the zone between dials) so it scans, not just hits one host.
///
/// Resolve-then-validate is NOT implemented here on purpose: this module is
/// I/O-free and `cfg`-free by contract (it compiles identically on wasm32,
/// Android and desktop), and a resolve-then-validate gate is a DNS-rebinding
/// TOCTOU anyway -- libp2p re-resolves at dial time and on every reconnect, so
/// a validated answer is not the answer that gets connected to.
///
/// So the rule is provenance-based: a name is only as trustworthy as whoever
/// supplied it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DnsPolicy {
    /// The address came from a remote peer (ledger exchange, invite
    /// `seed_ledger`, Identify `listen_addrs`, gossip) or is about to be
    /// disclosed to one. DNS forms are REJECTED.
    ///
    /// This is the [`Default`] so that a future call site which forgets to
    /// think about provenance fails closed.
    #[default]
    Reject,
    /// The address came from local configuration: an operator-supplied
    /// bootstrap list, a CLI flag, or an address this node itself connected to.
    /// DNS forms are allowed, because the operator chose the name.
    AllowLocallyConfigured,
}

/// Returns true iff `ip` is an IPv4 address a peer could legitimately be
/// reachable at. Rejects, unconditionally:
///
/// - loopback (127/8) -- SSRF into our own host
/// - unspecified (0.0.0.0) and the rest of 0/8 ("this network")
/// - link-local 169.254/16 -- includes the 169.254.169.254 cloud metadata
///   endpoint, the single highest-value SSRF target in existence
/// - multicast 224/4 and broadcast 255.255.255.255 -- not unicast peers;
///   dialing them is a local-segment amplification primitive
/// - 192.0.0.0/24 (IETF protocol assignments) -- mirrors the same carve-out
///   `swarm::is_discoverable_multiaddr` already makes for mobile/VPN internal
///   NAT addresses
///
/// RFC1918 private ranges are rejected only in [`NetworkMode::Public`].
fn is_dialable_ipv4(ip: &Ipv4Addr, mode: NetworkMode) -> bool {
    let o = ip.octets();
    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
    {
        return false;
    }
    // 0.0.0.0/8 "this network" -- only 0.0.0.0 itself is `is_unspecified`.
    if o[0] == 0 {
        return false;
    }
    // 192.0.0.0/24 IETF protocol assignments.
    if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        return false;
    }
    if mode == NetworkMode::Public && ip.is_private() {
        return false;
    }
    true
}

/// Returns true iff `ip` is an IPv6 address a peer could legitimately be
/// reachable at. Rejects loopback (`::1`), unspecified (`::`), multicast
/// (`ff00::/8`), link-local (`fe80::/10`) and site-local (`fec0::/10`)
/// unconditionally; unique-local (`fc00::/7`) is the IPv6 analogue of RFC1918
/// and is therefore gated on [`NetworkMode::Public`] exactly like RFC1918.
///
/// IPv4-mapped and IPv4-compatible forms (`::ffff:127.0.0.1`, `::127.0.0.1`)
/// are unwrapped and re-checked as IPv4 -- otherwise they are a trivial bypass
/// of every IPv4 rule above.
fn is_dialable_ipv6(ip: &Ipv6Addr, mode: NetworkMode) -> bool {
    if let Some(v4) = ip.to_ipv4() {
        // `to_ipv4` covers both ::a.b.c.d and ::ffff:a.b.c.d. `::` and `::1`
        // also map to 0.0.0.0 / 0.0.0.1, both of which is_dialable_ipv4
        // rejects via the 0/8 rule -- which is the answer we want anyway.
        return is_dialable_ipv4(&v4, mode);
    }
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    // std lacks stable helpers for these on the pinned toolchain, so check the
    // top bits of the first 16-bit segment directly.
    let seg0 = ip.segments()[0];
    if seg0 & 0xffc0 == 0xfe80 || seg0 & 0xffc0 == 0xfec0 {
        return false;
    }
    if mode == NetworkMode::Public && (seg0 & 0xfe00) == 0xfc00 {
        return false;
    }
    true
}

/// Returns true iff `addr` is worth dialing / safe to disclose.
///
/// Ordering matters and is load-bearing: components are examined in wire
/// order, so for a circuit address such as
/// `/ip4/R/tcp/443/p2p/QmRelay/p2p-circuit/p2p/QmTarget` the RELAY hop
/// (`/ip4/R/tcp/443`) is fully validated, and everything after the
/// `/p2p-circuit` marker is accepted unconditionally -- a relayed peer's own
/// address is not something we dial and not something we can reason about.
/// This reproduces the CLI's short-circuit semantics exactly while closing the
/// "relay hop is loopback" hole implicitly.
///
/// An address with no transport component at all (`""`, `/p2p/QmX`,
/// `/p2p-circuit`) is REJECTED. Note `"".parse::<Multiaddr>()` returns
/// `Ok(<empty>)`, so "it parsed" is not evidence of anything (review F9).
///
/// DNS-form components are governed by `dns`; see [`DnsPolicy`] for why a name
/// supplied by a remote peer is not validatable at all. Note the DNS check runs
/// BEFORE the `P2pCircuit` short-circuit can fire, so
/// `/dns4/evil.example/tcp/80/p2p-circuit` -- whose relay hop we would really
/// dial -- is rejected too.
pub fn is_dialable_multiaddr_parsed(addr: &Multiaddr, mode: NetworkMode, dns: DnsPolicy) -> bool {
    let mut has_transport = false;

    for proto in addr.iter() {
        match proto {
            // Everything beyond the relay hop belongs to the relayed peer.
            Protocol::P2pCircuit => return has_transport,
            Protocol::Ip4(ip) => {
                has_transport = true;
                if !is_dialable_ipv4(&ip, mode) {
                    return false;
                }
            }
            Protocol::Ip6(ip) => {
                has_transport = true;
                if !is_dialable_ipv6(&ip, mode) {
                    return false;
                }
            }
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_) => {
                if dns == DnsPolicy::Reject {
                    return false;
                }
                has_transport = true;
            }
            _ => {}
        }
    }

    has_transport
}

/// String convenience wrapper over [`is_dialable_multiaddr_parsed`].
///
/// An unparseable string is not dialable. (The CLI's previous implementation
/// split on `/` and returned `true` for garbage that happened to contain no
/// recognised IP component; that is now rejected.)
pub fn is_dialable_multiaddr(multiaddr: &str, mode: NetworkMode, dns: DnsPolicy) -> bool {
    match multiaddr.parse::<Multiaddr>() {
        Ok(addr) => is_dialable_multiaddr_parsed(&addr, mode, dns),
        Err(_) => false,
    }
}

/// Returns true iff `addr` is safe to HAND TO SOMEONE ELSE -- in a
/// `/sc/ledger-exchange/1.0.0` reply, or baked into an invite QR.
///
/// DISCLOSURE IS NOT DIALABILITY (re-review NEW-2). The first remediation pass
/// reused the dial predicate for the exchange reply and passed
/// `NetworkMode::Local` at the call site, because `Local` is what keeps the
/// LAN/mesh transport priority order working. But `Local` deliberately skips
/// the `is_private()` check, and `record_connection` is deliberately unfiltered,
/// so every LAN peer we ever dialed became a *proven, disclosable* record:
/// internal subnet, live host:port, and each neighbour's `last_peer_id`. That
/// is an internal network map handed to any peer that completed a Noise
/// handshake.
///
/// The two predicates answer different questions and must not share a mode:
/// - "can I reach it?" is contextual -- an RFC1918 peer on my own LAN is
///   perfectly reachable, which is why the dial path keeps `Local`;
/// - "may I tell a stranger about it?" is not -- an address the recipient
///   cannot route to is, by construction, only useful to them as
///   reconnaissance about us.
///
/// So this function takes NO `NetworkMode` parameter. There is no argument a
/// call site can pass to weaken it, which is the point: the previous bug was
/// exactly a call site passing the wrong mode.
///
/// DNS is rejected for a second, independent reason: a name like
/// `/dns4/nas.corp.internal/tcp/443` leaks internal naming even when it does
/// not resolve for the recipient.
pub fn is_disclosable_multiaddr_parsed(addr: &Multiaddr) -> bool {
    is_dialable_multiaddr_parsed(addr, NetworkMode::Public, DnsPolicy::Reject)
}

/// String convenience wrapper over [`is_disclosable_multiaddr_parsed`].
pub fn is_disclosable_multiaddr(multiaddr: &str) -> bool {
    match multiaddr.parse::<Multiaddr>() {
        Ok(addr) => is_disclosable_multiaddr_parsed(&addr),
        Err(_) => false,
    }
}

/// Remove the peer-id component(s) that identify the *endpoint* of a
/// multiaddr, leaving the transport path.
///
/// CIRCUIT CORRECTNESS (review F8): a naive `find("/p2p/")` truncation turns
/// `/ip4/A/tcp/443/p2p/QmRelay/p2p-circuit/p2p/QmTarget` into
/// `/ip4/A/tcp/443` -- the RELAY's address. Paired with a ledger entry whose
/// `last_peer_id` is still QmTarget, that produces a wire record asserting
/// "QmTarget is directly reachable at the relay's IP:port", which recipients
/// feed straight into `kademlia.add_address()`. That is DHT poisoning plus a
/// distributed dial amplifier aimed at an arbitrary host, and it happens with
/// no attacker present, from honest circuit entries.
///
/// So: keep everything up to and including the LAST `/p2p-circuit`, and strip
/// `/p2p/` components only after it (or everywhere, if there is no circuit).
/// The relay's own peer id is part of the *address* -- it is required to dial
/// the circuit -- and must survive.
pub fn strip_peer_id_multiaddr(addr: &Multiaddr) -> Multiaddr {
    let protocols: Vec<Protocol> = addr.iter().collect();
    let last_circuit = protocols
        .iter()
        .rposition(|p| matches!(p, Protocol::P2pCircuit));

    let mut out = Multiaddr::empty();
    for (idx, proto) in protocols.into_iter().enumerate() {
        let after_circuit = match last_circuit {
            Some(circuit_idx) => idx > circuit_idx,
            None => true,
        };
        if after_circuit && matches!(proto, Protocol::P2p(_)) {
            continue;
        }
        out.push(proto);
    }
    out
}

/// String convenience wrapper over [`strip_peer_id_multiaddr`].
///
/// Unparseable input is returned unchanged: this function's job is
/// normalisation, not validation. Callers must still run
/// [`is_dialable_multiaddr`] afterwards, which rejects it.
pub fn strip_peer_id(multiaddr: &str) -> String {
    match multiaddr.parse::<Multiaddr>() {
        Ok(addr) => strip_peer_id_multiaddr(&addr).to_string(),
        Err(_) => multiaddr.to_string(),
    }
}

/// Returns true iff `candidate` is one of this node's own known addresses
/// (listen or external) -- i.e. dialing it would be a self-dial.
///
/// Compares the transport address only (peer-id components stripped on both
/// sides), since the same node can be observed with or without its own peer id
/// attached depending on which ledger entry produced it.
pub fn is_self_address(candidate: &str, my_addrs: &[String]) -> bool {
    let stripped_candidate = strip_peer_id(candidate);
    if stripped_candidate.is_empty() {
        return false;
    }
    my_addrs
        .iter()
        .any(|a| strip_peer_id(a) == stripped_candidate)
}

/// Combined gate used by every core call site that turns remote-supplied
/// address data into a dial candidate, a stored ledger entry, or a disclosed
/// wire record: syntactically valid, routable under `mode`, and not us.
pub fn is_acceptable_peer_address(
    candidate: &str,
    mode: NetworkMode,
    dns: DnsPolicy,
    my_addrs: &[String],
) -> bool {
    is_dialable_multiaddr(candidate, mode, dns) && !is_self_address(candidate, my_addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: NetworkMode = NetworkMode::Local;
    const PUBLIC: NetworkMode = NetworkMode::Public;
    /// Provenance of every address in the tests below unless stated otherwise:
    /// a peer told us. That is the case that matters.
    const REMOTE: DnsPolicy = DnsPolicy::Reject;
    const CONFIGURED: DnsPolicy = DnsPolicy::AllowLocallyConfigured;

    #[test]
    fn rejects_non_routable_ipv4_in_every_mode() {
        for mode in [LOCAL, PUBLIC] {
            assert!(!is_dialable_multiaddr(
                "/ip4/127.0.0.1/tcp/8080",
                mode,
                REMOTE
            ));
            assert!(!is_dialable_multiaddr(
                "/ip4/0.0.0.0/tcp/9001",
                mode,
                REMOTE
            ));
            assert!(!is_dialable_multiaddr(
                "/ip4/0.1.2.3/tcp/9001",
                mode,
                REMOTE
            ));
            // Cloud metadata service -- the marquee SSRF target.
            assert!(!is_dialable_multiaddr(
                "/ip4/169.254.169.254/tcp/80",
                mode,
                REMOTE
            ));
            assert!(!is_dialable_multiaddr(
                "/ip4/224.0.0.1/udp/9001/quic-v1",
                mode,
                REMOTE
            ));
            assert!(!is_dialable_multiaddr(
                "/ip4/255.255.255.255/tcp/9001",
                mode,
                REMOTE
            ));
            assert!(!is_dialable_multiaddr(
                "/ip4/192.0.0.8/tcp/9001",
                mode,
                REMOTE
            ));
        }
    }

    #[test]
    fn rejects_non_routable_ipv6_in_every_mode() {
        for mode in [LOCAL, PUBLIC] {
            assert!(!is_dialable_multiaddr("/ip6/::1/tcp/9001", mode, REMOTE));
            assert!(!is_dialable_multiaddr("/ip6/::/tcp/9001", mode, REMOTE));
            assert!(!is_dialable_multiaddr(
                "/ip6/fe80::1897:a8ff:fec5:3d16/tcp/443",
                mode,
                REMOTE
            ));
            assert!(!is_dialable_multiaddr(
                "/ip6/fec0::1/tcp/9001",
                mode,
                REMOTE
            ));
            assert!(!is_dialable_multiaddr(
                "/ip6/ff02::1/tcp/9001",
                mode,
                REMOTE
            ));
        }
    }

    #[test]
    fn ipv4_mapped_ipv6_cannot_bypass_the_ipv4_rules() {
        // Without the to_ipv4() unwrap these all sail through as "some global
        // v6 address".
        assert!(!is_dialable_multiaddr(
            "/ip6/::ffff:127.0.0.1/tcp/8080",
            LOCAL,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr(
            "/ip6/::ffff:169.254.169.254/tcp/80",
            LOCAL,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr(
            "/ip6/::ffff:192.168.1.1/tcp/443",
            PUBLIC,
            REMOTE
        ));
    }

    #[test]
    fn private_ranges_follow_network_mode() {
        assert!(is_dialable_multiaddr(
            "/ip4/10.0.2.16/tcp/9001",
            LOCAL,
            REMOTE
        ));
        assert!(is_dialable_multiaddr(
            "/ip4/192.168.1.5/tcp/9001",
            LOCAL,
            REMOTE
        ));
        assert!(is_dialable_multiaddr(
            "/ip4/172.16.4.4/tcp/9001",
            LOCAL,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr(
            "/ip4/10.0.2.16/tcp/9001",
            PUBLIC,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr(
            "/ip4/192.168.1.5/tcp/9001",
            PUBLIC,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr(
            "/ip4/172.16.4.4/tcp/9001",
            PUBLIC,
            REMOTE
        ));
        // IPv6 unique-local is the RFC1918 analogue.
        assert!(is_dialable_multiaddr(
            "/ip6/fd00::1/tcp/9001",
            LOCAL,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr(
            "/ip6/fd00::1/tcp/9001",
            PUBLIC,
            REMOTE
        ));
    }

    #[test]
    fn accepts_globally_routable_addresses() {
        assert!(is_dialable_multiaddr(
            "/ip4/1.2.3.4/tcp/9001",
            LOCAL,
            REMOTE
        ));
        assert!(is_dialable_multiaddr(
            "/ip4/198.51.100.11/tcp/9000",
            PUBLIC,
            REMOTE
        ));
        assert!(is_dialable_multiaddr(
            "/ip6/2606:4700:4700::1111/tcp/9001",
            LOCAL,
            REMOTE
        ));
        // A name is fine when the OPERATOR chose it.
        assert!(is_dialable_multiaddr(
            "/dns4/relay.example/tcp/443",
            PUBLIC,
            CONFIGURED
        ));
    }

    // ------------------------------------------------------------------
    // NEW-1 -- DNS bypasses all address validation
    // ------------------------------------------------------------------

    /// The module previously had exactly ONE DNS assertion and it was positive,
    /// which is precisely why the bypass survived a full adversarial review.
    ///
    /// Every one of these strings sets `has_transport = true` and validates
    /// nothing under the old code, so every IPv4/IPv6 rule above is skipped and
    /// the desktop resolver dials whatever the zone says.
    #[test]
    fn remote_supplied_dns_is_rejected_in_every_form_and_mode() {
        let dns_forms = [
            "/dns4/evil.example/tcp/80",
            "/dns6/evil.example/tcp/80",
            "/dns/evil.example/tcp/80",
            "/dnsaddr/evil.example",
            "/dnsaddr/evil.example/tcp/443",
            "/dns4/evil.example/udp/9001/quic-v1",
            "/dns4/metadata.google.internal/tcp/80",
        ];
        for mode in [LOCAL, PUBLIC] {
            for addr in dns_forms {
                assert!(
                    !is_dialable_multiaddr(addr, mode, REMOTE),
                    "{addr} was accepted from a remote peer in {mode:?}: \
                     `A evil.example -> 169.254.169.254` is now a dial target"
                );
            }
        }
    }

    /// A DNS relay hop must not be laundered through the `/p2p-circuit`
    /// short-circuit: the hop is the part we actually connect a socket to.
    #[test]
    fn remote_supplied_dns_cannot_hide_behind_a_circuit_marker() {
        assert!(!is_dialable_multiaddr(
            "/dns4/evil.example/tcp/443/p2p-circuit",
            LOCAL,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr(
            "/dns4/evil.example/tcp/443/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN/p2p-circuit/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay",
            LOCAL,
            REMOTE
        ));
        // ...and the mixed form, where a legitimate-looking IP hop is followed
        // by a name.
        assert!(!is_dialable_multiaddr(
            "/ip4/1.2.3.4/tcp/443/dns4/evil.example/tcp/80",
            LOCAL,
            REMOTE
        ));
    }

    /// The rejection is provenance-based, not a blanket ban: an operator's own
    /// bootstrap relay name still works, which is what keeps the internet-relay
    /// tier of the transport priority order alive.
    #[test]
    fn locally_configured_dns_still_works() {
        assert!(is_dialable_multiaddr(
            "/dns4/relay.example/tcp/443",
            LOCAL,
            CONFIGURED
        ));
        assert!(is_dialable_multiaddr(
            "/dnsaddr/bootstrap.example",
            PUBLIC,
            CONFIGURED
        ));
        assert!(is_dialable_multiaddr(
            "/dns4/relay.example/tcp/443/p2p-circuit",
            PUBLIC,
            CONFIGURED
        ));
    }

    /// Fail-closed: a call site that forgets to think about provenance gets the
    /// strict answer.
    #[test]
    fn dns_policy_defaults_to_reject() {
        assert_eq!(DnsPolicy::default(), DnsPolicy::Reject);
        assert!(!is_dialable_multiaddr(
            "/dns4/evil.example/tcp/80",
            LOCAL,
            DnsPolicy::default()
        ));
    }

    // ------------------------------------------------------------------
    // NEW-2 -- disclosure is not dialability
    // ------------------------------------------------------------------

    /// The exchange reply used to run the dial predicate in
    /// `NetworkMode::Local`, which skips `is_private()` entirely. Every LAN peer
    /// we had ever dialed was therefore a disclosable record: internal subnet,
    /// live host:port, neighbour peer id.
    #[test]
    fn disclosure_always_drops_private_ranges() {
        for addr in [
            "/ip4/192.168.1.5/tcp/9001",
            "/ip4/10.0.2.16/tcp/9001",
            "/ip4/172.16.4.4/tcp/9001",
            "/ip6/fd00::1/tcp/9001",
            "/ip6/::ffff:192.168.1.1/tcp/443",
        ] {
            assert!(
                !is_disclosable_multiaddr(addr),
                "{addr} would be handed to any peer that completed a handshake"
            );
            // ...even though it is legitimately DIALABLE on our own LAN. This
            // pair of assertions is the whole point of the two predicates.
            assert!(is_dialable_multiaddr(addr, LOCAL, REMOTE));
        }
    }

    /// Everything the dial predicate rejects unconditionally is also
    /// undisclosable, and an internal hostname is not disclosable either.
    #[test]
    fn disclosure_drops_loopback_link_local_and_dns() {
        for addr in [
            "/ip4/127.0.0.1/tcp/8080",
            "/ip6/::1/tcp/8080",
            "/ip4/169.254.169.254/tcp/80",
            "/ip4/0.0.0.0/tcp/9001",
            "/ip4/224.0.0.1/tcp/9001",
            "/ip4/255.255.255.255/tcp/9001",
            "/dns4/nas.corp.internal/tcp/443",
            "/dnsaddr/vpn.corp.internal",
            "",
            "/p2p-circuit",
            "not-a-multiaddr",
        ] {
            assert!(!is_disclosable_multiaddr(addr), "{addr} was disclosable");
        }
    }

    #[test]
    fn disclosure_keeps_globally_routable_addresses() {
        assert!(is_disclosable_multiaddr("/ip4/198.51.100.11/tcp/9001"));
        assert!(is_disclosable_multiaddr(
            "/ip6/2606:4700:4700::1111/tcp/443"
        ));
        assert!(is_disclosable_multiaddr(
            "/ip4/203.0.113.9/tcp/443/p2p-circuit"
        ));
    }

    #[test]
    fn circuit_validates_the_relay_hop_and_allows_the_target() {
        // Routable relay hop -- allowed (CLI parity).
        assert!(is_dialable_multiaddr(
            "/ip4/1.2.3.4/tcp/9001/p2p-circuit",
            LOCAL,
            REMOTE
        ));
        assert!(is_dialable_multiaddr(
            "/ip4/1.2.3.4/tcp/443/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN/p2p-circuit/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay",
            LOCAL,
            REMOTE
        ));
        // Loopback relay hop -- rejected, because the hop is validated before
        // the circuit marker short-circuits.
        assert!(!is_dialable_multiaddr(
            "/ip4/127.0.0.1/tcp/443/p2p-circuit/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay",
            LOCAL,
            REMOTE
        ));
    }

    #[test]
    fn rejects_addresses_with_no_transport_component() {
        // F9: "" parses as Ok(<empty>).
        assert!("".parse::<Multiaddr>().is_ok());
        assert!(!is_dialable_multiaddr("", LOCAL, REMOTE));
        assert!(!is_dialable_multiaddr(
            "/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay",
            LOCAL,
            REMOTE
        ));
        assert!(!is_dialable_multiaddr("/p2p-circuit", LOCAL, REMOTE));
        assert!(!is_dialable_multiaddr("not-a-multiaddr", LOCAL, REMOTE));
    }

    #[test]
    fn strip_peer_id_keeps_plain_address() {
        assert_eq!(
            strip_peer_id(
                "/ip4/1.2.3.4/tcp/9001/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay"
            ),
            "/ip4/1.2.3.4/tcp/9001"
        );
        assert_eq!(
            strip_peer_id("/ip4/1.2.3.4/tcp/9001"),
            "/ip4/1.2.3.4/tcp/9001"
        );
    }

    #[test]
    fn strip_peer_id_does_not_collapse_circuit_to_relay_address() {
        // F8 regression: the naive find("/p2p/") implementation returned
        // "/ip4/1.2.3.4/tcp/443" here.
        let circuit = "/ip4/1.2.3.4/tcp/443/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN/p2p-circuit/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay";
        let stripped = strip_peer_id(circuit);
        assert_eq!(
            stripped,
            "/ip4/1.2.3.4/tcp/443/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN/p2p-circuit"
        );
        assert!(
            stripped.contains("/p2p-circuit"),
            "circuit marker must survive stripping"
        );
        assert!(
            !stripped.contains("12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay"),
            "target peer id must not survive stripping"
        );
    }

    #[test]
    fn strip_peer_id_of_bare_p2p_is_empty() {
        assert_eq!(
            strip_peer_id("/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay"),
            ""
        );
    }

    #[test]
    fn self_address_matches_regardless_of_peer_id_placement() {
        let my_addrs = vec![
            "/ip4/192.168.0.121/tcp/9001".to_string(),
            "/ip4/1.2.3.4/tcp/9001/p2p/12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay"
                .to_string(),
        ];
        assert!(is_self_address("/ip4/192.168.0.121/tcp/9001", &my_addrs));
        assert!(is_self_address(
            "/ip4/192.168.0.121/tcp/9001/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN",
            &my_addrs
        ));
        assert!(is_self_address("/ip4/1.2.3.4/tcp/9001", &my_addrs));
        assert!(!is_self_address("/ip4/10.0.2.16/tcp/9001", &my_addrs));
        // An empty candidate must never "match" an empty own-address entry.
        assert!(!is_self_address("", &["".to_string()]));
    }

    #[test]
    fn acceptable_peer_address_combines_both_gates() {
        let my_addrs = vec!["/ip4/1.2.3.4/tcp/9001".to_string()];
        assert!(!is_acceptable_peer_address(
            "/ip4/1.2.3.4/tcp/9001",
            LOCAL,
            REMOTE,
            &my_addrs
        ));
        assert!(!is_acceptable_peer_address(
            "/ip4/127.0.0.1/tcp/9001",
            LOCAL,
            REMOTE,
            &my_addrs
        ));
        assert!(!is_acceptable_peer_address(
            "/dns4/evil.example/tcp/80",
            LOCAL,
            REMOTE,
            &my_addrs
        ));
        assert!(is_acceptable_peer_address(
            "/ip4/5.6.7.8/tcp/9001",
            LOCAL,
            REMOTE,
            &my_addrs
        ));
    }
}
