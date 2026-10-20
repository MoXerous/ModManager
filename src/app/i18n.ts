import i18n from 'i18next';
import type { FormatFunction } from 'i18next';
import { initReactI18next } from 'react-i18next';

// v26 removes `format` from the InterpolationOptions *types* while the runtime
// still reads it (Interpolator: `options?.interpolation?.format`), so the
// option is re-declared here rather than worked around with a cast.
declare module 'i18next' {
  interface InterpolationOptions {
    format?: FormatFunction;
  }
}

import ar from '../i18n/ar.json';
import en from '../i18n/en.json';
import { formatNumber } from '../lib/format';
import { getPreferences, subscribe } from './preferences';

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    ar: { translation: ar },
  },
  lng: getPreferences().locale,
  fallbackLng: 'en',
  interpolation: {
    escapeValue: false,
    alwaysFormat: true,
    // Call sites pass `count` as a raw number so Intl.PluralRules still picks
    // the Arabic plural form; the digit preference is applied once here, so no
    // page pre-formats with formatNumber before t() (a string count would
    // collapse every Arabic plural into `_other`).
    format: (value) => (typeof value === 'number' ? formatNumber(value) : value),
  },
});

// Locale is a preference, so changing it in Settings must re-run i18next too.
subscribe(() => {
  const locale = getPreferences().locale;
  if (i18n.language !== locale) void i18n.changeLanguage(locale);
});

/** For code outside the React tree (formatting, logging). Components use `useTranslation`. */
export const t = i18n.t.bind(i18n);

export default i18n;