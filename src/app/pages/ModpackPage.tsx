import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { open as openFilePicker } from '@tauri-apps/plugin-dialog';
import { cn } from 'cn';

import type { InstanceSummaryDto } from '../../bindings/InstanceSummaryDto';
import type { IpcError } from '../../bindings/IpcError';
import type { ModpackApplyResultDto } from '../../bindings/ModpackApplyResultDto';
import type { ModpackInstallRequest } from '../../bindings/ModpackInstallRequest';
import type { ModpackPlanDto } from '../../bindings/ModpackPlanDto';
import type { PackEventDto } from '../../bindings/PackEventDto';
import type { PackWriteDto } from '../../bindings/PackWriteDto';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, LoadingState } from '../../components/state';
import {
  executeModpackInstall,
  listInstances,
  planModpackInstall,
  toIpcError,
} from '../../lib/ipc';
import { formatBytes, formatNumber } from '../../lib/format';

/**
 * One file the apply has reported on, reduced from the channel.
 *
 * `safePath` is the key because that is what core names: one destination gets
 * one row, and its last event wins.
 */
interface ProgressRow {
  safePath: string;
  phase: string;
  done: number;
  total: number | null;
}

type Busy = 'plan' | 'apply' | null;

/**
 * A `.mrpack` install: plan first, apply second (AGENTS.md §5.4).
 *
 * The plan is the screen's whole point. It shows every destination the pack
 * asks for, whether the apply creates it or replaces what is already there,
 * and the counts — before a single byte is written. `execute_modpack_install`
 * re-reads the archive and re-plans in Rust, so what is shown here is a
 * preview of a decision core makes again, never an instruction it accepts.
 *
 * Progress arrives on the Command's channel, keyed by `pack.phase.*` — core
 * says what is true, this screen says how to say it.
 */
export function ModpackPage() {
  const { t } = useTranslation();
  const [instances, setInstances] = useState<InstanceSummaryDto[] | null>(null);
  const [target, setTarget] = useState('');
  const [packPath, setPackPath] = useState('');
  const [plan, setPlan] = useState<ModpackPlanDto | null>(null);
  const [rows, setRows] = useState<ProgressRow[]>([]);
  const [packPhase, setPackPhase] = useState<string | null>(null);
  const [result, setResult] = useState<ModpackApplyResultDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState<Busy>(null);
  // Every plan and apply takes the next id. A response that arrives after a
  // newer request started describes paths the user has already left, so it is
  // dropped instead of painted over the newer state.
  const requestSeq = useRef(0);

  const loadInstances = useCallback(() => {
    setError(null);
    // Not `void`-and-forget, for the same reason BrowsePage does not: a failed
    // list would otherwise read as "you own no instances" rather than as an
    // unasked question.
    void listInstances()
      .then((list) => {
        setInstances(list);
        setTarget((current) => current || (list[0]?.id ?? ''));
      })
      .catch((e) => {
        setError(toIpcError(e));
      });
  }, []);

  useEffect(loadInstances, [loadInstances]);

  // The webview names an instance by id and reads its path back from the
  // registry; it never assembles a folder, and a stale id simply yields ''.
  const targetPath = useMemo(
    () => instances?.find((i) => i.id === target)?.path ?? '',
    [instances, target],
  );

  /** Any edit invalidates the plan: it describes a pair of paths that just
   *  changed, so keeping it would be showing a truth about another install.
   *
   *  While an install runs the fields describe the run on screen, so an edit is
   *  a no-op: clearing here would wipe the progress of a job that is still
   *  going and leave its eventual error with nothing to sit next to. */
  const change = (setter: (value: string) => void) => (value: string) => {
    if (busy !== null) {
      return;
    }
    requestSeq.current += 1;
    setter(value);
    setPlan(null);
    setRows([]);
    setPackPhase(null);
    setResult(null);
    setError(null);
  };

  /**
   * The native file picker, filtered to the pack's extension. A cancelled
   * dialog resolves to `null`, which is not an error and must leave the field
   * exactly as it was.
   */
  const pickPack = async () => {
    try {
      const picked = await openFilePicker({
        directory: false,
        multiple: false,
        title: t('pack.fieldPack'),
        filters: [{ name: t('pack.fileFilter'), extensions: ['mrpack'] }],
      });
      if (typeof picked === 'string' && picked.length > 0) {
        change(setPackPath)(picked);
      }
    } catch (e) {
      setError(toIpcError(e));
    }
  };

  /** The pair the plan is built from. No digest yet: the plan is what hands
   *  one back, and the apply adds it (see `apply`). */
  const request = useCallback(
    (): ModpackInstallRequest => ({
      packPath: packPath.trim(),
      instanceRoot: targetPath,
    }),
    [packPath, targetPath],
  );

  const ready = packPath.trim().length > 0 && targetPath.length > 0;

  const preview = useCallback(() => {
    const id = (requestSeq.current += 1);
    setBusy('plan');
    setError(null);
    setRows([]);
    setPackPhase(null);
    setResult(null);
    planModpackInstall(request())
      .then((next) => {
        if (id === requestSeq.current) {
          setPlan(next);
        }
      })
      .catch((e) => {
        if (id === requestSeq.current) {
          setError(toIpcError(e));
        }
      })
      .finally(() => {
        // A newer request owns `busy` now; clearing it here would re-enable
        // the fields mid-flight.
        if (id === requestSeq.current) {
          setBusy(null);
        }
      });
  }, [request]);

  const onEvent = useCallback((e: PackEventDto) => {
    // No path means the event is about the run, not a file: `done` and
    // `failed` end the narration, and core guarantees the last one is one of
    // those two.
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

  const apply = useCallback(() => {
    if (!plan) {
      return;
    }
    const id = (requestSeq.current += 1);
    setBusy('apply');
    setError(null);
    setRows([]);
    setPackPhase(null);
    setResult(null);
    // The channel is guarded by the same id as the promise: an event from a
    // run the screen has moved past must not land in its rows.
    //
    // The digest is the plan's own, not a fresh hash: it is what binds this
    // confirm to the archive the user just reviewed. If the file changed since,
    // the command refuses and that refusal lands in `error` below.
    executeModpackInstall({ ...request(), packSha512: plan.packSha512 }, (e) => {
      if (id === requestSeq.current) {
        onEvent(e);
      }
    })
      .then((next) => {
        if (id === requestSeq.current) {
          setResult(next);
        }
      })
      .catch((e) => {
        if (id === requestSeq.current) {
          setError(toIpcError(e));
        }
      })
      .finally(() => {
        if (id === requestSeq.current) {
          setBusy(null);
        }
      });
  }, [onEvent, plan, request]);

  const discard = () => {
    requestSeq.current += 1;
    setPlan(null);
    setRows([]);
    setPackPhase(null);
    setResult(null);
  };

  const header = (
    <div className="flex items-center gap-2.5 px-5 pt-4">
      <h1 className="text-page-title font-semibold">{t('nav.modpack')}</h1>
      {plan && (
        <span className="ms-1 text-small text-muted-foreground">
          {t('pack.count.files', { count: plan.writes.length })}
        </span>
      )}
    </div>
  );

  if (instances === null) {
    return (
      <div className="flex flex-col">
        {header}
        {error ? (
          <div className="px-5 pt-4">
            <ErrorState error={error} onRetry={loadInstances} />
          </div>
        ) : (
          <LoadingState className="px-5 pt-6" />
        )}
      </div>
    );
  }

  if (instances.length === 0) {
    return (
      <div className="flex flex-col">
        {header}
        <EmptyState
          title={t('pack.noInstancesTitle')}
          body={t('pack.noInstancesBody')}
        />
      </div>
    );
  }

  const isBusy = busy !== null;

  return (
    <div className="flex flex-col">
      {header}

      <div className="flex flex-col gap-2.5 px-5 pt-4">
        <div className="flex flex-col gap-1">
          {/* `htmlFor` rather than wrapping: a `<label>` around the row would
              also wrap the Browse button, and a click on the label would then
              reach the input instead of the picker. */}
          <label htmlFor="pack-path" className="text-small text-muted-foreground">
            {t('pack.fieldPack')}
          </label>
          <div className="flex gap-2">
            <input
              id="pack-path"
              value={packPath}
              onChange={(e) => change(setPackPath)(e.target.value)}
              disabled={isBusy}
              dir="ltr"
              placeholder={t('pack.fieldPackPlaceholder')}
              className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 font-mono text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-60"
            />
            <Button
              type="button"
              variant="outline"
              onClick={pickPack}
              disabled={isBusy}
              aria-label={t('pack.actionPick')}
            >
              {t('pack.actionPick')}
            </Button>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* A name, not a path: the path in a dropdown is unreadable and would
              need truncation, and the instance list already shows where each
              one lives. Only registered instances are offered — the install
              has to land somewhere core already knows. */}
          <label className="flex items-center gap-2 text-small text-muted-foreground">
            {t('pack.fieldInstance')}
            <select
              value={target}
              onChange={(e) => change(setTarget)(e.target.value)}
              disabled={isBusy}
              aria-label={t('pack.fieldInstance')}
              className="h-8 rounded-control border border-input bg-surface-sunken px-2 text-body text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-60"
            >
              {instances.map((i) => (
                <option key={i.id} value={i.id}>
                  {i.name}
                </option>
              ))}
            </select>
          </label>
          <Button className="ms-auto" type="button" onClick={preview} disabled={!ready || isBusy}>
            {t('pack.actionPlan')}
          </Button>
        </div>
      </div>

      {error && (
        <div className="px-5 pt-4">
          <ErrorState error={error} />
        </div>
      )}

      {plan && (
        <section className="mx-5 mt-4 rounded-control border border-border bg-card px-4 py-3">
          <div className="flex flex-wrap items-baseline gap-2">
            <h2 className="text-body font-semibold">{t('pack.planTitle')}</h2>
            <span className="text-small text-muted-foreground">
              {t('pack.version', { version: plan.versionId })}
            </span>
          </div>
          <p className="mt-0.5 truncate text-small text-foreground" title={plan.name}>
            {plan.name}
          </p>
          {plan.summary && (
            <p dir="auto" className="mt-1 text-small text-muted-foreground">
              {plan.summary}
            </p>
          )}

          {/* Counts come from the DTO, not from walking the list: the screen
              states the totals core computed, so a long plan cannot make them
              disagree with themselves. */}
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <Chip>{t('pack.count.files', { count: plan.writes.length })}</Chip>
            <Chip>{t('pack.count.downloads', { count: plan.downloadCount })}</Chip>
            <Chip>{t('pack.count.overrides', { count: plan.overrideCount })}</Chip>
            {plan.replaceCount > 0 && (
              <Chip tone="issue">
                {t('pack.count.replaces', { count: plan.replaceCount })}
              </Chip>
            )}
          </div>

          <ul className="mt-2.5 flex max-h-[46vh] flex-col gap-1 overflow-auto">
            {plan.writes.map((write) => (
              <WriteRow key={write.safePath} write={write} />
            ))}
          </ul>

          <p className="mt-2.5 text-caption text-muted-foreground">{t('pack.planNote')}</p>

          <div className="mt-3 flex gap-2">
            <Button type="button" onClick={apply} disabled={isBusy}>
              {t('pack.actionApply')}
            </Button>
            <Button type="button" variant="outline" onClick={discard} disabled={isBusy}>
              {t('state.action.cancel')}
            </Button>
          </div>
        </section>
      )}

      {(rows.length > 0 || packPhase !== null || result !== null) && (
        <section className="flex flex-col gap-1.5 px-5 pt-4">
          <div className="flex items-center gap-2.5">
            <h2 className="text-body font-semibold">{t('pack.progressTitle')}</h2>
            {isBusy && <LoadingState />}
          </div>
          {packPhase && (
            <p role="status" aria-live="polite" className="text-small text-muted-foreground">
              {t(packPhase)}
            </p>
          )}
          {result && (
            <p className="text-small text-muted-foreground">
              {t('pack.result', {
                written: formatNumber(result.written),
                skipped: formatNumber(result.skipped),
                size: formatBytes(result.bytes),
              })}
            </p>
          )}
          {rows.map((row) => (
            <ProgressRowView key={row.safePath} row={row} />
          ))}
        </section>
      )}

      {!plan && rows.length === 0 && packPhase === null && result === null && (
        <EmptyState title={t('pack.emptyTitle')} body={t('pack.emptyBody')} />
      )}
    </div>
  );
}

/** Phases that say a file is at its destination. */
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
 * One planned destination. The badge is the question the user is answering:
 * `overwrites` means the apply replaces a file that is already there.
 */
function WriteRow({ write }: { write: PackWriteDto }) {
  const { t } = useTranslation();
  return (
    <li className="flex items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5">
      <span
        className={cn(
          'shrink-0 rounded-chip border border-border px-[7px] py-px text-caption',
          write.overwrites ? 'text-status-issue' : 'text-muted-foreground',
        )}
      >
        {write.overwrites ? t('pack.badge.replace') : t('pack.badge.create')}
      </span>
      <span
        dir="ltr"
        title={write.safePath}
        className="min-w-0 flex-1 truncate font-mono text-caption text-foreground"
      >
        {write.safePath}
      </span>
      <span className="shrink-0 text-caption text-muted-foreground">
        {t(`pack.kind.${write.kind}`)}
      </span>
      {write.size !== null && (
        <span dir="ltr" className="shrink-0 font-mono text-caption text-muted-foreground">
          {formatBytes(write.size)}
        </span>
      )}
    </li>
  );
}

/**
 * One file, as the channel last described it.
 *
 * The bar fills only from a `total` core actually reported: a bar that guesses
 * is a lie, and a download with no declared size still gets its phase text.
 */
function ProgressRowView({ row }: { row: Pick<ProgressRow, 'safePath' | 'phase' | 'done' | 'total'> }) {
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
