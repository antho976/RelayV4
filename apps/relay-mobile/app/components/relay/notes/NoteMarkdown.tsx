import React from 'react'
import Markdown from 'react-native-markdown-display'

import { MarkdownStyle } from '@lib/markdown/Markdown'

/**
 * A note's body rendered the way chat renders markdown: the app's own rules (code fences with
 * copy, safe links only, LaTeX) and the reader's chosen text size.
 */
const NoteMarkdown: React.FC<{ text: string }> = ({ text }) => {
    const { markdown, rules, style } = MarkdownStyle.useCustomFormatting()
    return (
        <Markdown mergeStyle={false} markdownit={markdown} rules={rules} style={style}>
            {text}
        </Markdown>
    )
}

export default NoteMarkdown
