import { Image, ImageProps } from 'expo-image'
import { useEffect, useState } from 'react'
import { Image as RNImage } from 'react-native'

interface ScaledImageProps extends ImageProps {
    uri: string
}

const ScaledImage: React.FC<ScaledImageProps> = ({ uri, style, ...rest }) => {
    const [aspectRatio, setAspectRatio] = useState(1)
    useEffect(() => {
        RNImage.getSize(
            uri,
            (width, height) => {
                setAspectRatio(width / height)
            },
            () => {
                setAspectRatio(1)
            }
        )
    }, [uri])
    return <Image {...rest} source={{ uri: uri }} style={[style, { aspectRatio: aspectRatio }]} />
}

export default ScaledImage
