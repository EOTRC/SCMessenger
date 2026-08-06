# AWS Always-On Node -- CURRENT relay address

POLICY (operator directive 2026-08-04): IPs in this repo are ephemeral.
This file is the ONE place the orchestrator updates immediately after every
AWS node rebuild. Read it fresh at use time; never copy an IP from any
other doc, ticket, or config.

## Current (updated 2026-08-05, post PR-138 rollout rebuild)

- Public IP: PENDING-REBUILD (instance rebuild in progress; check back)
- Bootstrap multiaddr: /ip4/<IP above>/tcp/9001
- Health check: http://<IP above>:9876/health -> {"status":"healthy"}
- Instance tag: Name=scm-always-on-node (account 101533648751, us-east-1)
- Image: testbotz/scmessenger:latest at commit 6b2573fa (PR 136+137+138)

## Previous (STALE -- do not use)

- 34.203.213.35 (2026-08-04 rebuild, pre-PR-137 image)
- 54.242.56.150 (prior broken instance)
- 100.56.248.69 (original docs IP; obsolete)
