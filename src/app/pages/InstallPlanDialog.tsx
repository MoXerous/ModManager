import { type ReactNode, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Info, TriangleAlert } from 'lucide-react';

import type { InstallEventDto } from '../../bindings/InstallEventDto';
import type { InstallPlanDto } from '../../bindings/InstallPlanDto';
import type { InstanceSummaryDto } from '../../bindings/InstanceSummaryDto';
import type { IpcError } from '../../bindings/IpcError';
import type { PlanEntryDto } from '../../bindings/PlanEntryDto';
import type { ProjectDto } from '../../bindings/ProjectDto';
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
import { formatBytes } from '../../lib/format';
import { buildInstallPlan, executeInstall, toIpcError } from '../../lib/ipc';

/** `install.phase.*` keys that are a file's final answer. */
const SETTLED = new Set([
  'install.phase.landed',
  'install.phase.skipped',
  'install.phase.failed',
  'install.phase.cancelled',
]);

interface Props {
  open: boolean;
  project: ProjectDto | null;
  instance: InstanceSummaryDto | null;
  gameVersion: string;
  loader: string;
  onClose: () => void;
  /** The batch finished; the queue is where its progress and log live. */
  onInstalled: () => void;
}

/**
 * F-062/F-063. Plan first, install second.
 *
 * The dialog is the review `build_install_plan` exists for: the selected
 * project, the required dependencies it drags in, what is blocked and why, and
 * the local warnings — all before a single byte is written. Confirmation calls
 * `execute_install`, which rebuilds the plan in Rust rather than trusting this
 * screen (AGENTS.md §5.4).
 *
 * `reason` and warning `message` are resolver data pinned by exact-string core
 * tests, not UI copy: they are shown verbatim inside localized framing until
 * core can hand over codes.
 */
export function InstallPlanDialog({
  open,
  project,
  instance,
  gameVersion,
  loader,
  onClose,
  onInstalled,
}: Props) {
  const { t } = useTranslation();
  const [plan, setPlan] = useState<InstallPlanDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [loading, setLoading] = useState(false);
  const [installing, setInstalling] = useState(false);
  // Version id -> its latest event, so a row can say what is happening to it.
  const [phases, setPhases] = useState<Record<string, InstallEventDto>>({});
  // Bumped by "retry" after a plan read failed; the effect keys on it.
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!open || !project || !instance) {
      setPlan(null);
      setError(null);
      setLoading(false);
      setInstalling(false);
      setPhases({});
      return;
    }
    // A slow answer for a previous project must not land in this one's dialog.
    let live = true;
    setPlan(null);
    setError(null);
    setPhases({});
    setInstalling(false);
    setLoading(true);
    buildInstallPlan(
      [project.id],
      project.providerId,
      gameVersion.trim(),
      loader.trim().toLowerCase(),
      instance.path,
    )
      .then((next) => {
        if (live) {
          setPlan(next);
        }
      })
      .catch((e) => {
        if (live) {
          setError(toIpcError(e));
        }
      })
      .finally(() => {
        if (live) {
          setLoading(false);
        }
      });
    return () => {
      live = false;
    };
  }, [open, project, instance, gameVersion, loader, attempt]);

  const onEvent = (event: InstallEventDto) => {
    setPhases((prev) => {
      const prior = prev[event.itemId];
      // The terminal event is the truth about a file; a late chunk must not
      // reopen a row that already landed or failed.
      if (prior && SETTLED.has(prior.phase) && !SETTLED.has(event.phase)) {
        return prev;
      }
      return { ...prev, [event.itemId]: event };
    });
  };

  const confirm = () => {
    if (!project || !instance || !plan || installing) {
      return;
    }
    setInstalling(true);
    setError(null);
    setPhases({});
    void executeInstall(
      {
        instanceRoot: instance.path,
        projects: [project.id],
        provider: project.providerId,
        gameVersion: gameVersion.trim(),
        loader: loader.trim().toLowerCase(),
      },
      onEvent,
    )
      .then(onInstalled)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setInstalling(false));
  };

  const downloadables = plan?.summary.downloadableCount ?? 0;
  const blocked = plan?.summary.blockedCount ?? 0;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        // Closing mid-install would hide the only place that says what is
        // happening while the batch keeps writing to disk.
        if (!next && !installing) {
          onClose();
        }
      }}
    >
      <DialogContent
        className="flex max-h-[calc(100vh-2rem)] max-w-[560px] flex-col gap-3 overflow-hidden"
        showCloseButton={!installing}
        // Radix closes on Escape and on an outside click through the same
        // callback; the guard above is enough, and preventing default here
        // would also swallow the close button.
      >
        <DialogHeader className="shrink-0">
          <DialogTitle>{t('plan.title')}</DialogTitle>
          <DialogDescription>{t('plan.description')}</DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto pe-1">
          {project && instance && (
            <p className="text-small text-muted-foreground">
              {t('plan.target', {
                instance: instance.name,
                provider: t(`provider.${project.providerId}`),
                version: gameVersion.trim(),
                loader: loader.trim().toLowerCase(),
              })}
            </p>
          )}

          {loading && <LoadingState className="py-4" />}

          {error && !plan && (
            <ErrorState error={error} onRetry={() => setAttempt((n) => n + 1)} />
          )}

          {plan && (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <Chip>{t('plan.count.selected', { count: plan.summary.selectedCount })}</Chip>
                {plan.summary.dependencyCount > 0 && (
                  <Chip>
                    {t('plan.count.dependencies', { count: plan.summary.dependencyCount })}
                  </Chip>
                )}
                <Chip tone={downloadables === 0 ? 'issue' : undefined}>
                  {t('plan.count.downloadable', { count: downloadables })}
                </Chip>
                {blocked > 0 && (
                  <Chip tone="issue">{t('plan.count.blocked', { count: blocked })}</Chip>
                )}
              </div>

              {/* The plan states it; the screen never asks the user to infer
                  "nothing will download" from a row list. */}
              {downloadables === 0 && (
                <p role="status" className="text-small text-status-issue">
                  {t('plan.nothingDownloadable')}
                </p>
              )}

              <ul className="flex flex-col gap-0.5">
                {plan.entries.map((entry) => (
                  <EntryRow
                    key={`${entry.provider}:${entry.projectId}`}
                    entry={entry}
                    phase={entry.versionId ? phases[entry.versionId] : undefined}
                  />
                ))}
              </ul>

              {plan.warnings.length > 0 && (
                <div className="flex flex-col gap-1">
                  <h3 className="text-small font-semibold">
                    {t('plan.notes', { count: plan.warnings.length })}
                  </h3>
                  <ul className="flex flex-col gap-1">
                    {plan.warnings.map((warning, index) => (
                      <WarningRow key={index} severity={warning.severity} message={warning.message} />
                    ))}
                  </ul>
                </div>
              )}

              {error && (
                <ErrorState error={error} onRetry={installing ? undefined : confirm} />
              )}
            </>
          )}
        </div>

        <DialogFooter className="shrink-0">
          <Button type="button" variant="outline" onClick={onClose} disabled={installing}>
            {t('state.action.cancel')}
          </Button>
          <Button
            type="button"
            onClick={confirm}
            disabled={!plan || installing || downloadables === 0}
          >
            {installing
              ? t('plan.installing')
              : plan
                ? t('plan.confirm', { count: downloadables })
                : t('install.start')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** `plan-variable` because the dialog serves only Browse; no global chip yet. */
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
 * One planned download. Its second line is the provider's identity for the
 * file (version and filename), kept `dir="ltr"` because both are Latin data
 * inside an Arabic frame.
 */
function EntryRow({ entry, phase }: { entry: PlanEntryDto; phase?: InstallEventDto }) {
  const { t, i18n } = useTranslation();
  const isBlocked = entry.status === 'blocked';
  const dataAlign = i18n.dir() === 'rtl' ? 'text-right' : 'text-left';
  const done = phase?.done != null ? Number(phase.done) : null;
  const total = phase?.total != null ? Number(phase.total) : null;
  const percent =
    done !== null && total !== null && total > 0
      ? Math.min(100, Math.round((done / total) * 100))
      : null;
  const statusLabel = phase
    ? t(phase.phase)
    : isBlocked
      ? t('plan.status.blocked')
      : t('plan.status.downloadable');
  // Filenames usually carry the version already; printing both reads as a
  // duplicate. Show the version only when it is not part of the name.
  const showVersion =
    entry.versionNumber !== null &&
    !(entry.filename !== null && entry.filename.includes(entry.versionNumber));
  const statusTone = phase
    ? phase.phase === 'install.phase.failed' || phase.phase === 'install.phase.cancelled'
      ? 'text-status-issue'
      : phase.phase === 'install.phase.landed'
        ? 'text-status-success'
        : 'text-muted-foreground'
    : isBlocked
      ? 'text-status-issue'
      : 'text-status-success';

  return (
    <li className="flex flex-col gap-1 rounded-control border border-border bg-surface-sunken px-2.5 py-1">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-body">{entry.title}</span>
        <span className="shrink-0 rounded-chip border border-border px-[7px] py-px text-caption text-muted-foreground">
          {entry.source === 'dependency'
            ? t('plan.source.dependency')
            : t('plan.source.selected')}
        </span>
        <span
          className={cn(
            'shrink-0 rounded-chip border border-border px-[7px] py-px text-caption',
            statusTone,
          )}
        >
          {statusLabel}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-caption text-muted-foreground">
        {entry.filename && (
          <span dir="ltr" className="min-w-0 truncate font-mono">
            {entry.filename}
          </span>
        )}
        {showVersion && <span dir="ltr">{entry.versionNumber}</span>}
        <span>{entry.projectType}</span>
        {done !== null && total !== null && (
          <span dir="ltr" className="ms-auto">
            {formatBytes(done)} / {formatBytes(total)}
          </span>
        )}
      </div>

      {percent !== null && (
        <div
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent}
          className="h-1 w-full overflow-hidden rounded-full bg-border"
        >
          <div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} />
        </div>
      )}

      {isBlocked && !phase && entry.reason && (
        <p dir="auto" className={cn('text-caption text-status-issue', dataAlign)}>
          {entry.reason}
        </p>
      )}
      {phase?.message && (
        <p dir="auto" className={cn('text-caption text-status-issue', dataAlign)}>
          {phase.message}
        </p>
      )}
    </li>
  );
}

/** A plan note. Its text is core data; only the frame around it is ours. */
function WarningRow({ severity, message }: { severity: string; message: string }) {
  const { i18n } = useTranslation();
  const align = i18n.dir() === 'rtl' ? 'text-right' : 'text-left';
  return (
    <li className="flex items-start gap-1.5 text-caption text-muted-foreground">
      {severity === 'warning' ? (
        <TriangleAlert
          aria-hidden
          className="mt-px size-3.5 shrink-0 text-status-warning"
          strokeWidth={1.75}
        />
      ) : (
        <Info aria-hidden className="mt-px size-3.5 shrink-0" strokeWidth={1.75} />
      )}
      <span dir="auto" className={cn('min-w-0 flex-1', align)}>
        {message}
      </span>
    </li>
  );
}
