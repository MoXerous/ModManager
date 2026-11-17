/**
 * F-168: the staleness hint's state rules.
 *
 * The watcher signals per instance root; the screen decides what the signal
 * means. Three rules, kept pure because the component around them is DOM:
 * a signal for the open instance marks the content stale, a signal for any
 * other instance is ignored, and a manual refresh (a re-scan) clears it.
 */

/** Whether the event's root is the instance this screen is showing. */
export function isForInstance(eventRoot: string, instanceRoot: string): boolean {
  // Windows paths differ in casing without differing in identity; the scan
  // reports canonical paths, but the event is the watcher's echo of them.
  return (
    eventRoot.trim().toLowerCase() === instanceRoot.trim().toLowerCase()
  );
}

/** The hint's only axis: is the scan on screen possibly out of date. */
export interface StalenessState {
  stale: boolean;
}

/** A watcher signal: the content on disk moved. */
export function markStale(_state: StalenessState): StalenessState {
  return { stale: true };
}

/** The manual refresh: the scan was re-run, so the hint has no grounds. */
export function refreshed(_state: StalenessState): StalenessState {
  return { stale: false };
}
