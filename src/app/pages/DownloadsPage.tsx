import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { InstallEventDto } from '../../bindings/InstallEventDto';
import type { InstallRequest } from '../../bindings/InstallRequest';
import type { InstallResultDto } from '../../bindings/InstallResultDto';
import type { IpcError } from '../../bindings/IpcError';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState } from '../../components/state';
import { executeInstall, toIpcError } from '../../lib/ipc';
import { formatBytes } from '../../lib/format';

interface Row extends InstallEventDto {
  /** The last thing that happened, so a re-sent download event does not
   *  overwrite a terminal one. */
  phase: string;
}

/**
 * F-063. The queue, as it runs.
 *
 * Every row here came off a channel, so this page holds no state that Rust
 * does not also have: the journal has the record, and a reload loses the live
 * view rather than the truth. That is deliberate — a progress bar that survives
 * a restart but disagrees with the journal is worse than one that resets.
 */
export function DownloadsPage() {
  const { t, i18n } = useTranslation();
  const [rows, setRows] = useState<Row[]>([]);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);
  const [lastResult, setLastResult] = useState<InstallResultDto | null>(null);
  const [target, setTarget] = useState('');
  const [slugs, setSlugs] = useState('');

  const onEvent = useCallback((e: InstallEventDto) => {
    setRows((prev) => {
      const next = prev.filter((r) => r.itemId !== e.itemId);
      next.push({ ...e });
      return next;
    });
  }, []);

  const start = useCallback(() => {
    const req: InstallRequest = {
      instanceRoot: target.trim(),
      slugs: slugs
        .split(/[\s,]+/)
        .map((s) => s.trim())
        .filter(Boolean),
      gameVersion: '1.21.1',
      loader: 'fabric',
    };
    if (req.slugs.length === 0 || req.instanceRoot.length === 0) {
      return;
    }
    setBusy(true);
    setError(null);
    setRows([]);
    executeInstall(req, onEvent)
      .then(setLastResult)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  }, [target, slugs, onEvent]);

  const terminal = (phase: string) =>
    phase.endsWith('landed') || phase.endsWith('skipped') || phase.endsWith('failed') || phase.endsWith('cancelled');

  const content = (
    <>
      <div className="flex flex-col gap-2 px-5 pt-4">
        <div className="flex items-center gap-2">
          <input
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            dir="ltr"
            placeholder={t('install.target')}
            aria-label={t('install.target')}
            className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
          />
        </div>
        <div className="flex items-center gap-2">
          <input
            value={slugs}
            onChange={(e) => setSlugs(e.target.value)}
            dir="ltr"
            placeholder={t('install.slugs')}
            aria-label={t('install.slugs')}
            className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
          />
          <Button type="button" onClick={start} disabled={busy}>
            {t('install.start')}
          </Button>
        </div>
      </div>

      {lastResult && (
        <p className="px-5 pt-3 text-small text-muted-foreground">
          {t('install.installed', { count: lastResult.installed.length })}
        </p>
      )}

      <div className="grid grid-cols-1 gap-3 px-5 pb-5 sm:grid-cols-2 lg:grid-cols-3">
        {rows.map((r) => {
          // u64 crosses as bigint (ts-rs). Both are bounded by the byte counts
          // a download can reach, so Number() is exact here and nowhere else
          // needs it.
          const done = r.done === null ? null : Number(r.done);
          const total = r.total === null ? null : Number(r.total);
          const pct =
            total !== null && total > 0 && done !== null ? Math.min(100, (done / total) * 100) : null;
          const isDone = terminal(r.phase);
          return (
            <article
              key={r.itemId}
              className="flex flex-col gap-2 rounded-card border border-border bg-card p-3.5"
            >
              <p className="truncate text-body font-medium" title={r.path ?? r.itemId}>
                {r.path ?? r.itemId}
              </p>
              <div
                role="progressbar"
                aria-valuenow={pct ?? undefined}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-label={t(r.phase)}
                className="h-1.5 overflow-hidden rounded-chip bg-surface-sunken"
              >
                <div
                  className="h-full bg-primary"
                  style={{
                    // Unknown total: a bar that guesses is a lie. It fills only
                    // when the server actually told us how big the file is.
                    width: pct === null ? (isDone ? '100%' : '0%') : `${pct}%`,
                    transition: isDone ? 'none' : 'width 120ms linear',
                  }}
                />
              </div>
              <div className="flex items-center justify-between gap-2">
                <span
                  className={
                    done
                      ? 'text-caption text-muted-foreground'
                      : 'text-caption text-primary'
                  }
                >
                  {t(r.phase)}
                </span>
                {total !== null && done !== null && (
                  <span dir="ltr" className="font-mono text-caption text-muted-foreground">
                    {formatBytes(done)} / {formatBytes(total)}
                  </span>
                )}
              </div>
              {r.message && (
                <p dir="auto" className="font-mono text-caption text-status-issue">
                  {r.message}
                </p>
              )}
            </article>
          );
        })}
      </div>
    </>
  );

  if (error) {
    return (
      <div className="flex flex-col">
        <div className="px-5 pt-4">
          <h1 className="text-page-title font-semibold">{t('nav.downloads')}</h1>
        </div>
        <div className="px-5 pt-4">
          <ErrorState error={error} />
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-5 pt-4">
        <h1 className="text-page-title font-semibold">{t('nav.downloads')}</h1>
        <span className="ms-1 text-small text-muted-foreground">
          {i18n.language === 'ar' ? '' : ''}
          {rows.length > 0 ? t('install.installed', { count: rows.length }) : ''}
        </span>
      </div>
      {rows.length === 0 ? (
        <>
          <EmptyState title={t('page.downloads.emptyTitle')} body={t('page.downloads.emptyBody')} />
          {content}
        </>
      ) : (
        content
      )}
    </div>
  );
}
