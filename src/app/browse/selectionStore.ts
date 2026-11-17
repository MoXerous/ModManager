import { useSyncExternalStore } from 'react';

import type { ProjectDto } from '../../bindings/ProjectDto.ts';
import { projectKey } from './columns.ts';

/**
 * The rows the user ticked and the projects they starred.
 *
 * The old app's promise on its Selected tab: "Selected compatible projects
 * are preserved across searches and pages". Component state dies with the
 * page and resets on every search, so the two lists live here — module state
 * read through `useSyncExternalStore`, the same pattern `gameTarget.ts` uses.
 * Each entry keeps the project snapshot it was ticked from, so the tabs can
 * still name a project whose row has left the current results.
 *
 * Both lists are keyed by `providerId:projectId`: a Modrinth project and a
 * CurseForge project may share a slug or a numeric id, and one list spans
 * two providers.
 *
 * shortcut: session state, like the game target. The old app persisted
 * favourites in its settings store; move these to core's JSON settings
 * (core/src/settings.rs) when favourites must survive a restart.
 */

export interface SelectedEntry {
  key: string;
  project: ProjectDto;
}

export interface BrowseSelectionState {
  /** The ticked rows, in the order they were ticked. */
  selected: readonly SelectedEntry[];
  /** The starred projects, in the order they were starred. */
  favourites: readonly SelectedEntry[];
}

let current: BrowseSelectionState = { selected: [], favourites: [] };

const listeners = new Set<() => void>();

function commit(next: BrowseSelectionState): void {
  current = next;
  for (const listener of listeners) {
    listener();
  }
}

export function getBrowseSelection(): BrowseSelectionState {
  return current;
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Stable snapshot: `current` is only replaced on a write, so React never loops. */
export function useBrowseSelection(): BrowseSelectionState {
  return useSyncExternalStore(subscribe, getBrowseSelection, getBrowseSelection);
}

function withoutKey(entries: readonly SelectedEntry[], key: string): SelectedEntry[] {
  return entries.filter((entry) => entry.key !== key);
}

/** Tick or release one row. The row's project is kept for the tabs. */
export function toggleSelected(project: ProjectDto): void {
  const key = projectKey(project);
  const selected = current.selected.some((entry) => entry.key === key)
    ? withoutKey(current.selected, key)
    : [...current.selected, { key, project }];
  commit({ ...current, selected });
}

/** The old table's "Clear all selections". */
export function clearSelection(): void {
  if (current.selected.length === 0) {
    return;
  }
  commit({ ...current, selected: [] });
}

/** Remove one row from the Selected tab (the old double-click). */
export function removeSelected(key: string): void {
  if (!current.selected.some((entry) => entry.key === key)) {
    return;
  }
  commit({ ...current, selected: withoutKey(current.selected, key) });
}

/** Star or unstar one project. Never touches the selection. */
export function toggleFavourite(project: ProjectDto): void {
  const key = projectKey(project);
  const favourites = current.favourites.some((entry) => entry.key === key)
    ? withoutKey(current.favourites, key)
    : [...current.favourites, { key, project }];
  commit({ ...current, favourites });
}

/** Remove one favourite from the Favourites tab (the old double-click). */
export function removeFavourite(key: string): void {
  if (!current.favourites.some((entry) => entry.key === key)) {
    return;
  }
  commit({ ...current, favourites: withoutKey(current.favourites, key) });
}
