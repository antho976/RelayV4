import { defaultSamplerConfig, SamplerConfigData, SamplerID } from './SamplerData'

export type SamplerPreset = {
    name: string
    description: string
    data: SamplerConfigData
}

const withDefaults = (overrides: Partial<SamplerConfigData>): SamplerConfigData => ({
    ...defaultSamplerConfig,
    ...overrides,
})

/**
 * Shared anti-repetition settings.
 *
 * DRY (Don't Repeat Yourself) penalises the model for re-emitting sequences that already
 * appear in the context, which is the usual cause of a model "getting stuck" asking the same
 * question or restating the same reply. The values below are the commonly recommended ones.
 */
const antiRepetition: Partial<SamplerConfigData> = {
    [SamplerID.REPETITION_PENALTY]: 1.05,
    [SamplerID.REPETITION_PENALTY_RANGE]: 1024,
    [SamplerID.PRESENCE_PENALTY]: 0.5,
    [SamplerID.DRY_MULTIPLIER]: 0.8,
    [SamplerID.DRY_BASE]: 1.75,
    [SamplerID.DRY_ALLOWED_LENGTH]: 2,
    [SamplerID.DRY_PENALTY_LAST_N]: -1,
    [SamplerID.DRY_SEQUENCE_BREAK]: '\\n,:,",*',
}

/**
 * Built-in presets the user can add from the Sampler screen.
 * Existing presets with the same name are never overwritten.
 */
export const recommendedSamplerPresets: SamplerPreset[] = [
    {
        name: 'Anti-Repetition',
        description: 'DRY + light penalties. Use when the model loops or repeats itself.',
        data: withDefaults({
            [SamplerID.TEMPERATURE]: 0.8,
            [SamplerID.TOP_P]: 0.95,
            [SamplerID.TOP_K]: 40,
            [SamplerID.MIN_P]: 0.05,
            [SamplerID.GENERATED_LENGTH]: 512,
            ...antiRepetition,
        }),
    },
    {
        name: 'Qwen3 (No Thinking)',
        description: 'Qwen recommended non-thinking values plus anti-repetition.',
        data: withDefaults({
            [SamplerID.TEMPERATURE]: 0.7,
            [SamplerID.TOP_P]: 0.8,
            [SamplerID.TOP_K]: 20,
            [SamplerID.MIN_P]: 0,
            [SamplerID.GENERATED_LENGTH]: 512,
            [SamplerID.ENABLE_THINKING]: false,
            ...antiRepetition,
            [SamplerID.PRESENCE_PENALTY]: 1,
        }),
    },
    {
        name: 'Qwen3 (Thinking)',
        description: 'Qwen recommended thinking values plus anti-repetition.',
        data: withDefaults({
            [SamplerID.TEMPERATURE]: 0.6,
            [SamplerID.TOP_P]: 0.95,
            [SamplerID.TOP_K]: 20,
            [SamplerID.MIN_P]: 0,
            [SamplerID.GENERATED_LENGTH]: 2048,
            [SamplerID.ENABLE_THINKING]: true,
            ...antiRepetition,
            [SamplerID.PRESENCE_PENALTY]: 1,
        }),
    },
]
