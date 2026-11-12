import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TriangleAlert } from 'lucide-react';

import type { IpcError } from '../../bindings/IpcError';
import type { RemoveEntryDto } from '../../bindings/RemoveEntryDto';
import type { RemovePlanDto } from '../../bindings/RemovePlanDto';
import type { RemoveResultDto } from '../../bindings/RemoveResultDto';
import type { RemoveRunDto } from '../../bindings/RemoveRunDto';
import { Button } from '@/components/ui/button';
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
import { executeRemoveContent, previewRemoveContent, toIpcError } from '../../lib/ipc';

/** Mirrors `REMOVE_CONFIRMATION_WORD` in mm-core; the bridge checks it again. */
const REMOVE_WORD = 'REMOVE';

interface Props {
  root: string;
  /** `relPath` values, never bare filenames: the scan may find `disabled_mods/`. */
  files: string[];
  onClose: () => void;
  /** The files moved; the caller should rescan and clear its selection. */
  onRemoved: () => void;
}

/**
 * Review-then-move (F-111/F-112). The dialog shows every guard verdict, the
 * external/unknown warning and the typed word it demands, then reports what
 * each file did. Closing is refused while a run is in flight.
 */
export function RemoveDialog({ root, files, onClose, onRemoved }: Props) {
  const { t } = useTranslation();
  // Freeze the selection this dialog was opened with. The parent clears its
  // selection as soon as the run succeeds, and a fresh `files` array would
  // otherwise reset the preview before the results can be read.
  const [targetFiles] = useState(files);
  const [plan, setPlan] = useState<RemovePlanDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [confirmation, setConfirmation] = useState('');
  const [running, setRunning] = useState(false);
  const [run, setRun] = useState<RemoveRunDto | null>(null);

  useEffect(() => {
    let live = true;
    setPlan(null);
    setError(null);
    previewRemoveContent({ instanceRoot: root, files: targetFiles })
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
  }, [root, targetFiles, attempt]);

  const allowed = plan?.entries.filter((e) => e.allowed) ?? [];
  const external = allowed.filter((e) => e.requiresConfirmation).length;
  const needsWord = plan?.requiresConfirmation ?? false;
  const wordOk = !needsWord || confirmation.trim() === REMOVE_WORD;

  const confirm = () => {
    if (!plan || running || allowed.length === 0 || !wordOk) {
      return;
    }
    setRunning(true);
    setError(null);
    executeRemoveContent({
      instanceRoot: root,
      files: targetFiles,
      confirmation: confirmation.trim(),
    })
      .then((result) => {
        setRun(result);
        onRemoved();
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setRunning(false));
  };

  const moved = run?.results.filter((r) => r.state === 'moved-to-backup').length ?? 0;
  const failed = (run?.results.length ?? 0) - moved;

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        // A run is a move on disk, not a request the user can cancel by
        // closing the only place that reports its outcome.
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
          <DialogTitle>{t('remove.title')}</DialogTitle>
          <DialogDescription>{t('remove.description')}</DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto pe-1">
          {!run && (
            <p className="text-caption text-muted-foreground">{t('remove.backupNote')}</p>
          )}

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
              <p className="text-small" role="status">
                {t('remove.count', {
                  allowed: allowed.length,
                  refused: plan.entries.length - allowed.length,
                })}
              </p>

              {allowed.length === 0 && (
                <p className="text-small text-status-issue">{t('remove.nothingAllowed')}</p>
              )}

              {external > 0 && (
                <div className="flex items-start gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-2">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0 text-status-warning" aria-hidden />
                  <div className="min-w-0">
                    <p className="text-small font-semibold">{t('remove.externalTitle')}</p>
                    <p className="mt-0.5 text-caption text-muted-foreground">
                      {t('remove.externalWarning', { count: external })}
                    </p>
                  </div>
                </div>
              )}

              <ul className="flex flex-col gap-1">
                {plan.entries.map((entry) => (
                  <EntryRow key={entry.relPath} entry={entry} />
                ))}
              </ul>

              {needsWord && allowed.length > 0 && (
                <div className="rounded-control border border-border bg-card px-3 py-2.5">
                  <label htmlFor="remove-confirmation" className="text-small font-semibold">
                    {t('remove.confirmLabel')}
                  </label>
                  <p id="remove-confirmation-hint" className="mt-0.5 text-caption text-muted-foreground">
                    {t('remove.confirmHint')}
                  </p>
                  <input
                    id="remove-confirmation"
                    type="text"
                    dir="ltr"
                    autoComplete="off"
                    autoCorrect="off"
                    spellCheck={false}
                    value={confirmation}
                    disabled={running}
                    onChange={(e) => setConfirmation(e.target.value)}
                    placeholder={REMOVE_WORD}
                    aria-describedby="remove-confirmation-hint"
                    className="mt-2 h-8 w-full rounded-control border border-border bg-surface-sunken px-2.5 font-mono text-body text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50"
                  />
                </div>
              )}

              {error && (
                <p role="alert" className="text-small text-status-issue">
                  {t('remove.failed', {
                    reason: t(`error.${error.code}`, { defaultValue: error.message }),
                  })}
                </p>
              )}
            </>
          )}

          {run && (
            <>
              <p className="text-small" role="status">
                {t('remove.done', { count: moved })}
              </p>
              {failed > 0 && (
                <p className="text-small text-status-issue">
                  {t('remove.failedCount', { count: failed })}
                </p>
              )}
              <ul className="flex flex-col gap-1">
                {run.results.map((result) => (
                  <ResultRow key={result.relPath} result={result} />
                ))}
              </ul>
              {run.manifestError && (
                <p role="alert" className="text-caption text-status-warning">
                  {t('remove.manifestFailed')}
                </p>
              )}
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
                  ? t('remove.running')
                  : t('remove.actionConfirm', { count: allowed.length })}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function EntryRow({ entry }: { entry: RemoveEntryDto }) {
  const { t } = useTranslation();
  return (
    <li className="flex items-start gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5">
      <span
        aria-hidden
        className={cn(
          'mt-1.5 inline-block size-1.5 shrink-0 rounded-full',
          entry.allowed ? 'bg-status-success' : 'bg-status-issue',
        )}
      />
      <div className="min-w-0 flex-1">
        <p className="truncate text-body" dir="auto" title={entry.relPath}>
          {entry.filename}
        </p>
        <p className="truncate font-mono text-caption text-muted-foreground" dir="ltr">
          {entry.relPath}
        </p>
        <p
          className={cn(
            'text-caption',
            entry.allowed ? 'text-muted-foreground' : 'text-status-issue',
          )}
        >
          {t(`remove.reason.${entry.reasonCode}`, { defaultValue: entry.reason })}
        </p>
        {entry.requiresConfirmation && (
          <p className="text-caption text-status-warning">{t('remove.externalRow')}</p>
        )}
      </div>
    </li>
  );
}

function ResultRow({ result }: { result: RemoveResultDto }) {
  const { t } = useTranslation();
  const moved = result.state === 'moved-to-backup';
  return (
    <li className="flex items-start gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5">
      <span
        aria-hidden
        className={cn(
          'mt-1.5 inline-block size-1.5 shrink-0 rounded-full',
          moved ? 'bg-status-success' : 'bg-status-issue',
        )}
      />
      <div className="min-w-0 flex-1">
        <p className="truncate text-body" dir="auto" title={result.relPath}>
          {result.filename}
        </p>
        <p className="truncate font-mono text-caption text-muted-foreground" dir="ltr">
          {result.relPath}
        </p>
        <p
          className={cn(
            'text-caption',
            moved ? 'text-status-success' : 'text-status-issue',
          )}
        >
          {t(`remove.state.${result.state}`, { defaultValue: result.state })}
          {' — '}
          {t(`remove.reason.${result.reasonCode}`, { defaultValue: result.message })}
        </p>
      </div>
    </li>
  );
}
