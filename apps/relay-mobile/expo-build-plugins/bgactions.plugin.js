/**
 * This plugin is to apply changes to AndroidManifest.xml to add permissions and intent-filter's
 * for `react-native-background-actions` to run JS in background
 */

const { withAndroidManifest, AndroidConfig } = require('expo/config-plugins')
const { getMainApplicationOrThrow } = AndroidConfig.Manifest

module.exports = function withCustomIntentFilter(config) {
    return withAndroidManifest(config, async (config) => {
        const application = getMainApplicationOrThrow(config.modResults)
        let hasModified = false

        if (!application.service) {
            application.service = []
        }

        // A prebuild without --clean runs against the manifest it wrote last time, so every
        // entry is added only when it is not already there.
        const serviceName = 'com.asterinet.react.bgactions.RNBackgroundActionsTask'
        if (!application.service.some((service) => service.$?.['android:name'] === serviceName)) {
            application.service.push({
                $: {
                    'android:name': serviceName,
                    'android:foregroundServiceType': 'dataSync',
                },
            })
        }

        // Iterate over the activities to find the main activity
        if (application.activity) {
            for (const activity of application.activity) {
                if (activity['$'] && activity['$']['android:name'] === '.MainActivity') {
                    // Ensure android:launchMode is set to "singleTask"
                    activity['$']['android:launchMode'] = 'singleTask'

                    // Ensure the intent-filter is present
                    if (!activity['intent-filter']) {
                        activity['intent-filter'] = []
                    }

                    const hasScheme = activity['intent-filter'].some((filter) =>
                        filter.data?.some((data) => data.$?.['android:scheme'] === 'relayapp')
                    )
                    if (!hasScheme)
                        activity['intent-filter'].push({
                            $: { 'android:label': 'filter_react_native' },
                            action: [{ $: { 'android:name': 'android.intent.action.VIEW' } }],
                            category: [
                                { $: { 'android:name': 'android.intent.category.DEFAULT' } },
                                { $: { 'android:name': 'android.intent.category.BROWSABLE' } },
                            ],
                            data: [{ $: { 'android:scheme': 'relayapp' } }],
                        })

                    hasModified = true
                    break
                }
            }
        }

        if (!hasModified) {
            throw new Error(
                'withCustomIntentFilter: Could not find MainActivity in AndroidManifest.xml'
            )
        }

        return config
    })
}
