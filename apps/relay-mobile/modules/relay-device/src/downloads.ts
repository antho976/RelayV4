import { PermissionsAndroid, Platform } from 'react-native'

import { fileUriToPath } from './paths'
import { requireRelayDevice } from './RelayDeviceModule'
import type { SavedDownload } from './types'

export type SaveToDownloadsOptions = {
    /** Name in Downloads; defaults to the source file's name. */
    fileName?: string
    /** Defaults to a guess from the file extension. */
    mimeType?: string
}

export class StoragePermissionError extends Error {
    readonly neverAskAgain: boolean
    constructor(neverAskAgain: boolean) {
        super('Storage permission was not granted')
        this.name = 'StoragePermissionError'
        this.neverAskAgain = neverAskAgain
    }
}

/** Android 9 and below write Downloads as a plain file, which needs the storage permission. */
const ensureLegacyWritePermission = async () => {
    if (Platform.OS !== 'android' || Number(Platform.Version) >= 29) return
    const permission = PermissionsAndroid.PERMISSIONS.WRITE_EXTERNAL_STORAGE
    if (await PermissionsAndroid.check(permission)) return
    const result = await PermissionsAndroid.request(permission)
    if (result !== PermissionsAndroid.RESULTS.GRANTED)
        throw new StoragePermissionError(result === PermissionsAndroid.RESULTS.NEVER_ASK_AGAIN)
}

/**
 * Copies a local file into the public Downloads folder. Never overwrites: a clash is saved as
 * `name (1).ext`. On Android 10+ no permission is involved; below that the storage permission
 * is requested, and refusal rejects with `StoragePermissionError`.
 *
 * @param sourcePath a path or file:// URI
 */
export const saveToDownloads = async (
    sourcePath: string,
    { fileName = '', mimeType = '' }: SaveToDownloadsOptions = {}
): Promise<SavedDownload> => {
    const native = requireRelayDevice('saveToDownloads')
    await ensureLegacyWritePermission()
    const path = fileUriToPath(sourcePath)
    return native.saveToDownloads(path, fileName, mimeType)
}

/**
 * Re-encodes any image Android can decode (JPEG, WebP, HEIF, GIF ...) as a PNG file. Use it
 * before writing PNG text chunks into an image that is not a PNG.
 */
export const convertImageToPng = async (source: string, destPath: string) => {
    const dest = fileUriToPath(destPath)
    return requireRelayDevice('convertImageToPng').convertImageToPng(source, dest)
}
