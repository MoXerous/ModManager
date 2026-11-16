import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { BackupCandidateDto } from '../../bindings/BackupCandidateDto';
import type { IpcError } from '../../bindings/IpcError';
import type { RestoreEntryDto } from '../../bindings/RestoreEntryDto';
import type { RestorePlanDto } from '../../bindings/RestorePlanDto';
import type { RestoreResultEntryDto } from '../../bindings/RestoreResultEntryDto';
import type { RestoreRunDto } from '../../bindings/RestoreRunDto';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { cn } from 'cn';
import { ErrorState, LoadingState } from '../../components/state';
import { executeInstanceRestore, planInstanceRestore, toIpcError } from '../../lib/ipc';

/** Mirrors `CONFIRMATION_WORD` in mm-core; the bridge checks it again. */
const RESTORE_WORD = 'RESTORE';

interface Props {
  root: string;
  /** The backup root, already known from the listing: the candidate carries a stamp, not a path. */
  backupRoot: string;
  backup: BackupCandidateDto;
  onClose: () => void;
  /** The restore finished (complete, partial or cancelled); the caller rescans. */
  onRestored: () => void;
}

/**
 * Review-then-restore (F-114/F-115). The plan is rebuilt every time the dialog
 * opens and every time the `saves` flag changes, because the flag changes what
 * the plan is allowed to touch.
 *
 * The execute call sends the plan's `manifestSha256` back: the bridge re-plans
 * and refuses a folder whose manifest moved since this screen read it. The word
 * `RESTORE` is the second gate, checked in Rust as well.
 */
export function RestoreDialog({ root, backupRoot, backup, onClose, onRestored }: Props) {
  const { t } = useTranslation();
  // The listing returns one root and one stamp per folder; the dated folder is
  // their join. Core canonicalises and re-checks containment on every call, so
  // this string is a hint, never authority.
  const backupDir = `${backupRoot.replace(/[\\/]+$/, '')}/${backup.stamp}`;
  const [includeSaves, setIncludeSaves] = useState(false);
  const [plan, setPlan] = useState<RestorePlanDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [confirmation, setConfirmation] = useState('');
  const [running, setRunning] = useState(false);
  const [phase, setPhase] = useState<string | null>(null);
  const [run, setRun] = useState<RestoreRunDto | null>(null);

  useEffect(() => {
    let live = true;
    setPlan(null);
    setError(null);
    planInstanceRestore({
      instanceRoot: root,
      backupDir,
      includeSaves,
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
  }, [root, backupDir, includeSaves, attempt]);

  const allowed = plan?.allowed ?? [];
  const blocked = plan?.blocked ?? [];
  const rows = plan ? [...plan.allowed, ...plan.blocked] : [];
  const wordOk = confirmation.trim() === RESTORE_WORD;

  const confirm = () => {
    if (!plan || running || allowed.length === 0 || !wordOk) {
      return;
    }
    setRunning(true);
    setError(null);
    setPhase(null);
    executeInstanceRestore(
      {
        instanceRoot: root,
        // The plan's own path, not the guessed one: what was reviewed is what
        // is executed.
        backupDir: plan.backupDir,
        includeSaves: plan.includeSaves,
        expectedManifestSha256: plan.manifestSha256,
        confirmation: confirmation.trim(),
      },
      (event) => setPhase(event.phase),
    )
      .then((result) => {
        setRun(result);
        onRestored();
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => {
        setRunning(false);
        setPhase(null);
      });
  };

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        // A restore is a write already in flight; closing the only screen that
        // reports its outcome would hide it, not undo it.
        if (!next && !running) {
          onClose();
        }
      }}
    >
      <DialogContent
        className="flex max-h-[calc(100vh-2rem)] max-w-[560px] flex-col gap-3 overflow-hidden"
        showCloseButton={!running}
      >
        <DialogHeader className="shrink-0">
          <DialogTitle>{t('restore.title')}</DialogTitle>
          <DialogDescription>{t('restore.description')}</DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto pe-1">
          {!run && !plan && !error && <LoadingState className="py-4" />}

          {!run && error && !plan && (
            <ErrorState
              error={error}
              onRetry={() => {
                setAttempt((n) => n + 1);
              }}
            />
          )}

          {!run && plan && (
            <>
              <p
                dir="ltr"
                title={plan.backupDir}
                className="truncate rounded-control border border-border bg-surface-sunken px-2.5 py-1.5 font-mono text-caption text-muted-foreground"
              >
                {plan.createdAt === '' ? plan.backupDir : plan.createdAt}
              </p>

              <p className="text-small" role="status">
                {t('restore.count', { allowed: allowed.length, blocked: blocked.length })}
              </p>

              {phase && (
                <p role="status" aria-live="polite" className="text-small text-muted-foreground">
                  {t(phase)}
                </p>
              )}

              {allowed.length === 0 && (
                <p className="text-small text-status-issue">{t('restore.nothingAllowed')}</p>
              )}

              {backup.includeSaves ? (
                <div className="flex items-start gap-2">
                  <Checkbox
                    id="restore-include-saves"
                    checked={includeSaves}
                    disabled={running}
                    onCheckedChange={(next) => setIncludeSaves(next === true)}
                  />
                  <span className="min-w-0">
                    <label htmlFor="restore-include-saves" className="cursor-pointer text-small">
                      {t('restore.includeSaves')}
                    </label>
                    <span className="block text-caption text-muted-foreground">
                      {t('restore.includeSavesHint')}
                    </span>
                  </span>
                </div>
              ) : (
                <p className="text-caption text-muted-foreground">{t('restore.noSavesInBackup')}</p>
              )}

              <p className="text-caption text-muted-foreground">{t('restore.safetyNote')}</p>
              <p
                dir="ltr"
                title={plan.safetyDir}
                className="truncate rounded-control border border-border bg-surface-sunken px-2.5 py-1.5 font-mono text-caption text-muted-foreground"
              >
                {plan.safetyDir}
              </p>

              <ul className="flex flex-col gap-1">
                {rows.map((entry) => (
                  <EntryRow key={`${entry.source}-${entry.backupPath}`} entry={entry} />
                ))}
              </ul>

              {blocked.length > 0 && (
                <p className="text-caption text-muted-foreground">{t('restore.blockedNote')}</p>
              )}

              {allowed.length > 0 && (
                <div className="rounded-control border border-border bg-card px-3 py-2.5">
                  <label htmlFor="restore-confirmation" className="text-small font-semibold">
                    {t('restore.confirmLabel')}
                  </label>
                  <p id="restore-confirmation-hint" className="mt-0.5 text-caption text-muted-foreground">
                    {t('restore.confirmHint')}
                  </p>
                  <input
                    id="restore-confirmation"
                    type="text"
                    dir="ltr"
                    autoComplete="off"
                    autoCorrect="off"
                    spellCheck={false}
                    value={confirmation}
                    disabled={running}
                    onChange={(e) => setConfirmation(e.target.value)}
                    placeholder={RESTORE_WORD}
                    aria-describedby="restore-confirmation-hint"
                    className="mt-2 h-8 w-full rounded-control border border-border bg-surface-sunken px-2.5 font-mono text-body text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50"
                  />
                </div>
              )}

              {error && (
                <p role="alert" className="text-small text-status-issue">
                  {t('restore.failed', {
                    reason: t(`error.${error.code}`, { defaultValue: error.message }),
                  })}
                </p>
              )}
            </>
          )}

          {run && (
            <>
              <p
                role="status"
                className={cn(
                  'text-small',
                  run.outcome === 'complete' ? 'text-status-success' : 'text-status-warning',
                )}
              >
                {t(`restore.outcome.${run.outcome}`, { defaultValue: run.outcome })}
              </p>
              <ul className="flex flex-col gap-1">
                {run.entries.map((entry) => (
                  <ResultRow key={`${entry.source}-${entry.targetPath}`} entry={entry} />
                ))}
              </ul>
              {run.blocked.length > 0 && (
                <>
                  <p className="text-caption font-medium text-foreground">
                    {t('restore.leftAlone')}
                  </p>
                  <ul className="flex flex-col gap-1">
                    {run.blocked.map((entry) => (
                      <EntryRow key={`${entry.source}-${entry.backupPath}`} entry={entry} />
                    ))}
                  </ul>
                </>
              )}
              <p className="text-caption text-muted-foreground">{t('restore.recordLabel')}</p>
              <p
                dir="ltr"
                title={run.manifestPath}
                className="truncate rounded-control border border-border bg-surface-sunken px-2.5 py-1.5 font-mono text-caption text-muted-foreground"
              >
                {run.manifestPath}
              </p>
            </>
          )}
        </div>

        <DialogFooter className="shrink-0">
          {run ? (
            <Button type="button" onClick={onClose} disabled={running}>
              {t('state.action.close')}
            </Button>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={onClose} disabled={running}>
                {t('state.action.cancel')}
              </Button>
              <Button
                type="button"
                variant="destructive"
                onClick={confirm}
                disabled={!plan || running || allowed.length === 0 || !wordOk}
              >
                {running
                  ? t('restore.running')
                  : t('restore.actionExecute', { count: allowed.length })}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function EntryRow({ entry }: { entry: RestoreEntryDto }) {
  const { t } = useTranslation();
  return (
    <li className="flex items-start gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5">
      <span
        aria-hidden
        className={cn(
          'mt-1.5 inline-block size-1.5 shrink-0 rounded-full',
          entry.allowed ? 'bg-status-success' : 'bg-status-warning',
        )}
      />
      <div className="min-w-0 flex-1">
        <p className="truncate text-body" dir="auto" title={entry.source}>
          {t(`backup.source.${entry.source}`, { defaultValue: entry.source })}
        </p>
        <p className="truncate font-mono text-caption text-muted-foreground" dir="ltr">
          {entry.targetPath}
        </p>
        <p
          className={cn(
            'text-caption',
            entry.allowed ? 'text-muted-foreground' : 'text-status-warning',
          )}
        >
          {t(`restore.reason.${entry.reasonCode}`, { defaultValue: entry.reasonCode })}
        </p>
      </div>
      {entry.allowed && entry.fileCount > 0 && (
        <span className="shrink-0 text-caption text-muted-foreground">
          {t('restore.entryFiles', { count: entry.fileCount })}
        </span>
      )}
    </li>
  );
}

function ResultRow({ entry }: { entry: RestoreResultEntryDto }) {
  const { t } = useTranslation();
  const fine = entry.state === 'restored';
  return (
    <li className="flex items-start gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5">
      <span
        aria-hidden
        className={cn(
          'mt-1.5 inline-block size-1.5 shrink-0 rounded-full',
          fine ? 'bg-status-success' : 'bg-status-issue',
        )}
      />
      <div className="min-w-0 flex-1">
        <p className="truncate text-body" dir="auto" title={entry.source}>
          {t(`backup.source.${entry.source}`, { defaultValue: entry.source })}
        </p>
        <p className="truncate font-mono text-caption text-muted-foreground" dir="ltr">
          {entry.targetPath}
        </p>
        <p className={cn('text-caption', fine ? 'text-status-success' : 'text-status-issue')}>
          {t(`restore.state.${entry.state}`, { defaultValue: entry.state })}
          {!fine && (
            <>
              {' — '}
              {t(`restore.reason.${entry.reasonCode}`, { defaultValue: entry.reasonCode })}
            </>
          )}
        </p>
      </div>
    </li>
  );
}
