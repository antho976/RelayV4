import { reloadAppAsync } from 'expo'
import { getDocumentAsync } from 'expo-document-picker'
import { File, Paths } from 'expo-file-system'
import React from 'react'
import { useTranslation } from 'react-i18next'
import { Text, View } from 'react-native'

import appConfig from '@appconfig'
import ThemedButton from '@components/buttons/ThemedButton'
import SectionTitle from '@components/text/SectionTitle'
import Alert from '@components/views/Alert'
import { migrateData } from '@db/dataMigrations'
import { sqliteDB } from '@db/db'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { saveToDownloads } from '../../../modules/relay-device'

const appVersion = appConfig.expo.version

// eslint-disable-next-line i18next/no-literal-string
const dbDir = Paths.document.uri + '/SQLite/'
// eslint-disable-next-line i18next/no-literal-string
const dbPath = dbDir + 'db.db'
// eslint-disable-next-line i18next/no-literal-string
const sqliteHeader = 'SQLite format 3\0'

const toPath = (uri: string) => decodeURIComponent(uri.replace('file://', ''))

const isSQLiteFile = (uri: string) => {
    try {
        const handle = new File(uri).open()
        try {
            return String.fromCharCode(...handle.readBytes(16)) === sqliteHeader
        } finally {
            handle.close()
        }
    } catch (e) {
        // eslint-disable-next-line i18next/no-literal-string
        Logger.error(`Could not read ${uri}: ${e}`)
        return false
    }
}

const deleteIfExists = (file: File) => {
    if (file.exists) file.delete()
}

const DatabaseSettings = () => {
    const { t } = useTranslation()
    const { color, spacing } = Theme.useTheme()

    // the file name carries the app version, which import checks
    const exportDB = async (notify: boolean = true) => {
        const date = new Date().toISOString().slice(0, 10)
        // eslint-disable-next-line i18next/no-literal-string
        const backup = new File(Paths.cache, `${appVersion}-relay-${date}.db`)
        try {
            deleteIfExists(backup)
            // a consistent copy that includes what is still in the WAL
            // eslint-disable-next-line i18next/no-literal-string
            await sqliteDB.execAsync(`VACUUM INTO '${toPath(backup.uri)}'`)
            await saveToDownloads(toPath(backup.uri))
            if (notify) Logger.infoToast(t('settings.database.toast.downloadOk'))
            return true
        } catch (e) {
            Logger.errorToast(t('settings.database.toast.downloadFailed', { error: `${e}` }))
            return false
        } finally {
            try {
                deleteIfExists(backup)
            } catch {}
        }
    }

    const importDB = async (uri: string, name: string) => {
        if (!isSQLiteFile(uri)) {
            Logger.errorToast(t('settings.database.toast.invalidFile'))
            return
        }

        const copyDB = async () => {
            if (!(await exportDB(false))) {
                Logger.errorToast(t('settings.database.toast.backupFailed'))
                return
            }
            // eslint-disable-next-line i18next/no-literal-string
            const staged = new File(dbPath + '.import')
            try {
                deleteIfExists(staged)
                await new File(uri).copy(staged)
                // eslint-disable-next-line i18next/no-literal-string
                if (!isSQLiteFile(staged.uri)) throw new Error('copied file is not a database')
                await sqliteDB.closeAsync()
            } catch (e) {
                try {
                    deleteIfExists(staged)
                } catch {}
                Logger.errorToast(t('settings.database.toast.importFailed', { error: `${e}` }))
                return
            }
            // the old connection is closed and checkpointed: drop its WAL before the swap
            try {
                // eslint-disable-next-line i18next/no-literal-string
                deleteIfExists(new File(dbPath + '-wal'))
                // eslint-disable-next-line i18next/no-literal-string
                deleteIfExists(new File(dbPath + '-shm'))
                await staged.move(new File(dbPath), { overwrite: true })
            } catch (e) {
                // eslint-disable-next-line i18next/no-literal-string
                Logger.error(`Database swap failed: ${e}`)
            }
            reloadAppAsync()
        }

        // older exports are a plain db.db with no version to compare
        const dbAppVersion = name.match(/^(\d+\.\d+\.\d+)-/)?.[1]
        if (dbAppVersion && dbAppVersion !== appVersion) {
            Alert.alert({
                title: t('settings.database.alert.versionMismatch.title'),
                description: t('settings.database.alert.versionMismatch.description', {
                    importedVersion: dbAppVersion,
                    currentVersion: appVersion,
                }),
                buttons: [
                    { label: t('common.actions.cancel') },
                    {
                        label: t('settings.database.alert.versionMismatch.confirm'),
                        onPress: copyDB,
                        type: 'warning',
                    },
                ],
            })
        } else copyDB()
    }

    const rerunMigrations = () => {
        Alert.alert({
            title: t('settings.database.alert.rerunMigrations.title'),
            description: t('settings.database.alert.rerunMigrations.description'),
            buttons: [
                { label: t('common.actions.cancel') },
                {
                    label: t('settings.database.alert.rerunMigrations.confirm'),
                    onPress: () => migrateData({ bypass: true }),
                    type: 'warning',
                },
            ],
        })
    }

    return (
        <View style={{ rowGap: 8 }}>
            <SectionTitle>{t('settings.database.title')}</SectionTitle>

            <Text
                style={{
                    color: color.text._500,
                    paddingBottom: spacing.xs,
                    marginBottom: spacing.m,
                }}>
                {t('settings.database.warningText')}
            </Text>
            <ThemedButton
                label={t('settings.database.exportButton')}
                variant="secondary"
                onPress={() => {
                    Alert.alert({
                        title: t('settings.database.alert.export.title'),
                        description: t('settings.database.alert.export.description'),
                        buttons: [
                            { label: t('common.actions.cancel') },
                            {
                                label: t('settings.database.alert.export.confirm'),
                                onPress: () => void exportDB(),
                            },
                        ],
                    })
                }}
            />

            <ThemedButton
                label={t('settings.database.importButton')}
                variant="secondary"
                onPress={async () => {
                    getDocumentAsync({ type: ['application/*'] }).then(async (result) => {
                        if (result.canceled) return
                        Alert.alert({
                            title: t('settings.database.alert.import.title'),
                            description: t('settings.database.alert.import.description'),
                            buttons: [
                                { label: t('common.actions.cancel') },
                                {
                                    label: t('settings.database.alert.import.confirm'),
                                    onPress: () =>
                                        importDB(result.assets[0].uri, result.assets[0].name),
                                    type: 'warning',
                                },
                            ],
                        })
                    })
                }}
            />

            <ThemedButton
                label={t('settings.database.rerunMigrationsButton')}
                variant="secondary"
                onPress={rerunMigrations}
            />
        </View>
    )
}

export default DatabaseSettings
