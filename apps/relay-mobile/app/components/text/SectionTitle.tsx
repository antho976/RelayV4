import React, { ReactNode } from 'react'
import { TextProps, TextStyle } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

import TText from './TText'

const SectionTitle = ({
    children,
    style = undefined,
    visible = true,
    ...props
}: {
    props?: TextProps
    children?: ReactNode
    style?: TextStyle
    visible?: boolean
}) => {
    const { color, spacing } = Theme.useTheme()
    if (visible)
        return (
            <TText
                {...props}
                style={{
                    color: color.text._100,
                    fontFamily: 'serif',
                    fontSize: 19,
                    paddingTop: spacing.m,
                    paddingBottom: spacing.m,
                    ...style,
                }}>
                {children}
            </TText>
        )
}

export default SectionTitle
