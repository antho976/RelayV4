import merge from 'lodash.merge'

import { SamplerConfigData, SamplerID, Samplers } from '@lib/constants/SamplerData'
import { SamplersManager } from '@lib/state/SamplerState'
import { getNestedValue } from '@lib/utils/Parsing'

import { APIConfiguration, APISampler, APIValues } from './APIBuilder.types'
import { enforceAlternation, Message } from './ContextBuilder'

export interface RequestBuilderParams {
    apiConfig: APIConfiguration
    apiValues: APIValues
    samplers: SamplerConfigData
    stopSequence: string[]
    prompt: string | Message[]
}

type SamplerField = {
    [x: string]: string | number | boolean | object
}

export const buildRequest = async ({
    apiConfig,
    apiValues,
    samplers,
    prompt,
    stopSequence,
}: RequestBuilderParams) => {
    const samplerFields = getSamplerFields(apiConfig, apiValues, samplers)
    const fields = await buildFields(apiConfig, apiValues, samplerFields, stopSequence, prompt)

    switch (apiConfig.payload.type) {
        case 'openai':
            return openAIRequest(fields)
        case 'ollama':
            return ollamaRequest(fields)
        case 'cohere':
            return cohereRequest(apiConfig, fields)
        case 'horde':
            return hordeRequest(fields)
        case 'claude':
            return claudeRequest(apiConfig, fields)
        case 'custom':
            return customRequest(apiConfig, apiValues, stopSequence, fields)
    }
}

const openAIRequest = async ({ payloadFields, model, stop, prompt, custom }: Field) => {
    return {
        ...payloadFields,
        ...model,
        ...stop,
        ...prompt,
        ...custom,
    }
}

const ollamaRequest = async ({ payloadFields, model, stop, prompt, custom }: Field) => {
    let keep_alive = 5
    // 0 unloads the model at once and a negative value keeps it loaded
    if (typeof payloadFields.keep_alive === 'number') {
        keep_alive = payloadFields.keep_alive as number
        delete payloadFields.keep_alive
    }

    return {
        options: {
            ...payloadFields,
            ...stop,
        },
        keep_alive: keep_alive + 'm',
        ...model,
        ...prompt,
        raw: true,
        stream: true,
        ...custom,
    }
}

const cohereRequest = async (
    config: APIConfiguration,
    { payloadFields, model, stop, prompt, custom }: Field
) => {
    if (config.request.completionType.type === 'textCompletions') {
        return
    }

    const seedObject = config.request.samplerFields.filter(
        (item) => item.samplerID === SamplerID.SEED
    )

    if (payloadFields?.[seedObject?.[0]?.externalName] === -1)
        delete payloadFields?.[seedObject?.[0]?.externalName]

    const promptData = prompt?.[config.request.promptKey]
    if (!promptData || typeof promptData === 'string') return

    const [preamble, ...history] = promptData
    const last = history.pop()
    return {
        ...payloadFields,
        ...stop,
        ...model,
        preamble: preamble.message,
        chat_history: history,
        [config.request.promptKey]: last?.message ?? '',
        ...custom,
    }
}

const claudeRequest = async (
    config: APIConfiguration,
    { payloadFields, model, stop, prompt, custom }: Field
) => {
    const completionType = config.request.completionType
    const promptObject = prompt?.[config.request.promptKey]
    let system: string | undefined = undefined
    let finalPrompt = prompt
    if (completionType.type === 'chatCompletions' && Array.isArray(promptObject)) {
        const { systemRole, contentName } = completionType
        // the leading system message is the fully built system prompt: Claude takes it
        // as `system`, and any later system message becomes part of a user turn
        const [head, ...rest] = promptObject
        const hasSystem = head?.role === systemRole
        if (hasSystem && typeof head[contentName] === 'string') system = head[contentName]
        const history = (hasSystem ? rest : promptObject)
            .filter((item) => item[contentName]?.length > 0)
            .map((item) => ({ ...item, [contentName]: toClaudeContent(item[contentName]) }))
        finalPrompt = {
            [config.request.promptKey]: enforceAlternation(history, completionType),
        }
    }
    return {
        ...(system ? { system: system } : {}),
        ...payloadFields,
        stream: true,
        ...model,
        ...stop,
        ...finalPrompt,
        ...custom,
    }
}

// Claude takes base64 images as its own block and has no audio input
const toClaudeContent = (content: Message[string]): Message[string] => {
    if (typeof content === 'string') return content
    return content
        .filter((item) => item.type !== 'input_audio')
        .map((item) => {
            if (item.type !== 'image_url') return item
            const [, media_type, data] =
                item.image_url.url.match(/^data:([^;]+);base64,(.*)$/) ?? []
            if (!data) return item
            return { type: 'image', source: { type: 'base64', media_type: media_type, data: data } }
        })
}

const hordeRequest = async ({ payloadFields, model, stop, prompt, custom }: Field) => {
    return {
        params: {
            ...payloadFields,
            n: 1,
            frmtadsnsp: false,
            frmtrmblln: false,
            frmtrmspch: false,
            frmttriminc: true,
            ...stop,
        },
        ...prompt,
        trusted_workers: false,
        slow_workers: true,
        workers: [],
        worker_blacklist: false,
        models: model.model,
        dry_run: false,
        ...custom,
    }
}

const customRequest = async (
    config: APIConfiguration,
    values: APIValues,
    stopSequence: string[],
    { prompt }: Field
) => {
    if (config.payload.type !== 'custom') return {}
    const modelName = getModelName(config, values)

    const sampler = SamplersManager.getCurrentSampler()

    // values are inserted through a function so a `$` in them is kept as written
    let responseBody = config.payload.customPayload
    const insert = (macro: string, value: string) => {
        responseBody = responseBody.replaceAll(macro, () => value)
    }

    // the reply length, whether or not the template lists it as a sampler;
    // '{{generted_length}}' is the macro's old misspelling, still found in stored templates
    const replyLength = `${sampler[SamplerID.GENERATED_LENGTH]}`
    insert('{{generated_length}}', replyLength)
    insert('{{generted_length}}', replyLength)

    config.request.samplerFields.forEach((item) => {
        insert(Samplers[item.samplerID].macro, sampler?.[item.samplerID]?.toString() ?? '')
    })
    // prompt and stop are inserted as JSON values
    insert('{{stop}}', JSON.stringify(stopSequence))
    insert('{{prompt}}', JSON.stringify(prompt[config.request.promptKey]))
    insert('{{model}}', modelName?.toString() ?? '')
    return responseBody
}

type Field = Awaited<ReturnType<typeof buildFields>>

const buildFields = async (
    config: APIConfiguration,
    values: APIValues,
    payloadFields: SamplerField,
    stopSeq: string[],
    promptData: string | Message[]
) => {
    // Model Data
    const model = config.features.useModel
        ? {
              model: getModelName(config, values),
          }
        : {}
    // Stop Sequence
    const stop = config.request.useStop ? { [config.request.stopKey]: stopSeq } : {}

    // Seed Data
    const seedObject = config.request.samplerFields.filter(
        (item) => item.samplerID === SamplerID.SEED
    )

    const seedName = seedObject[0]?.externalName
    if (
        seedName &&
        config.request.removeSeedifNegative &&
        typeof payloadFields[seedName] === 'number' &&
        payloadFields[seedName] < 0
    ) {
        delete payloadFields[seedName]
    }

    // Context Length
    const contextLengthObject = config.request.samplerFields.filter(
        (item) => item.samplerID === SamplerID.CONTEXT_LENGTH
    )

    const lengthName = contextLengthObject[0]?.externalName
    const instructLengthField = lengthName ? payloadFields[lengthName] : undefined

    const modelLengthField = getModelContextLength(config, values)
    const instructLength =
        typeof instructLengthField === 'number' ? instructLengthField : (modelLengthField ?? 0)
    const modelLength = modelLengthField ?? instructLength
    const length = config.model.useModelContextLength
        ? Math.min(modelLength, instructLength)
        : instructLength

    // the length is used client-side either way; only APIs that take it are sent it
    if (lengthName && instructLengthField !== undefined) {
        if (config.request.removeLength) delete payloadFields[lengthName]
        else payloadFields[lengthName] = length
    }

    const prompt = { [config.request.promptKey]: promptData }

    const custom = values.customFields ? buildCustomFields(values.customFields) : {}

    return { payloadFields, model, stop, prompt, length, custom }
}

const buildCustomFields = (customFieldsBody: string) => {
    const fieldBody: Record<string, any> = {}

    customFieldsBody
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean)
        .forEach((line) => {
            const eqIndex = line.indexOf('=')
            if (eqIndex === -1) return

            const key = line.slice(0, eqIndex).trim()
            const rawValue = line.slice(eqIndex + 1).trim()

            let value: any = rawValue
            try {
                value = JSON.parse(rawValue)
            } catch {}

            const parts = key.split('.')
            let current = fieldBody

            for (let i = 0; i < parts.length - 1; i++) {
                const part = parts[i]

                if (
                    current[part] === undefined ||
                    typeof current[part] !== 'object' ||
                    current[part] === null
                ) {
                    current[part] = {}
                }

                current = current[part]
            }

            current[parts[parts.length - 1]] = value
        })

    return fieldBody
}

const getModelName = (config: APIConfiguration, values: APIValues) => {
    let model = undefined
    if (config.features.multipleModels) {
        model = values.model.map((item: any) => getNestedValue(item, config.model.nameParser))
    } else {
        model = getNestedValue(values.model, config.model.nameParser)
    }
    return model
}

const getModelContextLength = (config: APIConfiguration, values: APIValues): number | undefined => {
    const keys = config.model.contextSizeParser.split('.')
    const result = keys.reduce((acc, key) => acc?.[key], values.model)
    return Number.isInteger(result) ? result : undefined
}

const getSamplerFields = (
    config: APIConfiguration,
    values: APIValues,
    samplers: SamplerConfigData
) => {
    let max_length = undefined
    if (config.model.useModelContextLength) {
        max_length = getModelContextLength(config, values)
    }

    return config.request.samplerFields
        .map((item: APISampler) => {
            const value = samplers[item.samplerID]
            const samplerItem = Samplers[item.samplerID]
            let cleanvalue = value
            if (typeof value === 'number') {
                const type = samplerItem.values.type
                // any keep-alive value is meaningful to Ollama, -1 included
                if (
                    (type === 'float' || type === 'integer') &&
                    value === samplerItem.values.ignoreIf &&
                    item.samplerID !== SamplerID.KEEP_ALIVE_DURATION
                ) {
                    return {}
                }

                if (item.samplerID === 'max_length') {
                    cleanvalue = Math.min(value, max_length ?? value)
                } else if (type === 'integer') {
                    cleanvalue = Math.floor(value)
                }
            }
            if (item.samplerID === SamplerID.DRY_SEQUENCE_BREAK) {
                //@ts-expect-error. This is due to a migration
                cleanvalue = (value as string).split(',')
            }

            if (item.samplerID === SamplerID.REASONING_EFFORT && cleanvalue === 'disabled') {
                return {}
            }
            if (
                (item.samplerID === SamplerID.REASONING_EFFORT ||
                    item.samplerID === SamplerID.REASONING_MAX_TOKENS) &&
                !cleanvalue
            ) {
                return {}
            }
            if (reasoning.includes(item.samplerID)) {
                return { reasoning: { [item.externalName]: cleanvalue } }
            }
            return { [item.externalName]: cleanvalue }
        })
        .reduce((acc, obj) => merge(acc, obj), {})
}

const reasoning = [
    SamplerID.REASONING_EFFORT,
    SamplerID.REASONING_EXCLUDE,
    SamplerID.REASONING_MAX_TOKENS,
]
