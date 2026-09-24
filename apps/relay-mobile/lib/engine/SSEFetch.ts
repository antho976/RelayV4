import { fetch } from 'expo/fetch'

import { Logger } from '@lib/state/Logger'
type SSEValues = {
    endpoint: string
    method: 'POST' | 'GET'
    body: string
    headers: any
}

export class SSEFetch {
    private abortController: AbortController = new AbortController()
    private onEvent = (data: string) => {}
    private onError = () => {}
    private onClose = () => {}
    private closeStream = () => {}
    private cancelled = false
    public abort() {
        this.abortController.abort()

        this.closeStream()
        this.closeStream = () => {
            this.cancelled = true
        }
    }

    public async start(values: SSEValues) {
        this.abortController = new AbortController()
        const body = values.method === 'POST' ? { body: values.body } : {}
        this.cancelled = false
        // onError already ends the request, so onClose must not end it a second time
        let failed = false
        try {
            const res = await fetch(values.endpoint, {
                signal: this.abortController.signal,
                method: values.method,
                headers: values.headers,
                ...body,
            })
            if (res.status !== 200 || !res.body) {
                Logger.error('Status ' + res.status)
                Logger.error(await res.text())
                failed = true
                return this.onError()
            }
            const reader = res.body.getReader()
            this.closeStream = () => {
                try {
                    reader.cancel()
                    this.cancelled = true
                } catch {}
            }
            // a network chunk can end mid-line or mid-character, so decoding is streamed
            // and the unfinished last line waits for the next chunk
            const decoder = new TextDecoder()
            let pending = ''
            while (true) {
                const { value, done } = await reader.read()
                if (this.cancelled) break
                pending += done ? decoder.decode() : decoder.decode(value, { stream: true })
                const lines = pending.split(/\r?\n/)
                pending = done ? '' : (lines.pop() ?? '')
                parseSSE(lines).forEach((item) => this.onEvent(item))
                if (done) break
            }
        } catch (e) {
            if (this.abortController.signal.aborted) {
                Logger.debug('Abort caught')
            }
            Logger.error('Request Failed: ' + e)
        } finally {
            if (!failed) this.onClose()
        }
    }

    public setOnEvent(callback: (data: string) => void) {
        this.onEvent = callback
    }

    public setOnError(callback: () => void) {
        this.onError = callback
    }

    public setOnClose(callback: () => void) {
        this.onClose = callback
    }
}

function parseSSE(lines: string[]) {
    const output: string[] = []
    for (const line of lines) {
        // For some APIs like Ollama, they use a ndjson stream
        if (line.startsWith('{')) {
            try {
                JSON.parse(line)
                output.push(line)
            } catch {
                continue
            }
        }
        const colonIndex = line.indexOf(':')
        if (colonIndex === 0) continue
        const field = colonIndex > 0 ? line.slice(0, colonIndex).trim() : line.trim()
        const value = colonIndex > 0 ? line.slice(colonIndex + 1).trim() : ''
        if (field !== 'data' || value.startsWith('[DONE]')) continue
        output.push(value)
    }

    return output
}
