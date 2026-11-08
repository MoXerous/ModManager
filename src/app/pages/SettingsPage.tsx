import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CurseForgeKeyDto } from '../../bindings/CurseForgeKeyDto';
import type { IpcError } from '../../bindings/IpcError';
import { Segmented } from '@/components/ui/Segmented';
import { Button } from '@/components/ui/button';
import { ErrorState } from '../../components/state';
import { curseforgeKeyStatus, saveCurseforgeKey, toIpcError } from '../../lib/ipc';
import {
  LOCALES,
  THEMES,
  type Numerals,
  setPreferences,
  usePreferences,
} from '../preferences';

/** Board 17. General and Providers are here; Paths, Network, Features and Diagnostics land in 5c. */
export function SettingsPage() {
  const { t } = useTranslation();
  const prefs = usePreferences();

  // The key is write-only across IPC: this page can learn where it came from
  // and where the file is, never the value. The field is a one-way door onto
  // the disk.
  const [keyStatus, setKeyStatus] = useState<CurseForgeKeyDto | null>(null);
  const [key, setKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);

  const refreshStatus = useCallback(() => {
    curseforgeKeyStatus()
      .then(setKeyStatus)
      .catch((e) => setError(toIpcError(e)));
  }, []);

  useEffect(refreshStatus, [refreshStatus]);

  const save = useCallback(() => {
    const value = key.trim();
    if (value.length === 0) {
      return;
    }
    setSaving(true);
    setError(null);
    saveCurseforgeKey(value)
      .then((status) => {
        setKeyStatus(status);
        // Cleared the moment it is written: the value is not readable back,
        // and leaving it on screen only gives it more places to leak.
        setKey('');
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setSaving(false));
  }, [key]);

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

      <section className="mx-5 mt-3 flex max-w-2xl flex-col gap-2 rounded-card border border-border bg-card p-3.5">
        <b className="font-semibold">{t('settings.providers')}</b>
        <span className="text-small text-muted-foreground">
          {keyStatus === null
            ? t('state.loading')
            : t(`settings.providers.source.${keyStatus.source}`)}
        </span>
        {keyStatus !== null && (
          <p className="flex flex-wrap items-baseline gap-2 text-small text-muted-foreground">
            {t('settings.providers.path')}
            <code dir="ltr" className="break-all font-mono text-caption">
              {keyStatus.path}
            </code>
          </p>
        )}
        <div className="flex items-center gap-2">
          <input
            type="password"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                save();
              }
            }}
            dir="ltr"
            autoComplete="off"
            spellCheck={false}
            placeholder={t('settings.providers.keyPlaceholder')}
            aria-label={t('settings.providers.key')}
            className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
          />
          <Button type="button" onClick={save} disabled={saving || key.trim().length === 0}>
            {t('settings.providers.save')}
          </Button>
        </div>
        {/* ADR 0004 2-1-7: the file is in `config_dir`, which on Windows is
            %APPDATA% and may travel with a roaming profile. Stated where the
            key is entered, not buried in a document. */}
        <p className="text-caption text-muted-foreground">{t('settings.providers.roaming')}</p>
        {error !== null && <ErrorState error={error} className="mt-1" />}
      </section>
    </div>
  );
}
