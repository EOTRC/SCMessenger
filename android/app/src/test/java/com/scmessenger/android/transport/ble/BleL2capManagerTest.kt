package com.scmessenger.android.transport.ble

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class BleL2capManagerTest {

    @Test
    fun acceptRecoveryBacksOffAndStopsAtFailureCeiling() {
        val policy = BleL2capAcceptRecoveryPolicy()

        assertEquals(250L, policy.recordFailure())
        assertEquals(500L, policy.recordFailure())
        assertEquals(1_000L, policy.recordFailure())
        assertEquals(2_000L, policy.recordFailure())
        assertNull(policy.recordFailure())
        assertEquals(L2CAP_ACCEPT_MAX_RECOVERY_ATTEMPTS, policy.failureCount)

        // A terminal policy remains terminal until a successful accept is
        // observed; callers must not accidentally re-enter a tight retry loop.
        assertNull(policy.recordFailure())
    }

    @Test
    fun acceptRecoveryDelayIsCapped() {
        val policy = BleL2capAcceptRecoveryPolicy(maxAttempts = 8)

        repeat(5) { policy.recordFailure() }

        assertEquals(L2CAP_ACCEPT_MAX_BACKOFF_MS, policy.recordFailure())
        assertEquals(L2CAP_ACCEPT_MAX_BACKOFF_MS, policy.recordFailure())
    }

    @Test
    fun successfulAcceptResetsRecoveryBudget() {
        val policy = BleL2capAcceptRecoveryPolicy()

        policy.recordFailure()
        policy.recordFailure()
        policy.recordSuccess()

        assertEquals(0, policy.failureCount)
        assertEquals(L2CAP_ACCEPT_INITIAL_BACKOFF_MS, policy.recordFailure())
    }
}
