import { nativeApplicationVersion } from 'expo-application'
import { t } from 'i18next'

import { AppSettings, CLAUDE_VERSION } from '@lib/constants/GlobalValues'
import { SSEFetch } from '@lib/engine/SSEFetch'
import { Logger } from '@lib/state/Logger'
import { mmkv } from '@lib/storage/MMKV'
import { getNestedValue } from '@lib/utils/Parsing'

import { APIConfiguration } from './APIBuilder.types'
import { buildContext, ContextBuilderParams } from './ContextBuilder'
import { buildRequest, RequestBuilderParams } from './RequestBuilder'

export type DataOutputType = 'text' | 'reasoning' | 'tool_call'

type DataOutput = {
    type: DataOutputType
    content: string
}

export interface APIBuilderParams
    extends ContextBuilderParams,
        Omit<RequestBuilderParams, 'prompt'> {
    onData: (data: DataOutput) => void
    onEnd: (data: string) => void
    stopSequence: string[]
    stopGenerating: () => void
}

export const buildAndSendRequest = async (params: APIBuilderParams) => {
    const { stopGenerating, apiConfig, apiValues, stopSequence, onData, onEnd } = params
    try {
        let payload: any = undefined
        const bypassContextLength = mmkv.getBoolean(AppSettings.BypassContextLength)
        const prompt = await buildContext({ ...params, bypassContextLength })
        if (prompt === undefined) {
            Logger.errorToast(t('generation.errors.promptConstructionFailed'))
            stopGenerating()
            return
        }

        payload = await buildRequest({
            ...params,
            prompt,
        })

        if (!payload) {
            Logger.errorToast(t('generation.errors.payloadConstructionFailed'))
            stopGenerating()
            return
        }

        if (typeof payload !== 'string') {
            payload = JSON.stringify(payload)
        }

        let header: any = {}
        if (apiConfig.features.useKey) {
            const anthropicVersion =
                apiConfig.payload.type === 'claude' ? { 'anthropic-version': CLAUDE_VERSION } : {}

            header = {
                ...anthropicVersion,
                [apiConfig.request.authHeader]: apiConfig.request.authPrefix + apiValues.key,
            }
        }

        const response = responses[apiConfig.request.requestType]

        const replaceStrings = constructReplaceStrings(stopSequence)

        const parseOutput = (event: any, pattern: string | string[], type: DataOutputType) => {
            try {
                const data = getNestedValue(
                    typeof event === 'string' ? JSON.parse(event) : event,
                    pattern
                ) as string | null
                const text = data?.replaceAll(replaceStrings, '') ?? ''
                if (text) onData({ content: text, type: type })
                return !!text?.trim()
            } catch (e) {
                Logger.error(e)
            }
            return false
        }

        const patternMapping: { pattern: string | string[]; type: DataOutputType }[] = [
            { type: 'text', pattern: apiConfig.request.responseParsePattern },
        ]
        const reasonPattern = apiConfig.request.reasoningParsePattern

        const isChatCompletions = apiConfig.request.completionType.type === 'chatCompletions'
        if (reasonPattern && isChatCompletions) {
            patternMapping.push({ type: 'reasoning', pattern: reasonPattern })
        }

        return response({
            endpoint: apiValues.endpoint,
            payload: payload,
            onEvent: (event) => {
                for (const pattern of patternMapping) {
                    if (parseOutput(event, pattern.pattern, pattern.type)) break
                }
            },
            onEnd: onEnd,
            header: header,
            stopGenerating: stopGenerating,
        })
    } catch (e) {
        Logger.errorToast(t('generation.errors.completionFailed'), e)
        stopGenerating()
    }
}

type KeyHeader = {
    [key: string]: string
}

// done settles once the reply has finished, failed or been aborted
type SentRequest = {
    abort: () => void
    done: Promise<void>
}

type SenderParams = {
    endpoint: string
    payload: string
    header: KeyHeader
    onEnd: (data: string) => void
    onEvent: (event: any) => void
    stopGenerating: () => void
}

// polled every 5 s, so a job still queued after 20 minutes is given up on
const HORDE_MAX_POLLS = 240

const hordeResponse = (senderParams: SenderParams): SentRequest => {
    const hordeURL = `https://aihorde.net/api/v2/`
    let generation_id = ''
    let aborted = false

    const abortFn = () => {
        aborted = true
        if (generation_id) {
            fetch(`${hordeURL}generate/text/status/${generation_id}`, {
                method: 'DELETE',
                headers: {
                    'Client-Agent': `Relay:${nativeApplicationVersion}:https://github.com/antho976/RelayV4`,
                    accept: 'application/json',
                    'Content-Type': 'application/json',
                },
            }).catch(Logger.error)
        }
        senderParams.stopGenerating()
    }

    const sendRequest = async () => {
        Logger.info(`Using Horde`)

        const request = await fetch(`${hordeURL}generate/text/async`, {
            method: 'POST',
            body: senderParams.payload,
            headers: {
                ...senderParams.header,
                'Client-Agent': `Relay:${nativeApplicationVersion}:https://github.com/antho976/RelayV4`,
                accept: 'application/json',
                'content-type': 'application/json',
            },
        })

        if (request.status === 401) {
            Logger.error(`Invalid API Key`)
            senderParams.stopGenerating()
            return
        }

        if (request.status !== 202) {
            Logger.error(`Horde Request failed.`)
            senderParams.stopGenerating()
            const body = await request.json().catch(() => undefined)
            Logger.error(JSON.stringify(body))
            if (Array.isArray(body?.errors)) for (const e of body.errors) Logger.error(e)
            return
        }

        const body = await request.json()
        generation_id = body.id
        let result

        for (let poll = 0; ; poll++) {
            await new Promise((resolve) => setTimeout(resolve, 5000))
            if (aborted) return

            if (poll >= HORDE_MAX_POLLS) {
                Logger.errorToast(t('generation.errors.generationFailed'))
                Logger.error('Horde job did not finish in time')
                abortFn()
                return
            }

            Logger.info(`Checking...`)
            const response = await fetch(`${hordeURL}generate/text/status/${generation_id}`, {
                method: 'GET',
                headers: {
                    'Client-Agent': `Relay:${nativeApplicationVersion}:https://github.com/antho976/RelayV4`,
                    accept: 'application/json',
                    'content-type': 'application/json',
                },
            })

            // rate limited: try again on the next poll
            if (response.status === 429) continue

            if (!response.ok) {
                Logger.errorToast(t('generation.errors.generationFailed'))
                Logger.error(`Response failed with status ${response.status}.`)
                Logger.error((await response.json().catch(() => undefined))?.message)
                senderParams.stopGenerating()
                return
            }

            result = await response.json()
            if (result?.faulted || result?.is_possible === false) {
                Logger.errorToast(t('generation.errors.generationFailed'))
                Logger.error('Horde job faulted or no worker can run it')
                abortFn()
                return
            }
            if (result?.done) break
        }

        if (aborted) return
        if (result) senderParams.onEvent(result)
        // as with a stream, the finished reply can get an automatic title
        senderParams.onEnd('')
        senderParams.stopGenerating()
    }

    const done = sendRequest().catch((e) => {
        if (aborted) return
        Logger.errorToast(t('generation.errors.completionFailed'), e)
        senderParams.stopGenerating()
    })

    return { abort: abortFn, done: done }
}

const readableStreamResponse = (senderParams: SenderParams): SentRequest => {
    const sse = new SSEFetch()

    const closeStream = () => {
        Logger.debug('Running Close Stream')
        senderParams.onEnd('')
        senderParams.stopGenerating()
    }

    sse.setOnEvent((data) => {
        try {
            const a = JSON.parse(data)
            if (a?.error) {
                Logger.errorToast(t('generation.errors.sseFailed'))
                Logger.error(data)
            }
        } catch {}
        senderParams.onEvent(data)
    })

    sse.setOnError(() => {
        Logger.errorToast(t('generation.errors.generationFailed'))
        closeStream()
    })

    sse.setOnClose(() => {
        Logger.info('Stream Closed')
        closeStream()
    })

    const done = sse.start({
        endpoint: senderParams.endpoint,
        body: senderParams.payload,
        method: 'POST',
        headers: {
            accept: 'application/json',
            'Content-Type': 'application/json',
            ...senderParams.header,
        },
    })

    return { abort: () => sse.abort(), done: done }
}

const constructReplaceStrings = (stopSequence: string[]) => {
    const replace = RegExp(
        stopSequence.map((item) => item.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join(`|`),
        'g'
    )
    return replace
}

const responses: Record<
    APIConfiguration['request']['requestType'],
    (params: SenderParams) => SentRequest
> = {
    horde: hordeResponse,
    stream: readableStreamResponse,
}
