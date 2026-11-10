import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { DownloadHistoryEntryDto } from '../../bindings/DownloadHistoryEntryDto';
import type { InstallEventDto } from '../../bindings/InstallEventDto';
import type { InstallRequest } from '../../bindings/InstallRequest';
import type { IpcError } from '../../bindings/IpcError';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState } from '../../components/state';
import {
  clearDownloadHistory,
  type DownloadHistoryStatus,
  executeInstall,
  listDownloadHistory,
  toIpcError,
} from '../../lib/ipc';
import { formatBytes, formatRelativeTime } from '../../lib/format';
import { useGameTarget } from '../gameTarget';

interface Row extends InstallEventDto {
  /** The last thing that happened, so a re-sent download event does not
   *  overwrite a terminal one. */
  phase: string;
}

/** The three tabs of the reference screen: the queue, what worked, and all of it. */
type Tab = 'live' | 'completed' | 'history';

const TABS: readonly Tab[] = ['live', 'completed', 'history'];

function statusClass(status: string): string {
  switch (status) {
    case 'success':
      return 'text-status-success';
    case 'skipped':
      return 'text-status-manual';
    case 'failed':
      return 'text-status-issue';
    case 'cancelled':
      return 'text-status-warning';
    default:
      return 'text-muted-foreground';
  }
}

/**
 * F-063 and F-152. The queue, as it runs, and the log of what it did.
 *
 * The live rows came off a channel, so this page holds no state that Rust does
 * not also have: the journal has the record, and a reload loses the live view
 * rather than the truth. That is deliberate — a progress bar that survives a
 * restart but disagrees with the journal is worse than one that resets.
 *
 * The other two tabs read `download_history` instead, which is why they
 * survive the restart the queue does not.
 */
export function DownloadsPage() {
  const { t, i18n } = useTranslation();
  const [tab, setTab] = useState<Tab>('live');
  const [rows, setRows] = useState<Row[]>([]);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);
  const [target, setTarget] = useState('');
  const [projects, setProjects] = useState('');
  // The same version, loader and provider Browse resolved with. They used to
  // be hard-coded here, so a search for 1.20.4 followed by a manual install
  // quietly pulled the 1.21.1 build.
  const { gameVersion, loader, provider } = useGameTarget();

  // The log: `null` means "not read yet", which is different from an empty log.
  const [history, setHistory] = useState<DownloadHistoryEntryDto[] | null>(null);
  const [historyError, setHistoryError] = useState<IpcError | null>(null);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [search, setSearch] = useState('');
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);
  // A fast typist can have two reads in flight; only the newest may write.
  const readId = useRef(0);

  const onEvent = useCallback((e: InstallEventDto) => {
    setRows((prev) => {
      const existing = prev.find((r) => r.itemId === e.itemId);
      // A terminal row is the truth about that file. A later stray progress
      // event must not reopen it — a queue that says "installing" about a jar
      // that already landed is worse than a slightly stale row.
      if (existing && terminal(existing.phase) && !terminal(e.phase)) {
        return prev;
      }
      return [...prev.filter((r) => r.itemId !== e.itemId), { ...e }];
    });
  }, []);

  const start = useCallback(() => {
    const req: InstallRequest = {
      instanceRoot: target.trim(),
      projects: projects
        .split(/[\s,]+/)
        .map((s) => s.trim())
        .filter(Boolean),
      // One provider answers the whole request, like a search: the ids in the
      // field mean nothing to the other catalogue.
      provider,
      // The version and the loader come from the same place Browse reads
      // them, so the queue installs what the screen asked for.
      gameVersion: gameVersion.trim(),
      loader: loader.trim().toLowerCase(),
    };
    // An empty version or loader is refused: core reads an empty version as
    // "any version" and would install the newest build of the fallback
    // loader, which may not match the instance at all.
    if (
      req.projects.length === 0 ||
      req.instanceRoot.length === 0 ||
      req.gameVersion.length === 0 ||
      req.loader.length === 0
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    setRows([]);
    executeInstall(req, onEvent)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  }, [gameVersion, loader, provider, target, projects, onEvent]);

  /** Reads the log for the open tab. The tab is the filter; the field is the text. */
  const loadHistory = useCallback((status: DownloadHistoryStatus | undefined, term: string) => {
    const id = ++readId.current;
    setLoadingHistory(true);
    setHistoryError(null);
    listDownloadHistory({ status, search: term })
      .then((entries) => {
        if (id === readId.current) {
          setHistory(entries);
        }
      })
      .catch((e) => {
        if (id === readId.current) {
          setHistoryError(toIpcError(e));
        }
      })
      .finally(() => {
        if (id === readId.current) {
          setLoadingHistory(false);
        }
      });
  }, []);

  useEffect(() => {
    if (tab === 'live') {
      return;
    }
    // Typing should not fire one query per keystroke; the delay is short
    // enough that the table still reads as live.
    const status: DownloadHistoryStatus | undefined = tab === 'completed' ? 'success' : undefined;
    const term = search.trim();
    const id = window.setTimeout(() => loadHistory(status, term), 200);
    return () => window.clearTimeout(id);
  }, [tab, search, loadHistory]);

  const clearHistory = useCallback(() => {
    setClearing(true);
    setHistoryError(null);
    clearDownloadHistory()
      .then(() => {
        setHistory([]);
        setConfirmClear(false);
      })
      .catch((e) => setHistoryError(toIpcError(e)))
      .finally(() => setClearing(false));
  }, []);

  // The queue installs what Browse resolved with, so the pair has to be there.
  const hasVersion = gameVersion.trim().length > 0 && loader.trim().length > 0;

  const live = (
    <>
      <div className="flex flex-col gap-2 px-5 pt-4">
        {hasVersion && (
          <p className="text-small text-muted-foreground">
            {t('install.gameTarget', {
              provider: t(`provider.${provider}`),
              version: gameVersion.trim(),
              loader: loader.trim().toLowerCase(),
            })}
          </p>
        )}
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
            value={projects}
            onChange={(e) => setProjects(e.target.value)}
            dir="ltr"
            placeholder={t('install.projects')}
            aria-label={t('install.projects')}
            className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
          />
          <Button type="button" onClick={start} disabled={busy || !hasVersion}>
            {t('install.start')}
          </Button>
        </div>
      </div>

      {error && (
        <div className="px-5 pt-3">
          <ErrorState error={error} />
        </div>
      )}

      <div className="grid grid-cols-1 gap-3 px-5 pb-5 pt-4 sm:grid-cols-2 lg:grid-cols-3">
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

  const log = (
    <>
      <div className="flex flex-wrap items-center gap-2 px-5 pt-3">
        <input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t('downloads.searchPlaceholder')}
          aria-label={t('downloads.searchPlaceholder')}
          className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
        />
        {search.length > 0 && (
          <Button type="button" variant="ghost" size="sm" onClick={() => setSearch('')}>
            {t('downloads.clearSearch')}
          </Button>
        )}{tab === 'history' &&
          (confirmClear ? (
            <div className="flex items-center gap-1.5">
              <Button
                type="button"
                variant="destructive"
                size="sm"
                onClick={clearHistory}
                disabled={clearing}
              >
                {t('downloads.clearConfirm')}
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => setConfirmClear(false)}
                disabled={clearing}
              >
                {t('state.action.cancel')}
              </Button>
            </div>
          ) : (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setConfirmClear(true)}
              disabled={clearing || !history || history.length === 0}
            >
              {t('downloads.clear')}
            </Button>
          ))}
      </div>

      {historyError ? (
        <div className="px-5 pt-4">
          <ErrorState
            error={historyError}
            onRetry={() =>
              loadHistory(tab === 'completed' ? 'success' : undefined, search.trim())
            }
          />
        </div>
      ) : loadingHistory && history === null ? (
        <p className="px-5 pt-6 text-small text-muted-foreground">{t('downloads.loading')}</p>
      ) : !history || history.length === 0 ? (
        <EmptyState
          title={
            tab === 'completed'
              ? t('downloads.emptyCompletedTitle')
              : t('downloads.emptyHistoryTitle')
          }
          body={
            tab === 'completed' ? t('downloads.emptyCompletedBody') : t('downloads.emptyHistoryBody')
          }
        />
      ) : (
        <div className="overflow-x-auto px-5 pb-5 pt-3">
          {/* Fixed layout with declared widths: an auto-layout table sizes
              itself to the longest filename and then scrolls the project name
              out of view, which is the one column the eye starts from. */}
          <table className="w-full table-fixed border-collapse text-small">
            <colgroup>
              <col className="w-[40%]" />
              <col className="w-[14%]" />
              <col className="w-[18%]" />
              <col className="w-[12%]" />
              <col className="w-[16%]" />
            </colgroup>
            <thead>
              <tr className="text-caption text-muted-foreground">
                <th className="border-b border-border px-2 py-2 text-start font-medium">
                  {t('downloads.colProject')}
                </th>
                <th className="border-b border-border px-2 py-2 text-start font-medium">
                  {t('downloads.colVersion')}
                </th>
                <th className="border-b border-border px-2 py-2 text-start font-medium">
                  {t('downloads.colTarget')}
                </th>
                <th className="border-b border-border px-2 py-2 text-start font-medium">
                  {t('downloads.colStatus')}
                </th>
                <th className="border-b border-border px-2 py-2 text-start font-medium">
                  {t('downloads.colWhen')}
                </th>
              </tr>
            </thead>
            <tbody>
              {history.map((row) => (
                <tr key={row.id} className="align-top">
                  <td className="border-b border-border px-2 py-2">
                    <p className="truncate font-medium text-foreground" title={row.title}>
                      {row.title}
                    </p>
                    <p
                      className="truncate font-mono text-caption text-muted-foreground"
                      title={row.destinationPath ?? row.fileName}
                    >
                      {/* LTR characters, but laid out in the cell's own
                          direction: the filename should start where the title
                          starts, not jump to the far side of the column. */}
                      <span dir="ltr">{row.fileName}</span>
                    </p>
                  </td>
                  <td className="border-b border-border px-2 py-2 font-mono text-caption text-muted-foreground">
                    <span
                      dir="ltr"
                      className="block truncate"
                      title={row.versionNumber ?? row.versionId ?? undefined}
                    >
                      {row.versionNumber ?? row.versionId ?? '—'}
                    </span>
                  </td>
                  <td className="border-b border-border px-2 py-2 text-caption text-muted-foreground">
                    <span className="block truncate">
                      {row.gameVersion} · {row.loader}
                    </span>
                  </td>
                  <td className="border-b border-border px-2 py-2">
                    <span
                      className={`text-caption font-medium ${statusClass(row.status)}`}
                      title={row.message || undefined}
                    >
                      {t(`downloads.status.${row.status}`)}
                    </span>
                  </td>
                  <td className="border-b border-border px-2 py-2 text-caption text-muted-foreground">
                    {formatRelativeTime(row.createdAt, i18n.language)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-5 pt-4">
        <h1 className="text-page-title font-semibold">{t('nav.downloads')}</h1>
        {tab === 'live' && rows.length > 0 && (
          <span className="ms-1 text-small text-muted-foreground">
            {t('install.count', { count: rows.length })}
          </span>
        )}
      </div>

      <div className="mt-3 flex gap-1 border-b border-border px-5" role="tablist">
        {TABS.map((value) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={value === tab}
            onClick={() => setTab(value)}
            className={
              value === tab
                ? '-mb-px border-b-2 border-primary px-2.5 py-1.5 text-small font-semibold text-foreground'
                : '-mb-px border-b-2 border-transparent px-2.5 py-1.5 text-small text-muted-foreground hover:text-foreground'
            }
          >
            {t(`downloads.tab.${value}`)}
          </button>
        ))}
      </div>

      <div className="pt-1">
        {tab === 'live' ? (
          rows.length === 0 ? (
            <>
              <EmptyState
                title={t('page.downloads.emptyTitle')}
                body={t('page.downloads.emptyBody')}
              />
              {live}
            </>
          ) : (
            live
          )
        ) : (
          log
        )}
      </div>
    </div>
  );
}

// Explicit membership, not a suffix test: `install.phase.unlanded` would match
// `endsWith('landed')`, and a renamed key would stop being terminal without
// anything failing.
const TERMINAL = new Set([
  'install.phase.landed',
  'install.phase.skipped',
  'install.phase.failed',
  'install.phase.cancelled',
]);
const terminal = (phase: string) => TERMINAL.has(phase);
