/**
 * Config plugin for the "process text" entry in Android's text-selection menu.
 *
 *   ['./modules/relay-device/app.plugin.js', { label: 'Ask in Relay' }]
 *
 * Adds an <activity-alias> of the main activity that accepts ACTION_PROCESS_TEXT, so selected
 * text is delivered straight to MainActivity; the native module picks it up from there. The
 * label goes into strings.xml as `relay_process_text_label`, where it can be translated.
 *
 * Options:
 *   label            menu text (default 'Ask in Relay')
 *   enabledByDefault whether the entry shows before the app first sets it (default false)
 */
const {
    AndroidConfig,
    createRunOncePlugin,
    withAndroidManifest,
    withStringsXml,
} = require('expo/config-plugins')

/** Must match ProcessText.ALIAS in ProcessText.kt. */
const ALIAS_NAME = 'com.quietsoftware.relay.device.ProcessTextAlias'
const LABEL_RESOURCE = 'relay_process_text_label'

/** Adds or replaces the alias in a parsed AndroidManifest. Exported for tests. */
const applyProcessTextAlias = (manifest, { enabledByDefault = false } = {}) => {
    const application = AndroidConfig.Manifest.getMainApplicationOrThrow(manifest)
    const mainActivity = AndroidConfig.Manifest.getMainActivityOrThrow(manifest)

    const alias = {
        $: {
            'android:name': ALIAS_NAME,
            'android:targetActivity': mainActivity.$['android:name'],
            'android:label': `@string/${LABEL_RESOURCE}`,
            'android:exported': 'true',
            'android:enabled': enabledByDefault ? 'true' : 'false',
        },
        'intent-filter': [
            {
                action: [{ $: { 'android:name': 'android.intent.action.PROCESS_TEXT' } }],
                category: [{ $: { 'android:name': 'android.intent.category.DEFAULT' } }],
                data: [{ $: { 'android:mimeType': 'text/plain' } }],
            },
        ],
    }
    const others = (application['activity-alias'] ?? []).filter(
        (entry) => entry.$['android:name'] !== ALIAS_NAME
    )
    application['activity-alias'] = [...others, alias]
    return manifest
}

/**
 * Sets the label string in a parsed strings.xml. Exported for tests. Quotes and apostrophes
 * need no escaping here: config-plugins escapes string values when it writes the file.
 */
const applyLabelString = (strings, label) =>
    AndroidConfig.Strings.setStringItem(
        [
            AndroidConfig.Resources.buildResourceItem({
                name: LABEL_RESOURCE,
                value: label,
            }),
        ],
        strings
    )

const withRelayProcessText = (config, options = {}) => {
    const label = options.label ?? 'Ask in Relay'
    if (typeof label !== 'string' || label.trim() === '')
        throw new Error('relay-device: `label` must be a non-empty string')

    config = withStringsXml(config, (mod) => {
        mod.modResults = applyLabelString(mod.modResults, label)
        return mod
    })
    return withAndroidManifest(config, (mod) => {
        mod.modResults = applyProcessTextAlias(mod.modResults, options)
        return mod
    })
}

module.exports = createRunOncePlugin(withRelayProcessText, 'relay-device-process-text', '1.0.0')
module.exports.applyProcessTextAlias = applyProcessTextAlias
module.exports.applyLabelString = applyLabelString
module.exports.ALIAS_NAME = ALIAS_NAME
