import { filterRegex, useTextFilterStore } from '@lib/hooks/TextFilter'

export type Macro = {
    macro: string | RegExp
    value: string
}

type ReplaceMacroOptions = {
    extraMacros?: Macro[]
}

const weekday = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']

const getDefaultMacros = () => {
    const time = new Date()
    let rules: Macro[] = [
        { macro: '{{time}}', value: time.toLocaleTimeString() },
        { macro: '{{date}}', value: time.toLocaleDateString() },
        { macro: '{{weekday}}', value: weekday[time.getDay()] },
    ]

    const filterState = useTextFilterStore.getState()
    if (!filterState.sendFilteredText) {
        rules = [
            ...rules,
            ...filterState.filter
                .filter(Boolean)
                .map((item) => ({ macro: filterRegex(item), value: '' })),
        ]
    }
    return rules
}

export const replaceMacroBase = (
    text: string,
    options: ReplaceMacroOptions = { extraMacros: [] }
) => {
    let newtext: string = text
    const rules = [...getDefaultMacros(), ...(options?.extraMacros ?? [])]
    // a function replacer keeps `$&` or `$$` in card text as written
    for (const rule of rules) newtext = newtext.replaceAll(rule.macro, () => rule.value)
    return newtext
}
