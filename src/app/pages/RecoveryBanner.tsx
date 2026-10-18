import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TriangleAlert } from 'lucide-react';

import type { CreateRecoveryDto } from '../../bindings/CreateRecoveryDto';
import type { IpcError } from '../../bindings/IpcError';
import { Button } from '@/components/ui/button';
import { ErrorState } from '../../components/state';
import {
  abandonCreate,
  finishCreate,
  listPendingCreates,
  toIpcError,
} from '../../lib/ipc';

interface Props {
  /** Re-run after a choice, so the list below reflects what changed. */
  onSettled: () => void;
}

/**
 * F-146. Shown only when a create crashed and still needs a decision.
 *
 * The button comes from the `action` key core chose, so this file never decides
 * what is safe — it renders one of the two commands that exist. `Leave` has no
 * command behind it, so it renders no button rather than a dead one; core
 * settles those records itself before they ever reach here.
 */
export function RecoveryBanner({ onSettled }: Props) {
  const { t } = useTranslation();
  const [items, setItems] = useState<CreateRecoveryDto[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<IpcError | null>(null);

  const load = useCallback(() => {
    listPendingCreates()
      .then(setItems)
      .catch((e) => setError(toIpcError(e)));
  }, []);

  useEffect(load, [load]);

  const act = (item: CreateRecoveryDto) => {
    setBusy(item.operationId);
    setError(null);
    const done =
      item.action === 'recovery.instance.action.finish'
        ? finishCreate(item.operationId)
        : abandonCreate(item.operationId);
    done
      .then(() => {
        setItems((prev) => prev?.filter((i) => i.operationId !== item.operationId) ?? null);
        onSettled();
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(null));
  };

  if (error) return <ErrorState error={error} />;
  // Nothing crashed. The banner must not exist at all, not sit there empty:
  // space reserved for a problem that is not happening is noise on every launch.
  if (!items || items.length === 0) return null;

  return (
    <section aria-label={t('recovery.instance.title')} className="mb-5">
      {items.map((item) => (
        <article
          key={item.operationId}
          className="flex flex-wrap items-center gap-3 rounded-control border border-border bg-card px-4 py-3"
        >
          <TriangleAlert aria-hidden className="size-4 shrink-0 text-warning" />
          <div className="min-w-0 flex-1">
            <p className="text-body font-medium">{t(item.reason)}</p>
            <p dir="ltr" className="truncate font-mono text-caption text-muted-foreground">
              {item.root}
            </p>
          </div>
          {item.action === 'recovery.instance.action.leave' ? null : (
            <Button
              type="button"
              variant="outline"
              disabled={busy !== null}
              onClick={() => act(item)}
            >
              {t(item.action)}
            </Button>
          )}
        </article>
      ))}
    </section>
  );
}
