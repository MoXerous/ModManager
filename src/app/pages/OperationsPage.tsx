import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RefreshCw } from 'lucide-react';

import type { IpcError } from '../../bindings/IpcError';
import type { OperationAssessmentDto } from '../../bindings/OperationAssessmentDto';
import type { OperationIssueDto } from '../../bindings/OperationIssueDto';
import type { OperationSummaryDto } from '../../bindings/OperationSummaryDto';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, LoadingState } from '../../components/state';
import { assessOperation, listOperations, toIpcError } from '../../lib/ipc';
import { formatNumber, formatRelativeTime } from '../../lib/format';
import type { PageId } from '../Shell';

/**
 * F-145. The operation centre: one read-only table of every record the journal
 * holds — download batches, pack applies and instance creates side by side.
 *
 * Three rules shape the screen:
 *
 * - **Read-only.** Nothing here mutates. Where a recovery already exists it is
 *   LINKED to (the Instances or Modpack page that owns the banner), never
 *   duplicated: two places that can resume one record is two places that can
 *   disagree about it.
 * - **Grouped by state.** A state is what a user is looking for ("what
 *   crashed?"), so it is the section, not a column to sort by.
 * - **Evidence, not tallies.** The assessor reports what the disk holds, and
 *   the record's own counters are shown only where the record is the subject.
 */

/**
 * The groups, unfinished first. Explicit for the states this build knows; a
 * state a future core adds lands in the explicit "other" group below, never
 * silently outside the table the header counts.
 */
const GROUPS = ['planned', 'running', 'failed', 'cancelled', 'completed'] as const;

/** The bucket for a state this build has no group for — visible, not dropped. */
const OTHER_GROUP = 'other';

/**
 * Where a kind's recovery already lives, or `null` when none exists.
 *
 * The switch is explicit so a new kind cannot fall through to somebody else's
 * mutation screen: `download_batch` has no recovery surface, and the view
 * invents none.
 */
function recoveryPage(kind: string): PageId | null {
  switch (kind) {
    case 'instance_create':
      return 'instances';
    case 'mrpack_install':
      return 'modpack';
    default:
      return null;
  }
}

/** Finished items out of the plan — the old app's "progress" column. */
function finished(row: OperationSummaryDto): number {
  return row.completedCount + row.failedCount + row.cancelledCount + row.skippedCount;
}

export interface OperationsPageProps {
  onNavigate: (page: PageId) => void;
}

export function OperationsPage({ onNavigate }: OperationsPageProps) {
  const { t, i18n } = useTranslation();
  // `null` means "not read yet", which is different from an empty journal.
  const [rows, setRows] = useState<OperationSummaryDto[] | null>(null);
  // Records the scan could not read. Reported, never repaired: they are the
  // old centre's "unreadable record" notice.
  const [issues, setIssues] = useState<OperationIssueDto[]>([]);
  const [error, setError] = useState<IpcError | null>(null);
  const [loading, setLoading] = useState(false);
  // One assessment at a time: the panel is evidence about one record, and
  // opening a second must not leave the first one's evidence on screen under
  // another row.
  const [assessed, setAssessed] = useState<OperationAssessmentDto | null>(null);
  const [assessing, setAssessing] = useState<string | null>(null);
  // The row the failure belongs to, so a stale row's answer is not painted
  // under every other record in the table.
  const [assessError, setAssessError] = useState<{ id: string; error: IpcError } | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    setIssues([]);
    // Evidence belongs to the row it was read from; a reload is a new table.
    setAssessed(null);
    setAssessError(null);
    listOperations()
      .then((listing) => {
        setRows(listing.rows);
        setIssues(listing.issues);
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(load, [load]);

  const assess = useCallback(
    (row: OperationSummaryDto) => {
      // A second click on the open row closes the panel: the evidence is a
      // read, not a mode the row is stuck in.
      if (assessed && assessed.operationId === row.operationId) {
        setAssessed(null);
        return;
      }
      setAssessError(null);
      setAssessing(row.operationId);
      // The previous row's evidence goes now, not when the answer arrives: a
      // panel under another record while its own read is in flight reads as
      // that record's evidence.
      setAssessed(null);
      assessOperation(row.operationId)
        .then(setAssessed)
        .catch((e) => {
          setAssessed(null);
          setAssessError({ id: row.operationId, error: toIpcError(e) });
        })
        .finally(() => setAssessing(null));
    },
    [assessed],
  );

  const grouped = useMemo(() => {
    const byState = new Map<string, OperationSummaryDto[]>();
    for (const row of rows ?? []) {
      const group = byState.get(row.state);
      if (group) {
        group.push(row);
      } else {
        byState.set(row.state, [row]);
      }
    }
    const groups: { state: string; rows: OperationSummaryDto[] }[] = GROUPS.map((state) => ({
      state,
      rows: byState.get(state) ?? [],
    }));
    // A state this build does not know is shown, not dropped: the header
    // counts every row, and a count with missing rows is a lie.
    const known = new Set<string>(GROUPS);
    const other = (rows ?? []).filter((row) => !known.has(row.state));
    if (other.length > 0) {
      groups.push({ state: OTHER_GROUP, rows: other });
    }
    return groups.filter((group) => group.rows.length > 0);
  }, [rows]);

  return (
    <div className="flex flex-col">
      <div className="flex flex-wrap items-center gap-2.5 px-5 pt-4">
        <h1 className="text-page-title font-semibold">{t('nav.operations')}</h1>
        {rows && rows.length > 0 && (
          <span className="ms-1 text-small text-muted-foreground">
            {t('operations.summary', { count: rows.length })}
          </span>
        )}
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="ms-auto"
          onClick={load}
          disabled={loading}
        >
          <RefreshCw aria-hidden className="size-[18px]" strokeWidth={1.75} />
          {t('operations.action.refresh')}
        </Button>
      </div>

      <p className="max-w-3xl px-5 pt-2 text-small text-muted-foreground">
        {t('operations.readOnly')}
      </p>

      {issues.length > 0 && (
        <section
          aria-label={t('operations.issues.title')}
          className="mx-5 mt-4 rounded-md border border-border bg-surface-sunken px-3 py-2"
        >
          <h2 className="text-small font-semibold text-foreground">
            {t('operations.issues.title')}
          </h2>
          <p className="mb-1 text-caption text-muted-foreground">{t('operations.issues.body')}</p>
          <ul className="flex flex-col gap-1">
            {issues.map((issue) => (
              <li key={issue.filename} className="text-caption text-muted-foreground">
                <span dir="ltr" className="font-mono">
                  {issue.filename}
                </span>
                <span className="mx-1">—</span>
                <span>{issue.message}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {error ? (
        <div className="px-5 pt-4">
          <ErrorState error={error} onRetry={load} />
        </div>
      ) : rows === null ? (
        <div className="px-5 pt-6">
          <LoadingState />
        </div>
      ) : rows.length === 0 ? (
        <EmptyState title={t('operations.emptyTitle')} body={t('operations.emptyBody')} />
      ) : (
        <div className="flex flex-col gap-5 px-5 pb-5 pt-4">
          {grouped.map((group) => (
            <section key={group.state} aria-label={t(`operations.state.${group.state}`)}>
              <h2 className="mb-1.5 flex items-baseline gap-2 text-small font-semibold text-foreground">
                {t(`operations.state.${group.state}`)}
                <span className="font-normal text-muted-foreground">
                  {t('operations.summary', { count: group.rows.length })}
                </span>
              </h2>

              <div className="overflow-x-auto">
                <table className="w-full table-fixed border-collapse text-small">
                  <colgroup>
                    <col className="w-[30%]" />
                    <col className="w-[12%]" />
                    <col className="w-[13%]" />
                    <col className="w-[13%]" />
                    <col className="w-[24%]" />
                    <col className="w-[8%]" />
                  </colgroup>
                  <thead>
                    <tr className="text-caption text-muted-foreground">
                      <th className="border-b border-border px-2 py-2 text-start font-medium">
                        {t('operations.col.kind')}
                      </th>
                      <th className="border-b border-border px-2 py-2 text-start font-medium">
                        {t('operations.col.progress')}
                      </th>
                      <th className="border-b border-border px-2 py-2 text-start font-medium">
                        {t('operations.col.profile')}
                      </th>
                      <th className="border-b border-border px-2 py-2 text-start font-medium">
                        {t('operations.col.updated')}
                      </th>
                      <th className="border-b border-border px-2 py-2 text-start font-medium">
                        {t('operations.col.reason')}
                      </th>
                      <th className="border-b border-border px-2 py-2 text-start font-medium">
                        <span className="sr-only">{t('operations.action.assess')}</span>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {group.rows.map((row) => (
                      <OperationRow
                        key={row.operationId}
                        row={row}
                        locale={i18n.language}
                        assessed={
                          assessed && assessed.operationId === row.operationId ? assessed : null
                        }
                        assessing={assessing === row.operationId}
                        assessError={
                          assessError && assessError.id === row.operationId
                            ? assessError.error
                            : null
                        }
                        onAssess={() => assess(row)}
                        onNavigate={onNavigate}
                        onRefresh={load}
                      />
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

interface OperationRowProps {
  row: OperationSummaryDto;
  locale: string;
  assessed: OperationAssessmentDto | null;
  assessing: boolean;
  assessError: IpcError | null;
  onAssess: () => void;
  onNavigate: (page: PageId) => void;
  onRefresh: () => void;
}

function OperationRow({
  row,
  locale,
  assessed,
  assessing,
  assessError,
  onAssess,
  onNavigate,
  onRefresh,
}: OperationRowProps) {
  const { t } = useTranslation();
  const page = recoveryPage(row.kind);

  return (
    <>
      <tr className="align-top">
        <td className="border-b border-border px-2 py-2">
          <p className="truncate font-medium text-foreground">{t(`operations.kind.${row.kind}`)}</p>
          {row.packName && (
            <p className="truncate text-caption text-muted-foreground">{row.packName}</p>
          )}
          <p
            dir="ltr"
            title={row.targetPath}
            className="truncate font-mono text-caption text-muted-foreground"
          >
            {row.targetPath}
          </p>
        </td>
        <td className="border-b border-border px-2 py-2 font-mono text-caption text-muted-foreground">
          <span dir="ltr">
            {t('operations.progress', {
              done: formatNumber(finished(row)),
              planned: formatNumber(row.plannedCount),
            })}
          </span>
        </td>
        <td className="border-b border-border px-2 py-2 text-caption text-muted-foreground">
          <span dir="ltr" className="block truncate" title={row.profileId}>
            {row.profileId || '—'}
          </span>
        </td>
        <td className="border-b border-border px-2 py-2 text-caption text-muted-foreground">
          {formatRelativeTime(row.updatedAt, locale)}
        </td>
        <td className="border-b border-border px-2 py-2 text-caption text-muted-foreground">
          <span className="block truncate" title={row.reason ? t(row.reason) : undefined}>
            {row.reason ? t(row.reason) : '—'}
          </span>
        </td>
        <td className="border-b border-border px-2 py-2 text-end">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onAssess}
            disabled={assessing}
            aria-expanded={assessed !== null}
          >
            {t('operations.action.assess')}
          </Button>
        </td>
      </tr>

      {assessed && (
        <tr>
          <td colSpan={6} className="border-b border-border bg-surface-sunken px-3 py-3">
            <AssessmentPanel
              assessment={assessed}
              page={page}
              onNavigate={onNavigate}
            />
          </td>
        </tr>
      )}

      {assessError && (
        <tr>
          <td colSpan={6} className="border-b border-border px-3 py-3">
            <ErrorState error={assessError} onRetry={onRefresh} />
          </td>
        </tr>
      )}
    </>
  );
}

function AssessmentPanel({
  assessment,
  page,
  onNavigate,
}: {
  assessment: OperationAssessmentDto;
  page: PageId | null;
  onNavigate: (page: PageId) => void;
}) {
  const { t } = useTranslation();
  const present = (value: boolean) =>
    value ? t('operations.assess.present') : t('operations.assess.absent');

  return (
    <div className="flex flex-col gap-2">
      <p className="text-small font-semibold text-foreground">{t('operations.assess.title')}</p>
      <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-caption sm:grid-cols-3">
        <div className="flex justify-between gap-2">
          <dt className="text-muted-foreground">{t('operations.col.target')}</dt>
          <dd dir="ltr" className="truncate font-mono text-foreground" title={assessment.targetPath}>
            {assessment.targetPath}
          </dd>
        </div>
        <div className="flex justify-between gap-2">
          <dt className="text-muted-foreground">{t('operations.col.progress')}</dt>
          <dd className="font-mono text-foreground">
            {t('operations.assess.items', { count: assessment.itemCount })}
          </dd>
        </div>
        <div className="flex justify-between gap-2">
          <dt className="text-muted-foreground">{t('operations.assess.final')}</dt>
          <dd className="font-mono text-foreground">{formatNumber(assessment.finalPresent)}</dd>
        </div>
        <div className="flex justify-between gap-2">
          <dt className="text-muted-foreground">{t('operations.assess.part')}</dt>
          <dd className="font-mono text-foreground">{formatNumber(assessment.partPresent)}</dd>
        </div>
        <div className="flex justify-between gap-2">
          <dt className="text-muted-foreground">{t('operations.assess.staging')}</dt>
          <dd className="text-foreground">{present(assessment.stagingPresent)}</dd>
        </div>
        <div className="flex justify-between gap-2">
          <dt className="text-muted-foreground">{t('operations.assess.backup')}</dt>
          <dd className="text-foreground">{present(assessment.backupPresent)}</dd>
        </div>
      </dl>

      {assessment.reason ? (
        <p className="text-small text-foreground">
          <span className="text-muted-foreground">{t('operations.assess.reason')}: </span>
          {t(assessment.reason)}
        </p>
      ) : (
        <p className="text-small text-muted-foreground">{t('operations.assess.nothingToDo')}</p>
      )}

      {/* The link is the recovery. The view never repeats the mutation the
          owning surface already offers — it names it and goes there. */}
      {assessment.action !== '' && page && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-small text-muted-foreground">
            {t('operations.assess.action')}: {t(assessment.action)}
          </span>
          <Button type="button" variant="outline" size="sm" onClick={() => onNavigate(page)}>
            {t('operations.action.openRecovery', { page: t(`nav.${page}`) })}
          </Button>
        </div>
      )}
    </div>
  );
}
