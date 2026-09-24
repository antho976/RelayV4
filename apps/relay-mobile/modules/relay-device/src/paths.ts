/**
 * Turns a file:// URI into a plain path; anything else is returned unchanged. Percent escapes
 * are decoded when they are well formed, and a stray `%` is kept as it is.
 */
export const fileUriToPath = (uri: string): string => {
    if (!uri.startsWith('file://')) return uri
    const path = uri.slice('file://'.length)
    try {
        return decodeURIComponent(path)
    } catch {
        return path
    }
}
