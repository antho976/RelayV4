/**
 * Hosts that are only reachable from the device itself; plain HTTP to them never leaves it.
 */
const localHosts = ['localhost', '127.0.0.1', '::1', '[::1]', '10.0.2.2']

/**
 * True when an endpoint sends traffic in the clear over a network.
 * Loopback addresses are excluded since that traffic never leaves the device.
 */
export const isInsecureEndpoint = (endpoint?: string) => {
    if (!endpoint) return false
    try {
        const url = new URL(endpoint)
        if (url.protocol !== 'http:') return false
        return !localHosts.includes(url.hostname.toLowerCase())
    } catch {
        return endpoint.trim().toLowerCase().startsWith('http://')
    }
}
