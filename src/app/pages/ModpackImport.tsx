import { useCallback, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { open as openFilePicker, open as openFolderPicker } from '@tauri-apps/plugin-dialog';
import { cn } from 'cn';

import type { ExportArchivePreviewDto } from '../../bindings/ExportArchivePreviewDto';
import type { IpcError } from '../../bindings/IpcError';
import type { ModpackApplyResultDto } from '../../bindings/ModpackApplyResultDto';
import type { PackEventDto } from '../../bindings/PackEventDto';
import { Button } from '@/components/ui/button';
import { ErrorState } from '../../components/state';
import { executeModpackArchiveImport, previewModpackExportArchive, toIpcError } from '../../lib/ipc';
import { formatBytes, formatNumber } from '../../lib/format';

/**
 * One file the import run has reported on, reduced from the channel — the
 * same shape `ModpackPage` and `PackRecoveryBanner` reduce the apply's
 * channel to, so an import reads like the apply it is.
 */
interface ProgressRow {
  safePath: string;
  phase: string;
  done: number;
  total: number | null;
}

/**
 * F-158. Importing one of this app's own export archives as a NEW instance.
 *
 * The order the screen enforces is the order the command enforces: the
 * archive is previewed metadata-only first — the manifest must be there and
 * must agree with the index, or the file is not our export and nothing
 * happens — and only then can the user name the folder the new instance is
 * created in. The create plan itself is never built here: the command
 * rebuilds it in Rust from the folder and the name, so what this screen
 * collects is an answer to a question core re-asks, not an instruction.
 */
export function ModpackImport() {
  const { t } = useTranslation();
  const [packPath, setPackPath] = useState('');
  const [preview, setPreview] = useState<ExportArchivePreviewDto | null>(null);
  const [root, setRoot] = useState('');
  const [name, setName] = useState('');
  const [busy, setBusy] = useState<'preview' | 'import' | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [rows, setRows] = useState<ProgressRow[]>([]);
  const [packPhase, setPackPhase] = useState<string | null>(null);
  const [result, setResult] = useState<ModpackApplyResultDto | null>(null);

  /** An edit on the archive field forgets the preview: it described a file
   *  that is no longer the one on screen. */
  const changeArchive = (value: string) => {
    setPackPath(value);
    setPreview(null);
    setError(null);
    setResult(null);
    setRows([]);
    setPackPhase(null);
  };

  /** The native file picker. A cancelled dialog is not an error: the field
   *  stays exactly as it was. */
  const pickArchive = async () => {
    try {
      const picked = await openFilePicker({
        directory: false,
        multiple: false,
        title: t('pack.import.title'),
        filters: [{ name: t('pack.fileFilter'), extensions: ['mrpack'] }],
      });
      if (typeof picked === 'string' && picked.length > 0) {
        changeArchive(picked);
        setBusy('preview');
        previewModpackExportArchive({ packPath: picked })
          .then((next) => {
            setPreview(next);
            // The manifest's own name is the honest pre-fill; the user edits it.
            setName((current) => current || next.name);
          })
          .catch((e) => setError(toIpcError(e)))
          .finally(() => setBusy(null));
      }
    } catch (e) {
      setError(toIpcError(e));
    }
  };

  /** The native folder picker; the typed path stays editable, and the
   *  absolute-path rule is enforced in Rust, not here. */
  const pickRoot = async () => {
    try {
      const picked = await openFolderPicker({
        directory: true,
        multiple: false,
        title: t('pack.import.fieldRoot'),
      });
      if (typeof picked === 'string' && picked.length > 0) {
        setRoot(picked);
        setError(null);
      }
    } catch (e) {
      setError(toIpcError(e));
    }
  };

  const onEvent = useCallback((e: PackEventDto) => {
    if (e.safePath === null) {
      setPackPhase(e.phase);
      return;
    }
    const path = e.safePath;
    setRows((prev) => {
      const index = prev.findIndex((r) => r.safePath === path);
      if (index !== -1 && settled(prev[index].phase) && !settled(e.phase)) {
        return prev;
      }
      const row: ProgressRow = { safePath: path, phase: e.phase, done: e.done, total: e.total };
      if (index === -1) {
        return [...prev, row];
      }
      const next = [...prev];
      next[index] = row;
      return next;
    });
  }, []);

  const importPack = () => {
    if (!preview) {
      return;
    }
    setBusy('import');
    setError(null);
    setRows([]);
    setPackPhase(null);
    setResult(null);
    // The digest is the preview's own, not a fresh hash: it binds this confirm
    // to the archive the user just read. A file swapped in between is refused
    // by the command before anything is created.
    executeModpackArchiveImport(
      {
        packPath: packPath.trim(),
        instanceRoot: root.trim(),
        instanceName: name.trim(),
        packSha512: preview.packSha512,
      },
      onEvent,
    )
      .then((next) => setResult(next))
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(null));
  };

  const ready = preview !== null && root.trim().length > 0 && name.trim().length > 0;

  return (
    <section className="mx-5 mb-5 rounded-control border border-border bg-card px-4 py-3">
      <h2 className="text-body font-semibold">{t('pack.import.title')}</h2>
      <p className="mt-0.5 text-small text-muted-foreground">{t('pack.import.body')}</p>

      <div className="mt-2.5 flex flex-col gap-1">
        <label htmlFor="pack-import-path" className="text-small text-muted-foreground">
          {t('pack.import.actionPick')}
        </label>
        <div className="flex gap-2">
          <input
            id="pack-import-path"
            value={packPath}
            onChange={(e) => changeArchive(e.target.value)}
            disabled={busy !== null}
            dir="ltr"
            className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 font-mono text-body text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-60"
          />
          <Button
            type="button"
            variant="outline"
            onClick={pickArchive}
            disabled={busy !== null}
            aria-label={t('pack.import.actionPick')}
          >
            {t('pack.import.actionPick')}
          </Button>
        </div>
      </div>

      {error && (
        <div className="mt-2.5">
          <ErrorState error={error} />
        </div>
      )}

      {preview && (
        <div className="mt-2.5 rounded-control border border-border bg-surface-sunken px-3 py-2">
          <div className="flex flex-wrap items-baseline gap-2">
            <h3 className="text-body font-medium">{t('pack.import.previewTitle')}</h3>
            <span className="text-small text-muted-foreground">
              {t('pack.version', { version: preview.versionId })}
            </span>
          </div>
          <p className="mt-0.5 truncate text-small text-foreground" title={preview.name}>
            {preview.name}
          </p>
          {preview.summary && (
            <p dir="auto" className="mt-1 text-small text-muted-foreground">
              {preview.summary}
            </p>
          )}
          <p className="mt-1 text-small text-muted-foreground">
            {t('export.field.gameVersion')}: {preview.gameVersion} · {t('export.field.loader')}:{' '}
            {preview.loader}
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <Chip>{t('export.count.files', { count: preview.fileCount })}</Chip>
            <Chip>{t('export.count.overrides', { count: preview.overrideCount })}</Chip>
            <Chip>{t('export.count.skipped', { count: preview.skippedCount })}</Chip>
            {preview.excluded.length > 0 && (
              <Chip tone="issue">
                {t('export.count.excluded', { count: preview.excluded.length })}
              </Chip>
            )}
          </div>
          {preview.includedFiles.length > 0 && (
            <PlanList title={t('pack.import.planTitle')} subtitle={t('export.list.files')} paths={preview.includedFiles} />
          )}
          {preview.includedOverrides.length > 0 && (
            <PlanList title={t('export.list.overrides')} paths={preview.includedOverrides} />
          )}
          {preview.excluded.length > 0 && (
            <ul className="mt-2 flex flex-col gap-1">
              {preview.excluded.map((e) => (
                <li
                  key={e.path}
                  className="flex items-center gap-2 rounded-control border border-border bg-card px-2.5 py-1.5"
                >
                  <span
                    dir="ltr"
                    title={e.path}
                    className="min-w-0 flex-1 truncate font-mono text-caption text-foreground"
                  >
                    {e.path}
                  </span>
                  <span className="shrink-0 text-caption text-muted-foreground">
                    {t(`export.excluded.${e.reason}`)}
                  </span>
                </li>
              ))}
            </ul>
          )}
          <p className="mt-2 text-caption text-muted-foreground">{t('pack.import.previewNote')}</p>
        </div>
      )}

      {preview && (
        <div className="mt-2.5 flex flex-col gap-1">
          <label htmlFor="pack-import-root" className="text-small text-muted-foreground">
            {t('pack.import.fieldRoot')}
          </label>
          <div className="flex gap-2">
            <input
              id="pack-import-root"
              value={root}
              onChange={(e) => {
                setRoot(e.target.value);
                setError(null);
              }}
              disabled={busy !== null}
              dir="ltr"
              placeholder={t('pack.import.fieldRootPlaceholder')}
              className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 font-mono text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-60"
            />
            <Button
              type="button"
              variant="outline"
              onClick={pickRoot}
              disabled={busy !== null}
              aria-label={t('pack.import.fieldRoot')}
            >
              {t('instance.dialog.actionBrowse')}
            </Button>
          </div>
        </div>
      )}

      {preview && (
        <div className="mt-2.5 flex flex-col gap-1">
          <label htmlFor="pack-import-name" className="text-small text-muted-foreground">
            {t('pack.import.fieldName')}
          </label>
          <input
            id="pack-import-name"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              setError(null);
            }}
            disabled={busy !== null}
            className="h-8 w-full rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-60"
          />
        </div>
      )}

      {preview && (
        <div className="mt-3 flex items-center gap-2">
          <Button type="button" onClick={importPack} disabled={!ready || busy !== null}>
            {t('pack.import.actionImport')}
          </Button>
          {busy === 'preview' && (
            <span className="text-small text-muted-foreground">{t('export.folders.loading')}</span>
          )}
          {busy === 'import' && (
            <span className="text-small text-muted-foreground">{t('pack.import.importing')}</span>
          )}
        </div>
      )}

      {(rows.length > 0 || packPhase !== null || result !== null) && (
        <div className="mt-2.5 flex flex-col gap-1.5 border-t border-border pt-2.5">
          {packPhase && (
            <p role="status" aria-live="polite" className="text-small text-muted-foreground">
              {t(packPhase)}
            </p>
          )}
          {result && (
            <p className="text-small text-muted-foreground">
              {t('pack.import.result', {
                written: formatNumber(result.written),
                skipped: formatNumber(result.skipped),
                size: formatBytes(result.bytes),
              })}
            </p>
          )}
          {rows.map((row) => (
            <ProgressRowView key={row.safePath} row={row} />
          ))}
        </div>
      )}
    </section>
  );
}

/** Phases that say a file is at its destination — the apply's set. */
const SETTLED = new Set(['pack.phase.written', 'pack.phase.skipped']);

function settled(phase: string): boolean {
  return SETTLED.has(phase);
}

function Chip({ children, tone }: { children: ReactNode; tone?: 'issue' }) {
  return (
    <span
      className={cn(
        'rounded-chip border border-border px-[7px] py-px text-caption',
        tone === 'issue' ? 'text-status-issue' : 'text-muted-foreground',
      )}
    >
      {children}
    </span>
  );
}

/**
 * The write plan the manifest records: the paths the import will write,
 * shown before the user confirms — the same honesty the `.mrpack` flow owes
 * the user (AGENTS.md §5.4). Building this list read the manifest in place
 * only; nothing was written to plan it.
 */
function PlanList({ title, subtitle, paths }: { title: string; subtitle?: string; paths: string[] }) {
  return (
    <div className="mt-2 flex flex-col gap-0.5 rounded-control border border-border bg-card px-2.5 py-1.5">
      <p className="text-caption text-foreground">{title}</p>
      {subtitle && <p className="text-caption text-muted-foreground">{subtitle}</p>}
      <ul className="flex max-h-40 flex-col gap-0.5 overflow-y-auto">
        {paths.map((path) => (
          <li
            key={path}
            dir="ltr"
            title={path}
            className="truncate font-mono text-caption text-foreground"
          >
            {path}
          </li>
        ))}
      </ul>
    </div>
  );
}

/** One file, as the channel last described it — `ModpackPage`'s row view. */
function ProgressRowView({ row }: { row: ProgressRow }) {
  const { t } = useTranslation();
  const pct =
    row.total !== null && row.total > 0 ? Math.min(100, (row.done / row.total) * 100) : null;
  const done = settled(row.phase);
  return (
    <div className="flex flex-col gap-1 rounded-control border border-border bg-card px-3 py-2">
      <div className="flex items-center gap-2">
        <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-caption text-foreground">
          {row.safePath}
        </span>
        <span
          className={cn(
            'shrink-0 text-caption',
            done ? 'text-muted-foreground' : 'text-primary',
          )}
        >
          {t(row.phase)}
        </span>
        {row.total !== null && (
          <span dir="ltr" className="shrink-0 font-mono text-caption text-muted-foreground">
            {formatBytes(row.done)} / {formatBytes(row.total)}
          </span>
        )}
      </div>
      <div
        role="progressbar"
        aria-valuenow={pct ?? undefined}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={t(row.phase)}
        className="h-1.5 overflow-hidden rounded-chip bg-surface-sunken"
      >
        <div
          className="h-full bg-primary"
          style={{
            width: pct === null ? (done ? '100%' : '0%') : `${pct}%`,
            transition: done ? 'none' : 'width 120ms linear',
          }}
        />
      </div>
    </div>
  );
}
