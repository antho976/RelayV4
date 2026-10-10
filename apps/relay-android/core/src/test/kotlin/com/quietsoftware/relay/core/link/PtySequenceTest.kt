package com.quietsoftware.relay.core.link

import com.quietsoftware.relay.core.link.PtySequence.Step
import org.junit.Assert.assertEquals
import org.junit.Test

class PtySequenceTest {
    /** The desktop's own case (terminal.rs `catchup_duplicates_gaps_and_respawn_are_distinct`). */
    @Test
    fun `catch-up, duplicates, gaps and a respawn are told apart`() {
        val s = PtySequence().apply { catchUp = true }
        assertEquals(Step.Feed, s.observe(1, 12))
        assertEquals(Step.Duplicate, s.observe(1, 12))
        assertEquals(Step.Gap, s.observe(1, 14))
        assertEquals(1L to 12L, s.last)
        s.catchUp = true
        assertEquals(Step.Feed, s.observe(1, 20))
        assertEquals(Step.Feed, s.observe(1, 21))
        assertEquals(Step.Reset, s.observe(2, 0))
        assertEquals(Step.Duplicate, s.observe(1, 90))
    }
}
