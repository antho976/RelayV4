import Slider from '@react-native-community/slider'
import { useCallback, useState } from 'react'
import { StyleSheet, Text, TextInput, View } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

type ThemedSliderProps = {
    label: string
    value: number
    onValueChange: (value: number) => void
    min?: number
    max?: number
    step?: number
    precision?: number
    showInput?: boolean
    disabled?: boolean
}

const clamp = (val: number, min: number, max: number, precision: number) =>
    Math.min(Math.max(parseFloat(val?.toFixed(precision) ?? 0), min), max)

const ThemedSlider: React.FC<ThemedSliderProps> = ({
    label,
    value,
    onValueChange,
    min = 0,
    max = 1,
    step = 0,
    precision = 0,
    showInput = true,
    disabled = false,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    // the box shows value unless it is being typed in, so Reset and note switches reach it
    const [textValue, setTextValue] = useState<string | undefined>(undefined)

    const clampSlider = useCallback(
        (value: number) => clamp(value, min, max, precision),
        [min, max, precision]
    )

    const handleSliderChange = (v: number) => {
        if (!isNaN(clampSlider(v))) onValueChange(clampSlider(v))
    }

    const handleTextInputChange = (t: string) => {
        let v = 0
        setTextValue(t)
        v = parseFloat(t)
        if (!isNaN(v)) onValueChange(clampSlider(v))
    }

    const handleEndEdit = () => {
        if (textValue === undefined) return
        const v = parseFloat(textValue)
        if (!isNaN(v)) onValueChange(clampSlider(v))
        setTextValue(undefined)
    }

    return (
        <View style={{ alignItems: `center` }}>
            {label && (
                <Text style={disabled ? styles.itemNameDisabled : styles.itemName}>{label}</Text>
            )}
            <View style={styles.sliderContainer}>
                <Slider
                    disabled={disabled}
                    style={styles.slider}
                    step={step}
                    minimumValue={min}
                    maximumValue={max}
                    value={value}
                    onSlidingComplete={handleSliderChange}
                    minimumTrackTintColor={color.primary._400}
                    maximumTrackTintColor={color.neutral._600}
                    thumbTintColor={color.primary._500}
                />
                {showInput && (
                    <TextInput
                        editable={!disabled}
                        style={[
                            styles.textBox,
                            {
                                flex: value > 999999 ? 2 : 1.5,
                                borderColor: disabled ? color.neutral._700 : color.neutral._400,
                            },
                        ]}
                        value={textValue ?? clampSlider(value).toString()}
                        onFocus={() => setTextValue(clampSlider(value).toString())}
                        onChangeText={handleTextInputChange}
                        keyboardType="number-pad"
                        submitBehavior="blurAndSubmit"
                        multiline
                        numberOfLines={1}
                        onEndEditing={handleEndEdit}
                        onSubmitEditing={handleEndEdit}
                        onBlur={handleEndEdit}
                    />
                )}
            </View>
        </View>
    )
}

export default ThemedSlider

const useStyles = () => {
    const { color, spacing } = Theme.useTheme()
    return StyleSheet.create({
        itemName: {
            color: color.text._100,
        },

        itemNameDisabled: {
            color: color.text._700,
        },

        sliderContainer: {
            flexDirection: `row`,
        },

        slider: {
            flex: 9,
            height: 36,
        },

        textBox: {
            color: color.text._100,
            borderWidth: 1,
            borderRadius: spacing.l,
            textAlign: `center`,
        },
    })
}
