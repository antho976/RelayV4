/**
 * This plugin adds the required AndroidManifest headers to allow
 * user defined https certs
 */

const { Paths } = require('@expo/config-plugins/build/android')
const { AndroidConfig, withAndroidManifest } = require('expo/config-plugins')
const fs = require('fs')
const path = require('path')
const fsPromises = fs.promises

const { getMainApplicationOrThrow } = AndroidConfig.Manifest

const withTrustLocalCerts = (config) => {
    return withAndroidManifest(config, async (config) => {
        config.modResults = await setCustomConfigAsync(config, config.modResults)
        return config
    })
}

async function setCustomConfigAsync(config, androidManifest) {
    const src_file_pat = path.join(
        config.modRequest.projectRoot,
        'assets',
        'xml',
        'network_security_config.xml'
    )
    const res_file_path = path.join(
        await Paths.getResourceFolderAsync(config.modRequest.projectRoot),
        'xml',
        'network_security_config.xml'
    )

    const res_dir = path.resolve(res_file_path, '..')

    await fsPromises.mkdir(res_dir, { recursive: true })
    await fsPromises.copyFile(src_file_pat, res_file_path)

    const mainApplication = getMainApplicationOrThrow(androidManifest)
    mainApplication.$['android:networkSecurityConfig'] = '@xml/network_security_config'

    return androidManifest
}

module.exports = withTrustLocalCerts
