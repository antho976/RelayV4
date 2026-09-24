export type ContentFd = {
    /** The descriptor number. cui-llama.rn accepts `String(fd)` as a model path. */
    fd: number
    /** `/proc/self/fd/<fd>`, for native code that only opens paths. */
    path: string
    /** Size in bytes, or -1 when the provider does not say. */
    size: number
}

export type CopyProgress = {
    taskId: string
    bytesCopied: number
    /** -1 when the source's size is unknown. */
    totalBytes: number
    bytesPerSecond: number
}

export type CopyResult = { path: string; bytesCopied: number }

export type SavedDownload = {
    /** content:// URI of the new item (file:// below Android 10). */
    uri: string
    /** The name it was saved under, which differs from the request on a name clash. */
    name: string
}

export type CpuInfo = {
    abi: string
    availableThreads: number
    /** Cores the kernel knows about, including offline ones. */
    possibleCores: number
    /** Raw flags from the `Features` line of /proc/cpuinfo. */
    features: string[]
    dotprod: boolean
    i8mm: boolean
    fp16: boolean
    sve: boolean
    sve2: boolean
}

export type RelayDeviceErrorCode =
    | 'ERR_NOT_CONTENT_URI'
    | 'ERR_NOT_FOUND'
    | 'ERR_PERMISSION'
    | 'ERR_NO_SPACE'
    | 'ERR_COPY_BUSY'
    | 'ERR_COPY_CANCELLED'
    | 'ERR_COPY_FAILED'
    | 'ERR_SAVE_FAILED'
    | 'ERR_DECODE_FAILED'
    | 'ERR_ENCODE_FAILED'
    | 'ERR_NOT_CONFIGURED'
