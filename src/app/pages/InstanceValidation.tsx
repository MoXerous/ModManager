import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { IpcError } from '../../bindings/IpcError';
import type { ValidationItemDto } from '../../bindings/ValidationItemDto';
import type { ValidationReportDto } from '../../bindings/ValidationReportDto';
import { Button } from '@/components/ui/button';
import { cn } from 'cn';
import { toIpcError, validateInstance } from '../../lib/ipc';

/** The ladder, worst first: that is the order the user reads the report in. */
const SEVERITIES = ['error', 'warning', 'info', 'ok'] as const;

function severityClass(severity: string): string {
  switch (severity) {
    case 'error':
      return 'text-status-issue';
    case 'warning':
      return 'text-status-warning';
    case 'info':
      return 'text-muted-foreground';
    default:
      return 'text-status-success';
  }
}

function ItemLine({ item }: { item: ValidationItemDto }) {
  const { t } = useTranslation();
  return (
    <li className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
      <span className={cn('shrink-0 text-caption font-medium', severityClass(item.severity))}>
        {t(`details.validation.severity.${item.severity}`)}
      </span>
      <span className="text-caption">{t(`details.validation.check.${item.check}`)}</span>
      {/* The value and the path are technical tokens inside an Arabic row; a
          bare `{{value}}` would reverse them in RTL, so each is isolated. */}
      {item.value && (
        <bdi dir="ltr" className="min-w-0 break-all font-mono text-caption text-muted-foreground">
          {item.value}
        </bdi>
      )}
      {item.path && (
        <bdi dir="ltr" className="min-w-0 break-all font-mono text-caption text-muted-foreground">
          {item.path}
        </bdi>
      )}
      {item.count !== null && (
        <span dir="ltr" className="shrink-0 font-mono text-caption text-muted-foreground">
          {t('details.validation.times', { n: item.count })}
          {item.total !== null ? ` / ${item.total}` : ''}
        </span>
      )}
    </li>
  );
}

/**
 * F-095: the read-only validation report.
 *
 * There is no repair button on purpose: the old app paired the report with a
 * repair action, but features.md keeps this slice at "read-only", so the card
 * says what it found and nothing more. Every row names the concrete datum the
 * check read; a check that could not read its input says so instead of passing.
 */
export function InstanceValidation({ instanceId }: { instanceId: string }) {
  const { t } = useTranslation();
  const [report, setReport] = useState<ValidationReportDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [busy, setBusy] = useState(false);

  const run = () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    validateInstance(instanceId)
      .then(setReport)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setBusy(false));
  };

  const counts = (severity: string) =>
    report?.items.filter((item) => item.severity === severity).length ?? 0;
  const problems = report
    ? report.items.filter((item) => item.severity !== 'ok')
    : [];
  const passed = report ? report.items.filter((item) => item.severity === 'ok') : [];

  return (
    <div className="mx-5 mt-3 rounded-control border border-border bg-card px-3 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-body font-semibold">{t('details.validation.title')}</span>
        <span className="text-caption text-muted-foreground">{t('details.validation.readOnly')}</span>
        <Button
          type="button"
          variant="outline"
          className="ms-auto"
          aria-busy={busy}
          disabled={busy}
          onClick={run}
        >
          {busy ? t('details.validation.checking') : t('details.validation.check')}
        </Button>
      </div>

      {error && (
        <p role="alert" className="mt-2 text-small text-status-issue">
          {t('details.validation.failed', { reason: t(`error.${error.code}`) })}
        </p>
      )}

      {report && (
        <div className="mt-2" aria-live="polite">
          <p className={cn('text-small font-medium', severityClass(report.overall))}>
            {t(`details.validation.overall.${report.overall}`)}
          </p>
          <p className="mt-0.5 text-caption text-muted-foreground">
            {t('details.validation.summary', {
              errors: counts('error'),
              warnings: counts('warning'),
              info: counts('info'),
              ok: counts('ok'),
            })}
          </p>

          {problems.length > 0 && (
            <ul className="mt-2 flex flex-col gap-1">
              {SEVERITIES.filter((severity) => severity !== 'ok').flatMap((severity) =>
                problems
                  .filter((item) => item.severity === severity)
                  .map((item) => <ItemLine key={item.check} item={item} />),
              )}
            </ul>
          )}

          {passed.length > 0 && (
            <details className="mt-2">
              <summary className="cursor-pointer text-caption text-muted-foreground">
                {t('details.validation.passed', { count: passed.length })}
              </summary>
              <ul className="mt-1 flex flex-col gap-1">
                {passed.map((item) => (
                  <ItemLine key={item.check} item={item} />
                ))}
              </ul>
            </details>
          )}
        </div>
      )}
    </div>
  );
}
