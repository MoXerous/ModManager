import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { IpcError } from '../../bindings/IpcError';
import type { ProjectDto } from '../../bindings/ProjectDto';
import type { ProjectVersionDto } from '../../bindings/ProjectVersionDto';
import { Button } from '@/components/ui/button';
import { Segmented } from '@/components/ui/Segmented';
import { ErrorState } from '../../components/state';
import { formatNumber } from '../../lib/format';
import { listProjectVersions, toIpcError } from '../../lib/ipc';
import { dateCell, envCell } from './columns';
import { removeFavourite, removeSelected, useBrowseSelection } from './selectionStore';

/** The inspector's three tabs, the old app's right-hand rail. */
const TABS = ['details', 'selected', 'favourites'] as const;
type Tab = (typeof TABS)[number];

interface Props {
  /** The row the user opened; null shows the empty hint. */
  project: ProjectDto | null;
  gameVersion: string;
  loader: string;
  /** The same gate the row's Install button had. */
  canInstall: boolean;
  onInstall: (project: ProjectDto) => void;
}

/**
 * F-164: the project inspector — a details pane, the project's versions
 * behind a "Show versions" action, and the Selected / Favourites tabs. The
 * old app kept all three on its right-hand rail; installs are confirmed in
 * the plan dialog, never from here.
 */
export function Inspector({ project, gameVersion, loader, canInstall, onInstall }: Props) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>('details');

  return (
    <aside className="flex w-72 shrink-0 flex-col gap-3 border-s border-border ps-4" aria-label={t('browse.inspector.aria')}>
      <Segmented<Tab>
        label={t('browse.inspector.tabs')}
        value={tab}
        options={TABS.map((id) => ({ value: id, label: t(`browse.inspector.tab.${id}`) }))}
        onChange={setTab}
      />
      {tab === 'details' &&
        (project ? (
          <DetailsPane
            project={project}
            gameVersion={gameVersion}
            loader={loader}
            canInstall={canInstall}
            onInstall={onInstall}
          />
        ) : (
          <p className="text-small text-muted-foreground">
            {t('browse.inspector.noProject')}
          </p>
        ))}
      {tab === 'selected' && <ProjectList kind="selected" />}
      {tab === 'favourites' && <ProjectList kind="favourites" />}
    </aside>
  );
}

function DetailsPane({
  project,
  gameVersion,
  loader,
  canInstall,
  onInstall,
}: {
  project: ProjectDto;
  gameVersion: string;
  loader: string;
  canInstall: boolean;
  onInstall: (project: ProjectDto) => void;
}) {
  const { t } = useTranslation();
  const [versions, setVersions] = useState<{ project: string; rows: ProjectVersionDto[] } | null>(
    null,
  );
  const [loadingVersions, setLoadingVersions] = useState(false);
  const [versionsError, setVersionsError] = useState<IpcError | null>(null);

  /**
   * F-164: "Show versions" asks the provider what exists, read-only, narrowed
   * by the target the screen already knows. A stale answer can outlive the
   * project it was asked for, so the result is dropped when the pane has
   * moved to another project by the time it lands.
   */
  const showVersions = useCallback(() => {
    if (loadingVersions) {
      return;
    }
    const asked = project.id;
    setLoadingVersions(true);
    setVersionsError(null);
    listProjectVersions(project.providerId, project.id, gameVersion, loader)
      .then((rows) => setVersions({ project: asked, rows }))
      .catch((e) => setVersionsError(toIpcError(e)))
      .finally(() => setLoadingVersions(false));
  }, [gameVersion, loader, loadingVersions, project]);

  const env = envCell(project);
  const date = dateCell(project);

  const meta: { label: string; value: string }[] = [
    { label: t('browse.col.type'), value: project.projectType },
    { label: t('browse.col.categories'), value: project.categories.join(', ') },
    { label: t('browse.col.date'), value: date ?? t('browse.env.unknown') },
    { label: t('browse.col.env'), value: env ?? t('browse.env.unknown') },
    { label: t('browse.col.downloads'), value: formatNumber(project.downloads) },
    { label: t('browse.col.followers'), value: formatNumber(project.followers) },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-auto">
      {project.iconUrl ? (
        <img
          src={project.iconUrl}
          alt=""
          aria-hidden
          className="size-10 rounded-chip object-cover"
        />
      ) : null}
      <p className="text-body font-medium">{project.title}</p>
      <p className="text-caption text-muted-foreground">{project.author}</p>
      <p className="line-clamp-4 text-small text-muted-foreground">{project.description}</p>
      <dl className="flex flex-col gap-1 text-small">
        {meta.map((row) => (
          <div key={row.label} className="flex items-baseline justify-between gap-2">
            <dt className="shrink-0 text-caption text-muted-foreground">{row.label}</dt>
            <dd className="truncate text-end">{row.value}</dd>
          </div>
        ))}
      </dl>
      <Button type="button" size="sm" disabled={!canInstall} onClick={() => onInstall(project)}>
        {t('install.start')}
      </Button>
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={showVersions}
        disabled={loadingVersions}
      >
        {t('browse.inspector.showVersions')}
      </Button>
      {versionsError && <ErrorState error={versionsError} onRetry={showVersions} />}
      {loadingVersions && (
        <p role="status" className="text-small text-muted-foreground">
          {t('browse.inspector.versionsLoading')}
        </p>
      )}
      {versions !== null && !loadingVersions && versions.project === project.id && (
        <VersionsList versions={versions.rows} />
      )}
    </div>
  );
}

function VersionsList({ versions }: { versions: ProjectVersionDto[] }) {
  const { t } = useTranslation();
  if (versions.length === 0) {
    return (
      <p className="text-small text-muted-foreground">{t('browse.inspector.versionsEmpty')}</p>
    );
  }
  return (
    <ul className="flex flex-col gap-1">
      <li className="text-caption text-muted-foreground">
        {t('browse.inspector.versionsCount', { count: versions.length })}
      </li>
      {versions.map((version) => (
        <li
          key={version.id}
          className="rounded-control border border-border px-2 py-1 text-caption"
        >
          <span className="font-medium">{version.versionNumber}</span>
          <span className="text-muted-foreground">
            {' · '}
            {version.datePublished.slice(0, 10)}
            {' · '}
            {version.loaders.join(', ')}
          </span>
          {version.filename && (
            <span className="block truncate text-muted-foreground">{version.filename}</span>
          )}
        </li>
      ))}
    </ul>
  );
}

/** The Selected and Favourites tabs: the lists that survive a new search. */
function ProjectList({ kind }: { kind: 'selected' | 'favourites' }) {
  const { t } = useTranslation();
  const selection = useBrowseSelection();
  const entries = kind === 'selected' ? selection.selected : selection.favourites;
  const remove = (key: string) =>
    kind === 'selected' ? removeSelected(key) : removeFavourite(key);
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <p className="text-small text-muted-foreground">
        {t(`browse.inspector.${kind}Count`, { count: entries.length })}
      </p>
      {entries.length === 0 ? (
        <p className="text-small text-muted-foreground">{t(`browse.inspector.${kind}Empty`)}</p>
      ) : (
        <ul className="flex min-h-0 flex-1 flex-col gap-1 overflow-auto">
          {entries.map((entry) => (
            <li
              key={entry.key}
              className="flex items-center justify-between gap-2 rounded-control border border-border px-2 py-1"
            >
              <span className="min-w-0 truncate text-small">
                {entry.project.title}
                <span className="block truncate text-caption text-muted-foreground">
                  {t(`provider.${entry.project.providerId}`)}
                </span>
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => remove(entry.key)}
                aria-label={t(`browse.inspector.remove${kind === 'selected' ? 'Selected' : 'Favourite'}`, {
                  name: entry.project.title,
                })}
              >
                {t('browse.inspector.remove')}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
