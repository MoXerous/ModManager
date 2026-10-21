import { useSyncExternalStore } from 'react';

/**
 * The game version and the loader the app is working with.
 *
 * Two screens need the same pair: Browse filters and installs with it, and
 * the queue installs with it. They used to disagree — Browse asked for 1.20.4
 * and the queue installed the 1.21.1 build — so the pair lives here and both
 * screens read it instead of each keeping a copy.
 *
 * shortcut: the pair is session state, not a database row; move it to core's
 * SQLite settings table when an instance has to remember its own pair.
 */

/** Suggestions, not a closed set: Modrinth adds versions and loaders faster
 *  than a release can ship a list, and the fields accept anything typed. The
 *  install and the search always use what is in the field, never a row here. */
export const GAME_VERSIONS = [
  '1.21.1',
  '1.21',
  '1.20.6',
  '1.20.4',
  '1.20.1',
  '1.19.4',
  '1.18.2',
  '1.16.5',
  '1.12.2',
  '1.8.9',
] as const;

/** Modrinth's loader facets are lowercase ids. */
export const LOADERS = ['fabric', 'forge', 'neoforge', 'quilt'] as const;

export const DEFAULT_GAME_VERSION = '1.21.1';
export const DEFAULT_LOADER = 'fabric';

export interface GameTarget {
  /** The Minecraft version the mods are resolved for. */
  gameVersion: string;
  /** The mod loader id, lowercase. */
  loader: string;
}

let current: GameTarget = {
  gameVersion: DEFAULT_GAME_VERSION,
  loader: DEFAULT_LOADER,
};

const listeners = new Set<() => void>();

export function getGameTarget(): GameTarget {
  return current;
}

export function setGameTarget(patch: Partial<GameTarget>): void {
  current = { ...current, ...patch };
  for (const listener of listeners) {
    listener();
  }
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Stable snapshot: `current` is only replaced on a write, so React never loops. */
export function useGameTarget(): GameTarget {
  return useSyncExternalStore(subscribe, getGameTarget, getGameTarget);
}
