import { requireRelayDevice } from './RelayDeviceModule'
import type { CpuInfo } from './types'

/** Processors currently available to the app (online cores). Synchronous. */
export const availableThreads = (): number =>
    requireRelayDevice('availableThreads').availableThreads()

/** ABI, core counts and the arm64 feature flags llama.cpp cares about. Cached natively. */
export const cpuInfo = (): CpuInfo => requireRelayDevice('cpuInfo').cpuInfo()
