/**
 * Relay's device module: content-URI files, the Downloads folder, CPU facts and the
 * text-selection menu entry. Android only; elsewhere the functions that need native code throw
 * `RelayDeviceUnavailableError` and the process-text helpers report "off".
 */
export { RelayDeviceUnavailableError } from './src/RelayDeviceModule'
export * from './src/contentFiles'
export * from './src/downloads'
export * from './src/cpu'
export * from './src/processText'
export * from './src/types'

/** True when an error from this module carries the given code. */
export const isRelayDeviceError = (
    e: unknown,
    code: import('./src/types').RelayDeviceErrorCode
): boolean => typeof e === 'object' && e !== null && (e as { code?: unknown }).code === code
