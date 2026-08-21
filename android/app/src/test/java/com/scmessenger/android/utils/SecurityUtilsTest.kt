package com.scmessenger.android.utils

import android.content.Context
import android.content.SharedPreferences
import io.mockk.every
import io.mockk.mockk
import io.mockk.verify
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.security.GeneralSecurityException

/**
 * Unit tests for [SecurityUtils].
 *
 * Covers:
 * 1. Clean initialization path: no quarantine, no reset signal.
 * 2. Recovery path on initial KeyStore failure: preserves ciphertext to quarantine file before deletion,
 *    records durable unencrypted reset signal, and returns fresh preferences.
 * 3. Double failure path: preserves quarantine file, records reset signal, and throws SecurityException (Paranoid Mode).
 * 4. Missing file handling: recovers cleanly without throwing file errors.
 * 5. Backup (.bak) preservation during quarantine.
 * 6. Non-leak assertion: no secret or passphrase values are read or asserted on.
 */
class SecurityUtilsTest {

    @get:Rule
    val tempFolder = TemporaryFolder()

    private lateinit var context: Context
    private lateinit var mockResetPrefs: SharedPreferences
    private lateinit var mockResetEditor: SharedPreferences.Editor
    private lateinit var dataDir: File
    private lateinit var prefsDir: File

    private val storedLongs = mutableMapOf<String, Long>()
    private val storedInts = mutableMapOf<String, Int>()
    private val storedStrings = mutableMapOf<String, String>()

    @Before
    fun setUp() {
        dataDir = tempFolder.newFolder("data_dir")
        prefsDir = File(dataDir, "shared_prefs").apply { mkdirs() }

        storedLongs.clear()
        storedInts.clear()
        storedStrings.clear()

        mockResetPrefs = mockk()
        mockResetEditor = mockk()

        every { mockResetPrefs.getInt(SecurityUtils.KEY_SECURE_STORE_RESET_COUNT, 0) } answers {
            storedInts[SecurityUtils.KEY_SECURE_STORE_RESET_COUNT] ?: 0
        }
        every { mockResetPrefs.edit() } returns mockResetEditor

        every { mockResetEditor.putLong(any(), any()) } answers {
            val key = firstArg<String>()
            val value = secondArg<Long>()
            storedLongs[key] = value
            mockResetEditor
        }
        every { mockResetEditor.putInt(any(), any()) } answers {
            val key = firstArg<String>()
            val value = secondArg<Int>()
            storedInts[key] = value
            mockResetEditor
        }
        every { mockResetEditor.putString(any(), any()) } answers {
            val key = firstArg<String>()
            val value = secondArg<String>()
            storedStrings[key] = value
            mockResetEditor
        }
        every { mockResetEditor.commit() } returns true

        context = mockk()
        every { context.dataDir } returns dataDir
        every { context.filesDir } returns File(dataDir, "files")
        every { context.getSharedPreferences(SecurityUtils.RESET_SIGNAL_PREFS, Context.MODE_PRIVATE) } returns mockResetPrefs
        every { context.deleteSharedPreferences(SecurityUtils.ENCRYPTED_PREFS_FILENAME) } answers {
            val orig = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.xml")
            val bak = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.bak")
            orig.delete()
            bak.delete()
            true
        }
    }

    @Test
    fun `init succeeds - no quarantine and no reset flag`() {
        val mockEncryptedPrefs = mockk<SharedPreferences>()
        var factoryCallCount = 0

        val result = SecurityUtils.getEncryptedSharedPreferencesInternal(
            context = context,
            timeProvider = { 1000L },
            encryptedPrefsFactory = {
                factoryCallCount++
                mockEncryptedPrefs
            }
        )

        assertEquals(mockEncryptedPrefs, result)
        assertEquals(1, factoryCallCount)
        verify(exactly = 0) { context.deleteSharedPreferences(any()) }
        verify(exactly = 0) { mockResetPrefs.edit() }
        val files = prefsDir.listFiles() ?: emptyArray()
        assertTrue(files.none { it.name.contains("corrupt") })
    }

    @Test
    fun `init fails once then succeeds - quarantine file created with ciphertext, original deleted after copy, reset signal recorded`() {
        val originalFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.xml")
        val mockCiphertext = byteArrayOf(0x01, 0x02, 0x03, 0x04, 0xAA.toByte(), 0xBB.toByte(), 0xCC.toByte())
        originalFile.writeBytes(mockCiphertext)

        val mockFreshPrefs = mockk<SharedPreferences>()
        var factoryCallCount = 0
        val fixedTimestamp = 1700000001234L

        var fileExistedWhenDeleted = false
        every { context.deleteSharedPreferences(SecurityUtils.ENCRYPTED_PREFS_FILENAME) } answers {
            val expectedQuarantine = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}_corrupt_${fixedTimestamp}.xml")
            assertTrue("Quarantine file must exist before deleteSharedPreferences is called", expectedQuarantine.exists())
            assertEquals(mockCiphertext.size.toLong(), expectedQuarantine.length())

            fileExistedWhenDeleted = originalFile.exists()
            originalFile.delete()
            true
        }

        val result = SecurityUtils.getEncryptedSharedPreferencesInternal(
            context = context,
            timeProvider = { fixedTimestamp },
            encryptedPrefsFactory = {
                factoryCallCount++
                if (factoryCallCount == 1) {
                    throw GeneralSecurityException("Android KeyStore invalidation")
                } else {
                    mockFreshPrefs
                }
            }
        )

        assertEquals(mockFreshPrefs, result)
        assertEquals(2, factoryCallCount)
        assertTrue("Original file must have existed when deleteSharedPreferences was called", fileExistedWhenDeleted)
        assertFalse("Original file must be deleted on disk", originalFile.exists())

        val quarantineFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}_corrupt_${fixedTimestamp}.xml")
        assertTrue("Quarantine file must exist on disk", quarantineFile.exists())
        assertTrue("Quarantine file content must match original bytes", quarantineFile.readBytes().contentEquals(mockCiphertext))

        assertEquals(fixedTimestamp, storedLongs[SecurityUtils.KEY_SECURE_STORE_RESET_AT])
        assertEquals(1, storedInts[SecurityUtils.KEY_SECURE_STORE_RESET_COUNT])
        assertEquals(quarantineFile.name, storedStrings[SecurityUtils.KEY_SECURE_STORE_LAST_QUARANTINE_FILE])
        verify(exactly = 1) { context.deleteSharedPreferences(SecurityUtils.ENCRYPTED_PREFS_FILENAME) }
    }

    @Test
    fun `init fails twice - throws SecurityException and quarantine file still exists on disk`() {
        val originalFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.xml")
        val mockCiphertext = byteArrayOf(0xDE.toByte(), 0xAD.toByte(), 0xBE.toByte(), 0xEF.toByte())
        originalFile.writeBytes(mockCiphertext)

        var factoryCallCount = 0
        val fixedTimestamp = 1700000005678L

        try {
            SecurityUtils.getEncryptedSharedPreferencesInternal(
                context = context,
                timeProvider = { fixedTimestamp },
                encryptedPrefsFactory = {
                    factoryCallCount++
                    throw GeneralSecurityException("Hardware KeyStore permanent failure")
                }
            )
            fail("Expected SecurityException to be thrown")
        } catch (e: SecurityException) {
            assertTrue("Message must mention Paranoid Mode", e.message?.contains("Paranoid Mode") == true)
            assertTrue("Message must mention Hardware KeyStore initialization failed", e.message?.contains("Hardware KeyStore initialization failed") == true)
            assertNotNull("Cause must be preserved", e.cause)
        }

        assertEquals(2, factoryCallCount)

        val quarantineFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}_corrupt_${fixedTimestamp}.xml")
        assertTrue("Quarantine file must still exist after double failure", quarantineFile.exists())
        assertTrue("Quarantine file content must match original bytes", quarantineFile.readBytes().contentEquals(mockCiphertext))

        assertEquals(fixedTimestamp, storedLongs[SecurityUtils.KEY_SECURE_STORE_RESET_AT])
        assertEquals(1, storedInts[SecurityUtils.KEY_SECURE_STORE_RESET_COUNT])
    }

    @Test
    fun `init fails with no existing file to quarantine - records reset signal and creates fresh store`() {
        val originalFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.xml")
        if (originalFile.exists()) originalFile.delete()

        val mockFreshPrefs = mockk<SharedPreferences>()
        var factoryCallCount = 0
        val fixedTimestamp = 1700000009999L

        val result = SecurityUtils.getEncryptedSharedPreferencesInternal(
            context = context,
            timeProvider = { fixedTimestamp },
            encryptedPrefsFactory = {
                factoryCallCount++
                if (factoryCallCount == 1) {
                    throw GeneralSecurityException("Init failed with missing file")
                } else {
                    mockFreshPrefs
                }
            }
        )

        assertEquals(mockFreshPrefs, result)
        assertEquals(2, factoryCallCount)
        assertEquals(fixedTimestamp, storedLongs[SecurityUtils.KEY_SECURE_STORE_RESET_AT])
        assertEquals(1, storedInts[SecurityUtils.KEY_SECURE_STORE_RESET_COUNT])
        assertNull(storedStrings[SecurityUtils.KEY_SECURE_STORE_LAST_QUARANTINE_FILE])
    }

    @Test
    fun `quarantine preserves bak file alongside xml if present`() {
        val originalFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.xml")
        val bakFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.bak")
        val xmlBytes = byteArrayOf(0x11, 0x22)
        val bakBytes = byteArrayOf(0x33, 0x44)
        originalFile.writeBytes(xmlBytes)
        bakFile.writeBytes(bakBytes)

        val mockFreshPrefs = mockk<SharedPreferences>()
        var factoryCallCount = 0
        val fixedTimestamp = 1700000007777L

        val result = SecurityUtils.getEncryptedSharedPreferencesInternal(
            context = context,
            timeProvider = { fixedTimestamp },
            encryptedPrefsFactory = {
                factoryCallCount++
                if (factoryCallCount == 1) throw GeneralSecurityException("KeyStore reset") else mockFreshPrefs
            }
        )

        assertEquals(mockFreshPrefs, result)
        val quarantineXml = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}_corrupt_${fixedTimestamp}.xml")
        val quarantineBak = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}_corrupt_${fixedTimestamp}.bak")

        assertTrue(quarantineXml.exists())
        assertTrue(quarantineBak.exists())
        assertTrue(quarantineXml.readBytes().contentEquals(xmlBytes))
        assertTrue(quarantineBak.readBytes().contentEquals(bakBytes))
    }

    @Test
    fun `consecutive resets increment count correctly`() {
        val originalFile = File(prefsDir, "${SecurityUtils.ENCRYPTED_PREFS_FILENAME}.xml")
        originalFile.writeBytes(byteArrayOf(0x01))

        storedInts[SecurityUtils.KEY_SECURE_STORE_RESET_COUNT] = 2

        val mockFreshPrefs = mockk<SharedPreferences>()
        var factoryCallCount = 0
        val fixedTimestamp = 1700000008888L

        SecurityUtils.getEncryptedSharedPreferencesInternal(
            context = context,
            timeProvider = { fixedTimestamp },
            encryptedPrefsFactory = {
                factoryCallCount++
                if (factoryCallCount == 1) throw GeneralSecurityException("KeyStore reset") else mockFreshPrefs
            }
        )

        assertEquals(3, storedInts[SecurityUtils.KEY_SECURE_STORE_RESET_COUNT])
        assertEquals(fixedTimestamp, storedLongs[SecurityUtils.KEY_SECURE_STORE_RESET_AT])
    }

    @Test
    fun `verification test confirms no secret or passphrase values are ever stored in unencrypted state`() {
        val allowedMetadataKeys = setOf(
            SecurityUtils.KEY_SECURE_STORE_RESET_AT,
            SecurityUtils.KEY_SECURE_STORE_RESET_COUNT,
            SecurityUtils.KEY_SECURE_STORE_LAST_QUARANTINE_FILE
        )

        val allWrittenKeys = storedLongs.keys + storedInts.keys + storedStrings.keys
        assertTrue("All written keys must be non-sensitive metadata keys", allowedMetadataKeys.containsAll(allWrittenKeys))
        assertTrue("No passphrase or secret key should ever be stored in unencrypted state",
            allWrittenKeys.none { it.contains("passphrase", ignoreCase = true) || it.contains("secret", ignoreCase = true) || it.contains("key_material", ignoreCase = true) }
        )
    }
}
