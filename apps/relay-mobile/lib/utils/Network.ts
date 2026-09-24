/**
 * Hosts that are only reachable from the device itself; plain HTTP to them never leaves it.
 */
const localHosts = ['localhost', '127.0.0.1', '::1', '10.0.2.2']

/**
 * True when an endpoint sends traffic in the clear over a network.
 * Loopback addresses are excluded since that traffic never leaves the device.
 * Parsed by hand: React Native's URL keeps the scheme's case and cannot read an IPv6 host.
 */
export const isInsecureEndpoint = (endpoint?: string) => {
    if (!endpoint) return false
    const match = endpoint
        .trim()
        .match(/^([a-z][a-z\d+\-.]*):\/\/(?:[^@/?#]*@)?(\[[^\]]*\]|[^:/?#]*)/i)
    if (!match || match[1].toLowerCase() !== 'http') return false
    const host = match[2].toLowerCase().replace(/^\[|\]$/g, '')
    return !localHosts.includes(host)
}
