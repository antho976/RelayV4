import { NativeModule, requireOptionalNativeModule } from 'expo'

import type { ContentFd, CopyProgress, CopyResult, CpuInfo, SavedDownload } from './types'

type RelayDeviceEvents = {
    onCopyProgress: (progress: CopyProgress) => void
    onProcessText: () => void
}

declare class RelayDeviceNative extends NativeModule<RelayDeviceEvents> {
    openContentFd(uri: string): Promise<ContentFd>
    closeContentFd(fd: number): boolean
    closeAllContentFds(): number
    openContentFds(): number[]
    persistContentPermission(uri: string): Promise<boolean>
    releaseContentPermission(uri: string): Promise<boolean>
    copyContentToFile(source: string, destPath: string, taskId: string): Promise<CopyResult>
    cancelCopy(taskId: string): void

    saveToDownloads(sourcePath: string, fileName: string, mimeType: string): Promise<SavedDownload>
    convertImageToPng(
        source: string,
        destPath: string
    ): Promise<{ path: string; width: number; height: number }>

    availableThreads(): number
    cpuInfo(): CpuInfo

    isProcessTextEnabled(): Promise<boolean>
    setProcessTextEnabled(enabled: boolean): Promise<boolean>
    consumeProcessText(): string | null
}

/** `null` where the native side is not built in (iOS, web, Expo Go). */
export const RelayDevice = requireOptionalNativeModule<RelayDeviceNative>('RelayDevice')

export class RelayDeviceUnavailableError extends Error {
    constructor(what: string) {
        super(`${what} needs the RelayDevice native module, which this build does not include`)
        this.name = 'RelayDeviceUnavailableError'
    }
}

export const requireRelayDevice = (what: string): RelayDeviceNative => {
    if (!RelayDevice) throw new RelayDeviceUnavailableError(what)
    return RelayDevice
}
