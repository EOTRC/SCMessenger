package com.scmessenger.android.utils

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import timber.log.Timber
import java.io.File

/**
 * Utility for initializing Android KeyStore backed EncryptedSharedPreferences.
 * Ensures identity keys and sensitive device states are encrypted at rest using AES-256 GCM.
 *
 * Paranoid Mode Enforcement:
 * Unencrypted storage fallbacks (MODE_PRIVATE) are strictly prohibited to prevent secret leakage.
 *
 * Recovery Policy:
 * If EncryptedSharedPreferences fails to initialize (e.g., KeyStore invalidation after lock screen
 * or biometric changes, or device-to-device backup restore), the corrupted store is quarantined
 * to a timestamped backup file before deletion, a durable non-sensitive reset signal is recorded
 * in unencrypted preferences, and a clean KeyStore-backed store is initialized. If recovery fails,
 * a SecurityException is thrown (no unencrypted fallback permitted).
 */
object SecurityUtils {

    const val ENCRYPTED_PREFS_FILENAME = "scmessenger_secure_prefs"
    const val RESET_SIGNAL_PREFS = "scmessenger_security_state"
    const val KEY_SECURE_STORE_RESET_AT = "secure_store_reset_at"
    const val KEY_SECURE_STORE_RESET_COUNT = "secure_store_reset_count"
    const val KEY_SECURE_STORE_LAST_QUARANTINE_FILE = "secure_store_last_quarantine_file"

    fun getEncryptedSharedPreferences(context: Context): SharedPreferences {
        return getEncryptedSharedPreferencesInternal(
            context = context,
            timeProvider = { System.currentTimeMillis() },
            encryptedPrefsFactory = { ctx -> createEncryptedSharedPreferences(ctx) }
        )
    }

    internal fun getEncryptedSharedPreferencesInternal(
        context: Context,
        timeProvider: () -> Long,
        encryptedPrefsFactory: (Context) -> SharedPreferences
    ): SharedPreferences {
        return try {
            encryptedPrefsFactory(context)
        } catch (e: Exception) {
            val timestamp = timeProvider()
            val quarantineFileName = quarantineCorruptPreferences(context, timestamp)

            Timber.e(
                e,
                "Primary EncryptedSharedPreferences initialization failed; encrypted preferences could not be opened. " +
                "Previous store was quarantined to %s. Any data it held is unrecoverable without the original KeyStore key. " +
                "Attempting KeyStore reset recovery.",
                quarantineFileName ?: "none"
            )

            recordResetSignal(context, timestamp, quarantineFileName)

            try {
                // Recovery path: clear stale prefs file and retry KeyStore creation
                context.deleteSharedPreferences(ENCRYPTED_PREFS_FILENAME)
                encryptedPrefsFactory(context)
            } catch (recoveryException: Exception) {
                Timber.e(recoveryException, "Hardware KeyStore recovery failed")
                throw SecurityException(
                    "Hardware KeyStore initialization failed — unencrypted storage prohibited in Paranoid Mode",
                    recoveryException
                )
            }
        }
    }

    private fun createEncryptedSharedPreferences(context: Context): SharedPreferences {
        val masterKey = MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()

        return EncryptedSharedPreferences.create(
            context,
            ENCRYPTED_PREFS_FILENAME,
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
        )
    }

    private fun getSharedPrefsDir(context: Context): File {
        val dataDir = try {
            context.dataDir
        } catch (_: Throwable) {
            null
        } ?: try {
            context.filesDir?.parentFile
        } catch (_: Throwable) {
            null
        } ?: File(".")
        return File(dataDir, "shared_prefs")
    }

    private fun quarantineCorruptPreferences(context: Context, timestamp: Long): String? {
        return try {
            val prefsDir = getSharedPrefsDir(context)
            val originalFile = File(prefsDir, "$ENCRYPTED_PREFS_FILENAME.xml")
            if (!originalFile.exists() || originalFile.length() == 0L) {
                Timber.w("No existing encrypted preferences file found at %s to quarantine", originalFile.absolutePath)
                return null
            }

            val quarantineFileName = "${ENCRYPTED_PREFS_FILENAME}_corrupt_${timestamp}.xml"
            val quarantineFile = File(prefsDir, quarantineFileName)
            quarantineFile.parentFile?.mkdirs()
            originalFile.copyTo(quarantineFile, overwrite = true)

            val bakFile = File(prefsDir, "$ENCRYPTED_PREFS_FILENAME.bak")
            if (bakFile.exists() && bakFile.length() > 0L) {
                val quarantineBak = File(prefsDir, "${ENCRYPTED_PREFS_FILENAME}_corrupt_${timestamp}.bak")
                bakFile.copyTo(quarantineBak, overwrite = true)
            }

            Timber.i("Quarantined corrupt encrypted preferences to %s (%d bytes)", quarantineFile.absolutePath, quarantineFile.length())
            quarantineFileName
        } catch (e: Exception) {
            Timber.e(e, "Failed to quarantine corrupt encrypted preferences file")
            null
        }
    }

    private fun recordResetSignal(context: Context, timestamp: Long, quarantineFileName: String?) {
        try {
            val prefs = context.getSharedPreferences(RESET_SIGNAL_PREFS, Context.MODE_PRIVATE)
            val currentCount = prefs.getInt(KEY_SECURE_STORE_RESET_COUNT, 0)
            val editor = prefs.edit()
                .putLong(KEY_SECURE_STORE_RESET_AT, timestamp)
                .putInt(KEY_SECURE_STORE_RESET_COUNT, currentCount + 1)
            if (quarantineFileName != null) {
                editor.putString(KEY_SECURE_STORE_LAST_QUARANTINE_FILE, quarantineFileName)
            }
            editor.commit()
        } catch (e: Exception) {
            Timber.w(e, "Failed to record secure store reset signal")
        }
    }
}
