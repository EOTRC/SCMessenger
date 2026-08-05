# Ledger Sharing -- Android Node Missing Fleet Nodes iOS Sees

Status: OPEN
Last updated: 2026-08-05
Priority: HIGH (field-observed parity gap)
Class: AUDIT-GATE (core/src/{routing,transport} + app sync path; security
review per AGENTS.md rule 8 before any fix lands)

## Observation (operator, 2026-08-05, live home-network test)

Fresh Android build (APK from Mobile run 30985808228, code 50d20011) paired
with the iOS node (Christy) and exchanged messages BIDIRECTIONALLY -- but:

- iOS (Christy) node listing: 4 nodes + 1 headless visible
- Android node listing: only Christy visible

Same network, same physical test. Per architecture doctrine, discovery is
LEDGER SHARING between nodes -- every node should converge on the same fleet
view (invite/QR-seeded, gossip-propagated). The Android node did not.

## Hypotheses to rule out (in order)

1. Android ledger-sync path does not pull/push the full ledger on mesh
   connect (sync only on specific events?)
2. Android UI filters ledger entries (e.g., drops headless or foreign rows)
3. Gossip propagation only flows on new-contact events, not full-ledger
   reconciliation on connect
4. Identity canonicalization (PR #136) drops foreign nodes during Android
   ledger ingest

## Acceptance criteria

- Both devices converge on the same node set (4 nodes + 1 headless) within a
  bounded gossip convergence window on the same network
- Restarting either app re-converges the fleet view without re-pairing
- Regression coverage: fleet-view parity assertion in an e2e/mesh test
