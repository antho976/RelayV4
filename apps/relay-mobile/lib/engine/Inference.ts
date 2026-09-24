import { t } from 'i18next'
import BackgroundService from 'react-native-background-actions'

import { ChatSwipe } from '@db/schema'
import { AppSettings } from '@lib/constants/GlobalValues'
import { isCloseThinkTag, isOpenThinkTag } from '@lib/markdown/ThinkTags'
import { useAppModeStore } from '@lib/state/AppMode'
import { Chats, useInference } from '@lib/state/Chat'
import { ChatPresets } from '@lib/state/ChatPresets'
import { Instructs } from '@lib/state/Instructs'
import { SamplersManager } from '@lib/state/SamplerState'
import { useTTSStore } from '@lib/state/TTS'
import { mmkv } from '@lib/storage/MMKV'

import { Characters } from '../state/Characters'
import { Logger } from '../state/Logger'
import { APIBuilderParams, buildAndSendRequest } from './API/APIBuilder'
import { APIConfiguration, APIValues } from './API/APIBuilder.types'
import { APIManager } from './API/APIManagerState'
import { getDataSources } from './DataSources'
import { Llama } from './Local/LlamaLocal'
import { localInference } from './LocalInference'
import { summarizing } from './Relay/Summarize'
import { Tokenizer } from './Tokenizer'

export async function regenerateResponse(swipe: ChatSwipe, regenCache: boolean = true) {
    Logger.info('Regenerate Response' + (regenCache ? '' : ' , Resetting Message'))

    let replacement = ''
    if (regenCache)
        replacement = swipe.reset_length ? swipe.swipe.substring(0, swipe.reset_length) : ''

    Chats.useChatState.getState().setBuffer({ data: replacement })
    await Chats.db.mutate.updateChatSwipe(swipe.id, replacement, {
        updateFinished: true,
        updateStarted: true,
        resetTimings: true,
    })

    await generateResponse(swipe.id)
}

export async function continueResponse(swipe: ChatSwipe) {
    Logger.info(`Continuing Response`)
    await Chats.db.mutate.updateSwipeResetLength(swipe.id, swipe.swipe.length)
    Chats.useChatState.getState().insertToBuffer(swipe.swipe)
    await generateResponse(swipe.id)
}

const completionTaskOptions = {
    taskName: 'relay_completion_task',
    taskTitle: 'Running completion...',
    taskDesc: 'Relay is running a completion task',
    taskIcon: {
        name: 'ic_launcher',
        type: 'mipmap',
    },
    color: '#403737',
    linkingURI: 'relayapp://',
    progressBar: {
        max: 1,
        value: 0,
        indeterminate: true,
    },
}

export async function generateResponse(swipeId: number) {
    if (useInference.getState().nowGenerating) {
        Logger.infoToast(t('generation.errors.generationAlreadyInProgress'))
        return
    }
    const appMode = useAppModeStore.getState().appMode
    // a PC-tab summary holds the on-device model until it finishes
    if (appMode === 'local' && summarizing()) {
        Logger.infoToast(
            'The on-device model is summarizing a PC session. Try again when it is done.'
        )
        return
    }
    useInference.getState().startGenerating(swipeId)
    Logger.info(`Obtaining response.`)

    if (appMode === 'local') {
        const fallback = localFallbackConnection()
        if (fallback) {
            // Local first: the phone answers itself whenever it can. Only when no model can
            // run does a configured API step in, and never silently.
            Logger.warnToast(`No local model can run — using "${fallback}" for this message`)
            await BackgroundService.start(chatInferenceStream, completionTaskOptions)
            return
        }
        await BackgroundService.start(localInference, completionTaskOptions)
    } else {
        await BackgroundService.start(chatInferenceStream, completionTaskOptions)
    }
}

/**
 * The name of the API connection to use instead of the local engine, or `undefined` when the
 * phone should (or must) answer itself: a model is loaded or set to auto-load, the fallback is
 * switched off in Privacy settings, or there is no active connection to fall back to.
 */
export const localFallbackConnection = (): string | undefined => {
    if (!mmkv.getBoolean(AppSettings.LocalFirstFallback)) return undefined
    const llama = Llama.useLlamaModelStore.getState()
    if (llama.context || llama.model) return undefined
    const autoLoad = mmkv.getBoolean(AppSettings.AutoLoadLocal)
    if (autoLoad && Llama.useLlamaPreferencesStore.getState().lastModel) return undefined
    const apiState = APIManager.useConnectionsStore.getState()
    const active = apiState.values.find((item, index) => index === apiState.activeIndex)
    if (!active) return undefined
    return active.friendlyName || active.configName
}

async function chatInferenceStream() {
    const fields = await obtainFields()
    const stop = () => useInference.getState().stopGenerating()
    if (!fields) {
        Logger.error('Chat Inference Failed')
        stop()
        return
    }
    fields.stopGenerating = stop
    let reasoningMode: 'structured' | 'raw' | null = null
    fields.onData = (output) => {
        if (!reasoningMode && output.type === 'reasoning') {
            Chats.useChatState.getState().insertToBuffer('<think>')
            reasoningMode = 'raw'
        }

        if (reasoningMode === 'raw' && output.type !== 'reasoning' && reasoningMode === 'raw') {
            Chats.useChatState.getState().insertToBuffer('</think>\n')
            reasoningMode = null
        }

        /**
         * This is a naive implementation that expects output tags to be full tokens
         * Most LLMs are trained so that think_start and think_end tokens are not composite
         */
        if (!reasoningMode && output.type === 'text' && isOpenThinkTag(output.content)) {
            reasoningMode = 'structured'
        }

        if (
            reasoningMode === 'structured' &&
            output.type === 'text' &&
            isCloseThinkTag(output.content)
        ) {
            reasoningMode = null
        }

        Chats.useChatState.getState().insertToBuffer(output.content)

        /**
         * considerations
         * - add tool calls
         */
        if (!reasoningMode) useTTSStore.getState().insertBuffer(output.content)
    }

    fields.onEnd = async () => {
        const chatId = Chats.useChatState.getState().id
        if (!chatId) return
        const chatName = await Chats.db.query.chatName(chatId)
        if (!mmkv.getBoolean(AppSettings.AutoGenerateTitle) || chatName !== 'New Chat') return
        Logger.info('Generating Title')
        titleGeneratorStream(chatId)
    }
    const request = await buildAndSendRequest(fields)
    useInference.getState().setAbort(() => {
        Logger.debug('Running Abort')
        request?.abort()
    })
    // the background task lives until the reply has finished streaming
    await request?.done
}

const titleGeneratorStream = async (chatId: number) => {
    const fields = await obtainFields()
    if (!fields) {
        Logger.error('Title Generation Failed')
        return
    }
    fields.samplers.genamt = 50
    fields.samplers.reasoning_max_tokens = 0
    fields.samplers.reasoning_effort = 'low'
    fields.samplers.reasoning_exclude = true
    // rules, memory and presets are irrelevant for title generation
    fields.instruct.use_post_history = false
    fields.chatMemory = ''
    fields.chatPreset = null
    let titleOutput = ''
    fields.onData = (output) => {
        if (output.type === 'text') titleOutput += output.content
    }

    fields.onEnd = () => {
        Logger.debug('Autogenerated Name: ' + titleOutput)
        if (titleOutput)
            Chats.db.mutate.renameChat(
                chatId,
                titleOutput
                    .trim()
                    .replace(/["'.*]/g, '')
                    .replace(/\b\w/g, (char) => char.toUpperCase())
            )
        else Logger.warn('Autogenerated name was blank.')
    }
    const entry = {
        id: -1,
        chat_id: -1,
        name: '',
        is_user: true,
        order: 0,
        swipe_id: 0,
        swipes: [
            {
                id: -1,
                entry_id: -1,
                swipe: 'Generate a short 2-4 word title for this chat. Only Respond with the title and nothing else.',
                send_date: new Date(),
                gen_started: new Date(),
                gen_finished: new Date(),
                timings: null,
                active: true,
                token_length: null,
                reset_length: null,
            },
        ],
        attachments: [],
    }
    fields.messages.push(entry)

    await buildAndSendRequest(fields)
}

const getModelContextLength = (config: APIConfiguration, values: APIValues): number | undefined => {
    const keys = config.model.contextSizeParser.split('.')
    const result = keys.reduce((acc, key) => acc?.[key], values.model)
    return Number.isInteger(result) ? result : undefined
}

// This is the 'big orchestrator' which compiles fields from
// the whole app to send inference requests
async function obtainFields(): Promise<APIBuilderParams | void> {
    try {
        const userState = Characters.useUserStore.getState()
        const characterState = Characters.useCharacterStore.getState()
        const apiState = APIManager.useConnectionsStore.getState()
        const instructState = Instructs.useInstruct.getState()

        const userCard = userState.card
        if (!userCard) {
            Logger.errorToast(t('generation.errors.noUser'))
            return
        }

        const characterCard = characterState.card
        if (!characterCard) {
            Logger.errorToast(t('generation.errors.noCharacter'))
            return
        }

        const chatId = Chats.useChatState.getState().id
        if (!chatId) {
            Logger.errorToast(t('generation.errors.noActiveChat'))
            return
        }

        const messages = (await Chats.db.query.chat(chatId))?.messages
        if (!messages) {
            Logger.errorToast(t('generation.errors.noChatFound'))
            return
        }
        const chatData = await Chats.db.query.chatShallow(chatId)

        const apiValues = apiState.values.find((item, index) => index === apiState.activeIndex)
        if (!apiValues) {
            Logger.warnToast(t('generation.errors.noActiveAPI'))
            return
        }

        const configs = apiState.getTemplates().filter((item) => item.name === apiValues.configName)

        const apiConfig = configs[0]
        if (!apiConfig) {
            Logger.errorToast(
                t('generation.errors.configurationNotFound', { name: apiValues?.configName })
            )
            return
        }
        const samplers = SamplersManager.getCurrentSampler()
        const modelLengthField = getModelContextLength(apiConfig, apiValues)
        const instructLength = samplers.max_length as number
        const modelLength = modelLengthField ?? (instructLength as number)
        const contextLength = apiConfig.model.useModelContextLength
            ? Math.min(modelLength, instructLength)
            : instructLength
        const length = Math.max(contextLength - (samplers.genamt as number), 0)

        let stopSequence = instructState.getStopSequence()
        const stopSequenceLimit = apiConfig.request.stopSequenceLimit
        if (stopSequenceLimit && stopSequence?.length > stopSequenceLimit) {
            stopSequence = stopSequence.slice(0, stopSequenceLimit)
            Logger.warn('Stop sequence length exceeds defined stopSequenceLimit')
        }
        const tokenizer = Tokenizer.getTokenizer()

        const dataSources = await getDataSources()

        return {
            apiConfig: Object.assign({}, apiConfig),
            apiValues: Object.assign({}, apiValues),
            onData: () => {},
            onEnd: () => {},
            instruct: instructState.replacedMacros(),
            samplers: Object.assign({}, samplers),
            character: Object.assign({}, characterCard),
            user: Object.assign({}, userCard),
            messages: [...messages],
            chatMemory: chatData?.memory ?? '',
            chatPreset: chatData
                ? await ChatPresets.db.query.activeForChat(chatId, chatData.active_preset_id)
                : null,
            stopSequence: stopSequence,
            stopGenerating: () => {},
            chatTokenizer: async (entry, index) => {
                // IMPORTANT - we use -1 for dummy entries
                if (entry.id === -1) return 0
                const [activeSwipe] = entry.swipes.filter((item) => item.active)
                if (!activeSwipe) return 0
                const tokenCount = activeSwipe.token_length ?? 0
                if (tokenCount === 0 && activeSwipe.swipe.length > 0) {
                    // assume that token length hasnt been calculated
                    const freshCount = await tokenizer(
                        activeSwipe.swipe,
                        entry.attachments.map((item) => item.uri)
                    )
                    await Chats.db.mutate.updateSwipeTokenLength(activeSwipe.id, freshCount)
                    return freshCount
                }
                return tokenCount
            },
            tokenizer: tokenizer,
            maxLength: length,
            cache: {
                // each cache holds its own card's token counts, keyed by the other party's name
                userCache: await userState.getCache(characterCard.name),
                characterCache: await characterState.getCache(userCard.name),
                instructCache: await instructState.getCache(characterCard.name, userCard.name),
            },
            dataSources: dataSources,
        }
    } catch (e) {
        Logger.stackTrace(e)
        Logger.errorToast(t('generation.errors.failedToOrchestrateRequestBuild'), e)
    }
}
