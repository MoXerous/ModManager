/**
 * F-166: the paging rules of the results table.
 *
 * The old app preloaded the next page when the user neared the end of the
 * results and debounced that trigger to ~120 ms, so a fast scroll did not
 * start a storm of requests. Tauri `invoke()` has no AbortSignal, so the
 * cancellation of the old app is replaced by a generation counter: a response
 * whose request was superseded is dropped before it reaches the screen.
 *
 * The cache is a module singleton (like the selection store): the page state
 * dies on remount, but "returning to the same query key" must not refetch
 * from scratch. A manual refresh clears it.
 */

/** The old app's debounce, in ms. */
export const PAGING_DEBOUNCE_MS = 120;

/**
 * How many rows before the end of the loaded list count as "near the end".
 * The old app preloaded a little before the edge; one screenful of overscan
 * already renders ahead, so this is small.
 */
const NEAR_END_ROWS = 12;

/**
 * F-166: whether the visible window is near the end of the loaded rows.
 *
 * `end` is the virtualiser's exclusive visible end; `count` is how many rows
 * are loaded so far.
 */
export function nearEnd(end: number, count: number): boolean {
  return count > 0 && end >= count - NEAR_END_ROWS;
}

/**
 * F-166: the request generation. Every request takes a number when it starts;
 * only the newest number is current, so a late answer to a replaced request
 * can never render.
 */
export class RequestGate {
  private current = 0;

  next(): number {
    this.current += 1;
    return this.current;
  }

  isCurrent(generation: number): boolean {
    return generation === this.current;
  }

  /**
   * F-166: end every request in flight without sending a new one.
   *
   * A reset that arrives without a request — a provider switch, a facet
   * change, a cache hit — must still close the window: an older request that
   * lands after the reset sees a current generation and would render over
   * the new rows. Taking a new generation is the invalidation.
   */
  invalidate(): void {
    this.current += 1;
  }
}

/** One page's worth of state, as the screen keeps it. */
export interface CachedPage {
  hits: import('../../bindings/ProjectDto').ProjectDto[];
  total: number;
  loaded: number;
}

/** The search terms and facets a result list is an answer to. */
export interface QueryKeyInput {
  query: string;
  provider: string;
  gameVersion?: string;
  loader?: string;
  projectType?: string;
}

/** A canonical key for the question: terms, provider and facets. */
export function queryKeyOf(input: QueryKeyInput): string {
  return JSON.stringify({
    query: input.query.trim().toLowerCase(),
    provider: input.provider,
    gameVersion: input.gameVersion?.trim() || '',
    loader: input.loader?.trim().toLowerCase() || '',
    projectType: input.projectType?.trim() || 'mod',
  });
}

/**
 * F-166: pages by query key and page offset.
 *
 * A hit for `(key, offset)` is the same page the catalogue already answered;
 * rendering it again needs no round trip. Clearing it is the manual refresh.
 */
export class SearchCache {
  private pages = new Map<string, CachedPage>();

  static shared = new SearchCache();

  get(key: string, offset: number): CachedPage | null {
    return this.pages.get(`${key}#${offset}`) ?? null;
  }

  put(key: string, offset: number, page: CachedPage): void {
    this.pages.set(`${key}#${offset}`, page);
  }

  clear(): void {
    this.pages.clear();
  }

  get size(): number {
    return this.pages.size;
  }
}

/** A trailing debounce: the last call wins, after `ms` of quiet. */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): ((...args: A) => void) & { cancel(): void } {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const wrapped = (...args: A): void => {
    if (timer !== null) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = null;
      fn(...args);
    }, ms);
  };
  wrapped.cancel = (): void => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
  return wrapped;
}
