import { CameraCapturedPicture, CameraView, useCameraPermissions } from 'expo-camera'
import { useImperativeHandle, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import ThemedButton from '@components/buttons/ThemedButton'
import { Logger } from '@lib/state/Logger'

import BottomSheet, { BottomSheetRef, useBottomSheetRef } from './BottomSheet'

interface CameraSheetProps {
    ref: BottomSheetRef
    onTakePicture: (picture: CameraCapturedPicture) => void
}

const CameraSheet: React.FC<CameraSheetProps> = ({ ref, onTakePicture }) => {
    const { t } = useTranslation()
    const cameraRef = useRef<CameraView>(null)
    const sheetRef = useBottomSheetRef()
    const [, requestPermission] = useCameraPermissions()

    // the sheet only opens once the camera may be used
    useImperativeHandle(ref, () => ({
        open: async () => {
            const permission = await requestPermission().catch(() => undefined)
            if (!permission?.granted) {
                Logger.errorToast(t('chat.input.errors.cameraPermission'))
                return
            }
            sheetRef.current?.open()
        },
        close: () => sheetRef.current?.close(),
    }))

    const handleTakePicture = async () => {
        const camera = cameraRef.current
        if (!camera) return
        try {
            const picture = await camera.takePictureAsync()
            if (!picture) return
            onTakePicture(picture)
            sheetRef.current?.close()
        } catch (e) {
            Logger.errorToast(t('chat.input.errors.captureFailed'), e)
        }
    }

    return (
        <BottomSheet
            ref={sheetRef}
            sheetStyle={{ flex: 1, maxHeight: '70%', justifyContent: 'space-between' }}>
            <CameraView
                ref={cameraRef}
                autofocus="on"
                mode="picture"
                style={{ flex: 1, borderRadius: 8, marginBottom: 24 }}
            />
            <ThemedButton
                iconName="camera"
                accessibilityLabel={t('chat.input.actions.takePicture')}
                onPress={handleTakePicture}
            />
        </BottomSheet>
    )
}

export default CameraSheet
