import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { InstanceSummaryDto } from '../../bindings/InstanceSummaryDto';
import type { ProjectDto } from '../../bindings/ProjectDto';
import type { SearchPageDto } from '../../bindings/SearchPageDto';
import type { IpcError } from '../../bindings/IpcError';
import { Combobox } from '@/components/ui/Combobox';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, LoadingState } from '../../components/state';
import { executeInstall, listInstances, searchProjects, toIpcError } from '../../lib/ipc';
import { GAME_VERSIONS, LOADERS, setGameTarget, useGameTarget } from '../gameTarget';

interface Props {
  /** Where to send the user once a download starts: the queue is the only
   *  thing that can show progress. */
  onInstalled: () => void;
}

export function BrowsePage({ onInstalled }: Props) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  // The version and the loader are shared with the queue: what is filtered
  // here is what gets installed there.
  const { gameVersion, loader } = useGameTarget();
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
  // Nothing to install into is not a state the button can be pressed in. An
  // empty version or loader is not either: core reads an empty version as
  // "any version" and installs the newest build of the fallback loader,
  // which may not match the instance at all.
  const canInstall =
    targetPath.length > 0 &&
    gameVersion.trim().length > 0 &&
    loader.trim().length > 0;

  const search = useCallback(() => {
    const text = query.trim();
    if (text.length === 0) {
      return;
    }
    setSearching(true);
    setError(null);
    searchProjects({
      query: text,
      // The facets are identifiers, not prose: an empty field means "no
      // filter", and "Fabric" typed with a capital is still the fabric facet.
      gameVersion: gameVersion.trim() || undefined,
      loader: loader.trim().toLowerCase() || undefined,
      limit: 20,
    })
      .then(setPage)
      .catch((e) => setError(toIpcError(e)))
      .finally(() => setSearching(false));
  }, [gameVersion, loader, query]);

  const install = useCallback(
    (slug: string) => {
      setInstalling(slug);
      setError(null);
      executeInstall(
        {
          instanceRoot: targetPath,
          slugs: [slug],
          gameVersion: gameVersion.trim(),
          loader: loader.trim().toLowerCase(),
        },
        () => {},
      )
        .then(onInstalled)
        .catch((e) => setError(toIpcError(e)))
        .finally(() => setInstalling(null));
    },
    // `targetPath` is read inside, so it belongs in the deps. Without it this
    // closure keeps the first render's empty string and sends an empty root,
    // which is the exact failure this commit exists to remove.
    [gameVersion, loader, onInstalled, targetPath],
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
      <div className="mt-2.5 flex flex-wrap items-center gap-x-4 gap-y-2 text-small text-muted-foreground">
        <div className="flex items-center gap-2">
          <label htmlFor="browse-version">{t('browse.fieldVersion')}</label>
          <Combobox
            id="browse-version"
            label={t('browse.fieldVersion')}
            value={gameVersion}
            onValueChange={(next) => setGameTarget({ gameVersion: next })}
            options={GAME_VERSIONS}
            className="w-32"
          />
        </div>
        <div className="flex items-center gap-2">
          <label htmlFor="browse-loader">{t('browse.fieldLoader')}</label>
          <Combobox
            id="browse-loader"
            label={t('browse.fieldLoader')}
            value={loader}
            onValueChange={(next) => setGameTarget({ loader: next })}
            options={LOADERS}
            className="w-32"
          />
        </div>
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
