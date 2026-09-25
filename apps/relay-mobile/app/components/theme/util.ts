import { ThemeColor } from '@lib/theme/ThemeColor'

/** Relative luminance of a `#RGB` or `#RRGGBB` colour, from 0 (black) to 1 (white). */
export const luminance = (hex: string) => {
    const raw = hex.replace('#', '')
    const full = raw.length === 3 ? [...raw].map((c) => c + c).join('') : raw
    const [r, g, b] = [0, 2, 4].map((at) => {
        const channel = parseInt(full.slice(at, at + 2), 16) / 255
        return channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
    })
    return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** A theme is dark when its page background is; that is what a person sees first. */
export const isDarkTheme = (theme: ThemeColor) => luminance(theme.neutral._100) < 0.3
