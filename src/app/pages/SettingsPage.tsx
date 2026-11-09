import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { AppPathsDto } from '../../bindings/AppPathsDto';
import type { CurseForgeKeyDto } from '../../bindings/CurseForgeKeyDto';
import type { IpcError } from '../../bindings/IpcError';
import { Segmented } from '@/components/ui/Segmented';
import { Button } from '@/components/ui/button';
import { ErrorState } from '../../components/state';
import { formatBytes, formatNumber } from '../../lib/format';
import {
  appPaths,
  curseforgeKeyStatus,
  openAppFolder,
  saveCurseforgeKey,
  toIpcError,
  type AppFolderKind,
} from '../../lib/ipc';
import { LegacyImportCard } from './LegacyImportCard';
import {
  LOCALES,
  THEMES,
  type Numerals,
  setPreferences,
  usePreferences,
} from '../preferences';

/** One row of the paths card. The database row opens the data root, which is
 *  the folder that holds the file; there is no fifth kind on the wire. */
const PATH_ROWS: readonly {
  kind: AppFolderKind;
  labelKey: string;
  value: (paths: AppPathsDto) => string;
}[] = [
  { kind: 'data', labelKey: 'settings.paths.data', value: (p) => p.data },
  { kind: 'config', labelKey: 'settings.paths.config', value: (p) => p.config },
  { kind: 'cache', labelKey: 'settings.paths.cache', value: (p) => p.cache },
  { kind: 'logs', labelKey: 'settings.paths.logs', value: (p) => p.logs },
  { kind: 'data', labelKey: 'settings.paths.database', value: (p) => p.dbFile },
];

/** Board 17. General, Providers, Paths, Diagnostics and the legacy scan. */
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

  // Paths are read once and shown; the retry button exists because a failed
  // read is the only way the card can be empty while the app is running.
  const [paths, setPaths] = useState<AppPathsDto | null>(null);
  const [pathsError, setPathsError] = useState<IpcError | null>(null);
  const [openError, setOpenError] = useState<IpcError | null>(null);
  const [opening, setOpening] = useState<AppFolderKind | null>(null);

  const refreshStatus = useCallback(() => {
    curseforgeKeyStatus()
      .then(setKeyStatus)
      .catch((e) => setError(toIpcError(e)));
  }, []);

  const refreshPaths = useCallback(() => {
    setPathsError(null);
    appPaths()
      .then(setPaths)
      .catch((e) => setPathsError(toIpcError(e)));
  }, []);

  useEffect(refreshStatus, [refreshStatus]);
  useEffect(refreshPaths, [refreshPaths]);

  const openFolder = useCallback((kind: AppFolderKind) => {
    setOpening(kind);
    setOpenError(null);
    openAppFolder(kind)
      .catch((e) => setOpenError(toIpcError(e)))
      .finally(() => setOpening(null));
  }, []);

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

      <section className="mx-5 mt-3 flex max-w-2xl flex-col gap-2 rounded-card border border-border bg-card p-3.5">
        <b className="font-semibold">{t('settings.paths.title')}</b>
        {pathsError !== null ? (
          <>
            <p role="alert" className="text-small text-status-issue">
              {t('settings.paths.loadFailed', { reason: t(`error.${pathsError.code}`) })}
            </p>
            <Button type="button" variant="outline" className="w-fit" onClick={refreshPaths}>
              {t('state.action.retry')}
            </Button>
          </>
        ) : paths === null ? (
          <span className="text-small text-muted-foreground">{t('state.loading')}</span>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {PATH_ROWS.map((row) => {
              const label = t(row.labelKey);
              return (
                <li
                  key={row.labelKey}
                  className="flex flex-wrap items-center gap-x-2 gap-y-1 text-small"
                >
                  <span className="w-24 shrink-0 text-muted-foreground">{label}</span>
                  <code dir="ltr" className="min-w-0 flex-1 break-all font-mono text-caption">
                    {row.value(paths)}
                  </code>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={opening !== null}
                    aria-busy={opening === row.kind}
                    aria-label={t('settings.paths.openAria', { name: label })}
                    onClick={() => openFolder(row.kind)}
                  >
                    {t('settings.paths.open')}
                  </Button>
                </li>
              );
            })}
          </ul>
        )}
        <p className="text-caption text-muted-foreground">{t('settings.paths.note')}</p>
        {openError !== null && (
          <p role="alert" className="text-caption text-status-issue">
            {t('settings.paths.openFailed', { reason: t(`error.${openError.code}`) })}
          </p>
        )}
      </section>

      <section className="mx-5 mt-3 flex max-w-2xl flex-col gap-2 rounded-card border border-border bg-card p-3.5">
        <b className="font-semibold">{t('settings.diagnostics.title')}</b>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-small">
          <dt className="text-muted-foreground">{t('settings.diagnostics.version')}</dt>
          <dd dir="ltr" className="justify-self-start font-mono text-caption">
            {paths?.appVersion ?? '—'}
          </dd>
          <dt className="text-muted-foreground">{t('settings.diagnostics.schema')}</dt>
          <dd dir="ltr" className="justify-self-start font-mono text-caption">
            {paths ? formatNumber(paths.dbVersion) : '—'}
          </dd>
          <dt className="text-muted-foreground">{t('settings.diagnostics.dbSize')}</dt>
          <dd dir="ltr" className="justify-self-start font-mono text-caption">
            {paths ? formatBytes(paths.dbBytes) : '—'}
          </dd>
        </dl>
      </section>

      <LegacyImportCard />
    </div>
  );
}
