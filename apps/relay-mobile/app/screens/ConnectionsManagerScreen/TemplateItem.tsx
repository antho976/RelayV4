import { useTranslation } from 'react-i18next'
import { Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import Alert from '@components/views/Alert'
import { APIConfiguration } from '@lib/engine/API/APIBuilder.types'
import { APIManager } from '@lib/engine/API/APIManagerState'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'
import { saveStringToDownload } from '@lib/utils/File'

type TemplateItemProps = {
    item: APIConfiguration
    index: number
}

const TemplateItem: React.FC<TemplateItemProps> = ({ item, index }) => {
    const { t } = useTranslation()
    const { color, spacing, borderWidth, fontSize } = Theme.useTheme()

    const removeTemplate = APIManager.useConnectionsStore((state) => state.removeTemplate)

    const handleDelete = () => {
        const users = APIManager.useConnectionsStore
            .getState()
            .values.filter((value) => value.configName === item.name)
        if (users.length > 0) {
            Alert.alert({
                title: t('connections.templates.delete.title'),
                description: t('connections.templates.delete.inUse', {
                    name: item.name,
                    connections: users.map((value) => value.friendlyName).join(', '),
                }),
                buttons: [{ label: t('common.actions.close') }],
            })
            return
        }
        Alert.alert({
            title: t('connections.templates.delete.title'),
            description: t('connections.templates.delete.description', { name: item.name }),
            buttons: [
                { label: t('common.actions.cancel') },
                {
                    label: t('connections.templates.delete.confirm'),
                    onPress: () => {
                        removeTemplate(index)
                    },
                    type: 'warning',
                },
            ],
        })
    }

    const handleExport = () => {
        saveStringToDownload(JSON.stringify(item), `${item.name}.json`, 'utf8').then(() => {
            Logger.infoToast(t('connections.templates.exported', { name: item.name }))
        })
    }

    return (
        <View
            style={{
                borderColor: color.neutral._300,
                borderWidth: borderWidth.m,
                flexDirection: 'row',
                justifyContent: 'space-between',
                alignItems: 'center',
                borderRadius: 22,
                flex: 1,
                paddingLeft: spacing.l,
                paddingRight: spacing.xl2,
                paddingVertical: spacing.xl,
            }}>
            <View style={{ flexDirection: 'row', alignItems: 'center' }}>
                <View style={{ marginLeft: spacing.xl }}>
                    <Text style={{ color: color.text._100, fontSize: fontSize.l }}>
                        {item.name}
                    </Text>
                </View>
            </View>
            <View style={{ flexDirection: 'row', alignItems: 'center' }}>
                <ThemedButton
                    onPress={handleDelete}
                    iconName="delete"
                    iconSize={24}
                    variant="critical"
                    buttonStyle={{ borderWidth: 0 }}
                />
                <ThemedButton
                    onPress={handleExport}
                    iconName="download"
                    iconSize={24}
                    variant="tertiary"
                />
            </View>
        </View>
    )
}

export default TemplateItem
