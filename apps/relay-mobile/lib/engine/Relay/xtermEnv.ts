/**
 * What `@xterm/headless` reads from the host before it has parsed a byte. Imported for its
 * side effect, ahead of the library.
 *
 * At load it decides whether it runs in Node (`'title' in process`) and, when not, reads
 * `navigator.userAgent.includes(...)` to sniff a browser. React Native's `navigator` is
 * `{product: 'ReactNative'}` with no `userAgent`, so the import would throw. Giving it an
 * empty user agent and platform is all it needs: every browser test comes out false, and
 * nothing else in the headless build touches the DOM.
 */
const host = globalThis as unknown as { navigator?: Record<string, unknown> }
try {
    if (!host.navigator) host.navigator = {}
    const nav = host.navigator
    if (typeof nav.userAgent !== 'string') nav.userAgent = ''
    if (typeof nav.platform !== 'string') nav.platform = ''
} catch {}

export {}
