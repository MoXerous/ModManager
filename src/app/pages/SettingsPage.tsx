import { useTranslation } from 'react-i18next';

import {
  LOCALES,
  THEMES,
  type Numerals,
  setPreferences,
  usePreferences,
} from '../preferences';

interface SegmentedProps<T extends string> {
  label: string;
  value: T;
  options: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
}

function Segmented<T extends string>({ label, value, options, onChange }: SegmentedProps<T>) {
  return (
    <div className="flex items-center justify-between text-small text-muted-foreground">
      <span>{label}</span>
      <div
        role="radiogroup"
        aria-label={label}
        className="flex rounded-segmented border border-border bg-surface-sunken p-0.5"
      >
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={option.value === value}
            onClick={() => onChange(option.value)}
            className={
              option.value === value
                ? 'rounded-control bg-accent px-[9px] py-[3px] text-foreground'
                : 'rounded-control px-[9px] py-[3px] text-muted-foreground'
            }
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/** Board 17, General card. Paths, Network, Providers, Features and Diagnostics land in 5c. */
export function SettingsPage() {
  const { t } = useTranslation();
  const prefs = usePreferences();

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-5 pt-4">
        <h1 className="text-page-title font-semibold">{t('nav.settings')}</h1>
      </div>
      <section className="mx-5 mt-3 flex w-fit flex-col gap-2 rounded-card border border-border bg-card p-3.5">
        <b className="font-semibold">{t('settings.general')}</b>
        <Segmented
          label={t('settings.language')}
          value={prefs.locale}
          onChange={(locale) => setPreferences({ locale })}
          options={LOCALES.map((locale) => ({
            value: locale,
            label: locale === 'ar' ? t('language.arabic') : t('language.english'),
          }))}
        />
        <Segmented
          label={t('settings.theme')}
          value={prefs.theme}
          onChange={(theme) => setPreferences({ theme })}
          options={THEMES.map((theme) => ({ value: theme, label: t(`theme.${theme}`) }))}
        />
        <Segmented<Numerals>
          label={t('settings.digits')}
          value={prefs.numerals}
          onChange={(numerals) => setPreferences({ numerals })}
          options={[
            { value: 'latn', label: t('digits.western') },
            { value: 'arab', label: t('digits.arabic') },
          ]}
        />
      </section>
    </div>
  );
}