package com.quietsoftware.relay.core.wire

/**
 * Wake-on-LAN: six 0xFF bytes, then the network card's MAC sixteen times, sent as a UDP
 * broadcast on the PC's own network. It only reaches a sleeping PC from the same network, and
 * only if its firmware and network card are set to wake on it (docs/ANDROID.md, "Waking the PC").
 */
object Wake {
    const val PORT = 9

    fun macBytes(mac: String): ByteArray? {
        val hex = mac.filter { it.isLetterOrDigit() }
        if (hex.length != 12 || !hex.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }) return null
        return ByteArray(6) { i -> hex.substring(i * 2, i * 2 + 2).toInt(16).toByte() }
    }

    fun magicPacket(mac: String): ByteArray? {
        val bytes = macBytes(mac) ?: return null
        val packet = ByteArray(6 + 16 * 6)
        for (i in 0 until 6) packet[i] = 0xFF.toByte()
        for (r in 0 until 16) bytes.copyInto(packet, 6 + r * 6)
        return packet
    }
}
