# BLOCKER: the CI Android APK cannot upgrade an installed app in place -- signature mismatch

Status: BLOCKED, needs an operator/release decision
Severity: P1 release process (blocks the five-node gate at a matched SHA; also
implies real users could not upgrade without losing data)
Discovered: 2026-08-09, Windows lane, attempting the authorized in-place update

## What happened

The operator authorized an in-place Android update to anchor `7e527df0`,
explicitly preserving identity, contacts, history and `firstInstallTime`.
CI Mobile run `31364687397` went green at `7e527df0` and produced
`android-debug-apk` (468,321,439 bytes). Installing it:

```
adb install -r app-debug.apk
Performing Streamed Install
Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package
         com.scmessenger.android signatures do not match newer version;
         ignoring!]
```

**The device was not modified.** `-r` fails closed; no data was touched.

## Why this was NOT forced

Android only allows replacing an app with one signed by the same key. The only
way to install this APK is `adb uninstall` first, which **deletes the app's
private data**: identity keys, `contacts.db`, `history.db`, `ledger.json`, the
outbox, and `firstInstallTime` continuity.

That directly contradicts the operator's instruction and would destroy:
- the identity whose survival across upgrade is EVIDENCE in the current
  five-node question,
- 31 hours of CryptoError history under investigation,
- the 397-row ledger that confirms the render-filter root cause.

Not a judgement call the acting agent should make unilaterally. Escalated.

## What this means beyond the field test

If the installed build and CI builds are signed with different debug keys,
then **no device can move between a locally-built app and a CI-built app
without a data wipe**. Worth confirming before the tag: a release build signed
with a stable release keystore is the real fix, and if the intent is that
testers can update in place, the signing key must be consistent and managed.

The currently installed app is `versionCode=14`, `versionName=0.4.0`,
`firstInstallTime=2026-08-08 12:47:45`,
`lastUpdateTime=2026-08-09 16:00:26` -- so SOMETHING updated it in place on
08-09, meaning a compatible signing key exists somewhere. Identify what
produced that build; that is the key needed here.

## Options for the operator (none taken)

1. **Find the matching keystore** and rebuild `7e527df0` locally signed with
   it. Preserves everything. Requires knowing which key produced the 08-09
   update.
2. **Accept the mixed-SHA fleet** for this gate: Android stays at its current
   build, and the five-node run is scored with Android's SHA recorded as
   `versionCode=14 / 0.4.0` (exact commit unknown -- the installed build
   predates the `7e527df0` provenance stamp, so it cannot self-report).
3. **Wipe and reinstall**, accepting the loss of identity, contacts, history
   and all current diagnostic state. Only the operator can choose this, and
   the pre-upgrade snapshot at
   `C:\Users\SCM\Documents\SCM_fieldtest_snapshots\android_pre_upgrade_2026-08-09\`
   would become the sole record of what was lost.
4. **Defer the Android leg** and close the gate on the four legs already
   proven, noting Android as tested-at-a-different-SHA.

## Note on the evidence snapshot

Captured BEFORE the attempt, so it is intact either way: full `ledger.json`,
the 44,196-line mesh log, 103 MB logcat, `pending_outbox.json`, `history.db`,
`contacts.db`, `root/db`, and the `dumpsys` provenance baseline. Stored
outside the repo deliberately -- `batch_handoff.py` runs `git add -A`.
