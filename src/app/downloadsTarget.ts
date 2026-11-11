/**
 * One-shot destination for the next Downloads mount.
 *
 * Browse hands the user to Downloads after an install, and the live queue is
 * empty by then: the batch ran from Browse, so its `Channel` events never
 * reached the queue's local rows. The log is where the result actually is, so
 * that visit opens the log instead of an empty "In progress" tab. Reading the
 * hint clears it: a later nav click gets the normal default.
 */
export type DownloadsTab = 'live' | 'completed' | 'history';

let pending: DownloadsTab | null = null;

export function requestDownloadsTab(tab: DownloadsTab): void {
  pending = tab;
}

/** Reads and clears: the hint applies to one visit only. */
export function takeDownloadsTab(): DownloadsTab | null {
  const tab = pending;
  pending = null;
  return tab;
}
