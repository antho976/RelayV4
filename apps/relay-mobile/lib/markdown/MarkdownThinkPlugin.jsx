import { thinkTags } from './ThinkTags'

export default thinkPlugin

function thinkPlugin(md) {
    // listed as a terminator so a tag on the line after text still ends the paragraph
    md.block.ruler.before(
        'paragraph',
        'think',
        function (state, startLine, endLine, silent) {
            // indented four or more spaces is a code block
            if (state.sCount[startLine] - state.blkIndent >= 4) return false
            const tagLine = startLine
            let line = state.src.slice(
                state.bMarks[tagLine] + state.tShift[tagLine],
                state.eMarks[tagLine]
            )
            const activeTag = thinkTags.find((tag) => tag.open.test(line))
            if (!activeTag) return false
            if (silent) return true

            let trailing
            let hasCloseTag = false
            let nextLine = tagLine + 1
            const contentLines = []

            // 📦 Inline content after opening tag
            const inlineContent = line.slice(line.match(activeTag.open)[0].length)

            if (inlineContent.trim().length) {
                const closeIndex = inlineContent.indexOf(activeTag.close)

                if (closeIndex !== -1) {
                    contentLines.push(inlineContent.slice(0, closeIndex))

                    if (closeIndex + activeTag.close.length < inlineContent.length) {
                        trailing = inlineContent.slice(closeIndex + activeTag.close.length)
                    }

                    hasCloseTag = true
                    nextLine--
                } else {
                    contentLines.push(inlineContent)
                }
            }

            // 🔄 Accumulate until closing tag
            while (!hasCloseTag && nextLine < endLine) {
                line = state.src.slice(
                    state.bMarks[nextLine] + state.tShift[nextLine],
                    state.eMarks[nextLine]
                )

                const closeIndex = line.indexOf(activeTag.close)

                if (closeIndex !== -1) {
                    if (closeIndex > 0) {
                        contentLines.push(line.slice(0, closeIndex))
                    }

                    if (closeIndex + activeTag.close.length < line.length) {
                        trailing = line.slice(closeIndex + activeTag.close.length)
                    }

                    hasCloseTag = true
                    break
                }

                contentLines.push(line.trim())
                nextLine++
            }

            state.line = hasCloseTag ? nextLine + 1 : endLine

            // 🧱 Create token
            const token = state.push('think', '', 0)
            token.hidden = true
            token.map = [tagLine, state.line]
            token.info = hasCloseTag

            token.children = md.parse(contentLines.join('\n').trim(), state.env)

            // 🧾 Trailing content
            if (trailing) {
                const textToken = state.push('paragraph', '', 0)
                textToken.children = md.parse(trailing, state.env)
            }

            return true
        },
        { alt: ['paragraph', 'reference', 'blockquote', 'list'] }
    )
}
