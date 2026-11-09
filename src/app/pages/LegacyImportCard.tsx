import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { LegacyDatabaseDto } from '../../bindings/LegacyDatabaseDto';
import type { LegacyJsonFactDto } from '../../bindings/LegacyJsonFactDto';
import type { LegacyProfileDto } from '../../bindings/LegacyProfileDto';
import type { LegacyScanDto } from '../../bindings/LegacyScanDto';
import type { IpcError } from '../../bindings/IpcError';
import { Button } from '@/components/ui/button';
import { ErrorState } from '../../components/state';
import { formatBytes } from '../../lib/format';
import { scanLegacyImport, toIpcError } from '../../lib/ipc';

/**
 * F-181, slice 0: what the old application holds, read through a copy.
 *
 * Nothing here is imported: the card is the report half of the feature, and
 * the scan writes nothing into the old home (the Rust reader owns that
 * guarantee). It runs only when the user presses the button - the reader is
 * deliberately not called at startup or on mount - and every path is shown
 * `dir="ltr"` because it is a filesystem path, not prose.
 */
export function LegacyImportCard() {
  const { t } = useTranslation();
  const [home, setHome] = useState('');
  const [scan, setScan] = useState<LegacyScanDto | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);

  const run = useCallback(() => {
    setBusy(true);
    setError(null);
    // A new scan replaces the last report: leaving a stale one under a failed
    // scan would describe a folder the user no longer asked about.
    setScan(null);
    scanLegacyImport(home)
      .then(setScan)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  }, [home]);

  return (
    <section className="mx-5 mt-3 flex max-w-3xl flex-col gap-2 rounded-card border border-border bg-card p-3.5">
      <b className="font-semibold">{t('settings.legacy.title')}</b>
      <p className="text-small text-muted-foreground">{t('settings.legacy.body')}</p>
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={home}
          onChange={(e) => setHome(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !busy) {
              run();
            }
          }}
          dir="ltr"
          autoComplete="off"
          spellCheck={false}
          placeholder={t('settings.legacy.homePlaceholder')}
          aria-label={t('settings.legacy.homeLabel')}
          className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
        />
        <Button type="button" onClick={run} disabled={busy}>
          {busy ? t('settings.legacy.scanning') : t('settings.legacy.scan')}
        </Button>
      </div>
      {scan === null && error === null && (
        <p className="text-caption text-muted-foreground">{t('settings.legacy.notScanned')}</p>
      )}
      {error !== null && <ErrorState error={error} className="mt-1" />}
      {scan !== null && <LegacyReport scan={scan} />}
    </section>
  );
}

function LegacyReport({ scan }: { scan: LegacyScanDto }) {
  const { t } = useTranslation();
  const paths: [string, string][] = [
    [t('settings.legacy.path.data'), scan.roots.data],
    [t('settings.legacy.path.config'), scan.roots.config],
    [t('settings.legacy.path.cache'), scan.roots.cache],
    [t('settings.legacy.path.logs'), scan.roots.logs],
    [t('settings.legacy.path.db'), scan.roots.dbFile],
  ];

  return (
    <div className="mt-1 flex flex-col gap-3">
      <p className="flex flex-wrap items-baseline gap-2 text-small">
        <span className="text-muted-foreground">{t('settings.legacy.source')}</span>
        <span>{t(`settings.legacy.source.${scan.roots.source}`)}</span>
      </p>

      <dl className="grid grid-cols-[max-content_minmax(0,1fr)] items-baseline gap-x-3 gap-y-1">
        {paths.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-caption text-muted-foreground">{label}</dt>
            <dd dir="ltr" className="min-w-0 break-all font-mono text-caption">
              {value || '—'}
            </dd>
          </div>
        ))}
      </dl>

      {scan.database !== null && <DatabaseFacts database={scan.database} />}

      <div className="flex flex-col gap-2">
        <b className="text-body font-semibold">{t('settings.legacy.profiles')}</b>
        {scan.profiles.length === 0 ? (
          <p className="text-small text-muted-foreground">{t('settings.legacy.profiles.empty')}</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {scan.profiles.map((profile) => (
              <li
                key={profile.id || profile.name}
                className="rounded-control border border-border bg-surface-sunken px-3 py-2"
              >
                <ProfileFacts profile={profile} />
              </li>
            ))}
          </ul>
        )}
      </div>

      {scan.warnings.length > 0 && (
        <div className="flex flex-col gap-1">
          <b className="text-body font-semibold text-status-issue">
            {t('settings.legacy.warnings')}
          </b>
          <ul className="flex flex-col gap-1.5">
            {scan.warnings.map((warning, index) => (
              <li
                key={`${warning.code}-${warning.profile ?? ''}-${index}`}
                className="flex flex-col gap-0.5 text-caption"
              >
                <span className="text-status-issue">
                  {t(`settings.legacy.warning.${warning.code}`)}
                  {warning.profile !== null && ` — ${warning.profile}`}
                </span>
                {warning.detail !== '' && (
                  <span dir="ltr" className="break-all font-mono text-muted-foreground">
                    {warning.detail}
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      <p className="text-caption text-muted-foreground">{t('settings.legacy.previewOnly')}</p>
    </div>
  );
}

function DatabaseFacts({ database }: { database: LegacyDatabaseDto }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-1">
      <b className="text-body font-semibold">{t('settings.legacy.database')}</b>
      <dl className="grid grid-cols-[max-content_max-content] items-baseline gap-x-2 gap-y-1 text-caption">
        <div className="contents">
          <dt className="text-muted-foreground">{t('settings.legacy.database.version')}</dt>
          <dd dir="ltr" className="font-mono">
            {database.userVersion}
          </dd>
        </div>
        <div className="contents">
          <dt className="text-muted-foreground">{t('settings.legacy.database.table')}</dt>
          <dd>
            {database.hasProfilesTable
              ? t('settings.legacy.database.tablePresent')
              : t('settings.legacy.database.tableMissing')}
          </dd>
        </div>
        <div className="contents">
          <dt className="text-muted-foreground">{t('settings.legacy.database.size')}</dt>
          <dd dir="ltr" className="font-mono">
            {formatBytes(database.bytes)}
          </dd>
        </div>
        <div className="contents">
          <dt className="text-muted-foreground">{t('settings.legacy.database.state')}</dt>
          <dd className={database.supported ? undefined : 'text-status-issue'}>
            {database.supported
              ? t('settings.legacy.database.supported')
              : t('settings.legacy.database.unsupported')}
          </dd>
        </div>
      </dl>
      <details>
        <summary className="cursor-pointer text-caption text-muted-foreground">
          {t('settings.legacy.database.columns')} ({database.columns.length})
        </summary>
        <p dir="ltr" className="mt-1 break-all font-mono text-caption text-muted-foreground">
          {database.columns.join(', ')}
        </p>
      </details>
    </div>
  );
}

function ProfileFacts({ profile }: { profile: LegacyProfileDto }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <b className="text-body">{profile.name || `#${profile.id}`}</b>
        <span className="text-caption text-muted-foreground">
          {t('settings.legacy.profile.version')} {profile.gameVersion || '—'}
        </span>
        <span className="text-caption text-muted-foreground">
          {t('settings.legacy.profile.loader')} {profile.loader || '—'}
        </span>
        {profile.kind !== '' && (
          <span className="text-caption text-muted-foreground">
            {t('settings.legacy.profile.kind')} {profile.kind}
          </span>
        )}
        {profile.ownership !== '' && (
          <span className="text-caption text-muted-foreground">
            {t('settings.legacy.profile.ownership')} {profile.ownership}
          </span>
        )}
      </div>
      <p className="flex flex-wrap items-baseline gap-x-2 text-caption">
        <span className="text-muted-foreground">{t('settings.legacy.profile.root')}</span>
        <code dir="ltr" className="break-all font-mono">
          {profile.root || '—'}
        </code>
        {!profile.rootExists && (
          <span className="text-status-issue">{t('settings.legacy.profile.rootMissing')}</span>
        )}
      </p>
      <p className="text-caption">
        <span className="text-muted-foreground">{t('settings.legacy.profile.profileJson')}: </span>
        <JsonStatus fact={profile.profileJson} />
        <span className="text-muted-foreground"> · {t('settings.legacy.profile.installedMods')}: </span>
        {profile.installedMods.present && profile.installedMods.ok ? (
          <>
            {t('settings.legacy.fact.ok')}
            {' · '}
            {t('settings.legacy.mods.count', {
              managed: profile.installedMods.managed,
              skipped: profile.installedMods.skipped,
            })}
          </>
        ) : (
          <JsonStatus fact={profile.installedMods} />
        )}
      </p>
    </div>
  );
}

function JsonStatus({ fact }: { fact: Pick<LegacyJsonFactDto, 'present' | 'ok' | 'detail'> }) {
  const { t } = useTranslation();
  if (!fact.present) {
    return <span className="text-muted-foreground">{t('settings.legacy.fact.missing')}</span>;
  }
  if (fact.ok) {
    return <span className="text-muted-foreground">{t('settings.legacy.fact.ok')}</span>;
  }
  return (
    <span className="text-status-issue">
      {t('settings.legacy.fact.bad', { detail: fact.detail })}
    </span>
  );
}
