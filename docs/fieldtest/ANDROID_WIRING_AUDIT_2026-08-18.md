# Android Wiring Audit: Comprehensive Report

**Status:** Completed
**Audit Target:** `android/app/src/main/java` & `android/app/src/main/AndroidManifest.xml`
**Base Commit / Anchor:** `ebf5411b^` vs `HEAD`
**Rule Compliance:** AGENTS.md Rules 1, 13 (cite file:line & command), 15 (no truncation).

---

## Executive Summary

Commit `ebf5411b` stripped multiple Android source files and call sites. While commit `00e09d62` and follow-ups restored the deleted source files, multiple critical **call sites, navigation graph routes, UI entry points, and manifest registrations were NOT restored**.

This audit identified **9 distinct orphaned or disconnected features** in the Android codebase:
1. **Diagnostics & In-App Log Viewer** (`DiagnosticsScreen.kt` + `DiagnosticsBundleFormatter.kt` + `NetworkStatusDialog.kt`)
2. **Local QR / HTTP APK Sharing** (`ApkShareDialog.kt` + `ApkShareManager.kt`)
3. **QR Join-Mesh Protocol Flow** (`JoinMeshScreen.kt`)
4. **Mesh VPN Tunnel Service** (`MeshVpnService.kt`)
5. **Boot Completion Auto-Restart** (`BootReceiver.kt`)
6. **System Share Intent Handling** (`ShareReceiver.kt` & `MainActivity` intent filters)
7. **IronCore Summarized Logging Bridge** (`FileLoggingTree.setIronCore`)
8. **EncryptedSharedPreferences Integration** (`SecurityUtils.kt`)
9. **BLE Specific Backoff Strategy** (`BleBackoffStrategy.kt`)

---

## 1. Confirmed Regressions & Reproductions

### 1.1 `ApkShareDialog.kt` & `ApkShareManager.kt`
- **Implementation:** `android/app/src/main/java/com/scmessenger/android/ui/dialogs/ApkShareDialog.kt:36` (`fun ApkShareDialog`) and `android/app/src/main/java/com/scmessenger/android/utils/ApkShareManager.kt:28` (`object ApkShareManager`)
- **Command:** `git grep -rn "ApkShareDialog" android/app/src/main/java` -> exactly 1 hit (`ApkShareDialog.kt:36`).
- **Call site in `ebf5411b^` (`SettingsScreen.kt`):**
  - Line 79: `var showApkShareDialog by remember { mutableStateOf(false) }`
  - Line 255: `onShareApkClick = { showApkShareDialog = true }`
  - Line 447: `com.scmessenger.android.ui.dialogs.ApkShareDialog(onDismiss = { showApkShareDialog = false })`
  - Lines 1208-1216: Share APK button inside `AdvancedSettingsSection`
- **Call site in `HEAD`:** Completely absent from `SettingsScreen.kt`. `ApkShareManager` is only referenced from inside `ApkShareDialog.kt` (lines 46, 60, 68, 74, 101, 183).
- **Consequence:** User cannot share APK via local HTTP server or QR code. Embedded HTTP sideload server never starts.

### 1.2 `AndroidManifest.xml` Component Registrations
- **Implementations:**
  - `android/app/src/main/java/com/scmessenger/android/service/MeshVpnService.kt:21`
  - `android/app/src/main/java/com/scmessenger/android/service/BootReceiver.kt:23`
  - `android/app/src/main/java/com/scmessenger/android/utils/ShareReceiver.kt:31`
- **Command:** `git diff ebf5411b^ HEAD -- android/app/src/main/AndroidManifest.xml`
- **Missing Declarations:**
  - `<service android:name=".service.MeshVpnService" android:permission="android.permission.BIND_VPN_SERVICE">...`
  - `<receiver android:name=".service.BootReceiver" android:permission="android.permission.RECEIVE_BOOT_COMPLETED">...`
  - `<uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED" />`
  - `<uses-permission android:name="android.permission.ACCESS_FINE_LOCATION" />`
  - `<uses-permission android:name="android.permission.ACCESS_COARSE_LOCATION" />`
  - `MainActivity` `<intent-filter>` for `ACTION_SEND` (text/plain)
  - `MainActivity` `<intent-filter>` for `scmessenger://invite` and `scmessenger://add`
  - `<meta-data android:name="privacy_policy_url" android:value="@string/privacy_policy_url" />`
- **Consequence:** Service/receivers cannot be instantiated by Android OS. Background reboot startup fails; VPN cannot start; external sharing/deep links dead.

---

## 2. Comprehensive Inventory of Orphaned / Disconnected Items

### Item 1: Diagnostics Screen, Log Viewer & Network Status Dialog
- **Files & Lines:**
  - `android/app/src/main/java/com/scmessenger/android/ui/screens/DiagnosticsScreen.kt:41` (`fun DiagnosticsScreen`)
  - `android/app/src/main/java/com/scmessenger/android/ui/diagnostics/DiagnosticsBundleFormatter.kt:20` (`object DiagnosticsBundleFormatter`)
  - `android/app/src/main/java/com/scmessenger/android/ui/dialogs/NetworkStatusDialog.kt:44` (`fun NetworkStatusDialog`)
- **Evidence & Caller Graph:**
  - `DiagnosticsScreen.kt`: 0 callers in entire tree.
  - `DiagnosticsBundleFormatter.kt`: 0 external callers. `SettingsViewModel.kt:768` replaced formatter invocation with static string `"Paranoid Mode active: Telemetry and diagnostic exports disabled."`.
  - `NetworkStatusDialog.kt`: Called ONLY from `DiagnosticsScreen.kt:106` (dead-to-dead reference).
  - `MeshApp.kt:327`: `composable(Screen.Diagnostics.route)` was deleted in `ebf5411b` and never restored.
  - `SettingsScreen.kt:1200`: Diagnostics button in `AdvancedSettingsSection` was deleted in `ebf5411b`.
- **Consequence:** P0 diagnostics feature is completely unreachable. Attempting to navigate to `"diagnostics"` would throw `IllegalArgumentException` in `NavController`.

### Item 2: Join Mesh Screen (QR Bundle Discovery & Bootstrap)
- **Files & Lines:**
  - `android/app/src/main/java/com/scmessenger/android/ui/join/JoinMeshScreen.kt:49` (`fun JoinMeshScreen`)
  - Sub-composables: `QrScannerView` (:127), `ParsingView` (:195), `ConnectingView` (:210), `SuccessView` (:265), `ErrorView` (:296)
- **Evidence & Caller Graph:**
  - `JoinMeshScreen.kt`: 0 callers outside own file.
  - `MeshApp.kt`: Route is NOT declared in `Screen` sealed class, NOT registered in `MeshNavHost`.
  - No button or screen links to `JoinMeshScreen`.
- **Consequence:** 446 lines of join-mesh logic (scanning QR join bundle, parsing bootstrap peers/topics, dialing peers via FFI, topic subscription) are completely dead.

### Item 3: Mesh VPN Service Reactive Lifecycle
- **Files & Lines:**
  - `android/app/src/main/java/com/scmessenger/android/service/MeshVpnService.kt:21` (`class MeshVpnService`)
  - `android/app/src/main/java/com/scmessenger/android/service/MeshForegroundService.kt:265` & `:393`
- **Evidence & Caller Graph:**
  - In `ebf5411b^`, `MeshForegroundService.kt:268-285` collected `preferencesRepository.vpnModeEnabled` to call `startService(Intent(..., MeshVpnService::class.java))` with `ACTION_START` / `ACTION_STOP`.
  - In `ebf5411b^`, `MeshForegroundService.kt:396-400` stopped `MeshVpnService` on service shutdown.
  - In `HEAD`, both lifecycle blocks are missing.
- **Consequence:** Even if registered in `AndroidManifest.xml`, `MeshVpnService` is never started or stopped when the user toggles VPN settings.

### Item 4: IronCore Summarized Log Injection
- **Files & Lines:**
  - `android/app/src/main/java/com/scmessenger/android/utils/FileLoggingTree.kt:24` (`fun setIronCore`)
  - `android/app/src/main/java/com/scmessenger/android/data/MeshRepository.kt:1372`
- **Evidence & Caller Graph:**
  - In `ebf5411b^`, `MeshRepository.kt:1375-1380` iterated `Timber.forest()` to inject `ironCore` via `tree.setIronCore(ironCore)`.
  - In `HEAD`, this call was deleted. `FileLoggingTree.ironCore` remains `null` forever, falling back to raw file logging without core summarization.
- **Consequence:** Rust `IronCore` log recording is bypassed.

### Item 5: SecurityUtils Encrypted Storage Helper
- **Files & Lines:**
  - `android/app/src/main/java/com/scmessenger/android/utils/SecurityUtils.kt:16` (`object SecurityUtils`)
- **Evidence & Caller Graph:**
  - Created in `ebf5411b` to provide `EncryptedSharedPreferences`.
  - Has 0 callers across the repository. `MeshRepository.kt` and `PerformanceMonitor.kt` continue to use standard `Context.MODE_PRIVATE`.
- **Consequence:** Dead code / incomplete migration.

### Item 6: BleBackoffStrategy
- **Files & Lines:**
  - `android/app/src/main/java/com/scmessenger/android/transport/ble/BleBackoffStrategy.kt:16` (`class BleBackoffStrategy`)
- **Evidence & Caller Graph:**
  - 0 callers in codebase. `BleScanner.kt:14` imports `com.scmessenger.android.utils.BackoffStrategy` instead.
- **Consequence:** Unused duplicate class.

---

## 3. Per-Hunk Diff Analysis of Six Flagged Files

| File | Line Delta | Hunk Analysis & Verdict |
|---|---|---|
| `MeshForegroundService.kt` | -15 | **Call Site Removal:** Removed lines 268-285 (reactive `preferencesRepository.vpnModeEnabled` listener starting/stopping `MeshVpnService`) and lines 396-400 (shutdown stop for `MeshVpnService`). Target `MeshVpnService` exists. |
| `PerformanceMonitor.kt` | -17 | **Telemetry Suppression:** Lines 157-176 replaced `writeAnrEvent` file persistence with a no-op comment. In-memory monitoring and diagnostics getters still exist for `DiagnosticsScreen`. |
| `SettingsScreen.kt` | -19 | **Call Site Removal:** Removed `showApkShareDialog` state (:79), `onShareApkClick` (:255), `ApkShareDialog` invocation (:447), Share APK button (:1208), and Diagnostics button (:1220). Targets `ApkShareDialog` and `DiagnosticsScreen` exist. |
| `SettingsViewModel.kt` | -18 | **Call Site Removal:** Lines 770-788 replaced `buildTesterDiagnosticsBundle()` implementation using `DiagnosticsBundleFormatter` with a hardcoded static string. Target `DiagnosticsBundleFormatter` exists. |
| `MeshApplication.kt` | +9 | **Legitimate Growth:** Fully restored logging/crash infrastructure and added structured `BuildConfig` git metadata logging. |
| `android/build.gradle` | +113 | **Legitimate Growth:** Added `security-crypto` dependency, build metadata passing, KSP task ordering dependencies (`ksp*Kotlin dependsOn generateUniFFIBindings`), and UniFFI output file validation assertion. |

---

## 4. Navigation Graph Reachability Table

| Screen Route | Sealed Object | NavHost Registered? | Navigated From UI? | Reachability Status |
|---|---|---|---|---|
| `"conversations"` | `Screen.Conversations` | Yes (`MeshApp.kt:169`) | Bottom Nav | **REACHABLE** |
| `"contacts"` | `Screen.Contacts` | Yes (`MeshApp.kt:183`) | Bottom Nav | **REACHABLE** |
| `"add_contact"` | `Screen.AddContact` | Yes (`MeshApp.kt:235`) | Contacts FAB, Deep Links | **REACHABLE** |
| `"dashboard"` | `Screen.Dashboard` | Yes (`MeshApp.kt:250`) | Bottom Nav | **REACHABLE** |
| `"settings"` | `Screen.Settings` | Yes (`MeshApp.kt:280`) | Bottom Nav | **REACHABLE** |
| `"identity"` | `Screen.Identity` | Yes (`MeshApp.kt:318`) | Settings Screen (:284) | **REACHABLE** |
| `"diagnostics"` | `Screen.Diagnostics` | **NO (Deleted)** | Settings Screen (:287, dead button) | **DEAD / UNREACHABLE** |
| `"blocked_peers"` | `Screen.BlockedPeers` | Yes (`MeshApp.kt:329`) | Settings Screen (:290) | **REACHABLE** |
| `"requests_inbox"` | `Screen.RequestsInbox` | Yes (`MeshApp.kt:335`) | Notification Action (`MainActivity:158`) | **REACHABLE** |
| `"chat/{peerId}"` | Dynamic | Yes (`MeshApp.kt:342`) | Conversations, Contacts | **REACHABLE** |
| `"contact/{contactId}"` | Dynamic | Yes (`MeshApp.kt:201`) | ContactsScreen | **REACHABLE** |
| `"verify_safety_number/{contactId}"` | Dynamic | Yes (`MeshApp.kt:223`) | ContactDetailScreen | **REACHABLE** |
| `"peer_list"` | Dynamic | Yes (`MeshApp.kt:264`) | DashboardScreen | **REACHABLE** |
| `"topology"` | Dynamic | Yes (`MeshApp.kt:272`) | DashboardScreen | **REACHABLE** |
| `"mesh_settings"` | Dynamic | Yes (`MeshApp.kt:302`) | SettingsScreen | **REACHABLE** |
| `"power_settings"` | Dynamic | Yes (`MeshApp.kt:310`) | SettingsScreen | **REACHABLE** |
| `JoinMeshScreen` | None | **NO** | None | **DEAD / UNREACHABLE** |

---

## 5. Summary of Excluded Items

Per AGENTS.md and audit instructions:
- Previews annotated with `@Preview` were excluded from orphan detection.
- Test files under `android/app/src/test` and `android/app/src/androidTest` were excluded.
