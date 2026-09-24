import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'

import en from './locales/en.json'

// The app ships in English only; the strings still live in en.json so that screens read them by key.
// eslint-disable-next-line import/no-named-as-default-member
i18n.use(initReactI18next).init({
    resources: { en: { translation: en } },
    lng: 'en',
    fallbackLng: 'en',
    interpolation: {
        escapeValue: false, // react already escapes
    },
})

export default i18n
