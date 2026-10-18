import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { InstanceSnapshotDto } from '../../bindings/InstanceSnapshotDto';
import type { IpcError } from '../../bindings/IpcError';
import type { TogglePlanDto } from '../../bindings/TogglePlanDto';
import { Button } from '@/components/ui/button';
import { ErrorState, LoadingState } from '../../components/state';
import { executeToggle, previewToggle, scanInstance, toIpcError } from '../../lib/ipc';
import { formatBytes } from '../../lib/format';

type Action = 'enable' | 'disable';

interface Props {
  root: string;
  onChanged: () => void;
}

/**
 * The one tab `screens.md` §6 keeps for instance details: installed content.
 * Validate, repair, backup, restore and export are gone on purpose — four tabs
 * and five dialogs in the old app, for things a context menu does.
 *
 * Toggle is preview-then-execute. The preview carries the refusals too, which
 * is the part worth showing: `allowed: false` with a `reason` is the only place
 * the user learns why a file was left alone.
 */
export function InstanceDetails({ root, onChanged }: Props) {
  const { t } = useTranslation();
  const [snap, setSnap] = useState<InstanceSnapshotDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);
  const [action, setAction] = useState<Action | null>(null);
  const [plan, setPlan] = useState<TogglePlanDto | null>(null);

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