import { t } from 'i18next'

import { ChatPresetType } from '@db/schema'
import { AppSettings } from '@lib/constants/GlobalValues'
import { buildThinkRules } from '@lib/markdown/ThinkTags'
import { useAppModeStore } from '@lib/state/AppMode'
import { CharacterCardData, CharacterTokenCache } from '@lib/state/Characters'
import { ChatEntry } from '@lib/state/Chat'
import { defaultSystemPromptFormat, InstructTokenCache, InstructType } from '@lib/state/Instructs'
import { Logger } from '@lib/state/Logger'
import { replaceMacros } from '@lib/state/Macros'
import { mmkv } from '@lib/storage/MMKV'
import { readBase64Async } from '@lib/utils/File'
import { Macro } from '@lib/utils/Macros'

import { APIConfiguration, APIValues } from './APIBuilder.types'
import type { DataSource, DataSourceResult } from '../DataSources/types'

export type MessageLoader = {
    retrieve: (page: number) => Promise<ChatEntry[]> // must retrieve messages in chronological order from oldest to newest
    pageSize: number // we use this to determine if a last page has been reached, if (await retrieve()).length < pageSize
    initialPage: number // usually 0
}

export type TokenCache = {
    userCache: CharacterTokenCache
    characterCache: CharacterTokenCache
    instructCache: InstructTokenCache
}

const printContext = (context: string) => {
    if (!mmkv.getBoolean(AppSettings.PrintContext)) return
    Logger.info('Input Context')
    Logger.info(JSON.stringify(context))
}

export interface ContextBuilderParams {
    apiConfig: APIConfiguration
    apiValues: APIValues
    messages: ChatEntry[]
    character: CharacterCardData
    instruct: InstructType
    user: CharacterCardData
    tokenizer: (data: string, media_paths?: string[]) => Promise<number> | number
    chatTokenizer: (entry: ChatEntry, index: number) => Promise<number>
    maxLength: number
    cache: TokenCache
    bypassContextLength?: boolean
    messageLoader?: MessageLoader
    dataSources?: DataSource[]
    /** per-chat memory notes, injected after the chat history */
    chatMemory?: string
    /** active chat preset, overrides the system prompt, persona and rules when set */
    chatPreset?: ChatPresetType | null
}

/**
 * A chat preset resolved to macro-replaced text with token counts.
 * Blank fields are dropped so the card / instruct values apply instead.
 */
export type ResolvedChatPreset = {
    system_prompt?: { text: string; length: number }
    persona?: { text: string; length: number }
    rules?: { text: string; length: number }
}

export const resolveChatPreset = async (
    preset: ChatPresetType | null | undefined,
    instruct: InstructType,
    tokenizer: ContextBuilderParams['tokenizer']
): Promise<ResolvedChatPreset> => {
    const resolved: ResolvedChatPreset = {}
    if (!preset) return resolved
    const fields = ['system_prompt', 'persona', 'rules'] as const
    for (const field of fields) {
        const raw = preset[field]?.trim() ?? ''
        if (!raw) continue
        const text = replaceMacrosInternal(raw, instruct)
        resolved[field] = { text: text, length: await tokenizer(text) }
    }
    return resolved
}

type TextData = { type: 'input_text' | 'text'; text: string }
type ImageData = { type: 'image_url'; image_url: { url: string } }
type AudioData = { type: 'input_audio'; input_audio: { data: string; format: string } }

type ContentTypes = TextData | ImageData | AudioData

export type Message = { role: string; [x: string]: ContentTypes[] | string }

export const buildContext = async (params: ContextBuilderParams) => {
    const buildFn =
        params.apiConfig.request.completionType.type === 'chatCompletions'
            ? buildChatCompletionContext
            : buildTextCompletionContext
    const output = await buildFn(params)
    return output
}

export type ContextMessage = {
    // 'system' is used for the post-history note (chat memory and rules) inserted mid-chat
    role: 'user' | 'assistant' | 'system'
    content: string
    attachments?: ContentTypes[]
}

export type CompletionState =
    | 'initial_truncated'
    | 'initial_completed'
    | 'loader_completed'
    | 'loader_truncated'

export const collectContext = async (params: ContextBuilderParams & { mode: 'chat' | 'text' }) => {
    const {
        apiConfig,
        messages,
        character,
        user,
        cache,
        instruct,
        tokenizer,
        chatTokenizer,
        maxLength,
        bypassContextLength,
        messageLoader,
        mode,
        dataSources,
        chatMemory,
        chatPreset,
    } = params

    const delta = performance.now()

    const { characterCache, userCache, instructCache } = cache

    const usePrefix = mode === 'text'
    const useSuffix = false

    const sortedDataSources = [...(dataSources ?? [])].sort((a, b) => a.priority - b.priority)

    const preset = await resolveChatPreset(chatPreset, instruct, tokenizer)

    let { systemPrompt, systemPromptLength } = getSystemPrompt({
        instruct,
        user,
        character,
        userCache,
        characterCache,
        instructCache,
        usePrefix,
        useSuffix,
        preset,
    })

    const reservedBudget = sortedDataSources.reduce((acc, curr) => acc + curr.tokenBudget, 0)

    let totalLength = systemPromptLength + reservedBudget

    // post-history note (chat memory + card rules) is reserved before history is added
    const postHistory = await getPostHistoryNote({
        instruct,
        character,
        characterCache,
        chatMemory,
        tokenizer,
        preset,
    })
    // strict alternation cannot carry a mid-chat system message, so it implies the
    // note goes into the latest user message
    const noteInUser = instruct.note_in_user_message || instruct.strict_alternation
    if (postHistory.text) {
        totalLength += postHistory.length
        if (mode === 'text' && !noteInUser) {
            totalLength += instructCache.system_prefix_length + instructCache.system_suffix_length
        }
    }

    let hasImage = false
    // number of messages already collected, newest first, used for the attachment depth
    let processedCount = 0
    // true when the newest message is a character reply being continued
    let lastMessageIncluded = false
    let completionState: CompletionState = 'initial_completed'

    const contextMessages: ContextMessage[] = []

    /**
     * Shared processor
     */
    const processMessage = async (
        message: ChatEntry,
        index: number,
        isLast: boolean
    ): Promise<boolean> => {
        const swipe = message.swipes[0]
        if (!swipe) {
            Logger.errorToast(t('generation.warn.entryWithoutValidSwipeFound'))
            return false
        }
        const swipeLen = await chatTokenizer(message, index)

        const timestamp = instruct.timestamp
            ? `[${swipe.send_date.toDateString()} ${swipe.send_date.toLocaleTimeString()}]\n`
            : ''

        const name = instruct.names ? `${message.name}: ` : ''

        const timestampLen = instruct.timestamp ? await tokenizer(timestamp) : 0
        const nameLen = instruct.names ? await tokenizer(name) : 0

        let instructLen = 0
        if (mode === 'text') {
            instructLen += message.is_user
                ? instructCache.input_prefix_length
                : instructCache.output_prefix_length
        }

        const shardLen = swipeLen + timestampLen + nameLen + instructLen

        // HARD LIMIT (always enforced)
        if (totalLength + shardLen > maxLength && !bypassContextLength) {
            return false
        }

        if (!swipe.swipe && isLast) {
            return true
        }

        const role: 'user' | 'assistant' = message.is_user ? 'user' : 'assistant'

        let content = replaceMacrosInternal(`${timestamp}${name}${swipe.swipe}`, instruct)

        let attachments: ContentTypes[] | undefined

        if (mode === 'chat' && apiConfig.request.completionType.type === 'chatCompletions') {
            // attachments beyond the depth are dropped so an old image is not re-encoded
            // on every turn; the text history still mentions that one was there
            const withinDepth =
                instruct.attachment_depth <= 0 || processedCount < instruct.attachment_depth
            const result = getValidAttachments(
                message,
                apiConfig.request.completionType,
                instruct,
                hasImage,
                withinDepth
            )

            hasImage = result.hasImageNew
            if (result.omitted > 0) {
                content += t('generation.attachmentsOmitted', { count: result.omitted })
            }

            if (result.attachments.length > 0) {
                attachments = await Promise.all(
                    result.attachments.map(async (item) => {
                        const base64 = await readBase64Async(item.uri)

                        if (item.type === 'image') {
                            return {
                                type: 'image_url',
                                image_url: {
                                    url: `data:${item.mime_type};base64,${base64}`,
                                },
                            }
                        }

                        return {
                            type: 'input_audio',
                            input_audio: {
                                data: base64,
                                format: item.mime_type.split('/')[1],
                            },
                        }
                    })
                )
            }
        }

        contextMessages.push({ role, content, attachments })
        if (isLast && role === 'assistant') lastMessageIncluded = true

        totalLength += shardLen
        processedCount++

        return true
    }

    // initial message collector
    let index = messages.length - 1

    for (let i = messages.length - 1; i >= 0; i--) {
        const success = await processMessage(messages[i], index, i === messages.length - 1)

        if (!success) {
            completionState = 'initial_truncated'
            break
        }
        index--
    }

    if (messageLoader && completionState === 'initial_completed') {
        let page = messageLoader.initialPage
        while (true) {
            let batch: ChatEntry[] | null = null

            batch = await messageLoader.retrieve(page)

            for (let i = batch.length - 1; i >= 0; i--) {
                const success = await processMessage(batch[i], -1, false)
                if (!success) {
                    completionState = 'loader_truncated'
                    break
                }
            }

            if (completionState === 'loader_truncated') break

            if (batch.length < messageLoader.pageSize || batch.length === 0) {
                completionState = 'loader_completed'
                break
            }

            page++
        }
    }

    if (completionState === 'initial_truncated') {
        warnTruncated(index + 1, messages.length, totalLength, maxLength)
    }

    // the note goes just before the latest user message so the model answers the user
    // rather than the note; when continuing a partial reply it stays before that reply
    if (postHistory.text) {
        // contextMessages is newest-first: the first user role found is the latest user message
        const latestUser = contextMessages.findIndex((item) => item.role === 'user')
        if (noteInUser && latestUser !== -1) {
            // templates such as Gemma reject a system message mid-chat, so the note is
            // prepended to the latest user message instead
            const target = contextMessages[latestUser]
            target.content = `${postHistory.text}\n\n${target.content}`
        } else {
            // inserting after the latest user message puts the note chronologically before it
            const insertAt = latestUser !== -1 ? latestUser + 1 : lastMessageIncluded ? 1 : 0
            contextMessages.splice(insertAt, 0, { role: 'system', content: postHistory.text })
        }
    }

    const lastMessageReached =
        completionState === 'loader_completed' || completionState === 'initial_completed'

    const pendingInsertions: DataSourceResult[] = []

    const runDataSources = async (sources: DataSource[]) => {
        for (const source of sources) {
            const remaining = maxLength - totalLength
            const opportunistic = source.tokenBudget === 0
            if (remaining <= 0 && opportunistic) {
                Logger.info(`[DataSource:${source.name}] skipped (no remaining budget)`)
                continue
            }

            const budget = opportunistic ? remaining : source.tokenBudget

            const results = await source.retrieve(
                params,
                contextMessages,
                maxLength,
                totalLength,
                budget,
                lastMessageReached
            )
            for (const result of results) {
                pendingInsertions.push(result)
                totalLength += result.tokenLength

                Logger.info(
                    `[DataSource:${source.name}] inserted ${result.tokenLength} tokens from ${result.source}`
                )
            }
        }
    }

    await runDataSources(sortedDataSources)

    const insertMessage = (index: number, message: ContextMessage) => {
        contextMessages.splice(index, 0, message)
    }

    for (const insertion of pendingInsertions) {
        const syntheticMessage: ContextMessage = {
            role: 'user',
            content: insertion.content,
        }

        if (insertion.position.type === 'relative') {
            switch (insertion.position.location) {
                case 'afterLast':
                    contextMessages.push(syntheticMessage)
                    break

                case 'beforeLast':
                    contextMessages.splice(
                        Math.max(contextMessages.length - 1, 0),
                        0,
                        syntheticMessage
                    )
                    break

                case 'afterSystem':
                    systemPrompt += '\n' + insertion.content
                    break
            }

            continue
        }

        const index = insertion.position.location

        if (index >= contextMessages.length) {
            contextMessages.unshift(syntheticMessage)
        } else {
            insertMessage(index, syntheticMessage)
        }
    }

    Logger.info(`Approximate Context Size: ${totalLength}`)
    Logger.info(`${(performance.now() - delta).toFixed(2)}ms`)

    if (contextMessages.length === 0) warnNoMessages()
    return {
        systemPrompt: systemPrompt,
        messages: contextMessages.reverse(),
    }
}

export const buildChatCompletionContext = async (params: ContextBuilderParams) => {
    if (params.apiConfig.request.completionType.type !== 'chatCompletions') return

    const { systemPrompt, messages } = await collectContext({
        ...params,
        mode: 'chat',
    })

    const feats = params.apiConfig.request.completionType

    let payload: Message[] = [
        {
            role: feats.systemRole,
            [feats.contentName]: replaceMacrosInternal(systemPrompt, params.instruct),
        },
    ]

    const roleName = (role: ContextMessage['role']) => {
        if (role === 'user') return feats.userRole
        if (role === 'assistant') return feats.assistantRole
        return feats.systemRole
    }

    for (const msg of messages) {
        if (msg.attachments?.length) {
            payload.push({
                role: roleName(msg.role),
                [feats.contentName]: [{ type: 'text', text: msg.content }, ...msg.attachments],
            })
        } else {
            payload.push({
                role: roleName(msg.role),
                [feats.contentName]: msg.content,
            })
        }
    }
    if (params.instruct.strict_alternation) payload = enforceAlternation(payload, feats)
    printContext(
        JSON.stringify(
            payload.map((item) => {
                const content = item[feats.contentName]
                if (typeof content === 'string') return item
                else return content.filter((item) => item.type === 'text')
            })
        )
    )
    return payload
}

export const buildTextCompletionContext = async (params: ContextBuilderParams) => {
    const { systemPrompt, messages } = await collectContext({
        ...params,
        mode: 'text',
    })

    const { instruct } = params
    let hasMedia = false
    let output = systemPrompt + instruct.system_suffix
    let len = 0
    let endedAtAssistant = false
    for (const msg of messages) {
        let outPrefix = instruct.output_prefix
        let outSuffix = instruct.output_suffix
        if (len === messages.length - 1 && msg.role === 'assistant') {
            outPrefix = instruct.last_output_prefix
            outSuffix = ''
            endedAtAssistant = true
        }

        let shard = ''
        if (msg.role === 'system') {
            // the post-history note is wrapped like the system prompt
            shard = instruct.system_prefix + msg.content + instruct.system_suffix
        } else {
            shard = msg.role === 'user' ? instruct.input_prefix : outPrefix
            shard += msg.content
            shard += msg.role === 'user' ? instruct.input_suffix : outSuffix
        }

        if (instruct.wrap && !endedAtAssistant) shard += '\n'
        if (!hasMedia && msg.attachments?.length) {
            hasMedia = true
        }
        output += shard
        len++
    }
    if (hasMedia) {
        Logger.errorToast('Text Completions does not support multimodal')
        if (useAppModeStore.getState().appMode === 'local') {
            Logger.warn(
                "[HINT] You probably have built-in templates disabled. Enable it in 'Formatting > Use Built-In Local Model' template"
            )
        }
    }

    if (!endedAtAssistant) output += instruct.last_output_prefix
    const result = replaceMacrosInternal(output, instruct)
    printContext(result)
    return result
}

const thinkRule = buildThinkRules()

/**
 * Reshapes a chat-completion payload for templates that require strict
 * user/assistant alternation (Gemma):
 * - a single leading system message is kept
 * - any other system message becomes part of a user turn
 * - consecutive same-role messages are merged into one
 * - a leading assistant message (the character's greeting) gets a placeholder user turn
 */
const enforceAlternation = (
    output: Message[],
    roles: { userRole: string; systemRole: string; assistantRole: string; contentName: string }
): Message[] => {
    const { userRole, systemRole, assistantRole, contentName } = roles
    const hasSystem = output.length > 0 && output[0].role === systemRole
    const head = hasSystem ? [output[0]] : []
    const rest = hasSystem ? output.slice(1) : output

    const mergeContent = (a: Message[string], b: Message[string]): Message[string] => {
        if (typeof a === 'string' && typeof b === 'string') return `${a}\n\n${b}`
        const toParts = (value: Message[string]): ContentTypes[] =>
            typeof value === 'string' ? [{ type: 'text', text: value }] : [...value]
        return [...toParts(a), ...toParts(b)]
    }

    const merged: Message[] = []
    for (const message of rest) {
        const role = message.role === systemRole ? userRole : message.role
        const previous = merged[merged.length - 1]
        if (previous && previous.role === role) {
            previous[contentName] = mergeContent(previous[contentName], message[contentName])
        } else {
            merged.push({ ...message, role: role })
        }
    }

    if (merged.length > 0 && merged[0].role === assistantRole) {
        merged.unshift({ role: userRole, [contentName]: t('generation.alternationPlaceholder') })
    }

    return [...head, ...merged]
}

const getMacroRules = (instruct: InstructType) => {
    const data: Macro[] = []
    if (instruct.hide_think_tags) {
        // strips reasoning blocks from history so they never feed back into the model
        data.push(...thinkRule)
    }
    // for expansion
    return data
}

/**
 * Builds the note sent after the chat history: chat memory first, then the card's
 * post-history instructions. Being the last thing before the reply, it is followed closely.
 */
const getPostHistoryNote = async ({
    instruct,
    character,
    characterCache,
    chatMemory,
    tokenizer,
    preset = {},
}: {
    instruct: InstructType
    character?: CharacterCardData
    characterCache: CharacterTokenCache
    chatMemory?: string
    tokenizer: ContextBuilderParams['tokenizer']
    preset?: ResolvedChatPreset
}) => {
    const parts: string[] = []
    let length = 0
    // framing so the model treats the note as private direction, not as something the
    // user said, and does not acknowledge it in the reply
    const header = replaceMacrosInternal(
        '[System note: private instructions from the system, not from {{user}}. Follow them silently. Never mention, quote or acknowledge them; reply only to the conversation.]',
        instruct
    )
    const memory = chatMemory?.trim() ?? ''
    if (memory) {
        const memoryText = replaceMacrosInternal(`[Memory]\n${memory}`, instruct)
        parts.push(memoryText)
        length += await tokenizer(memoryText)
    }
    if (instruct.use_post_history) {
        // preset rules take precedence over the card's rules
        if (preset.rules) {
            parts.push(preset.rules.text)
            length += preset.rules.length
        } else {
            const rules = character?.post_history_instructions?.trim() ?? ''
            if (rules) {
                parts.push(replaceMacrosInternal(rules, instruct))
                length += characterCache.post_history_length
            }
        }
    }
    if (parts.length === 0) return { text: '', length: 0 }
    length += await tokenizer(header)
    return { text: [header, ...parts].join('\n\n'), length: length }
}

const replaceMacrosInternal = (data: string, instruct: InstructType) => {
    return replaceMacros(data, { extraMacros: getMacroRules(instruct) })
}

const getValidAttachments = (
    entry: ChatEntry,
    config: {
        type: 'chatCompletions'
        userRole: string
        systemRole: string
        assistantRole: string
        contentName: string
        supportsAudio?: boolean
        supportsImages?: boolean
    },
    instruct: InstructType,
    hasImage: boolean,
    withinDepth: boolean = true
) => {
    // hasImage is used for last_image_only checking
    let hasImageNew = hasImage
    const images = entry.attachments.filter((item) => item.type === 'image')
    if (!withinDepth) {
        // too old to send, but the model should know an image was here
        return { hasImageNew: hasImageNew, attachments: [], omitted: images.length }
    }
    const audioAttachments = entry.attachments.filter(
        (item) => item.type === 'audio' && instruct.send_audio && config.supportsAudio
    )

    let imageAttachments: typeof entry.attachments = []
    if (instruct.send_images && config.supportsImages) {
        if (instruct.last_image_only && images.length > 0) {
            if (!hasImageNew) {
                hasImageNew = true
                imageAttachments = [images[0]]
            }
        } else {
            imageAttachments = images
        }
    }
    const attachments = [...audioAttachments, ...imageAttachments]
    return { hasImageNew: hasImageNew, attachments: attachments, omitted: 0 }
}

export const getSystemPrompt = ({
    instruct,
    user,
    character,
    userCache,
    characterCache,
    instructCache,
    usePrefix = true,
    useSuffix = true,
    preset = {},
}: {
    instruct: InstructType
    user?: CharacterCardData
    character?: CharacterCardData
    userCache: CharacterTokenCache
    characterCache: CharacterTokenCache
    instructCache: InstructTokenCache
    usePrefix?: boolean
    useSuffix?: boolean
    preset?: ResolvedChatPreset
}) => {
    let systemPrompt = instruct.system_prompt_format
    if (systemPrompt === undefined) {
        Logger.warn('System Prompt Format is undefined, falling back to default')
        systemPrompt = defaultSystemPromptFormat
    }
    if (systemPrompt === '') {
        Logger.warn('System Prompt Format is blank')
    }

    let systemPromptLength = 0

    // a card may define its own system prompt, {{original}} inserts the instruct's prompt
    const instructSystemPrompt = instruct.system_prompt ?? ''
    const cardSystemPrompt = character?.system_prompt?.trim() ?? ''
    const useCardPrompt = instruct.use_card_system_prompt && cardSystemPrompt.length > 0
    let finalSystemPrompt = useCardPrompt
        ? cardSystemPrompt.replaceAll('{{original}}', instructSystemPrompt)
        : instructSystemPrompt
    let finalSystemPromptLength = useCardPrompt
        ? characterCache.system_prompt_length +
          (cardSystemPrompt.includes('{{original}}') ? instructCache.system_prompt_length : 0)
        : instructCache.system_prompt_length

    // an active chat preset has the final say on the system prompt and persona
    if (preset.system_prompt) {
        const base = finalSystemPrompt
        finalSystemPrompt = preset.system_prompt.text.replaceAll('{{original}}', base)
        finalSystemPromptLength =
            preset.system_prompt.length +
            (preset.system_prompt.text.includes('{{original}}') ? finalSystemPromptLength : 0)
    }
    const finalUserDesc = preset.persona ? preset.persona.text : (user?.description ?? '')
    const finalUserDescLength = preset.persona
        ? preset.persona.length
        : userCache.description_length

    // Labels keep the two identities apart. Small models otherwise read the user's
    // description as part of the character. Empty sections get no label.
    const LABEL_LENGTH = 12
    const labelled = (title: string, text: string, length: number) => {
        if (!instruct.label_sections || !text.trim()) return { text, length }
        return { text: `${title}\n${text}`, length: length + LABEL_LENGTH }
    }
    const characterDesc = labelled(
        '[About {{char}}]',
        character?.description ?? '',
        characterCache.description_length
    )
    const personality = labelled(
        "[{{char}}'s personality]",
        instruct.personality ? (character?.personality ?? '') : '',
        instruct.personality ? characterCache.personality_length : 0
    )
    const scenario = labelled(
        '[Scenario]',
        instruct.scenario ? (character?.scenario ?? '') : '',
        instruct.scenario ? characterCache.scenario_length : 0
    )
    const userDesc = labelled(
        '[About {{user}}, the person {{char}} is talking to. {{user}} is not {{char}}.]',
        finalUserDesc,
        finalUserDescLength
    )

    const macros = [
        {
            macro: '{{system_prefix}}',
            value: (usePrefix && instruct.system_prefix) || '',
            length: instructCache.system_prefix_length,
        },
        {
            macro: '{{system_suffix}}',
            value: (useSuffix && instruct.system_suffix) || '',
            length: instructCache.system_suffix_length,
        },
        {
            macro: '{{system_prompt}}',
            value: finalSystemPrompt,
            length: finalSystemPromptLength,
        },
        {
            macro: '{{character_desc}}',
            value: characterDesc.text,
            length: characterDesc.length,
        },
        {
            macro: '{{user_desc}}',
            value: userDesc.text,
            length: userDesc.length,
        },
        {
            macro: '{{personality}}',
            value: personality.text,
            length: personality.length,
        },
        {
            macro: '{{scenario}}',
            value: scenario.text,
            length: scenario.length,
        },
    ]
    macros.forEach((m) => {
        systemPrompt = systemPrompt.replaceAll(m.macro, m.value)
        systemPromptLength += m.length
    })
    return { systemPrompt, systemPromptLength }
}

/**
 * Logs how much history was dropped, and toasts when almost nothing fits so the
 * user learns why the model has "forgotten" the conversation.
 */
const warnTruncated = (dropped: number, total: number, used: number, maxLength: number) => {
    if (dropped <= 0) return
    const kept = total - dropped
    Logger.warn(
        `Context full: ${dropped} of ${total} messages dropped (${used}/${maxLength} tokens). Raise the context length or shorten the card.`
    )
    if (kept <= 2) {
        Logger.warnToast(t('generation.warn.contextFull', { count: kept }))
    }
}

const warnNoMessages = () => {
    Logger.warnToast(t('generation.warn.noMessagesAddedCheckLogs'))
    Logger.warn(
        'No messages were added to the context. This can be caused by:\n- Generated Length is too high, lower it in Formatting\n- Your context length is too low\n- Your first message is too long'
    )
}
