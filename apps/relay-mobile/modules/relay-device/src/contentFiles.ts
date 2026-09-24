import { fileUriToPath } from './paths'
import { requireRelayDevice } from './RelayDeviceModule'
import type { ContentFd, CopyProgress, CopyResult } from './types'

const assertContentUri = (uri: string) => {
    if (!uri.startsWith('content://')) throw new TypeError(`Not a content:// URI: ${uri}`)
}

/**
 * Opens a content URI for reading and returns its descriptor. The module keeps track of it:
 * close it with `closeContentFd` once native code has opened the file for itself (cui-llama.rn
 * duplicates the descriptor, so that can be right after the load call returns).
 */
export const openContentFd = async (uri: string): Promise<ContentFd> => {
    assertContentUri(uri)
    return requireRelayDevice('openContentFd').openContentFd(uri)
}

/**
 * Closes a descriptor from `openContentFd`. Takes the object, the number, or the string forms
 * (`"42"`, `"/proc/self/fd/42"`). Returns false when it was not open.
 */
export const closeContentFd = (fd: ContentFd | number | string): boolean => {
    const n =
        typeof fd === 'number'
            ? fd
            : typeof fd === 'string'
              ? Number(fd.slice(fd.lastIndexOf('/') + 1))
              : fd.fd
    if (!Number.isInteger(n) || n < 0) return false
    return requireRelayDevice('closeContentFd').closeContentFd(n)
}

/** Closes every descriptor still open. Returns how many there were. */
export const closeAllContentFds = (): number =>
    requireRelayDevice('closeAllContentFds').closeAllContentFds()

/** Descriptors opened by `openContentFd` and not yet closed. */
export const openContentFds = (): number[] => requireRelayDevice('openContentFds').openContentFds()

/**
 * Keeps read access (and write, when granted) to a picked document across restarts.
 * Resolves to whether write access was kept.
 */
export const persistContentPermission = async (uri: string): Promise<boolean> => {
    assertContentUri(uri)
    return requireRelayDevice('persistContentPermission').persistContentPermission(uri)
}

/** Gives back a persisted grant, e.g. when the model that used it is removed. */
export const releaseContentPermission = async (uri: string): Promise<boolean> => {
    assertContentUri(uri)
    return requireRelayDevice('releaseContentPermission').releaseContentPermission(uri)
}

export type CopyOptions = {
    onProgress?: (progress: CopyProgress) => void
    /** Aborting cancels the copy; the promise rejects with code `ERR_COPY_CANCELLED`. */
    signal?: AbortSignal
}

let copyCounter = 0

/**
 * Copies a content URI (or file URI / path) to a local file, off the JS thread, in 1 MiB
 * blocks. Progress arrives at most every 200 ms and once at the end. On failure or cancel the
 * partial file is removed, and the destination only appears once the copy is complete.
 */
export const copyContentToFile = async (
    source: string,
    destPath: string,
    { onProgress, signal }: CopyOptions = {}
): Promise<CopyResult> => {
    const native = requireRelayDevice('copyContentToFile')
    const taskId = `copy-${Date.now()}-${++copyCounter}`
    const dest = fileUriToPath(destPath)

    const progress = onProgress
        ? native.addListener('onCopyProgress', (event) => {
              if (event.taskId === taskId) onProgress(event)
          })
        : undefined
    const abort = () => native.cancelCopy(taskId)
    signal?.addEventListener('abort', abort)
    try {
        // an abort that lands before the native task starts is remembered and honoured there
        if (signal?.aborted) abort()
        return await native.copyContentToFile(source, dest, taskId)
    } finally {
        progress?.remove()
        signal?.removeEventListener('abort', abort)
    }
}
