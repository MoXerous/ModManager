import type { ProjectDto } from '../../bindings/ProjectDto.ts';

/** The 13 columns of the old results table, in the old app's order. */
export const COLUMN_IDS = [
  'select',
  'fav',
  'type',
  'title',
  'slug',
  'status',
  'version',
  'date',
  'env',
  'downloads',
  'followers',
  'categories',
  'url',
] as const;

export type ColumnId = (typeof COLUMN_IDS)[number];

/** Columns a header click sorts by; the two check columns never sort. */
export type SortColumn = Exclude<ColumnId, 'select' | 'fav'>;
export type SortDirection = 'asc' | 'desc';

/** The row identity the selection and the favourites are keyed by. */
export function projectKey(project: ProjectDto): string {
  return `${project.providerId}:${project.id}`;
}

/** The columns whose cells are raw numbers; a header sorts them numerically. */
const NUMERIC: ReadonlySet<string> = new Set(['downloads', 'followers']);
/** The Date column sorts by the raw provider timestamp. */
const DATES: ReadonlySet<string> = new Set(['date']);

function cell(project: ProjectDto, column: SortColumn): string | number {
  switch (column) {
    case 'type':
      return project.projectType;
    case 'title':
      return project.title;
    case 'slug':
      return project.slug ?? '';
    case 'status':
    case 'version':
    case 'date':
    case 'env':
    case 'url':
      return displayCell(project, column);
    case 'categories':
      return project.categories.join(', ');
    case 'downloads':
      return project.downloads;
    case 'followers':
      return project.followers;
  }
}

/**
 * The value a cell shows. Empty means "the provider said nothing", which is
 * what keeps sorting honest: an unknown value never sorts as a real one.
 */
function displayCell(project: ProjectDto, column: SortColumn): string {
  switch (column) {
    case 'status':
      return statusCell(project) ?? '';
    case 'version':
      return project.latestVersion ?? '';
    case 'date':
      return project.published;
    case 'env':
      return envCell(project) ?? '';
    case 'url':
      return project.webUrl ?? '';
    default:
      return '';
  }
}

/**
 * The old table showed `C:{client} / S:{server}`. A provider that states no
 * side (CurseForge) gets "unknown", never a guessed side.
 */
export function envCell(project: ProjectDto): string | null {
  const client = project.clientSide.trim();
  const server = project.serverSide.trim();
  if (client === '' && server === '') {
    return null;
  }
  return `C:${client || '?'} / S:${server || '?'}`;
}

/** The old table sliced the timestamp to its day; the same day, or nothing. */
export function dateCell(project: ProjectDto): string | null {
  const day = project.published.slice(0, 10);
  return day.length === 10 ? day : null;
}

/**
 * The old Status column was the resolver's verdict per row. Browse resolves
 * at plan time, not per row, so the honest status is whether the catalogue
 * handed this row a version at all.
 */
export function statusCell(project: ProjectDto): 'resolved' | 'unknown' {
  return project.latestVersion !== null ? 'resolved' : 'unknown';
}

/**
 * Sort the loaded rows in place-free (a new array) by one column.
 *
 * Sorting is client-side over what is on screen — the old promise "sort and
 * select without reloading". A missing value sorts last in both directions:
 * it is not a small number and it is not an early date.
 */
export function sortRows(
  rows: readonly ProjectDto[],
  column: SortColumn,
  direction: SortDirection,
): ProjectDto[] {
  const factor = direction === 'asc' ? 1 : -1;
  return [...rows].sort((a, b) => {
    const left = cell(a, column);
    const right = cell(b, column);
    const leftEmpty = left === '' || left === null;
    const rightEmpty = right === '' || right === null;
    if (leftEmpty && rightEmpty) {
      return 0;
    }
    if (leftEmpty) {
      return 1;
    }
    if (rightEmpty) {
      return -1;
    }
    if (typeof left === 'number' && typeof right === 'number') {
      return factor * (left - right);
    }
    if (DATES.has(column)) {
      // ISO-8601 timestamps from one provider compare as text.
      return factor * String(left).localeCompare(String(right));
    }
    if (NUMERIC.has(column)) {
      return factor * (Number(left) - Number(right));
    }
    return factor * String(left).localeCompare(String(right));
  });
}
