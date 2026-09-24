/**
 * Ask the on-device model what an agent has been doing, from the terminal text the phone
 * already holds. Nothing leaves the phone: the PC link brought the output here, and the local
 * model reads it here. Off when no model is loaded or a chat is generating.
 */
import { Llama } from '@lib/engine/Local/LlamaLocal'
import { useInference } from '@lib/state/Chat'

/** Roughly two screens of an agent's output: enough to say what it is doing, small enough to
 *  fit a phone-sized context with room for the answer. */
const MAX_INPUT_CHARS = 6000
const MAX_OUTPUT_TOKENS = 320

export const summarizeReason = (): string | undefined => {
    if (!Llama.useLlamaModelStore.getState().context) return 'Load a local model first (Models).'
    if (useInference.getState().nowGenerating) return 'A chat is generating right now.'
    return undefined
}

/** The summary that has the model now, and the number of the latest one asked for. */
let current: { id: number; done: Promise<unknown> } | undefined
let latest = 0

/**
 * True while a summary holds the local model. A chat started now would run on the same
 * context mid-completion.
 */
export const summarizing = (): boolean => current !== undefined

/**
 * Stream a summary of `terminalText` for the session `name`. Resolves with the full text;
 * `onToken` receives each piece as it arrives. One summary runs at a time: a new one stops
 * the last and waits for the model to be free, and a run overtaken while it waited rejects
 * with `superseded` without touching the model.
 */
export const summarizeTerminal = async (
    name: string,
    terminalText: string,
    onToken: (piece: string) => void
): Promise<string> => {
    const id = ++latest
    while (current) {
        const previous = current
        await stopSummary().catch(() => {})
        await previous.done.catch(() => {})
        if (current === previous) current = undefined
    }
    if (id !== latest) throw new Error('superseded')
    const done = run(name, terminalText, onToken)
    current = { id: id, done: done }
    try {
        return await done
    } finally {
        if (current?.id === id) current = undefined
    }
}

const run = async (
    name: string,
    terminalText: string,
    onToken: (piece: string) => void
): Promise<string> => {
    const context = Llama.useLlamaModelStore.getState().context
    if (!context) throw new Error('No local model is loaded')
    const tail =
        terminalText.length > MAX_INPUT_CHARS ? terminalText.slice(-MAX_INPUT_CHARS) : terminalText
    const messages = [
        {
            role: 'system',
            content:
                'You read the terminal output of a coding agent and tell its operator, in plain language, what happened. Answer in at most six short bullet points: what the agent did, what it is doing now, and anything it is waiting on or asking. Do not repeat the output. Do not invent details that are not in it.',
        },
        {
            role: 'user',
            content: `Terminal output of the agent session "${name}" (most recent last):\n\n${tail}\n\nWhat has it done and what does it need from me?`,
        },
    ]
    let prompt: string
    const formatted = await context.getFormattedChat(messages, null, {
        jinja: true,
        add_generation_prompt: true,
        enable_thinking: false,
    })
    if (typeof formatted === 'string') prompt = formatted
    else prompt = formatted.prompt
    const result = await context.completion(
        {
            prompt: prompt,
            n_predict: MAX_OUTPUT_TOKENS,
            temperature: 0.3,
            top_p: 0.9,
            penalty_repeat: 1.05,
        },
        (data) => onToken(data.token)
    )
    return result.text
}

export const stopSummary = async () => {
    await Llama.useLlamaModelStore.getState().context?.stopCompletion()
}
