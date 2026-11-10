import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { save as saveFile } from '@tauri-apps/plugin-dialog';

import type { IpcError } from '../../bindings/IpcError';
import type { ModpackExportPlanDto } from '../../bindings/ModpackExportPlanDto';
import type { ModpackExportSettingsDto } from '../../bindings/ModpackExportSettingsDto';
import type { ModpackExportSummaryDto } from '../../bindings/ModpackExportSummaryDto';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  listExportFolders,
  planModpackExport,
  toIpcError,
  writeModpackExport,
} from '../../lib/ipc';
import { formatBytes } from '../../lib/format';

/** Loaders the `.mrpack` format can name; core maps each to its dependency key. */
const LOADERS = ['fabric', 'quilt', 'forge', 'neoforge'] as const;

type Busy = 'folders' | 'plan' | 'write' | null;

interface Props {
  /** Absolute instance root from the registry; never assembled here. */
  root: string;
  instanceName: string;
  gameVersion: string | null;
  loader: string | null;
}

/**
 * F-081/F-082: turn one instance into a `.mrpack`.
 *
 * Plan then write, like every mutation in this app: the preview resolves the
 * lock and reads the selected folders, and the write command re-plans against
 * the disk with the preview's lock digest as its token. Any edit clears the
 * plan, because a review of different settings describes a different pack.
 */
export function InstanceExport({ root, instanceName, gameVersion, loader }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState(instanceName);
  const [versionId, setVersionId] = useState(defaultVersionId);
  const [summary, setSummary] = useState('');
  const [targetVersion, setTargetVersion] = useState(gameVersion ?? '');
  const [targetLoader, setTargetLoader] = useState(() => normalizeLoader(loader));
  const [folders, setFolders] = useState<string[] | null>(null);
  const [foldersError, setFoldersError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [plan, setPlan] = useState<ModpackExportPlanDto | null>(null);
  const [result, setResult] = useState<ModpackExportSummaryDto | null>(null);
  const [busy, setBusy] = useState<Busy>(null);
  const [error, setError] = useState<IpcError | null>(null);

  const loadFolders = useCallback(() => {
    setFoldersError(null);
    setBusy('folders');
    listExportFolders(root)
      .then((next) => {
        setFolders(next);
        // An instance with no overridable folder keeps an empty selection; a
        // folder that disappeared is dropped rather than silently kept.
        setSelected((prev) => prev.filter((f) => next.includes(f)));
      })
      .catch(() => setFoldersError(t('export.folders.failed')))
      .finally(() => setBusy(null));
  }, [root, t]);

  useEffect(() => {
    if (open && folders === null && foldersError === null && busy !== 'folders') {
      loadFolders();
    }
  }, [open, folders, foldersError, busy, loadFolders]);

  /**
   * Every setting change invalidates the review. Keeping a plan while its
   * inputs change would let the save button write a pack the user never saw —
   * the confirm only checks the instance, not the settings.
   */
  const change = <T,>(setter: (value: T) => void) => (value: T) => {
    setter(value);
    setPlan(null);
    setResult(null);
    setError(null);
  };

  const toggleFolder = (folder: string, checked: boolean) => {
    const next = checked ? [...selected, folder].sort() : selected.filter((f) => f !== folder);
    change(setSelected)(next);
  };

  const settings = (): ModpackExportSettingsDto => ({
    name: name.trim(),
    versionId: versionId.trim(),
    summary: summary.trim() === '' ? null : summary.trim(),
    gameVersion: targetVersion.trim(),
    loader: targetLoader,
    overrideFolders: selected,
  });

  const ready =
    name.trim() !== '' &&
    versionId.trim() !== '' &&
    targetVersion.trim() !== '' &&
    targetLoader !== '';

  const preview = () => {
    if (!ready || busy !== null) {
      return;
    }
    setBusy('plan');
    setError(null);
    setPlan(null);
    setResult(null);
    planModpackExport({ instanceRoot: root, settings: settings() })
      .then(setPlan)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(null));
  };

  const write = async () => {
    if (!plan || busy !== null) {
      return;
    }
    // Freeze the reviewed pair before the dialog opens. The dialog is modal on
    // the desktop shell, but a review must never depend on that: the settings
    // and the digest the write sends are the ones this plan was built from.
    const reviewed = settings();
    const token = plan.lockSha512;
    setError(null);
    try {
      const outputPath = await saveFile({
        title: t('export.action.write'),
        defaultPath: `${plan.name}-${plan.versionId}.mrpack`,
        filters: [{ name: t('pack.fileFilter'), extensions: ['mrpack'] }],
      });
      // A cancelled dialog is not an error and changes nothing.
      if (!outputPath) {
        return;
      }
      setBusy('write');
      writeModpackExport({
        instanceRoot: root,
        settings: reviewed,
        expectedLockSha512: token,
        outputPath,
      })
        .then(setResult)
        .catch((e) => setError(toIpcError(e)))
        .finally(() => setBusy(null));
    } catch (e) {
      setError(toIpcError(e));
    }
  };

  const pending = busy !== null;

  return (
    <div className="mx-5 mt-3 rounded-control border border-border bg-card px-3 py-2.5">
      <div className="flex items-center gap-2">
        <span className="text-body font-semibold">{t('export.title')}</span>
        <Button
          type="button"
          variant="outline"
          className="ms-auto"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
        >
          {open ? t('export.action.hide') : t('export.action.open')}
        </Button>
      </div>

      {!open && <p className="mt-1.5 text-caption text-muted-foreground">{t('export.body')}</p>}

      {open && (
        <div className="mt-2.5 flex flex-col gap-2">
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <Field label={t('export.field.name')} htmlFor="export-name">
              <Input
                id="export-name"
                value={name}
                onChange={(e) => change(setName)(e.target.value)}
                disabled={pending}
                placeholder={t('export.field.namePlaceholder')}
              />
            </Field>
            <Field label={t('export.field.versionId')} htmlFor="export-version">
              <Input
                id="export-version"
                value={versionId}
                onChange={(e) => change(setVersionId)(e.target.value)}
                disabled={pending}
                dir="ltr"
                placeholder={t('export.field.versionIdPlaceholder')}
              />
            </Field>
            <Field label={t('export.field.gameVersion')} htmlFor="export-game">
              <Input
                id="export-game"
                value={targetVersion}
                onChange={(e) => change(setTargetVersion)(e.target.value)}
                disabled={pending}
                dir="ltr"
                placeholder="1.21.1"
              />
            </Field>
            <Field label={t('export.field.loader')} htmlFor="export-loader">
              <select
                id="export-loader"
                value={targetLoader}
                onChange={(e) => change(setTargetLoader)(e.target.value)}
                disabled={pending}
                className="h-8 w-full rounded-control border border-input bg-surface-sunken px-2 text-body text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-60"
              >
                {LOADERS.map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))}
              </select>
            </Field>
          </div>

          <Field label={t('export.field.summary')} htmlFor="export-summary">
            <Input
              id="export-summary"
              value={summary}
              onChange={(e) => change(setSummary)(e.target.value)}
              disabled={pending}
              placeholder={t('export.field.summaryPlaceholder')}
            />
          </Field>

          <fieldset className="flex flex-col gap-1" disabled={pending}>
            <legend className="mb-1 text-small text-muted-foreground">
              {t('export.field.folders')}
            </legend>
            {busy === 'folders' && (
              <p className="text-caption text-muted-foreground">{t('export.folders.loading')}</p>
            )}
            {foldersError && (
              <div className="flex items-center gap-2">
                <p className="text-caption text-status-issue">{foldersError}</p>
                <Button
                  type="button"
                  variant="outline"
                  className="h-7"
                  onClick={loadFolders}
                  disabled={pending}
                >
                  {t('state.action.retry')}
                </Button>
              </div>
            )}
            {folders !== null && folders.length === 0 && (
              <p className="text-caption text-muted-foreground">{t('export.folders.empty')}</p>
            )}
            {folders !== null && folders.length > 0 && (
              <div className="flex flex-wrap gap-x-3 gap-y-1">
                {folders.map((folder) => (
                  <label key={folder} className="flex items-center gap-1.5 text-small">
                    <input
                      type="checkbox"
                      checked={selected.includes(folder)}
                      onChange={(e) => toggleFolder(folder, e.target.checked)}
                      className="size-3.5 accent-[var(--color-primary)]"
                    />
                    <span dir="ltr" className="font-mono">
                      {folder}
                    </span>
                  </label>
                ))}
              </div>
            )}
          </fieldset>

          <div className="flex items-center gap-2">
            <Button type="button" onClick={preview} disabled={!ready || pending}>
              {t('export.action.plan')}
            </Button>
            <p className="text-caption text-muted-foreground">{t('export.planNote')}</p>
          </div>
        </div>
      )}

      {error && (
        <p role="alert" className="mt-2 text-small text-status-issue">
          {t('export.failed', { reason: t(`error.${error.code}`) })}
        </p>
      )}

      {plan && (
        <section className="mt-3 flex flex-col gap-2">
          <div className="flex flex-wrap items-baseline gap-2">
            <h3 className="text-body font-semibold">{t('export.planTitle')}</h3>
            <span className="text-small text-muted-foreground">
              {t('pack.version', { version: plan.versionId })}
            </span>
          </div>
          <p className="truncate text-small text-foreground" title={plan.name}>
            {plan.name}
          </p>
          {plan.summary && (
            <p dir="auto" className="text-small text-muted-foreground">
              {plan.summary}
            </p>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <Chip>{t('export.count.files', { count: plan.files.length })}</Chip>
            <Chip>{t('export.count.overrides', { count: plan.overrideCount })}</Chip>
            {plan.skipped.length > 0 && (
              <Chip tone="issue">{t('export.count.skipped', { count: plan.skipped.length })}</Chip>
            )}
            {plan.excluded.length > 0 && (
              <Chip>{t('export.count.excluded', { count: plan.excluded.length })}</Chip>
            )}
            {plan.totalOverrideBytes > 0 && <Chip>{formatBytes(plan.totalOverrideBytes)}</Chip>}
          </div>

          {plan.files.length > 0 && (
            <PlanList title={t('export.list.files')}>
              {plan.files.map((f) => (
                <li
                  key={`${f.projectId}-${f.path}`}
                  className="flex items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5"
                >
                  <span className="min-w-0 flex-1 truncate text-small">{f.title}</span>
                  <span
                    dir="ltr"
                    title={f.path}
                    className="min-w-0 flex-1 truncate font-mono text-caption text-muted-foreground"
                  >
                    {f.path}
                  </span>
                  <span dir="ltr" className="shrink-0 font-mono text-caption text-muted-foreground">
                    {formatBytes(f.size)}
                  </span>
                </li>
              ))}
            </PlanList>
          )}

          {plan.overrides.length > 0 && (
            <PlanList title={t('export.list.overrides')}>
              {plan.overrides.map((o) => (
                <li
                  key={o.archivePath}
                  className="flex items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5"
                >
                  <span
                    dir="ltr"
                    title={o.archivePath}
                    className="min-w-0 flex-1 truncate font-mono text-caption text-foreground"
                  >
                    {o.archivePath}
                  </span>
                  <span dir="ltr" className="shrink-0 font-mono text-caption text-muted-foreground">
                    {formatBytes(o.size)}
                  </span>
                </li>
              ))}
            </PlanList>
          )}

          {plan.skipped.length > 0 && (
            <PlanList title={t('export.list.skipped')} note={t('export.skippedNote')}>
              {plan.skipped.map((s) => (
                <li
                  key={`${s.projectId}-${s.versionId}`}
                  className="flex items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5"
                >
                  <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-caption">
                    {s.projectId}
                  </span>
                  <span className="shrink-0 text-caption text-status-issue">
                    {t(`export.skip.${s.reason}`)}
                  </span>
                </li>
              ))}
            </PlanList>
          )}

          {plan.excluded.length > 0 && (
            <PlanList title={t('export.list.excluded')} note={t('export.excludedNote')}>
              {plan.excluded.map((x) => (
                <li
                  key={`${x.path}-${x.reason}`}
                  className="flex items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5"
                >
                  <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-caption">
                    {x.path}
                  </span>
                  <span className="shrink-0 text-caption text-muted-foreground">
                    {t(`export.excluded.${x.reason}`)}
                  </span>
                </li>
              ))}
            </PlanList>
          )}

          <div className="mt-1 flex gap-2">
            <Button type="button" onClick={write} disabled={pending}>
              {busy === 'write' ? t('export.saving') : t('export.action.write')}
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setPlan(null);
                setResult(null);
              }}
              disabled={pending}
            >
              {t('state.action.cancel')}
            </Button>
          </div>
        </section>
      )}

      {result && (
        <div className="mt-2 flex flex-col gap-1.5">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-small font-semibold text-foreground">{t('export.savedLabel')}</span>
            <Chip>{t('export.count.files', { count: result.fileCount })}</Chip>
            <Chip>{t('export.count.overrides', { count: result.overrideCount })}</Chip>
            <Chip>{formatBytes(result.totalSize)}</Chip>
          </div>
          <p
            role="status"
            aria-live="polite"
            dir="ltr"
            title={result.output}
            className="truncate rounded-control border border-border bg-surface-sunken px-2.5 py-1.5 font-mono text-caption text-muted-foreground"
          >
            {result.output}
          </p>
        </div>
      )}
    </div>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={htmlFor} className="text-small text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  );
}

function PlanList({
  title,
  note,
  children,
}: {
  title: string;
  note?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <p className="text-small font-medium text-foreground">{title}</p>
      {note && <p className="text-caption text-muted-foreground">{note}</p>}
      <ul className="flex max-h-48 flex-col gap-1 overflow-auto">{children}</ul>
    </div>
  );
}

function Chip({ children, tone }: { children: ReactNode; tone?: 'issue' }) {
  return (
    <span
      className={
        tone === 'issue'
          ? 'rounded-chip border border-border px-[7px] py-px text-caption text-status-issue'
          : 'rounded-chip border border-border px-[7px] py-px text-caption text-muted-foreground'
      }
    >
      {children}
    </span>
  );
}

/** The old exporter's timestamp shape, in UTC so a pack id is timezone-free. */
function defaultVersionId(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}.${p(d.getUTCMonth() + 1)}.${p(d.getUTCDate())}-${p(
    d.getUTCHours(),
  )}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}`;
}

function normalizeLoader(loader: string | null): string {
  const value = (loader ?? '').trim().toLowerCase();
  return (LOADERS as readonly string[]).includes(value) ? value : 'fabric';
}
