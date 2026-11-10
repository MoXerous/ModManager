import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { InstanceSnapshotDto } from '../../bindings/InstanceSnapshotDto';
import type { IpcError } from '../../bindings/IpcError';
import type { TogglePlanDto } from '../../bindings/TogglePlanDto';
import type { UpdatePreviewDto } from '../../bindings/UpdatePreviewDto';
import type { UpdateStatusDto } from '../../bindings/UpdateStatusDto';
import { Button } from '@/components/ui/button';
import { Segmented } from '@/components/ui/Segmented';
import { ErrorState, LoadingState } from '../../components/state';
import {
  checkInstanceUpdates,
  executeToggle,
  previewToggle,
  scanInstance,
  toIpcError,
} from '../../lib/ipc';
import { formatBytes } from '../../lib/format';
import { type ProviderId, PROVIDERS, getGameTarget } from '../gameTarget';
import { InstanceExport } from './InstanceExport';

type Action = 'enable' | 'disable';

function statusClass(status: UpdateStatusDto): string {
  switch (status) {
    case 'available':
      return 'text-status-update';
    case 'upToDate':
      return 'text-status-success';
    case 'unidentified':
      return 'text-status-disabled';
    case 'unsupported':
      return 'text-status-warning';
  }
}

/**
 * The answer, and only the answer. It names the current file, the latest
 * version and a status word; there is no install button because installing an
 * update needs a plan, a backup and a re-check, and none of that exists yet.
 */
function UpdateResults({ preview }: { preview: UpdatePreviewDto }) {
  const { t } = useTranslation();
  const available = preview.candidates.filter((c) => c.status === 'available').length;
  const current = preview.candidates.filter((c) => c.status === 'upToDate').length;
  const unknown = preview.candidates.length - available - current;

  return (
    <div className="mt-2" aria-live="polite">
      {preview.warnings.map((warning) => (
        <p key={warning} className="text-caption text-status-warning">
          {t(`details.updates.warn.${warning}`)}
        </p>
      ))}

      {preview.checked > 0 ? (
        <>
          <p className="text-small">{t('details.updates.checked', { count: preview.checked })}</p>
          <p className="mt-0.5 text-caption text-muted-foreground">
            {t('details.updates.tally', { available, current, unknown })}
          </p>
          <ul className="mt-1.5 flex flex-col gap-1">
            {preview.candidates.map((c) => (
              <li
                key={c.filename}
                className="flex items-center gap-2 rounded-control bg-surface-sunken px-2.5 py-1.5 text-caption"
              >
                <span className="min-w-0 flex-1 truncate" title={c.filename} dir="auto">
                  {c.filename}
                </span>
                {c.latestVersionNumber && (
                  <span
                    dir="ltr"
                    className="shrink-0 font-mono text-muted-foreground"
                    title={c.latestFilename ?? undefined}
                  >
                    {c.latestVersionNumber}
                  </span>
                )}
                <span className={`shrink-0 ${statusClass(c.status)}`}>
                  {t(`details.updates.status.${c.status}`)}
                </span>
              </li>
            ))}
          </ul>
          <p className="mt-1.5 text-caption text-muted-foreground">
            {t('details.updates.installDeferred')}
          </p>
        </>
      ) : (
        preview.warnings.length === 0 && (
          <p className="text-caption text-muted-foreground">{t('details.updates.nothing')}</p>
        )
      )}
    </div>
  );
}

interface Props {
  root: string;
  /** The registry's name for this instance; only the export default uses it. */
  instanceName: string;
  /** The registered instance's own target; the check is worthless without it. */
  gameVersion: string | null;
  loader: string | null;
  onChanged: () => void;
}

/**
 * The one tab `screens.md` §6 keeps for instance details: installed content.
 * Validate, repair, backup and restore are gone on purpose — four tabs and five
 * dialogs in the old app, for things a context menu does. Export stayed because
 * it is the one action whose result leaves the app: a `.mrpack` the user can
 * share, and its plan is worth reading before it is written (F-081/F-082).
 *
 * Toggle is preview-then-execute. The preview carries the refusals too, which
 * is the part worth showing: `allowed: false` with a `reason` is the only place
 * the user learns why a file was left alone.
 */
export function InstanceDetails({ root, instanceName, gameVersion, loader, onChanged }: Props) {
  const { t } = useTranslation();
  const [snap, setSnap] = useState<InstanceSnapshotDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);
  const [action, setAction] = useState<Action | null>(null);
  const [plan, setPlan] = useState<TogglePlanDto | null>(null);
  // The catalogue check is per provider; the Browse selection is a sensible
  // default, but changing it here must not silently re-point the search page.
  const [provider, setProvider] = useState<ProviderId>(() => getGameTarget().provider);
  const [updates, setUpdates] = useState<UpdatePreviewDto | null>(null);
  const [updatesError, setUpdatesError] = useState<IpcError | null>(null);
  const [updatesBusy, setUpdatesBusy] = useState(false);

  const load = useCallback(() => {
    setError(null);
    scanInstance(root)
      .then(setSnap)
      .catch((e) => setError(toIpcError(e)));
  }, [root]);

  useEffect(load, [load]);

  // The scan reports filenames only, and the command takes names inside the
  // instance's mods folder — so the two line up with no path building on this
  // side at all. Sending an empty list instead would plan nothing and apply
  // nothing, while looking like a button that works.
  const names = snap?.mods.map((m) => m.filename) ?? [];

  const preview = (a: Action) => {
    setBusy(true);
    setError(null);
    previewToggle({ instanceRoot: root, action: a, files: names })
      .then((p) => {
        setPlan(p);
        setAction(a);
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  const apply = () => {
    if (!action) {
      return;
    }
    setBusy(true);
    executeToggle({ instanceRoot: root, action, files: names })
      .then(() => {
        setPlan(null);
        setAction(null);
        load();
        onChanged();
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  // `any` and `unknown` are sentinels, not loaders: a provider can only filter
  // by a real one. The button stays disabled and the hint says why, rather
  // than sending a request whose only possible answer is an empty page.
  const targetLoader = (loader ?? '').trim().toLowerCase();
  const targetVersion = (gameVersion ?? '').trim();
  const hasTarget =
    targetVersion !== '' &&
    targetLoader !== '' &&
    targetLoader !== 'any' &&
    targetLoader !== 'unknown';

  const checkUpdates = () => {
    if (!hasTarget || updatesBusy) {
      return;
    }
    setUpdatesBusy(true);
    setUpdatesError(null);
    // A result belongs to the request that produced it. Keeping the old table
    // under "the check failed" reads as if it were the answer to this check.
    setUpdates(null);
    checkInstanceUpdates({
      instanceRoot: root,
      provider,
      gameVersion: targetVersion,
      loader: targetLoader,
    })
      .then(setUpdates)
      .catch((e) => setUpdatesError(toIpcError(e)))
      .finally(() => setUpdatesBusy(false));
  };

  const chooseProvider = (next: ProviderId) => {
    setProvider(next);
    // A result belongs to the provider that produced it; keeping it after a
    // switch would show one catalogue's answer under another's name.
    setUpdates(null);
    setUpdatesError(null);
  };

  // Only a failed *load* replaces the view. A failed toggle keeps the table:
  // the user pressed one button, and losing the whole list to a refusal from
  // that button reads as data loss rather than as an error.
  if (error && !snap) {
    return (
      <div className="px-5 pt-4">
        <ErrorState error={error} onRetry={load} />
      </div>
    );
  }
  if (!snap) {
    return <LoadingState className="px-5 pt-6" />;
  }

  const disabledCount = snap.mods.filter((m) => m.disabled).length;
  const total = snap.mods.length;

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-5 pt-4">
        <h2 className="text-body font-semibold">{t('details.installedContent')}</h2>
        <span className="text-small text-muted-foreground">{t('details.count', { count: total })}</span>
        <div className="ms-auto flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            // Nothing left to enable is `disabledCount === 0`, not
            // `=== total`: those two are opposite states, and writing `total`
            // here disables the button exactly when it is most useful.
            disabled={busy || total === 0 || disabledCount === 0}
            onClick={() => preview('enable')}
          >
            {t('details.actionEnable')}
          </Button>
          <Button
            type="button"
            variant="outline"
            // The mirror image: nothing left to disable means everything is
            // already disabled, so this is `=== total`, not `=== 0`.
            disabled={busy || total === 0 || disabledCount === total}
            onClick={() => preview('disable')}
          >
            {t('details.actionDisable')}
          </Button>
        </div>
      </div>

      {/* The check is its own card, not one more button in the toggle row:
          it carries a provider choice and a result table, and mixing those
          with "enable/disable all" would make two different questions look
          like one. */}
      <div className="mx-5 mt-3 rounded-control border border-border bg-card px-3 py-2.5">
        <div className="flex items-center gap-2">
          <span className="text-body font-semibold">{t('details.updates.title')}</span>
          <Button
            type="button"
            variant="outline"
            className="ms-auto"
            aria-busy={updatesBusy}
            disabled={updatesBusy || !hasTarget}
            onClick={checkUpdates}
          >
            {updatesBusy ? t('details.updates.checking') : t('details.updates.check')}
          </Button>
        </div>
        <div className="mt-2">
          <Segmented<ProviderId>
            label={t('details.updates.provider')}
            value={provider}
            options={PROVIDERS.map((id) => ({ value: id, label: t(`provider.${id}`) }))}
            onChange={chooseProvider}
          />
        </div>
        <p className="mt-1.5 text-caption text-muted-foreground">{t('details.updates.readOnly')}</p>
        {!hasTarget && (
          <p className="mt-1 text-caption text-status-warning">{t('details.updates.needsTarget')}</p>
        )}
        {updatesError && (
          <p role="alert" className="mt-2 text-small text-status-issue">
            {t('details.updates.failed', { reason: t(`error.${updatesError.code}`) })}
          </p>
        )}
        {updates && <UpdateResults preview={updates} />}
      </div>

      {/* The pack leaves the app, so it gets its own review card rather than a
          menu item: the preview is the only place the user sees what will and
          will not travel before a file is written. */}
      <InstanceExport
        root={root}
        instanceName={instanceName}
        gameVersion={gameVersion}
        loader={loader}
      />

      {plan && (
        <div className="mx-5 mt-3 rounded-control border border-border bg-card px-4 py-3">
          <p className="text-body">
            {t('details.plan', {
              shown: plan.entries.filter((e) => e.allowed).length,
              hidden: plan.entries.filter((e) => !e.allowed).length,
            })}
          </p>
          <ul className="mt-2 flex flex-col gap-1">
            {plan.entries.map((e) => (
              <li
                key={e.filename}
                className="flex items-center gap-2 text-caption"
                title={e.reason || undefined}
              >
                <span
                  className={
                    e.allowed
                      ? 'text-muted-foreground'
                      : 'text-status-issue'
                  }
                >
                  {e.allowed ? e.filename : `${e.filename} — ${e.reason}`}
                </span>
              </li>
            ))}
          </ul>
          <div className="mt-3 flex gap-2">
            <Button type="button" onClick={apply} disabled={busy}>
              {t('details.actionApply')}
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setPlan(null);
                setAction(null);
              }}
              disabled={busy}
            >
              {t('state.action.cancel')}
            </Button>
          </div>
        </div>
      )}

      {total === 0 ? (
        <p className="px-5 pt-4 text-small text-muted-foreground">{t('details.empty')}</p>
      ) : (
        <ul className="flex flex-col gap-1.5 px-5 py-4">
          {snap.mods.map((m) => (
            <li
              key={m.filename}
              className="flex items-center gap-2.5 rounded-control border border-border bg-card px-3 py-2"
            >
              <span
                className={
                  m.disabled ? 'text-caption text-status-disabled' : 'text-caption text-muted-foreground'
                }
              >
                {m.resourceType}
              </span>
              <span className="min-w-0 flex-1 truncate text-body">{m.filename}</span>
              <span dir="ltr" className="font-mono text-caption text-muted-foreground">
                {formatBytes(m.size)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}