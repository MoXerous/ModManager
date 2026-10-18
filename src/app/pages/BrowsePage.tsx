import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { ProjectDto } from '../../bindings/ProjectDto';
import type { SearchPageDto } from '../../bindings/SearchPageDto';
import type { IpcError } from '../../bindings/IpcError';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, LoadingState } from '../../components/state';
import { executeInstall, searchProjects, toIpcError } from '../../lib/ipc';

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
        { instanceRoot: '', slugs: [slug], gameVersion: GAME_VERSION, loader: LOADER },
        () => {},
      )
        .then(onInstalled)
        .catch((e) => setError(toIpcError(e)))
        .finally(() => setInstalling(null));
    },
    [onInstalled],
  );

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
          <Card key={p.id} project={p} installing={installing} onInstall={install} />
        ))}
      </div>
    </div>
  );
}

function Card({
  project,
  installing,
  onInstall,
}: {
  project: ProjectDto;
  installing: string | null;
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
          disabled={installing !== null}
          onClick={() => onInstall(slug)}
        >
          {t('install.start')}
        </Button>
      </div>
    </article>
  );
}
