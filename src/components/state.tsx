import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Inbox, LoaderCircle, TriangleAlert } from 'lucide-react';

import type { ErrorCode } from '../bindings/ErrorCode';
import type { IpcError } from '../bindings/IpcError';
import { cn } from 'cn';

// State patterns shared by every screen. The design package leaves loading,
// empty and error boards undesigned; these follow its vocabulary (card
// surface, 1.75 icon stroke, muted body, status colours) so the real screens
// inherit them instead of inventing per-page markup.

export function LoadingState({ className }: { className?: string }) {
  const { t } = useTranslation();
  return (
    <div
      role="status"
      aria-live="polite"
      className={cn('flex items-center gap-2 text-muted-foreground', className)}
    >
      <LoaderCircle aria-hidden className="size-[18px] shrink-0 animate-spin" strokeWidth={1.75} />
      <span>{t('state.loading')}</span>
    </div>
  );
}

export interface EmptyStateProps {
  title: string;
  body: string;
  action?: ReactNode;
  className?: string;
}

export function EmptyState({ title, body, action, className }: EmptyStateProps) {
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center gap-2 py-16 text-center',
        className,
      )}
    >
      <Inbox aria-hidden className="mb-1 size-7 text-muted-foreground" strokeWidth={1.75} />
      <p className="font-semibold text-foreground">{title}</p>
      <p className="max-w-sm text-small text-muted-foreground">{body}</p>
      {action}
    </div>
  );
}

export interface ErrorStateProps {
  error: IpcError;
  onRetry?: () => void;
  className?: string;
}

/**
 * The UI renders the catalog entry for `code`, never `message`: `message` is
 * core's own text and is shown verbatim only under Details (AGENTS.md 5.3).
 */
export function ErrorState({ error, onRetry, className }: ErrorStateProps) {
  const { t } = useTranslation();
  return (
    <div
      role="alert"
      className={cn(
        'flex max-w-xl flex-col gap-3 rounded-card border border-border bg-card p-4',
        className,
      )}
    >
      <div className="flex items-start gap-2.5">
        <TriangleAlert
          aria-hidden
          className="mt-px size-[18px] shrink-0 text-status-issue"
          strokeWidth={1.75}
        />
        <div className="flex flex-col gap-0.5">
          <p className="font-semibold text-foreground">{t('state.error.title')}</p>
          <p className="text-small text-muted-foreground">{t(`error.${error.code satisfies ErrorCode}`)}</p>
        </div>
      </div>
      {onRetry && (
        <div>
          <button
            type="button"
            onClick={onRetry}
            className="h-8 rounded-control border border-border bg-accent px-3 font-medium text-accent-foreground hover:bg-secondary"
          >
            {t('state.action.retry')}
          </button>
        </div>
      )}
      <details className="text-small text-muted-foreground">
        <summary className="cursor-pointer">{t('state.action.details')}</summary>
        <p className="mt-1 font-mono text-caption" dir="auto">
          {error.message}
        </p>
      </details>
    </div>
  );
}