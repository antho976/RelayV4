const IS_DEV = process.env.APP_VARIANT === 'development'

module.exports = {
    expo: {
        name: IS_DEV ? 'Relay (DEV)' : 'Relay',
        newArchEnabled: true,
        slug: 'Relay',
        version: '0.10.0-beta5',
        orientation: 'default',
        icon: './assets/images/icon.png',
        scheme: 'relayapp',
        userInterfaceStyle: 'automatic',
        assetBundlePatterns: ['**/*'],
        ios: {
            icon: {
                dark: './assets/images/ios-dark.png',
                light: './assets/images/ios-light.png',
                tinted: './assets/images/icon.png',
            },
            supportsTablet: true,
            bundleIdentifier: IS_DEV ? 'com.quietsoftware.relay.dev' : 'com.quietsoftware.relay',
        },
        android: {
            adaptiveIcon: {
                foregroundImage: './assets/images/adaptive-icon-foreground.png',
                backgroundImage: './assets/images/adaptive-icon-background.png',
                monochromeImage: './assets/images/adaptive-icon-foreground.png',
                backgroundColor: '#141416',
            },
            package: IS_DEV ? 'com.quietsoftware.relay.dev' : 'com.quietsoftware.relay',
            userInterfaceStyle: 'dark',
            permissions: [
                'android.permission.FOREGROUND_SERVICE',
                'android.permission.WAKE_LOCK',
                'android.permission.FOREGROUND_SERVICE_DATA_SYNC',
            ],
        },
        web: {
            bundler: 'metro',
            output: 'static',
            favicon: './assets/images/adaptive-icon.png',
        },
        plugins: [
            [
                'expo-asset',
                {
                    assets: ['./assets/models/assistant.raw', './assets/models/llama3tokenizer.gguf'],
                },
            ],
            [
                'expo-build-properties',
                {
                    android: {
                        largeHeap: true,
                        usesCleartextTraffic: true,
                        enableProguardInReleaseBuilds: true,
                        enableShrinkResourcesInReleaseBuilds: true,
                        useLegacyPackaging: true,
                        extraProguardRules: '-keep class com.rnllama.** { *; }',
                    },
                },
            ],
            [
                'expo-splash-screen',
                {
                    backgroundColor: '#141416',
                    image: './assets/images/adaptive-icon.png',
                    imageWidth: 200,
                },
            ],
            [
                'expo-notifications',
                {
                    icon: './assets/images/notification.png',
                },
            ],
            [
                './expo-build-plugins/androidattributes.plugin.js',
                {
                    'android:largeHeap': true,
                },
            ],
            ['@vali98/react-native-process-text', { label: 'Ask in Relay' }],
            [
                'expo-camera',
                {
                    cameraPermission: 'Allow Relay to access your camera',
                },
            ],
            ['expo-sqlite', { withSQLiteVecExtension: true }],
            [
                'expo-image-picker',
                {
                    photosPermission: 'Relay needs photo access for vision models',
                    colors: {
                        cropToolbarColor: '#000000',
                    },
                    dark: {
                        colors: {
                            cropToolbarColor: '#000000',
                        },
                    },
                },
            ],
            'expo-router',
            'expo-font',
            'expo-image',
            './expo-build-plugins/bgactions.plugin.js',
            './expo-build-plugins/usercert.plugin.js',
            './expo-build-plugins/rnllama.plugin.js',
            './expo-build-plugins/copyhtp.plugin.js',
            /**
             * Future icon usage will need to be added here
             * https://github.com/oblador/react-native-vector-icons/blob/master/docs/SETUP-EXPO.md
             */
            '@react-native-vector-icons/ant-design',
            '@react-native-vector-icons/octicons',
            '@react-native-vector-icons/material-icons',
        ],
        experiments: {
            typedRoutes: true,
            reactCompiler: true,
        },
        extra: {
            router: {
                origin: false,
            },
        },
    },
}
