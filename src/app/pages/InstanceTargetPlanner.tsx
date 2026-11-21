import { useState } from 'react';
import type { TFunction } from 'i18next';
import { Trans, useTranslation } from 'react-i18next';

import type { IpcError } from '../../bindings/IpcError';
import type { PrunePlanDto } from '../../bindings/PrunePlanDto';
import type { PruneReportDto } from '../../bindings/PruneReportDto';
import type { TargetPlanDto } from '../../bindings/TargetPlanDto';
import type { TargetRowDto } from '../../bindings/TargetRowDto';
import type { TargetStatusDto } from '../../bindings/TargetStatusDto';
import { Button } from '@/components/ui/button';
import { executeLockPrune, planTargetVersions, previewLockPrune, toIpcError } from '../../lib/ipc';
import type { ProviderId } from '../gameTarget';

function statusClass(status: TargetStatusDto): string {
  switch (status) {
    case 'download':
      return 'text-status-update';
    case 'syncOnly':
      return 'text-status-warning';
    case 'upToDate':
      return 'text-status-success';
    case 'incompatible':
      return 'text-status-issue';
    case 'otherProvider':
      return 'text-status-disabled';
    default:
      return 'text-status-warning';
  }
}

/** A refusal or a failure names its code; the reason detail rides in the title. */
function reasonTitle(row: TargetRowDto, t: TFunction): string | undefined {
  const text = t(`details.target.reason.${row.reasonCode}`);
  return row.reasonDetail ? `${text} — ${row.reasonDetail}` : text;
}

/**
 * F-125: the eight columns. `download` and `sync` are columns in their own
 * right because the old app's table asked two separate questions — is there a
 * file to fetch, and does the lock need to catch up — and a single status word
 * answered only one of them.
 */
function Row({ row }: { row: TargetRowDto }) {
  const { t } = useTranslation();
  return (
    <tr className="border-t border-border align-top">
      <td className="py-1.5 pe-2" dir="auto">
        <span className="block truncate" title={row.currentFilename}>
          {row.title}
        </span>
      </td>
      <td className="py-1.5 pe-2 font-mono text-caption text-muted-foreground" dir="ltr">
        {row.currentVersion}
      </td>
      <td className="py-1.5 pe-2 font-mono text-caption" dir="ltr">
        {row.targetVersion || '—'}
      </td>
      <td className="py-1.5 pe-2 font-mono text-caption" dir="ltr">
        {row.gameVersion}
      </td>
      <td className="py-1.5 pe-2 text-caption" dir="ltr">
        {row.loader}
      </td>
      <td className="py-1.5 pe-2 text-caption" dir="auto">
        {row.download ? t('details.target.yes') : t('details.target.no')}
      </td>
      <td className="py-1.5 pe-2 text-caption" dir="auto">
        {row.sync ? t('details.target.yes') : t('details.target.no')}
      </td>
      <td className={`py-1.5 text-caption ${statusClass(row.status)}`} title={reasonTitle(row, t)}>
        {t(`details.target.status.${row.status}`)}
      </td>
    </tr>
  );
}

function PlanTable({ plan }: { plan: TargetPlanDto }) {
  const { t } = useTranslation();
  return (
    <table className="mt-2 w-full table-fixed border-collapse text-start">
      <thead>
        <tr className="text-caption text-muted-foreground">
          {(
            [
              'title',
              'currentVersion',
              'targetVersion',
              'gameVersion',
              'loader',
              'download',
              'sync',
              'status',
            ] as const
          ).map((column) => (
            <th key={column} className="pb-1 pe-2 text-start font-normal">
              {t(`details.target.column.${column}`)}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {plan.rows.map((row, index) => (
          <Row key={`${row.projectId}-${row.currentFilename}-${index}`} row={row} />
        ))}
      </tbody>
    </table>
  );
}

function CommonTarget({ plan }: { plan: TargetPlanDto }) {
  const { t } = useTranslation();
  const common = plan.common;
  if (!common) {
    return null;
  }
  return (
    <div className="mt-2 text-caption text-muted-foreground">
      {common.suggested ? (
        <p>
          {t('details.target.common.suggested', {
            target: common.suggested,
            checked: common.checked,
          })}
        </p>
      ) : (
        <p className="text-status-warning">{t('details.target.common.none')}</p>
      )}
      {common.bestUpgrade && (
        <p className="mt-0.5">
          {t('details.target.common.bestUpgrade', {
            target: common.bestUpgrade.target,
            supported: common.bestUpgrade.supported,
            total: common.bestUpgrade.total,
            blockers: common.bestUpgrade.blockers.join(', '),
          })}
        </p>
      )}
      {common.commonVersions.length > 0 && (
        <p className="mt-0.5">
          <Trans
            i18nKey="details.target.common.versions"
            values={{ versions: common.commonVersions.join(', ') }}
            components={{ token: <bdi dir="ltr" className="font-mono" /> }}
          />
        </p>
      )}
      {common.skipped.length > 0 && (
        <ul className="mt-0.5">
          {common.skipped.map((skip) => (
            <li key={skip.projectId}>
              {t('details.target.common.skippedReason', {
                title: skip.title,
                reason: t(`details.target.common.skipReason.${skip.reason}`),
              })}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Cleanup({ plan }: { plan: TargetPlanDto }) {
  const { t } = useTranslation();
  const { cleanup } = plan;
  return (
    <div className="mt-2 text-caption text-muted-foreground">
      <p>
        {t('details.target.cleanup.counts', {
          present: cleanup.presentFiles,
          eligible: cleanup.eligibleFiles,
        })}
      </p>
      {cleanup.keptUnknown.length > 0 && (
        <p className="mt-0.5">
          {t('details.target.cleanup.keptUnknown', {
            n: cleanup.keptUnknown.length,
            files: cleanup.keptUnknown.map((file) => file.filename).join(', '),
          })}
        </p>
      )}
      {cleanup.recognizedIncompatible.length > 0 && (
        <p className="mt-0.5">
          {t('details.target.cleanup.recognized', {
            n: cleanup.recognizedIncompatible.length,
            files: cleanup.recognizedIncompatible.join(', '),
          })}
        </p>
      )}
    </div>
  );
}

/** F-130: the dry run is a button, and the confirm only exists after it. */
function PruneSection({ root, onChanged }: { root: string; onChanged: () => void }) {
  const { t } = useTranslation();
  const [plan, setPlan] = useState<PrunePlanDto | null>(null);
  const [report, setReport] = useState<PruneReportDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);

  const preview = () => {
    setBusy(true);
    setError(null);
    setReport(null);
    previewLockPrune({ instanceRoot: root })
      .then(setPlan)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  const confirm = () => {
    if (!plan) {
      return;
    }
    setBusy(true);
    setError(null);
    executeLockPrune({ instanceRoot: root, expectedDigest: plan.digest })
      .then((next) => {
        setPlan(null);
        setReport(next);
        onChanged();
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  return (
    <div className="mt-3 border-t border-border pt-2">
      <div className="flex items-center gap-2">
        <span className="text-caption font-semibold">{t('details.target.prune.title')}</span>
        <Button
          type="button"
          variant="outline"
          className="ms-auto"
          aria-busy={busy}
          disabled={busy}
          onClick={preview}
        >
          {busy ? t('details.target.prune.preparing') : t('details.target.prune.preview')}
        </Button>
      </div>
      <p className="mt-1 text-caption text-muted-foreground">
        {t('details.target.prune.description')}
      </p>
      {error && (
        <p role="alert" className="mt-1 text-caption text-status-issue">
          {t('details.target.prune.failed', { reason: t(`error.${error.code}`) })}
        </p>
      )}
      {plan && (
        <div className="mt-1.5 text-caption">
          {plan.candidates.length === 0 ? (
            <p className="text-muted-foreground">{t('details.target.prune.none')}</p>
          ) : (
            <>
              <p>
                {t('details.target.prune.candidates', {
                  n: plan.candidates.length,
                  files: plan.candidates.map((candidate) => candidate.filename).join(', '),
                })}
              </p>
              <Button
                type="button"
                variant="outline"
                className="mt-1.5 text-status-issue"
                disabled={busy}
                onClick={confirm}
              >
                {busy ? t('details.target.prune.confirming') : t('details.target.prune.confirm')}
              </Button>
            </>
          )}
        </div>
      )}
      {report && (
        <div className="mt-1.5 text-caption text-muted-foreground">
          <p>
            {t('details.target.prune.removed', {
              n: report.removed.length,
              files: report.removed.join(', ') || '—',
            })}
          </p>
          {report.keptPresent.length > 0 && (
            <p className="mt-0.5 text-status-warning">
              {t('details.target.prune.keptPresent', {
                files: report.keptPresent.join(', '),
              })}
            </p>
          )}
          <p className="mt-0.5">
            {t('details.target.prune.counts', {
              before: report.lockEntriesBefore,
              after: report.lockEntriesAfter,
            })}
          </p>
        </div>
      )}
    </div>
  );
}

interface Props {
  root: string;
  gameVersion: string;
  loader: string;
  provider: ProviderId;
  onChanged: () => void;
}

/**
 * F-125..F-127, F-131: the target-version planner, read-only.
 *
 * It is a separate card from "check for updates" because it answers a
 * different question: not "is there a newer file" but "what would this
 * instance look like at another Minecraft version". Nothing here downloads or
 * writes; the only write in the card is the lock cleanup, and it is a dry run
 * first (F-130).
 */
export function InstanceTargetPlanner({ root, gameVersion, loader, provider, onChanged }: Props) {
  const { t } = useTranslation();
  const [plan, setPlan] = useState<TargetPlanDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);

  // `any` and `unknown` are sentinels, not a Minecraft version or a loader: a
  // plan against them would have to guess, and core refuses rather than
  // guessing.
  const targetGame = gameVersion.trim().toLowerCase();
  const targetLoader = loader.trim().toLowerCase();
  const concrete =
    targetGame !== '' &&
    targetGame !== 'any' &&
    targetGame !== 'unknown' &&
    targetLoader !== '' &&
    targetLoader !== 'any' &&
    targetLoader !== 'unknown';

  const request = () => {
    if (!concrete) {
      return;
    }
    setBusy(true);
    setError(null);
    // A plan belongs to the target that produced it.
    setPlan(null);
    planTargetVersions({ instanceRoot: root, provider, gameVersion, loader })
      .then(setPlan)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  return (
    <div className="mx-5 mt-3 rounded-control border border-border bg-card px-3 py-2.5">
      <div className="flex items-center gap-2">
        <span className="text-body font-semibold">{t('details.target.title')}</span>
        <Button
          type="button"
          variant="outline"
          className="ms-auto"
          aria-busy={busy}
          disabled={busy || !concrete}
          onClick={request}
        >
          {busy ? t('details.target.planning') : t('details.target.plan')}
        </Button>
      </div>
      <p className="mt-1.5 text-caption text-muted-foreground">
        {t('details.target.description')}
      </p>
      {!concrete && (
        <p className="mt-1 text-caption text-status-warning">{t('details.target.needsTarget')}</p>
      )}
      {error && (
        <p role="alert" className="mt-1.5 text-small text-status-issue">
          {t('details.target.failed', { reason: t(`error.${error.code}`) })}
        </p>
      )}
      {plan && (
        <div aria-live="polite">
          {plan.warnings.map((warning, index) => (
            <p key={`${warning.code}-${index}`} className="mt-1.5 text-caption text-status-warning">
              {t(`details.target.warn.${warning.code}`)}
            </p>
          ))}
          <PlanTable plan={plan} />
          <CommonTarget plan={plan} />
          <Cleanup plan={plan} />
        </div>
      )}
      <PruneSection
        root={root}
        onChanged={() => {
          // The prune removed lock records: a plan (and its cleanup counts)
          // computed from the old lock is stale, not just the scan behind it.
          setPlan(null);
          onChanged();
        }}
      />
    </div>
  );
}
