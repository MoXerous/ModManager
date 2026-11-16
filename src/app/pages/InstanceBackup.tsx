import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { BackupCandidateDto } from '../../bindings/BackupCandidateDto';
import type { BackupResultDto } from '../../bindings/BackupResultDto';
import type { IpcError } from '../../bindings/IpcError';
import type { PendingRestoreDto } from '../../bindings/PendingRestoreDto';
import type { WorldsPreviewDto } from '../../bindings/WorldsPreviewDto';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { ErrorState } from '../../components/state';
import {
  executeInstanceBackup,
  listInstanceBackups,
  openInstanceFolder,
  pendingInstanceRestores,
  previewInstanceWorlds,
  toIpcError,
} from '../../lib/ipc';
import { formatBytes } from '../../lib/format';
import { RestoreDialog } from './RestoreDialog';

/**
 * F-113/F-115/F-118: the backup list, the one-button backup, the read-only
 * world inventory, and the banner a crashed restore leaves behind.
 *
 * The folder is the truth (there is no `backups` table), so the list is read
 * every time this section is shown and after every run — never cached.
 *
 * The `saves` checkbox starts unticked and is sent explicitly. A backup that
 * silently included worlds would be the large one; a restore that silently
 * restored them would be the destructive one. Neither is inferred here, and
 * the Rust side takes the flag as a `bool`, not an `Option`.
 */
export function InstanceBackup({ root }: { root: string }) {
  const { t } = useTranslation();
  const [backups, setBackups] = useState<BackupCandidateDto[] | null>(null);
  const [backupRoot, setBackupRoot] = useState('');
  const [listError, setListError] = useState<IpcError | null>(null);
  const [includeSaves, setIncludeSaves] = useState(false);
  const [creating, setCreating] = useState(false);
  const [phase, setPhase] = useState<string | null>(null);
  const [created, setCreated] = useState<BackupResultDto | null>(null);
  const [createError, setCreateError] = useState<IpcError | null>(null);
  const [restoring, setRestoring] = useState<BackupCandidateDto | null>(null);
  const [worlds, setWorlds] = useState<WorldsPreviewDto | null>(null);
  const [worldsError, setWorldsError] = useState<IpcError | null>(null);
  const [pending, setPending] = useState<PendingRestoreDto[]>([]);

  const loadBackups = useCallback(() => {
    setListError(null);
    listInstanceBackups(root)
      .then((next) => {
        setBackupRoot(next.backupRoot);
        setBackups(next.backups);
      })
      .catch((e) => setListError(toIpcError(e)));
  }, [root]);

  const loadWorlds = useCallback(() => {
    setWorldsError(null);
    previewInstanceWorlds(root)
      .then(setWorlds)
      .catch((e) => setWorldsError(toIpcError(e)));
  }, [root]);

  // The crash surface is advisory: a failed scan renders no banner, never an
  // error panel. A restore that is not pending must not look like a problem.
  const loadPending = useCallback(() => {
    pendingInstanceRestores(root)
      .then(setPending)
      .catch(() => setPending([]));
  }, [root]);

  useEffect(() => {
    loadBackups();
    loadWorlds();
    loadPending();
  }, [loadBackups, loadWorlds, loadPending]);

  const create = () => {
    if (creating) {
      return;
    }
    setCreating(true);
    setPhase(null);
    setCreated(null);
    setCreateError(null);
    executeInstanceBackup(
      { instanceRoot: root, includeSaves },
      (event) => setPhase(event.phase),
    )
      .then((result) => {
        setCreated(result);
        loadBackups();
      })
      .catch((e) => setCreateError(toIpcError(e)))
      .finally(() => {
        setCreating(false);
        setPhase(null);
      });
  };

  return (
    <div className="mx-5 mt-3 flex flex-col gap-3">
      {pending.map((item) => (
        <PendingBanner key={item.safetyDir} item={item} />
      ))}

      <section className="rounded-control border border-border bg-card px-3 py-2.5">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-body font-semibold">{t('backup.title')}</h3>
          <Button
            type="button"
            variant="outline"
            className="ms-auto"
            aria-busy={creating}
            disabled={creating}
            onClick={create}
          >
            {creating ? t('backup.action.creating') : t('backup.action.create')}
          </Button>
        </div>
        <p className="mt-1.5 text-caption text-muted-foreground">{t('backup.body')}</p>

        <div className="mt-2 flex items-start gap-2">
          <Checkbox
            id="backup-include-saves"
            checked={includeSaves}
            disabled={creating}
            onCheckedChange={(next) => setIncludeSaves(next === true)}
          />
          <span className="min-w-0">
            <label htmlFor="backup-include-saves" className="cursor-pointer text-small">
              {t('backup.includeSaves')}
            </label>
            <span className="block text-caption text-muted-foreground">
              {t('backup.includeSavesHint')}
            </span>
          </span>
        </div>

        {phase && (
          <p role="status" aria-live="polite" className="mt-2 text-small text-muted-foreground">
            {t(phase)}
          </p>
        )}
        {createError && (
          <p role="alert" className="mt-2 text-small text-status-issue">
            {t('backup.failed', { reason: t(`error.${createError.code}`) })}
          </p>
        )}
        {created && (
          <div className="mt-2 flex flex-col gap-1">
            <p className="text-small text-status-success">{t('backup.created')}</p>
            <p
              dir="ltr"
              title={created.backupDir}
              className="truncate rounded-control border border-border bg-surface-sunken px-2.5 py-1.5 font-mono text-caption text-muted-foreground"
            >
              {created.backupDir}
            </p>
            {created.skipped.length > 0 && (
              <div>
                <p className="text-caption font-medium text-foreground">
                  {t('backup.skipped.title')}
                </p>
                <ul className="mt-0.5 flex flex-col gap-0.5">
                  {created.skipped.map((skip) => (
                    <li key={`${skip.source}-${skip.path}`} className="text-caption">
                      <span className="text-foreground">
                        {t(`backup.source.${skip.source}`, { defaultValue: skip.source })}
                      </span>{' '}
                      <span className="text-muted-foreground">
                        {t(`backup.skip.${skip.reasonCode}`, { defaultValue: skip.reasonCode })}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        )}

        {listError && <ErrorState error={listError} onRetry={loadBackups} className="mt-2 py-2" />}
        {backups !== null && backups.length === 0 && (
          <p className="mt-2 text-small text-muted-foreground">{t('backup.list.empty')}</p>
        )}
        {backups !== null && backups.length > 0 && (
          <ul className="mt-2 flex flex-col gap-1">
            {backups.map((backup) => (
              <BackupRow
                key={backup.stamp}
                backup={backup}
                onRestore={() => setRestoring(backup)}
              />
            ))}
          </ul>
        )}
      </section>

      <section className="rounded-control border border-border bg-card px-3 py-2.5">
        <h3 className="text-body font-semibold">{t('worlds.title')}</h3>
        <p className="mt-1.5 text-caption text-muted-foreground">{t('worlds.body')}</p>
        {worldsError ? (
          <ErrorState error={worldsError} onRetry={loadWorlds} className="mt-2 py-2" />
        ) : worlds === null ? (
          <p className="mt-2 text-caption text-muted-foreground">{t('worlds.loading')}</p>
        ) : worlds.worlds.length === 0 ? (
          <p className="mt-2 text-small text-muted-foreground">{t(worlds.messageCode)}</p>
        ) : (
          <>
            <table className="mt-2 w-full text-small">
              <thead>
                <tr className="text-caption text-muted-foreground">
                  <th scope="col" className="py-1 text-start font-medium">
                    {t('worlds.column.name')}
                  </th>
                  <th scope="col" className="py-1 text-start font-medium">
                    {t('worlds.column.datapacks')}
                  </th>
                  <th scope="col" className="py-1 text-start font-medium">
                    {t('worlds.column.levelDat')}
                  </th>
                </tr>
              </thead>
              <tbody>
                {worlds.worlds.map((world) => (
                  <tr key={world.name} className="border-t border-border">
                    <td className="max-w-0 py-1.5 pe-2">
                      <span className="block truncate" dir="auto" title={world.name}>
                        {world.name}
                      </span>
                      <span className="block text-caption text-muted-foreground">
                        {t(world.messageCode)}
                      </span>
                    </td>
                    <td dir="ltr" className="w-16 shrink-0 py-1.5 font-mono text-caption">
                      {world.datapackCount}
                    </td>
                    <td className="w-24 shrink-0 py-1.5 text-caption">
                      <span
                        className={
                          world.hasLevelDat ? 'text-status-success' : 'text-status-warning'
                        }
                      >
                        {world.hasLevelDat
                          ? t('worlds.levelDatFound')
                          : t('worlds.levelDatMissing')}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="mt-1.5 text-caption text-muted-foreground">{t(worlds.messageCode)}</p>
          </>
        )}
      </section>

      {restoring && (
        <RestoreDialog
          root={root}
          backupRoot={backupRoot}
          backup={restoring}
          onClose={() => setRestoring(null)}
          onRestored={() => {
            loadBackups();
            loadWorlds();
            loadPending();
          }}
        />
      )}
    </div>
  );
}

function BackupRow({ backup, onRestore }: { backup: BackupCandidateDto; onRestore: () => void }) {
  const { t } = useTranslation();
  const restorable = backup.manifestState === 'present' && backup.instanceMatches;
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5 py-1.5">
      <span className="min-w-0 flex-1">
        <span className="block truncate text-small" dir="ltr" title={backup.createdAt}>
          {backup.createdAt === '' ? backup.stamp : backup.createdAt}
        </span>
        <span className="block text-caption text-muted-foreground">
          {t('backup.row.meta', {
            files: backup.entryCount,
            size: formatBytes(backup.totalBytes),
          })}
          {' · '}
          {backup.includeSaves ? t('backup.badge.saves') : t('backup.badge.noSaves')}
        </span>
      </span>
      {!backup.instanceMatches && backup.manifestState === 'present' && (
        <span className="shrink-0 text-caption text-status-warning">
          {t('backup.badge.otherInstance')}
        </span>
      )}
      <span
        className={
          backup.manifestState === 'present'
            ? 'shrink-0 text-caption text-status-success'
            : 'shrink-0 text-caption text-status-warning'
        }
      >
        {t(`backup.manifest.${backup.manifestState}`)}
      </span>
      <Button type="button" variant="outline" disabled={!restorable} onClick={onRestore}>
        {t('backup.action.restore')}
      </Button>
    </li>
  );
}

/**
 * F-114: a safety folder with no manifest is a restore that died between moving
 * the target aside and copying it back. The banner says where the old files are
 * and opens the folder; it offers no "fix" button, because choosing between the
 * safety copy and a retry is the user's decision, not a guess this app makes.
 */
function PendingBanner({ item }: { item: PendingRestoreDto }) {
  const { t } = useTranslation();
  return (
    <div
      role="status"
      className="rounded-control border border-status-warning/40 bg-card px-3 py-2.5"
    >
      <p className="text-body font-semibold text-status-warning">{t(item.messageCode)}</p>
      {item.groups.length > 0 && (
        <p className="mt-1 text-caption text-muted-foreground">
          {t('restore.pending.body', { groups: item.groups.join(', ') })}
        </p>
      )}
      <p
        dir="ltr"
        className="mt-1 truncate font-mono text-caption text-muted-foreground"
        title={item.safetyDir}
      >
        {item.safetyDir}
      </p>
      <Button
        type="button"
        variant="outline"
        className="mt-2"
        onClick={() => {
          // Opening is the one action that changes nothing; a failed open is
          // not worth an error panel over a banner.
          void openInstanceFolder(item.safetyDir).catch(() => undefined);
        }}
      >
        {t('restore.pending.open')}
      </Button>
    </div>
  );
}
