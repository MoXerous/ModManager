import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TriangleAlert } from 'lucide-react';

import type { BackupDecisionDto } from '../../bindings/BackupDecisionDto';
import type { IpcError } from '../../bindings/IpcError';
import type { UpdateDownloadResultDto } from '../../bindings/UpdateDownloadResultDto';
import type { UpdateInstallPlanDto } from '../../bindings/UpdateInstallPlanDto';
import type { UpdateInstallResultDto } from '../../bindings/UpdateInstallResultDto';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { ErrorState, LoadingState } from '../../components/state';import {
  executeUpdateDownloadOnly,
  executeUpdateInstall,
  previewUpdateInstall,
  toIpcError,
} from '../../lib/ipc';

interface Props {
  root: string;
  gameVersion: string;
  loader: string;
  provider: string;
  onClose: () => void;
  /** Something on disk changed; the caller should rescan. */
  onApplied: () => void;
}

/** What the user is looking at: the plan, the second question, or a result. */
type Phase = 'review' | 'noBackup' | 'running' | 'done';

/** Which run produced `outcome`. */
type Kind = 'install' | 'download';

type Outcome =
  | { kind: 'install'; result: UpdateInstallResultDto }
  | { kind: 'download'; result: UpdateDownloadResultDto };

/**
 * Plan, contract, decision, then write (F-121..F-124).
 *
 * The order on screen is the order on disk: the plan is read-only, the five
 * promises come from the core's own table, the backup question is a real
 * three-way choice with no default, and the result names every file that
 * moved and every file that was skipped and why.
 */
export function UpdateInstallDialog({
  root,
  gameVersion,
  loader,
  provider,
  onClose,
  onApplied,
}: Props) {
  const { t } = useTranslation();
  const [plan, setPlan] = useState<UpdateInstallPlanDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [phase, setPhase] = useState<Phase>('review');
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  /** Which run is in flight, so only that button says "working". */
  const [active, setActive] = useState<Kind | null>(null);

  useEffect(() => {
    let live = true;
    setPlan(null);
    setError(null);
    previewUpdateInstall({
      instanceRoot: root,
      provider,
      gameVersion,
      loader,
      selected: [],
    })
      .then((next) => {
        if (live) {
          setPlan(next);
        }
      })
      .catch((e) => {
        if (live) {
          setError(toIpcError(e));
        }
      });
    return () => {
      live = false;
    };
  }, [root, provider, gameVersion, loader, attempt]);

  const count = plan?.entries.length ?? 0;
  const busy = phase === 'running';

  const request = (backup: BackupDecisionDto, confirmNoBackup: boolean) => ({
    instanceRoot: root,
    provider,
    gameVersion,
    loader,
    selected: [] as string[],
    expectedToken: plan?.token ?? '',
    backup,
    confirmNoBackup,
  });

  /**
   * One run at a time. `download` never consults the backup answer (it writes
   * nothing installed), so it carries `cancel` — the value the core ignores
   * for that mode, not a fourth choice shown to the user.
   */
  const run = (kind: Kind, backup: BackupDecisionDto, confirmNoBackup: boolean) => {
    if (!plan || busy) {
      return;
    }
    setPhase('running');
    setActive(kind);
    setError(null);
    const done =
      kind === 'download'
        ? executeUpdateDownloadOnly(request(backup, confirmNoBackup)).then((result) => {
            setOutcome({ kind: 'download', result });
          })
        : executeUpdateInstall(request(backup, confirmNoBackup)).then((result) => {
            setOutcome({ kind: 'install', result });
          });
    done
      .then(() => {
        setPhase('done');
        onApplied();
      })
      .catch((e) => {
        setError(toIpcError(e));
        setPhase('review');
      })
      .finally(() => {
        setActive(null);
      });
  };

  const stale = error?.code === 'updateContextChanged';

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        // A run is a write on disk; closing would hide the only report of it.
        if (!next && !busy) {
          onClose();
        }
      }}
    >
      <DialogContent
        className="flex max-h-[calc(100vh-2rem)] max-w-[600px] flex-col gap-3 overflow-hidden"
        showCloseButton={!busy}
      >
        <DialogHeader className="shrink-0">
          <DialogTitle>{t('updates.install.title')}</DialogTitle>
          <DialogDescription>
            {t('updates.install.description', { count })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto pe-1">
          {!outcome && !plan && !error && <LoadingState className="py-4" />}

          {!outcome && error && !plan && (
            <ErrorState
              error={error}
              onRetry={() => {
                setAttempt((n) => n + 1);
              }}
            />
          )}

          {!outcome && plan && phase !== 'noBackup' && (
            <>
              <ul className="flex flex-col gap-1">
                {plan.entries.map((entry) => (
                  <li
                    key={entry.projectId}
                    className="flex items-center gap-2 rounded-control bg-surface-sunken px-2.5 py-1.5 text-caption"
                  >
                    <span className="min-w-0 flex-1 truncate" dir="auto" title={entry.filename}>
                      {entry.oldFilename}
                    </span>
                    <span aria-hidden className="shrink-0 text-muted-foreground">
                      →
                    </span>
                    <span
                      className="min-w-0 flex-1 truncate font-mono"
                      dir="auto"
                      title={entry.filename}
                    >
                      {entry.filename}
                    </span>
                    <span dir="ltr" className="shrink-0 font-mono text-muted-foreground">
                      {entry.versionNumber}
                    </span>
                  </li>
                ))}
              </ul>

              {/* F-122: the promises, from the core table the guards live in. */}
              <div className="rounded-control border border-border bg-surface-sunken px-3 py-2.5">
                <p className="text-small font-semibold">{t('updates.install.contractTitle')}</p>
                <ul className="mt-1 flex list-disc flex-col gap-0.5 ps-4">
                  {plan.contract.map((item) => (
                    <li key={item.id} className="text-caption text-muted-foreground">
                      {t(item.key, { defaultValue: item.text })}
                    </li>
                  ))}
                </ul>
              </div>

              {error && (
                <div role="alert" className="flex flex-col gap-1.5">
                  <p className="text-small text-status-issue">
                    {t('updates.install.failed', {
                      reason: t(`error.${error.code}`, { defaultValue: error.message }),
                    })}
                  </p>
                  {stale && (
                    <Button
                      type="button"
                      variant="outline"
                      className="self-start"
                      onClick={() => {
                        setOutcome(null);
                        setAttempt((n) => n + 1);
                      }}
                    >
                      {t('updates.install.planAgain')}
                    </Button>
                  )}
                </div>
              )}
            </>
          )}

          {!outcome && plan && phase === 'noBackup' && (
            <div className="flex items-start gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-2">
              <TriangleAlert className="mt-0.5 size-4 shrink-0 text-status-warning" aria-hidden />
              <div className="min-w-0">
                <p className="text-small font-semibold">{t('updates.install.noBackupTitle')}</p>
                <p className="mt-0.5 text-caption text-muted-foreground">
                  {t('updates.install.noBackupBody')}
                </p>
              </div>
            </div>
          )}

          {outcome && <ResultPanel outcome={outcome} />}
        </div>

        <DialogFooter className="shrink-0">
          {outcome ? (
            <Button type="button" onClick={onClose}>
              {t('state.action.close')}
            </Button>
          ) : phase === 'noBackup' ? (
            <>
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setPhase('review');
                }}
              >
                {t('updates.install.back')}
              </Button>
              <Button
                type="button"
                variant="destructive"
                onClick={() => {
                  run('install', 'no', true);
                }}
              >
                {t('updates.install.noBackupConfirm')}
              </Button>
            </>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
                {t('state.action.cancel')}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  run('download', 'cancel', false);
                }}
                disabled={!plan || busy}
              >
                {active === 'download'
                  ? t('updates.install.preparing')
                  : t('updates.install.downloadOnly')}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setPhase('noBackup');
                }}
                disabled={!plan || busy}
              >
                {t('updates.install.backupNo')}
              </Button>
              <Button
                type="button"
                onClick={() => {
                  run('install', 'yes', false);
                }}
                disabled={!plan || count === 0 || busy}
              >
                {active === 'install'
                  ? t('updates.install.running')
                  : t('updates.install.backupYes')}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ResultPanel({ outcome }: { outcome: Outcome }) {
  const { t } = useTranslation();
  if (outcome.kind === 'download') {
    const { result } = outcome;
    return (
      <>
        <p className="text-small" role="status">
          {t('updates.install.downloadOnlyDone', { count: result.prepared.length })}
        </p>
        <p className="truncate font-mono text-caption text-muted-foreground" dir="ltr">
          {result.directory}
        </p>
        <ul className="flex flex-col gap-1">
          {result.prepared.map((file) => (
            <li
              key={file.projectId}
              className="truncate rounded-control bg-surface-sunken px-2.5 py-1.5 font-mono text-caption"
              dir="ltr"
              title={file.path}
            >
              {file.path}
            </li>
          ))}
        </ul>
      </>
    );
  }

  const { result } = outcome;
  return (
    <>
      <p className="text-small" role="status">
        {t('updates.install.installedHeader', { count: result.installed.length })}
      </p>
      {result.installed.length > 0 && (
        <ul className="flex flex-col gap-1">
          {result.installed.map((row) => (
            <li
              key={row.projectId}
              className="flex items-center gap-2 rounded-control bg-surface-sunken px-2.5 py-1.5 text-caption"
            >
              <span className="min-w-0 flex-1 truncate" dir="auto">
                {row.filename}
              </span>
              <span dir="ltr" className="shrink-0 font-mono text-muted-foreground">
                {row.versionNumber}
              </span>
            </li>
          ))}
        </ul>
      )}

      {result.skipped.length > 0 && (
        <>
          <p className="text-small text-status-warning">
            {t('updates.install.skippedHeader', { count: result.skipped.length })}
          </p>
          <ul className="flex flex-col gap-1">
            {result.skipped.map((row) => (
              <li
                key={row.projectId}
                className="rounded-control bg-surface-sunken px-2.5 py-1.5 text-caption"
              >
                <p className="truncate" dir="auto">
                  {row.filename}
                </p>
                <p className="truncate text-status-warning">
                  {t(`updates.skip.${row.reasonCode}`, { defaultValue: row.message })}
                </p>
              </li>
            ))}
          </ul>
        </>
      )}

      <p className="truncate font-mono text-caption text-muted-foreground" dir="ltr">
        {t('updates.install.replacedDir', { path: result.replacedBackupDir })}
      </p>
      {result.profileBackupDir && (
        <p className="truncate font-mono text-caption text-muted-foreground" dir="ltr">
          {t('updates.install.profileBackup', { path: result.profileBackupDir })}
        </p>
      )}
    </>
  );
}
