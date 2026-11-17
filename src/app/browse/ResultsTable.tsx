import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { ProjectDto } from '../../bindings/ProjectDto';
import { Checkbox } from '@/components/ui/checkbox';
import { formatNumber } from '../../lib/format';
import {
  COLUMN_IDS,
  dateCell,
  envCell,
  projectKey,
  sortRows,
  statusCell,
  type ColumnId,
  type SortColumn,
  type SortDirection,
} from './columns';
import { useBrowseSelection, toggleFavourite, toggleSelected } from './selectionStore';
import { useVirtualRows } from './useVirtualRows';

/** The old table's column widths, unlabelled px: the grid that both the
 *  header and every row must follow, plus its total for the min-width. */
const WIDTHS: Record<ColumnId, number> = {
  select: 44,
  fav: 36,
  type: 72,
  title: 180,
  slug: 100,
  status: 84,
  version: 100,
  date: 78,
  env: 84,
  downloads: 76,
  followers: 66,
  categories: 130,
  url: 170,
};

const GRID = `grid-cols-[${COLUMN_IDS.map((id) => `${WIDTHS[id]}px`).join(' ')}]`;
const MIN_WIDTH = COLUMN_IDS.reduce((sum, id) => sum + WIDTHS[id], 0);

/** The sticky header's own height; the row window starts under it. */
const HEADER_HEIGHT = 32;
const ROW_HEIGHT = 40;

interface Props {
  rows: ProjectDto[];
  /** A row the user clicked to inspect. */
  onOpen: (project: ProjectDto) => void;
}

/**
 * F-165: the results table — the old app's thirteen columns, sortable
 * without reloading, with a per-row Select and a per-project favourite, and
 * only the visible rows mounted (the old table was a `QTableView`).
 */
export function ResultsTable({ rows, onOpen }: Props) {
  const { t } = useTranslation();
  const [sort, setSort] = useState<{ column: SortColumn; direction: SortDirection } | null>(null);
  const virtual = useVirtualRows(rows.length, ROW_HEIGHT, 8, HEADER_HEIGHT);

  const sorted = useMemo(
    () => (sort ? sortRows(rows, sort.column, sort.direction) : rows),
    [rows, sort],
  );
  const visible = sorted.slice(virtual.start, virtual.end);
  const selection = useBrowseSelection();
  const selectedKeys = new Set(selection.selected.map((entry) => entry.key));
  const favouriteKeys = new Set(selection.favourites.map((entry) => entry.key));

  /** The old table: sorting is a header click, client-side, no reload. */
  const clickSort = (column: ColumnId) => {
    if (column === 'select' || column === 'fav') {
      return;
    }
    setSort((prev) => {
      if (prev && prev.column === column) {
        return prev.direction === 'asc' ? { column, direction: 'desc' } : null;
      }
      return { column, direction: 'asc' };
    });
  };

  return (
    <div
      ref={virtual.ref}
      onScroll={virtual.onScroll}
      className="min-h-0 flex-1 overflow-auto border-t border-border"
    >
      <div style={{ minWidth: MIN_WIDTH }} role="grid" aria-rowcount={rows.length}>
        <div
          role="row"
          className={`${GRID} sticky top-0 z-10 h-8 items-center border-b border-border bg-surface-sunken text-caption font-medium text-muted-foreground`}
        >
          {COLUMN_IDS.map((id) => (
            <div
              key={id}
              role="columnheader"
              aria-sort={
                sort?.column === id
                  ? sort.direction === 'asc'
                    ? 'ascending'
                    : 'descending'
                  : undefined
              }
              className="px-2"
            >
              {id === 'select' || id === 'fav' ? (
                <span aria-hidden>{t(`browse.col.${id}`)}</span>
              ) : (
                <button
                  type="button"
                  onClick={() => clickSort(id)}
                  aria-label={t('browse.sortBy', { column: t(`browse.col.${id}`) })}
                  className="flex w-full items-center gap-1 text-start hover:text-foreground"
                >
                  {t(`browse.col.${id}`)}
                  {sort?.column === id ? <SortMark direction={sort.direction} /> : null}
                </button>
              )}
            </div>
          ))}
        </div>
        <div style={{ height: virtual.topPad }} aria-hidden />
        {visible.map((project) => (
          <Row
            key={projectKey(project)}
            project={project}
            selected={selectedKeys.has(projectKey(project))}
            favourite={favouriteKeys.has(projectKey(project))}
            onOpen={onOpen}
          />
        ))}
        <div style={{ height: virtual.bottomPad }} aria-hidden />
      </div>
    </div>
  );
}

function SortMark({ direction }: { direction: SortDirection }) {
  // A text mark instead of an icon: two columns of direction glyphs stay
  // legible in RTL and LTR alike, and lucide has no direction-neutral pair.
  return <span aria-hidden className="text-[10px]">{direction === 'asc' ? '▲' : '▼'}</span>;
}

function Row({
  project,
  selected,
  favourite,
  onOpen,
}: {
  project: ProjectDto;
  selected: boolean;
  favourite: boolean;
  onOpen: (project: ProjectDto) => void;
}) {
  const { t } = useTranslation();
  const resolved = project.latestVersion !== null;
  const env = envCell(project);
  const date = dateCell(project);
  return (
    <div
      role="row"
      // The row is the old table's row click: it opens the inspector, never
      // an install by itself.
      onClick={() => onOpen(project)}
      className={`${GRID} h-10 cursor-pointer items-center border-b border-border text-body hover:bg-surface-sunken`}
    >
      <div role="gridcell" className="px-2" onClick={(e) => e.stopPropagation()}>
        {/* The old rule: a row with no resolved version cannot be selected,
            because there would be nothing to download for it. */}
        <Checkbox
          checked={selected}
          disabled={!resolved}
          onCheckedChange={() => toggleSelected(project)}
          aria-label={t('details.selectRow', { name: project.title })}
        />
      </div>
      <div role="gridcell" className="px-2" onClick={(e) => e.stopPropagation()}>
        <Checkbox
          checked={favourite}
          onCheckedChange={() => toggleFavourite(project)}
          aria-label={t('browse.favToggle', { name: project.title })}
        />
      </div>
      <div role="gridcell" className="truncate px-2 text-muted-foreground">
        {project.projectType}
      </div>
      <div role="gridcell" className="truncate px-2 font-medium">
        {project.title}
      </div>
      <div role="gridcell" className="truncate px-2 text-muted-foreground">
        {project.slug ?? ''}
      </div>
      <div role="gridcell" className="truncate px-2 text-muted-foreground">
        {t(`browse.status.${statusCell(project)}`)}
      </div>
      <div role="gridcell" className="truncate px-2">
        {project.latestVersion ?? ''}
      </div>
      <div role="gridcell" className="truncate px-2 text-muted-foreground">
        {date ?? ''}
      </div>
      <div role="gridcell" className="truncate px-2 text-muted-foreground">
        {env ?? t('browse.env.unknown')}
      </div>
      <div role="gridcell" className="truncate px-2 text-end tabular-nums">
        {formatNumber(project.downloads)}
      </div>
      <div role="gridcell" className="truncate px-2 text-end tabular-nums">
        {formatNumber(project.followers)}
      </div>
      <div role="gridcell" className="truncate px-2 text-muted-foreground">
        {project.categories.join(', ')}
      </div>
      <div role="gridcell" className="truncate px-2 text-caption text-muted-foreground">
        {project.webUrl ?? ''}
      </div>
    </div>
  );
}
