import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

import ar from '../i18n/ar.json';
import en from '../i18n/en.json';
import { getPreferences, subscribe } from './preferences';

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    ar: { translation: ar },
  },
  lng: getPreferences().locale,
  fallbackLng: 'en',
  interpolation: { escapeValue: false },
});

// Locale is a preference, so changing it in Settings must re-run i18next too.
subscribe(() => {
  const locale = getPreferences().locale;
  if (i18n.language !== locale) void i18n.changeLanguage(locale);
});

/** For code outside the React tree (formatting, logging). Components use `useTranslation`. */
export const t = i18n.t.bind(i18n);

export default i18n;