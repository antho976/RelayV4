package com.quietsoftware.relay.core.link

/**
 * Where a terminal stands in its PTY's stream (BUS.md §7), as the desktop keeps it
 * (apps/relay-native/src/terminal.rs `Sequence`): frames arrive numbered by (epoch, seq); a new
 * epoch is the agent started again, a seq already seen is a duplicate, and a jump is a gap to
 * recover by attaching again from the last frame fed. The first frame after an attach is the
 * engine's catch-up, which may jump: [catchUp] lets exactly that one through.
 */
class PtySequence {
    enum class Step { Feed, Reset, Duplicate, Gap }

    var last: Pair<Long, Long>? = null
        private set
    var catchUp = false

    fun observe(epoch: Long, seq: Long): Step {
        val catching = catchUp
        catchUp = false
        val l = last
        val step = when {
            l == null -> Step.Feed
            epoch > l.first -> Step.Reset
            epoch < l.first -> Step.Duplicate
            seq <= l.second -> Step.Duplicate
            catching || seq == l.second + 1 -> Step.Feed
            else -> Step.Gap
        }
        if (step == Step.Feed || step == Step.Reset) last = epoch to seq
        return step
    }

    fun forget() {
        last = null
        catchUp = false
    }
}
