import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TriangleAlert } from 'lucide-react';

import type { IpcError } from '../../bindings/IpcError';
import type { PackEventDto } from '../../bindings/PackEventDto';
import type { PackRecoveryDto } from '../../bindings/PackRecoveryDto';
import type { PackRecoveryResultDto } from '../../bindings/PackRecoveryResultDto';
import { Button } from '@/components/ui/button';
import { ErrorState } from '../../components/state';
import {
  dismissPackApply,
  pendingPackRecoveries,
  recoverPackApply,
  toIpcError,
} from '../../lib/ipc';
import { formatBytes } from '../../lib/format';

/**
 * One file the recovery run has reported on, reduced from the channel — the
 * same shape `ModpackPage` reduces the apply's channel to, so a resumed file
 * reads like an applied one.
 */
interface ProgressRow {
  safePath: string;
  phase: string;
  done: number;
  total: number | null;
}

/**
 * F-076. Shown only when a pack apply died and still needs a decision.
 *
 * Modeled on `RecoveryBanner` (F-146). Two rules carry over unchanged:
 *
 * - A failed list load renders NOTHING. This banner is advisory — if the
 *   records cannot be read, the instances are not broken, and an error panel
 *   here would say they were.
 * - The button comes from the `action` key core chose, decided in an explicit,
 *   total switch: a branch a future core adds must not fall through to the
 *   one that writes into an instance, so an unknown key renders no button at
 *   all.
 *
 * The UI sends only the operation id; the root, the archive and the staging
 * tree come from the record, never from the webview.
 */
export function PackRecoveryBanner() {
  const { t } = useTranslation();
  const [items, setItems] = useState<PackRecoveryDto[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  // The run whose progress is on screen, so rows never spill into another
  // frame: only one resume runs at a time (busy disables every button), and
  // only that frame shows its channel.
  const [runId, setRunId] = useState<string | null>(null);
  const [rows, setRows] = useState<ProgressRow[]>([]);
  const [packPhase, setPackPhase] = useState<string | null>(null);
  const [result, setResult] = useState<PackRecoveryResultDto | null>(null);

  const load = useCallback(() => {
    // `void`: a useCallback whose body returns a promise hands that promise to
    // useEffect as a cleanup function, which React rejects.
    // And a failed load is not shown at all — the reason `RecoveryBanner`
    // gives applies here twice, because this banner is advisory.
    void pendingPackRecoveries()
      .then(setItems)
      .catch(() => setItems([]));
  }, []);

  useEffect(load, [load]);

  // A re-downloaded archive turns «dismiss» back into «resume», and core reads
  // the disk to decide — so the moment the window is back in front is the
  // moment to ask again. Focus is the only event that can change this answer
  // without a user action here: no polling, and the listener goes away with
  // the banner.
  useEffect(() => {
    window.addEventListener('focus', load);
    return () => window.removeEventListener('focus', load);
  }, [load]);

  const onEvent = useCallback((e: PackEventDto) => {
    // No path means the event is about the run, not a file: `done` and
    // `failed` end the narration, and core guarantees the last one is one of
    // those two (or a refusal standing in for one).
    if (e.safePath === null) {
      setPackPhase(e.phase);
      return;
    }
    const path = e.safePath;
    setRows((prev) => {
      const index = prev.findIndex((r) => r.safePath === path);
      // A settled file stays settled: a late progress event must not reopen a
      // row that already landed.
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

  const act = (item: PackRecoveryDto) => {
    // Explicit, and total. `recovery.pack.action.resume` is the only key that
    // writes into the instance, and it is chosen by its own case arm: a
    // default `return` means an unknown action key renders a dead button
    // rather than a live destructive one.
    let action: Promise<PackRecoveryResultDto | void>;
    switch (item.action) {
      case 'recovery.pack.action.resume':
        action = recoverPackApply(item.operationId, onEvent);
        break;
      case 'recovery.pack.action.dismiss':
        // `dismiss` answers with no result: the record is closed by the write
        // itself, so there is no `next` state for the branch below to read.
        // The row leaves here, by id, without a reload — and the banner
        // renders nothing if that was the last case.
        action = dismissPackApply(item.operationId).then(() => {
          setItems((prev) => prev?.filter((i) => i.operationId !== item.operationId) ?? null);
        });
        break;
      default:
        return;
    }
    setBusy(item.operationId);
    setRunId(item.operationId);
    setError(null);
    // Every previous run's events were narrating a different decision.
    setRows([]);
    setPackPhase(null);
    setResult(null);
    action
      .then((next) => {
        if (next && next.state === 'completed') {
          // The record is closed, so the decision is made and made at last:
          // the row leaves the banner, exactly as `RecoveryBanner` drops one.
          setItems((prev) => prev?.filter((i) => i.operationId !== item.operationId) ?? null);
          setRunId(null);
        } else if (next) {
          // `failed` leaves the record RUNNING (F-076): the instance is half
          // applied exactly like a crash, so the frame stays and its counts
          // are what the run answered with.
          setResult(next);
        }
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(null));
  };

  if (error) return <ErrorState error={error} />;
  // Nothing crashed. The banner must not exist at all, not sit there empty:
  // space reserved for a problem that is not happening is noise on every launch.
  if (!items || items.length === 0) return null;

  return (
    <section aria-label={t('recovery.pack.title')} className="mb-5">
      {items.map((item) => (
        <article
          key={item.operationId}
          className="mb-2.5 rounded-control border border-border bg-card px-4 py-3 last:mb-0"
        >
          <div className="flex flex-wrap items-center gap-3">
            <TriangleAlert aria-hidden className="size-4 shrink-0 text-warning" />
            <div className="min-w-0 flex-1">
              <p className="text-body font-medium">{t(item.reason)}</p>
              <p className="truncate text-caption text-muted-foreground">
                {item.packName} · {t('pack.version', { version: item.packVersionId })}
              </p>
              <p dir="ltr" className="truncate font-mono text-caption text-muted-foreground">
                {item.instanceRoot}
              </p>
              {/* The disk's evidence, not the record's counters: what the
                  assessment measured where the files live. */}
              <p className="text-caption text-muted-foreground">
                {t('recovery.pack.progress', {
                  count: item.doneCount,
                  planned: item.plannedCount,
                })}
              </p>
            </div>
            <Button
              type="button"
              variant="outline"
              disabled={busy !== null}
              onClick={() => act(item)}
            >
              {t(item.action)}
            </Button>
          </div>

          {/* A resumed apply narrates through the same channel and the same
              row view the apply on the pack page uses, so a recovery is
              rendered the way the run it continues was. */}
          {runId === item.operationId && (rows.length > 0 || packPhase !== null || result !== null) && (
            <div className="mt-2.5 flex flex-col gap-1.5 border-t border-border pt-2.5">
              {packPhase && (
                <p role="status" aria-live="polite" className="text-small text-muted-foreground">
                  {t(packPhase)}
                </p>
              )}
              {result && (
                <p className="text-small text-muted-foreground">
                  {t('recovery.pack.result', {
                    written: result.written,
                    skipped: result.skipped,
                    failed: result.failed,
                  })}
                </p>
              )}
              {rows.map((row) => (
                <ProgressRowView key={row.safePath} row={row} />
              ))}
            </div>
          )}
        </article>
      ))}
    </section>
  );
}

/** Phases that say a file is at its destination — the apply's set. */
const SETTLED = new Set(['pack.phase.written', 'pack.phase.skipped']);

function settled(phase: string): boolean {
  return SETTLED.has(phase);
}

/**
 * One file, as the channel last described it — `ModpackPage`'s row view. The
 * bar fills only from a `total` core actually reported: a bar that guesses is
 * a lie, and a download with no declared size still gets its phase text.
 */
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
          className={`shrink-0 text-caption ${done ? 'text-muted-foreground' : 'text-primary'}`}
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
