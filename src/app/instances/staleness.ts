/**
 * F-168: the staleness hint's state rules.
 *
 * The watcher signals per instance root; the screen decides what the signal
 * means. Three rules, kept pure because the component around them is DOM:
 * a signal for the open instance marks the content stale, a signal for any
 * other instance is ignored, and a manual refresh clears it only once the
 * re-scan has succeeded.
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

/**
 * The rescan's verdict on the hint. Only a scan that succeeded may clear it:
 * a failed scan has not re-read the disk, so the hint keeps its grounds and
 * the failure is shown next to it.
 */
export function afterRescan(state: StalenessState, ok: boolean): StalenessState {
  return ok ? { stale: false } : state;
}
