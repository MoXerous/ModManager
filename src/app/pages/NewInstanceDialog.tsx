import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from 'cn';

import type { CreateInstancePlanDto } from '../../bindings/CreateInstancePlanDto';
import type { CreateInstanceRequest } from '../../bindings/CreateInstanceRequest';
import type { LinkInstanceRequest } from '../../bindings/LinkInstanceRequest';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { ErrorState, LoadingState } from '../../components/state';
import {
  executeCreateInstance,
  linkInstance,
  planCreateInstance,
  toIpcError,
} from '../../lib/ipc';

type Mode = 'create' | 'link';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDone: () => void;
}

/**
 * Board 12 / F-106: "link instead of create", because creating over somebody's
 * Minecraft folder is the mistake this dialog exists to prevent.
 *
 * Create is plan-first: the preview lists the folders Rust will create, and
 * `execute_create_instance` rebuilds that plan itself, so a folder taken in
 * between fails there rather than being written over.
 */
export function NewInstanceDialog({ open, onOpenChange, onDone }: Props) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<Mode>('create');
  const [name, setName] = useState('');
  const [root, setRoot] = useState('');
  const [minecraftVersion, setMinecraftVersion] = useState('');
  const [loader, setLoader] = useState('');
  const [plan, setPlan] = useState<CreateInstancePlanDto | null>(null);
  const [error, setError] = useState<ReturnType<typeof toIpcError> | null>(null);
  const [busy, setBusy] = useState(false);

  const reset = () => {
    setPlan(null);
    setError(null);
    setBusy(false);
  };

  const createRequest = (): CreateInstanceRequest => ({
    name: name.trim(),
    root: root.trim(),
    minecraftVersion: minecraftVersion.trim() || null,
    loader: loader.trim() || null,
  });

  const preview = () => {
    setBusy(true);
    setError(null);
    planCreateInstance(createRequest())
      .then(setPlan)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  const confirm = () => {
    setBusy(true);
    setError(null);
    executeCreateInstance(createRequest())
      .then(() => {
        reset();
        onDone();
        onOpenChange(false);
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  const link = () => {
    const req: LinkInstanceRequest = { name: name.trim(), root: root.trim() };
    setBusy(true);
    setError(null);
    linkInstance(req)
      .then(() => {
        reset();
        onDone();
        onOpenChange(false);
      })
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  const ready = name.trim().length > 0 && root.trim().length > 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogHeader>
        <DialogTitle>{t('instance.dialog.title')}</DialogTitle>
        <DialogDescription>{t(`instance.dialog.modeHint.${mode}`)}</DialogDescription>
      </DialogHeader>
      <DialogContent
        showCloseButton={false}
        className="w-[560px] max-w-[calc(100%-2rem)] gap-3.5"
      >
        <div className="flex rounded-segmented border border-border bg-surface-sunken p-0.5">
          {(['create', 'link'] as const).map((m) => (
            <button
              key={m}
              type="button"
              role="radio"
              aria-checked={mode === m}
              onClick={() => {
                setMode(m);
                setPlan(null);
                setError(null);
              }}
              className={cn(
                'flex-1 rounded-control px-2.5 py-[3px] text-small',
                mode === m ? 'bg-accent text-foreground' : 'text-muted-foreground',
              )}
            >
              {t(`instance.dialog.mode.${m}`)}
            </button>
          ))}
        </div>

        <Field label={t('instance.dialog.fieldName')}>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label={t('instance.dialog.fieldName')}
            className="h-8 w-full rounded-control border border-input bg-transparent px-3 text-body outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
          />
        </Field>

        <Field label={t('instance.dialog.fieldRoot')}>
          <input
            value={root}
            onChange={(e) => setRoot(e.target.value)}
            dir="ltr"
            aria-label={t('instance.dialog.fieldRoot')}
            className="h-8 w-full rounded-control border border-input bg-transparent px-3 font-mono text-body outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
          />
        </Field>

        {mode === 'create' && (
          <div className="flex gap-3.5">
            <Field label={t('instance.dialog.fieldVersion')} className="flex-1">
              <input
                value={minecraftVersion}
                onChange={(e) => setMinecraftVersion(e.target.value)}
                dir="ltr"
                aria-label={t('instance.dialog.fieldVersion')}
                className="h-8 w-full rounded-control border border-input bg-transparent px-3 font-mono text-body outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
              />
            </Field>
            <Field label={t('instance.dialog.fieldLoader')} className="flex-1">
              <input
                value={loader}
                onChange={(e) => setLoader(e.target.value)}
                dir="ltr"
                aria-label={t('instance.dialog.fieldLoader')}
                className="h-8 w-full rounded-control border border-input bg-transparent px-3 font-mono text-body outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
              />
            </Field>
          </div>
        )}

        {plan && (
          <div className="flex flex-col gap-1 rounded-control border border-border bg-surface-sunken p-3">
            <span className="text-small font-semibold text-muted-foreground">
              {t('instance.dialog.planTitle')}
            </span>
            <ul className="flex flex-col gap-0.5">
              {plan.directories.map((dir) => (
                <li key={dir} dir="ltr" className="truncate font-mono text-caption text-foreground">
                  {dir}
                </li>
              ))}
            </ul>
          </div>
        )}

        {busy && <LoadingState />}
        {error && <ErrorState error={error} />}

        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
            {t('state.action.cancel')}
          </Button>
          {mode === 'create' ? (
            plan ? (
              <Button onClick={confirm} disabled={busy}>
                {t('instance.dialog.actionCreate')}
              </Button>
            ) : (
              <Button onClick={preview} disabled={!ready || busy}>
                {t('instance.dialog.actionPreview')}
              </Button>
            )
          ) : (
            <Button onClick={link} disabled={!ready || busy}>
              {t('instance.dialog.actionLink')}
            </Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  label,
  className,
  children,
}: {
  label: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <label className={cn('flex flex-col gap-1', className)}>
      <span className="text-small text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}