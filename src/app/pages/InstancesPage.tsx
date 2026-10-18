import { useCallback, useEffect, useMemo, useState } from 'react';
import { openPath } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { TriangleAlert, CircleDashed, Plus } from 'lucide-react';
import { cn } from 'cn';

import type { IpcError } from '../../bindings/IpcError';
import type { InstanceSummaryDto } from '../../bindings/InstanceSummaryDto';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, LoadingState } from '../../components/state';
import { listInstances, toIpcError } from '../../lib/ipc';
import { formatRelativeTime } from '../../lib/format';

import { NewInstanceDialog } from './NewInstanceDialog';

// Board 05. Only what the four IPC commands actually return is rendered: the
// board's "N mods", "N updates" and "Review issues" chips need a scan result
// and an update check that do not exist yet, so they are absent rather than
// faked.

function scannedLabel(
  row: InstanceSummaryDto,
  locale: string,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  if (row.scannedAt === null || row.scannedAt === undefined) {
    return t('page.instances.neverScanned');
  }
  return t('page.instances.scannedAgo', {
    time: formatRelativeTime(row.scannedAt, locale),
  });
}

/** One chip, or nothing: a healthy instance gets no badge, as on the board. */
function StatusChip({ row }: { row: InstanceSummaryDto }) {
  const { t } = useTranslation();
  if (row.pathMissing) {
    return (
      <span className="ms-auto flex items-center gap-1 text-small whitespace-nowrap text-status-issue">
        <TriangleAlert aria-hidden className="size-[18px]" strokeWidth={1.75} />
        {t('page.instances.statusMissing')}
      </span>
    );
  }
  if (row.scannedAt === null || row.scannedAt === undefined) {
    return (
      <span className="ms-auto flex items-center gap-1 text-small whitespace-nowrap text-status-disabled">
        <CircleDashed aria-hidden className="size-[18px]" strokeWidth={1.75} />
        {t('page.instances.statusUnscanned')}
      </span>
    );
  }
  return null;
}

function Card({ row, locale, unknownLabel }: { row: InstanceSummaryDto; locale: string; unknownLabel: string }) {
  const { t } = useTranslation();
  const initial = row.name.trim().charAt(0).toUpperCase() || '?';

  return (
    <article className="flex flex-col gap-2 rounded-card border border-border bg-card p-3.5">
      <div className="flex items-center gap-2">
        <span
          aria-hidden
          className="grid size-10 shrink-0 place-items-center rounded-control border border-border bg-accent text-small font-semibold text-primary"
        >
          {initial}
        </span>
        <div className="min-w-0">
          <div className="truncate font-semibold text-foreground">{row.name}</div>
          <div className="text-small text-muted-foreground">
            {scannedLabel(row, locale, t)}
          </div>
        </div>
        <StatusChip row={row} />
      </div>

      <div className="flex items-center gap-2">
        <span className="rounded-chip border border-border px-[7px] py-px text-caption text-muted-foreground">
          {row.minecraftVersion ?? unknownLabel}
        </span>
        <span className="rounded-chip border border-border px-[7px] py-px text-caption text-muted-foreground">
          {row.loader ?? unknownLabel}
        </span>
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => {
            void openPath(row.path);
          }}
          disabled={row.pathMissing}
          className="inline-flex h-8 items-center gap-1.5 rounded-control border border-border bg-card px-3 font-medium text-foreground disabled:cursor-not-allowed disabled:opacity-50"
        >
          {t('page.instances.action.openFolder')}
        </button>
      </div>
    </article>
  );
}

export function InstancesPage() {
  const { t, i18n } = useTranslation();
  const [rows, setRows] = useState<InstanceSummaryDto[] | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [query, setQuery] = useState('');
  const [onlyAttention, setOnlyAttention] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);

  const load = useCallback(() => {
    setError(null);
    listInstances()
      .then(setRows)
      .catch((e) => setError(toIpcError(e)));
  }, []);

  useEffect(load, [load]);

  const attentionCount = useMemo(
    () => (rows ?? []).filter((r) => r.pathMissing || r.scannedAt === null).length,
    [rows],
  );

  const visible = useMemo(() => {
    const all = rows ?? [];
    const needle = query.trim().toLowerCase();
    return all.filter((row) => {
      if (onlyAttention && !row.pathMissing && row.scannedAt !== null) return false;
      if (!needle) return true;
      return (
        row.name.toLowerCase().includes(needle) ||
        (row.minecraftVersion ?? '').toLowerCase().includes(needle) ||
        (row.loader ?? '').toLowerCase().includes(needle)
      );
    });
  }, [rows, query, onlyAttention]);

  const header = (
    <div className="flex items-center gap-2.5 px-5 pt-4">
      <h1 className="text-page-title font-semibold">{t('nav.instances')}</h1>
      {rows && (
        <span className="ms-1 text-small text-muted-foreground">
          {t('page.instances.count', { count: rows.length })}
        </span>
      )}
      <Button
        className="ms-auto"
        onClick={() => setDialogOpen(true)}
        aria-label={t('instance.dialog.title')}
      >
        <Plus aria-hidden />
        {t('instance.dialog.actionCreateShort')}
      </Button>
    </div>
  );

  const page = error ? (
    <div className="flex flex-col">
      {header}
      <div className="px-5 pt-4">
        <ErrorState error={error} onRetry={load} />
      </div>
    </div>
  ) : (
    <div className="flex flex-col">
      {header}

      {rows === null ? (
        <LoadingState className="px-5 pt-6" />
      ) : rows.length === 0 ? (
        <EmptyState
          title={t('page.instances.emptyTitle')}
          body={t('page.instances.emptyBody')}
        />
      ) : (
        <>
          <div className="flex items-center gap-2.5 px-5 py-2.5">
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('page.instances.search')}
              aria-label={t('page.instances.search')}
              className={cn(
                'h-8 w-[170px] rounded-control border border-input bg-surface-sunken px-2.5',
                'text-body text-foreground outline-none placeholder:text-muted-foreground',
              )}
            />
            <FilterToggle
              label={t('page.instances.filterAll', { count: rows.length })}
              selected={!onlyAttention}
              onSelect={() => setOnlyAttention(false)}
            />
            <FilterToggle
              label={t('page.instances.filterAttention', { count: attentionCount })}
              selected={onlyAttention}
              onSelect={() => setOnlyAttention(true)}
            />
          </div>

          {visible.length === 0 ? (
            <EmptyState title={t('page.instances.noMatch')} body={t('page.instances.search')} />
          ) : (
            <div className="grid grid-cols-1 gap-3 px-5 pb-5 sm:grid-cols-2 lg:grid-cols-3">
              {visible.map((row) => (
                <Card
                  key={row.id}
                  row={row}
                  locale={i18n.language}
                  unknownLabel={t('page.instances.unknown')}
                />
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );

  return (
    <>
      {page}
      <NewInstanceDialog open={dialogOpen} onOpenChange={setDialogOpen} onDone={load} />
    </>
  );
}

function FilterToggle({
  label,
  selected,
  onSelect,
}: {
  label: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={cn(
        'rounded-control px-2.5 py-[3px] text-small',
        selected ? 'bg-accent text-foreground' : 'text-muted-foreground',
      )}
    >
      {label}
    </button>
  );
}