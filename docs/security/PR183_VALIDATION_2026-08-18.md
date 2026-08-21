# PR #183 Validation Report -- Android Wiring Restoration

**Date:** 2026-08-18
**Validator:** CRITICAL_VALIDATOR (Independent dual-pass review: Gemini 3.1 Pro High / CTO)
**Target:** PR #183 (`fix/android-restore-wiring` into `main`)
**Base:** `origin/main` (`b4ccd30a`)
**Commits:** `6fbb9e7e`, `47a11c9e`, `abed7b39`, `1076639e`, `3c85b7d6`
**Verdict:** **APPROVE_WITH_FINDINGS**

---

## 1. Subject

PR #183, "restore wiring for all nine wired-out Android features".
Commits: `6fbb9e7e`, `47a11c9e`, `abed7b39`, `1076639e`, `3c85b7d6`. Base: `origin/main` (`b4ccd30a`).

Restores call sites, a `NavHost` route, and a manifest registration that commit `ebf5411b` stripped. The underlying source files were never missing -- only their callers and registration points.

---

## 2. The Gate

Gate command: `python scripts/check_wiring.py`

- **Baseline on `origin/main`:** 32 findings (10 `C1_ZERO_CALLERS`, 1 `C2_UNREGISTERED_ROUTE`, 1 `C3_MANIFEST_MISSING`, 20 `C4_TRANSITIVE_DEAD`).
- **Post-PR #183:** Exit 0, **ZERO** findings (`[OK] All components, composables, routes, and utilities are correctly wired`).
- Both baseline and post-PR metrics were independently verified and reproduced by the CTO, not accepted solely from worker reports.

**Note for the record:** The audit document `ANDROID_WIRING_AUDIT_2026-08-18.md` listed `MeshVpnService` and `BootReceiver` as unregistered. They were NOT -- PR #176 had already restored them. Only `ShareReceiver` was genuinely missing from the manifest. The automated script was correct and the prose document was stale. `AndroidManifest.xml` went from 148 lines on main to 165 here, against 158 at `ebf5411b^`.

---

## 3. Pass 1 -- Gemini 3.1 Pro High

**Verdict:** **BLOCK** (Three HIGH findings)

1. **`MeshRepository.kt` (HIGH):** The encrypted-prefs migration ignored the `Boolean` returned by `.commit()`. If the encrypted write failed, the legacy plaintext passphrase was deleted anyway, permanently orphaning the user's encrypted backups.
2. **`MainViewModel.kt` (HIGH):** `handleDeepLink` dialed attacker-supplied addresses immediately upon parsing without user consent. `AndroidManifest.xml` declares the `scmessenger://` intent filter with `CATEGORY_BROWSABLE`, allowing any untrusted web page or application to trigger an outbound connection attempt. Because `DeepLinkValidator` deliberately permits same-subnet RFC1918 addresses (essential for local mesh operations), the connection target could be an internal host on the victim's private LAN.
3. **`ShareReceiver.kt` (HIGH):** Hardcoded user-facing strings, violating hook-enforced repository rules.

**Disposition:** The CTO verified all three findings against the source and ACCEPTED them. None was overridden.

---

## 4. Remediation -- Commit `1076639e`

1. **`MeshRepository.kt` (`resolvePlatformSecuredPassphrase`):** Captures `committed = encryptedPrefs.edit().putString(...).commit()`. It removes the legacy plaintext key only when `committed == true`. On `false`, it logs an error via `Timber.e` (never logging the secret value) and returns the legacy passphrase directly so application functionality is preserved without data loss.
2. **`MainViewModel.kt` & `MeshApp.kt`:** Outbound dialing was completely removed from `handleDeepLink` (`MainViewModel.kt:310-371`). Connection logic was moved to `confirmAndDialPendingDeepLink` (`MainViewModel.kt:372`), which is invoked from `MeshApp.kt:273` only after the user explicitly accepts a confirmation dialog displaying the peer ID / nickname and target multiaddresses. `DeepLinkValidator` was deliberately NOT restricted -- same-subnet dialing is expected mesh behavior; the invocation action itself was gated behind user consent.
3. **`ShareReceiver.kt` / `strings.xml`:** Extracted all user-facing strings to `android/app/src/main/res/values/strings.xml`.

---

## 5. Pass 2 -- Gemini 3.1 Pro High (Independent)

**Verdict:** **APPROVE_WITH_FINDINGS**
**Prior Block Cleared:** **YES**

All three Pass 1 findings were confirmed fixed. One NEW finding was identified:

- **`ShareReceiver.kt` (HIGH - Runtime Crash):** `Toast.makeText(...).show()` was invoked inside a coroutine running on `CoroutineScope(SupervisorJob() + Dispatchers.IO)`. While this compiled cleanly without warnings, Android throws a runtime exception (`java.lang.RuntimeException: Can't create handler inside thread that has not called Looper.prepare()`) when showing a Toast from a background worker thread. This defect was pre-existing in `ShareReceiver.kt`, but was unreachable while `ShareReceiver` was omitted from `AndroidManifest.xml`. Restoring the manifest registration in PR #183 made this code path live, which would have crashed the host process during share operations. (Neither Pass 1 nor the initial human review caught this).

**Remediation (Commit `3c85b7d6`):**
- In `ShareReceiver.kt:sendMessageToContact`, the two `Toast.makeText` invocations inside the coroutine were wrapped with `withContext(Dispatchers.Main)`. `repository.sendMessage` remains on `Dispatchers.IO`.
- Other `Toast` calls in `ShareReceiver.kt` execute directly on `BroadcastReceiver.onReceive` (the Android main thread) and were verified safe.

---

## 6. Gates Run by the CTO

The following gates were executed independently by the CTO on the host environment:

| Gate | Command | Result |
| :--- | :--- | :--- |
| **Wiring Gate** | `python scripts/check_wiring.py` | `[OK]` Exit 0, 0 findings |
| **Kotlin / Core Build** | `./gradlew :app:compileDebugKotlin` | `[OK]` BUILD SUCCESSFUL in 51m 53s, Exit 0 |

The Gradle compilation compiled the Rust core via `cargo-ndk` and resolved all newly referenced `R.string` identifiers, fully clearing the compilation risk flagged during Pass 2.

---

## 7. Open Architectural Item (Not Cleared -- Operator Decision Required)

- **`SecurityUtils.kt:26` KeyStore Recovery Path:**
  `SecurityUtils.getEncryptedSharedPreferences` catches initialization exceptions and executes `context.deleteSharedPreferences("scmessenger_secure_prefs")` before retrying KeyStore creation.
  On Android, KeyStore key invalidation is a standard occurrence following lock-screen credential changes or biometric updates. This catch-and-delete fallback silently destroys the stored preferences file, which now holds the backup passphrase migrated by `MeshRepository.kt`.
  This behavior is PRE-EXISTING and was deliberately untouched in PR #183 to maintain narrow PR scope. However, restoring the backup passphrase migration through `SecurityUtils` makes this fallback load-bearing for user data for the first time.
  **Recommendation:** Require an explicit operator architectural ruling and mitigation before the `v0.4.0` release tag.

---

## 8. Conclusion

- **PR #183 Verdict:** **APPROVE_WITH_FINDINGS**
- **Wiring Status:** Complete (0 dead components, 0 unregistered routes, 0 missing manifest components)
- **Security & Stability:** Pass 1 and Pass 2 findings remediated in `1076639e` and `3c85b7d6`.
