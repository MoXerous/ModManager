import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { InstanceSummaryDto } from '../../bindings/InstanceSummaryDto';
import type { ProjectDto } from '../../bindings/ProjectDto';
import type { SearchPageDto } from '../../bindings/SearchPageDto';
import type { IpcError } from '../../bindings/IpcError';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, LoadingState } from '../../components/state';
import { executeInstall, listInstances, searchProjects, toIpcError } from '../../lib/ipc';

// shortcut: game version and loader are pinned, matching the queue. Ceiling:
// the search and the install are for Fabric 1.21.1 only. Upgrade path: a filter
// bar here, and then passing both to `execute_install` instead of letting it
// re-resolve with its own defaults.
const GAME_VERSION = '1.21.1';
const LOADER = 'fabric';

interface Props {
  /** Where to send the user once a download starts: the queue is the only
   *  thing that can show progress. */
  onInstalled: () => void;
}

export function BrowsePage({ onInstalled }: Props) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [page, setPage] = useState<SearchPageDto | null>(null);
  const [error, setError] = useState<IpcError | null>(null);
  const [searching, setSearching] = useState(false);
  const [installing, setInstalling] = useState<string | null>(null);
  const [instances, setInstances] = useState<InstanceSummaryDto[]>([]);
  const [target, setTarget] = useState('');

  // The install has to land somewhere, and execute_install refuses an empty
  // root. The instances are already in the database; asking for one here is
  // smaller than teaching the queue to accept a path.
  //
  // shortcut: only registered instances are offered, with no way to type a
  // folder. Ceiling: an install cannot target a folder that is not an instance
  // yet. Upgrade path: share the New Instance dialog's folder picker once that
  // picker is worth extracting.
  useEffect(() => {
    // Not `void`-and-forget: an IPC failure here is an unhandled rejection in the
    // console and a select that silently stays empty, which reads as "you have
    // no instances" rather than "we could not ask".
    void listInstances()
      .then((rows) => {
        setInstances(rows);
        setTarget(rows[0]?.id ?? '');
      })
      .catch((e) => setError(toIpcError(e)));
  }, []);

  const targetPath = instances.find((i) => i.id === target)?.path ?? '';
  // Nothing to install into is not a state the button can be pressed in.
  const canInstall = targetPath.length > 0;

  const search = useCallback(() => {
    if (query.trim().length === 0) {
      return;
    }
    setSearching(true);
    setError(null);
    searchProjects({
      query: query.trim(),
      gameVersion: GAME_VERSION,
      loader: LOADER,
      limit: 20,
    })
      .then(setPage)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setSearching(false));
  }, [query]);

  const install = useCallback(
    (slug: string) => {
      setInstalling(slug);
      setError(null);
      executeInstall(
        { instanceRoot: targetPath, slugs: [slug], gameVersion: GAME_VERSION, loader: LOADER },
        () => {},
      )
        .then(onInstalled)
        .catch((e) => setError(toIpcError(e)))
        .finally(() => setInstalling(null));
    },
    // `targetPath` is read inside, so it belongs in the deps. Without it this
    // closure keeps the first render's empty string and sends an empty root,
    // which is the exact failure this commit exists to remove.
    [onInstalled, targetPath],
  );

  // A name, not a path: the list already shows where each instance is, and a
  // path in a dropdown is unreadable and would need truncation.
  const header = (
    <div className="px-5 pt-4">
      <h1 className="text-page-title font-semibold">{t('nav.browse')}</h1>
      <div className="mt-2.5 flex items-center gap-2">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              search();
            }
          }}
          placeholder={t('browse.searchPlaceholder')}
          aria-label={t('browse.searchPlaceholder')}
          className="h-8 min-w-0 flex-1 rounded-control border border-input bg-surface-sunken px-2.5 text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
        />
        <Button type="button" onClick={search} disabled={searching}>
          {t('browse.actionSearch')}
        </Button>
      </div>
      {instances.length > 0 && (
        <label className="mt-2.5 flex items-center gap-2 text-small text-muted-foreground">
          {t('browse.target')}
          <select
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            aria-label={t('browse.target')}
            className="h-8 rounded-control border border-input bg-surface-sunken px-2 text-body text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
          >
            {instances.map((i) => (
              <option key={i.id} value={i.id}>
                {i.name}
              </option>
            ))}
          </select>
        </label>
      )}
    </div>
  );

  if (error) {
    return (
      <div className="flex flex-col">
        {header}
        <div className="px-5 pt-4">
          <ErrorState error={error} onRetry={search} />
        </div>
      </div>
    );
  }

  if (searching && !page) {
    return (
      <div className="flex flex-col">
        {header}
        <LoadingState className="px-5 pt-6" />
      </div>
    );
  }

  if (!page) {
    return (
      <div className="flex flex-col">
        {header}
        <EmptyState title={t('page.browse.emptyTitle')} body={t('page.browse.emptyBody')} />
      </div>
    );
  }

  return (
    <div className="flex flex-col">
      {header}
      <p className="px-5 pt-3 text-small text-muted-foreground">
        {t('browse.results', { count: page.total })}
      </p>
      <div className="grid grid-cols-1 gap-3 px-5 py-4 sm:grid-cols-2 lg:grid-cols-3">
        {page.hits.map((p) => (
          <Card
            key={p.id}
            project={p}
            installing={installing}
            canInstall={canInstall}
            onInstall={install}
          />
        ))}
      </div>
    </div>
  );
}

function Card({
  project,
  installing,
  canInstall,
  onInstall,
}: {
  project: ProjectDto;
  installing: string | null;
  canInstall: boolean;
  onInstall: (slug: string) => void;
}) {
  const { t } = useTranslation();
  const slug = project.slug ?? project.id;
  return (
    <article className="flex flex-col gap-2 rounded-card border border-border bg-card p-3.5">
      <div className="flex items-start gap-2.5">
        {project.iconUrl ? (
          <img
            src={project.iconUrl}
            alt=""
            aria-hidden
            className="size-9 shrink-0 rounded-chip object-cover"
          />
        ) : null}
        <div className="min-w-0 flex-1">
          <p className="truncate text-body font-medium">{project.title}</p>
          <p className="truncate text-caption text-muted-foreground">{project.author}</p>
        </div>
      </div>
      <p className="line-clamp-2 text-small text-muted-foreground">{project.description}</p>
      <div className="mt-auto flex items-center gap-2 pt-1">
        <span className="rounded-chip border border-border px-[7px] py-px text-caption text-muted-foreground">
          {t('browse.downloads', { count: project.downloads })}
        </span>
        <span className="rounded-chip border border-border px-[7px] py-px text-caption text-muted-foreground">
          {project.projectType}
        </span>
        <Button
          type="button"
          variant="outline"
          className="ms-auto"
          disabled={installing !== null || !canInstall}
          onClick={() => onInstall(slug)}
        >
          {t('install.start')}
        </Button>
      </div>
    </article>
  );
}
