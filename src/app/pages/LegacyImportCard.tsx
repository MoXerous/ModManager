import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { LegacyDatabaseDto } from '../../bindings/LegacyDatabaseDto';
import type { LegacyImportPlanDto } from '../../bindings/LegacyImportPlanDto';
import type { LegacyImportReportDto } from '../../bindings/LegacyImportReportDto';
import type { LegacyJsonFactDto } from '../../bindings/LegacyJsonFactDto';
import type { LegacyProfileDto } from '../../bindings/LegacyProfileDto';
import type { LegacyScanDto } from '../../bindings/LegacyScanDto';
import type { IpcError } from '../../bindings/IpcError';
import { Button } from '@/components/ui/button';
import { ErrorState } from '../../components/state';
import { formatBytes } from '../../lib/format';
import {
  executeLegacyImport,
  planLegacyImport,
  scanLegacyImport,
  toIpcError,
} from '../../lib/ipc';

/** What the import controls are doing; `idle` enables them. */
type ImportBusy = 'idle' | 'plan' | 'import';

/**
 * F-181: what the old application holds, read through a copy, and the write
 * path that registers selected profiles as instances of this app.
 *
 * The scan is the report half: it writes nothing into the old home (the Rust
 * reader owns that guarantee) and runs only when the user presses the button -
 * the reader is deliberately not called at startup or on mount. Registration
 * writes into **our** database only; the plan is shown before anything is
 * written, and the confirm sends profile ids, never a plan.
 *
 * Every path is shown `dir="ltr"` because it is a filesystem path, not prose.
 */
export function LegacyImportCard() {
  const { t } = useTranslation();
  const [home, setHome] = useState('');
  const [scan, setScan] = useState<LegacyScanDto | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [plan, setPlan] = useState<LegacyImportPlanDto | null>(null);
  const [report, setReport] = useState<LegacyImportReportDto | null>(null);
  const [busy, setBusy] = useState(false);
  const [action, setAction] = useState<ImportBusy>('idle');
  const [error, setError] = useState<IpcError | null>(null);
  const [importError, setImportError] = useState<IpcError | null>(null);

  const run = useCallback(() => {
    setBusy(true);
    setError(null);
    setImportError(null);
    setPlan(null);
    setReport(null);
    setSelected([]);
    // A new scan replaces the last report: leaving a stale one under a failed
    // scan would describe a folder the user no longer asked about.
    setScan(null);
    scanLegacyImport(home)
      .then((result) => {
        setScan(result);
        // Preselect what can be registered; a missing root stays selectable
        // so the plan can explain the blocker, but it is not chosen for you.
        setSelected(
          result.profiles
            .filter((profile) => profile.rootExists && profile.id !== '')
            .map((p) => p.id),
        );
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  }, [home]);

  const changeSelection = useCallback((next: string[]) => {
    setSelected(next);
    // A plan and a report describe the previous selection; keeping them would
    // let a confirm button next to stale rows register what is now selected.
    setPlan(null);
    setReport(null);
    setImportError(null);
  }, []);

  const toggle = useCallback((id: string) => {
    setSelected((current) =>
      current.includes(id) ? current.filter((seen) => seen !== id) : [...current, id],
    );
    setPlan(null);
    setReport(null);
    setImportError(null);
  }, []);

  const selectAll = useCallback(() => {
    if (scan === null) {
      return;
    }
    changeSelection(scan.profiles.map((profile) => profile.id).filter((id) => id !== ''));
  }, [changeSelection, scan]);

  const selectNone = useCallback(() => changeSelection([]), [changeSelection]);

  const preview = useCallback(() => {
    setAction('plan');
    setImportError(null);
    // An import result describes the previous selection; a new plan must not
    // sit next to it.
    setReport(null);
    planLegacyImport(home, selected)
      .then(setPlan)
      .catch((e) => setImportError(toIpcError(e)))
      .finally(() => setAction('idle'));
  }, [home, selected]);

  const confirm = useCallback(() => {
    setAction('import');
    setImportError(null);
    executeLegacyImport(home, selected)
      .then((result) => {
        setPlan(null);
        setReport(result);
      })
      .catch((e) => setImportError(toIpcError(e)))
      .finally(() => setAction('idle'));
  }, [home, selected]);

  const cancel = useCallback(() => setPlan(null), []);

  const scanning = busy || action !== 'idle';

  return (
    <section className="mx-5 mt-3 flex max-w-3xl flex-col gap-2 rounded-card border border-border bg-card p-3.5">
      <b className="font-semibold">{t('settings.legacy.title')}</b>
      <p className="text-small text-muted-foreground">{t('settings.legacy.body')}</p>
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={home}
          onChange={(e) => {
            setHome(e.target.value);
            // A plan and a report describe the previous path; a confirm next
            // to them would run against a folder the user never previewed.
            setPlan(null);
            setReport(null);
            setImportError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !scanning) {
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
        <Button type="button" onClick={run} disabled={scanning}>
          {busy ? t('settings.legacy.scanning') : t('settings.legacy.scan')}
        </Button>
      </div>
      {scan === null && error === null && (
        <p className="text-caption text-muted-foreground">{t('settings.legacy.notScanned')}</p>
      )}
      {error !== null && <ErrorState error={error} className="mt-1" />}
      {scan !== null && (
        <LegacyReport
          scan={scan}
          selected={selected}
          busy={action}
          plan={plan}
          report={report}
          importError={importError}
          onToggle={toggle}
          onSelectAll={selectAll}
          onSelectNone={selectNone}
          onPreview={preview}
          onConfirm={confirm}
          onCancel={cancel}
        />
      )}
    </section>
  );
}

interface LegacyReportProps {
  scan: LegacyScanDto;
  selected: string[];
  busy: ImportBusy;
  plan: LegacyImportPlanDto | null;
  report: LegacyImportReportDto | null;
  importError: IpcError | null;
  onToggle: (id: string) => void;
  onSelectAll: () => void;
  onSelectNone: () => void;
  onPreview: () => void;
  onConfirm: () => void;
  onCancel: () => void;
}

function LegacyReport({
  scan,
  selected,
  busy,
  plan,
  report,
  importError,
  onToggle,
  onSelectAll,
  onSelectNone,
  onPreview,
  onConfirm,
  onCancel,
}: LegacyReportProps) {
  const { t } = useTranslation();
  const paths: [string, string][] = [
    [t('settings.legacy.path.data'), scan.roots.data],
    [t('settings.legacy.path.config'), scan.roots.config],
    [t('settings.legacy.path.cache'), scan.roots.cache],
    [t('settings.legacy.path.logs'), scan.roots.logs],
    [t('settings.legacy.path.db'), scan.roots.dbFile],
  ];
  const importable = scan.profiles.filter((profile) => profile.id !== '').length;
  const idle = busy === 'idle';

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
            {scan.profiles.map((profile, index) => (
              <li
                key={`${profile.id || profile.name}-${index}`}
                className="rounded-control border border-border bg-surface-sunken px-3 py-2"
              >
                <ProfileFacts
                  profile={profile}
                  checkboxId={`legacy-profile-${index}`}
                  selected={selected.includes(profile.id)}
                  disabled={!idle}
                  onToggle={() => onToggle(profile.id)}
                />
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <b className="text-body font-semibold">{t('settings.legacy.importTitle')}</b>
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-caption text-muted-foreground">
            {t('settings.legacy.selectedCount', { count: selected.length })}
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onSelectAll}
            disabled={!idle || importable === 0}
          >
            {t('settings.legacy.selectAll')}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onSelectNone}
            disabled={!idle || selected.length === 0}
          >
            {t('settings.legacy.selectNone')}
          </Button>
          <Button
            type="button"
            size="sm"
            onClick={onPreview}
            disabled={!idle || selected.length === 0}
          >
            {busy === 'plan' ? t('settings.legacy.previewing') : t('settings.legacy.preview')}
          </Button>
        </div>

        {importError !== null && <ErrorState error={importError} />}
        {plan !== null && <ImportPlan plan={plan} busy={busy} onConfirm={onConfirm} onCancel={onCancel} />}
        {report !== null && <ImportReport report={report} />}
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

function ImportPlan({
  plan,
  busy,
  onConfirm,
  onCancel,
}: {
  plan: LegacyImportPlanDto;
  busy: ImportBusy;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2 rounded-control border border-border bg-surface-sunken p-3">
      <b className="text-body font-semibold">{t('settings.legacy.planTitle')}</b>
      {plan.blockedReason !== null ? (
        <p className="text-small text-status-issue">
          {t(`settings.legacy.code.${plan.blockedReason}`, { defaultValue: plan.blockedReason })}
        </p>
      ) : (
        <p className="text-small text-muted-foreground">
          {t('settings.legacy.planSummary', {
            ready: plan.ready,
            skipped: plan.skipped,
            blocked: plan.blocked,
          })}
        </p>
      )}

      {plan.entries.length > 0 && (
        <ul className="flex flex-col gap-1.5">
          {plan.entries.map((entry, index) => {
            const blocked = entry.action === 'block';
            return (
              <li
                key={`${entry.legacyId}-${index}`}
                className="flex min-w-0 flex-col gap-0.5 text-caption"
              >
                <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  <span className={blocked ? 'font-semibold text-status-issue' : 'font-semibold'}>
                    {entry.name || `#${entry.legacyId}`}
                  </span>
                  <span className="text-muted-foreground">
                    {t(`settings.legacy.source.${entry.source}`, { defaultValue: entry.source })}
                  </span>
                  <span className="text-muted-foreground">
                    {t(`settings.legacy.action.${entry.action}`, { defaultValue: entry.action })}
                  </span>
                  <span className={blocked ? 'text-status-issue' : 'text-muted-foreground'}>
                    {t(`settings.legacy.code.${entry.code}`, { defaultValue: entry.code })}
                  </span>
                </div>
                <code dir="ltr" className="min-w-0 break-all font-mono text-muted-foreground">
                  {entry.root || '—'}
                </code>
              </li>
            );
          })}
        </ul>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {plan.ready > 0 && (
          <Button type="button" size="sm" onClick={onConfirm} disabled={busy !== 'idle'}>
            {busy === 'import'
              ? t('settings.legacy.importing')
              : t('settings.legacy.confirm', { count: plan.ready })}
          </Button>
        )}
        <Button type="button" variant="outline" size="sm" onClick={onCancel} disabled={busy !== 'idle'}>
          {t('settings.legacy.cancel')}
        </Button>
      </div>
    </div>
  );
}

function ImportReport({ report }: { report: LegacyImportReportDto }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2 rounded-control border border-border bg-surface-sunken p-3">
      <b className="text-body font-semibold">{t('settings.legacy.reportTitle')}</b>
      {report.backup !== null ? (
        <p className="flex flex-col gap-0.5 text-caption">
          <span className="text-muted-foreground">{t('settings.legacy.backup')}</span>
          <code dir="ltr" className="min-w-0 break-all font-mono">
            {report.backup}
          </code>
        </p>
      ) : (
        <p className="text-caption text-muted-foreground">{t('settings.legacy.noBackup')}</p>
      )}

      {report.blockedReason !== null ? (
        <p className="text-small text-status-issue">
          {t(`settings.legacy.code.${report.blockedReason}`, {
            defaultValue: report.blockedReason,
          })}
        </p>
      ) : (
        <p className="text-small text-muted-foreground">
          {t('settings.legacy.reportSummary', {
            imported: report.imported,
            skipped: report.skipped,
            blocked: report.blocked,
          })}
        </p>
      )}

      {report.entries.length > 0 && (
        <ul className="flex flex-col gap-1.5">
          {report.entries.map((entry, index) => (
            <li
              key={`${entry.legacyId}-${index}`}
              className="flex min-w-0 flex-col gap-0.5 text-caption"
            >
              <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                <span className="font-semibold">{entry.name || `#${entry.legacyId}`}</span>
                <span className="text-muted-foreground">
                  {entry.applied
                    ? t('settings.legacy.outcome.applied')
                    : t('settings.legacy.outcome.notApplied')}
                </span>
                <span className="text-muted-foreground">
                  {t(`settings.legacy.code.${entry.code}`, { defaultValue: entry.code })}
                </span>
              </div>
              {entry.detail !== '' && (
                <code dir="ltr" className="min-w-0 break-all font-mono text-muted-foreground">
                  {entry.detail}
                </code>
              )}
            </li>
          ))}
        </ul>
      )}
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

function ProfileFacts({
  profile,
  checkboxId,
  selected,
  disabled,
  onToggle,
}: {
  profile: LegacyProfileDto;
  checkboxId: string;
  selected: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <label
          htmlFor={checkboxId}
          className="flex items-center gap-1.5 text-caption text-muted-foreground"
        >
          <input
            id={checkboxId}
            type="checkbox"
            checked={selected}
            disabled={disabled}
            onChange={onToggle}
            className="size-3.5 accent-primary"
          />
          {t('settings.legacy.select')}
        </label>
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
