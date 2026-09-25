/**
 * Typing into an agent's terminal from the phone, so that Enter submits.
 *
 * A phone sends a whole line at once. `text + '\r'` in one write reaches the PTY as one read,
 * and a TUI that watches for pastes (Claude Code, Codex: bracketed paste on, plus a burst
 * heuristic for terminals without it) takes a multi-character burst that ends in CR as pasted
 * text: the CR becomes a newline in its prompt and nothing is submitted. A person at a keyboard
 * never produces that burst, their Enter comes as its own read. So the text goes first, and
 * Enter follows in its own `session.input` once the text has been written and the program has
 * had a moment to read it.
 *
 * Several lines go as one bracketed paste (ESC [200~ … ESC [201~, newlines as CR, the way a
 * desktop terminal pastes) when the program has asked for bracketed paste, then Enter: the
 * agent gets one multi-line message instead of one message per line. A program that has not
 * asked for it (a plain shell script, `read`) gets the lines one at a time, each with its Enter.
 */

/** Between the text and its Enter: longer than one read, shorter than a person notices. */
export const ENTER_DELAY_MS = 90

export type TerminalSender = {
    /** Write and wait for the engine to have written it to the PTY. */
    write: (data: string) => Promise<void>
    /** Write without waiting (keystrokes). */
    key: (data: string) => void
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms))

export const PASTE_START = '\x1b[200~'
export const PASTE_END = '\x1b[201~'

/** Send `text` and submit it. Empty text is a bare Enter. */
export const submitText = async (
    send: TerminalSender,
    text: string,
    opts: { bracketedPaste: boolean; delayMs?: number }
) => {
    const delay = opts.delayMs ?? ENTER_DELAY_MS
    if (!text) {
        send.key('\r')
        return
    }
    const lines = text.replace(/\r\n?/g, '\n').split('\n')
    if (lines.length === 1) {
        await send.write(text)
        await sleep(delay)
        send.key('\r')
        return
    }
    if (opts.bracketedPaste) {
        // Pasted text must not end the paste early: a stray end marker inside it is dropped.
        const body = lines.join('\r').split(PASTE_END).join('')
        await send.write(PASTE_START + body + PASTE_END)
        await sleep(delay)
        send.key('\r')
        return
    }
    for (const line of lines) {
        if (line) {
            await send.write(line)
            await sleep(delay)
        }
        await send.write('\r')
        await sleep(delay)
    }
}

/** A key that answers a prompt (`y`, `n`): the key, then its Enter on its own. */
export const answerKey = async (send: TerminalSender, key: string, delayMs = ENTER_DELAY_MS) => {
    await send.write(key)
    await sleep(delayMs)
    send.key('\r')
}
