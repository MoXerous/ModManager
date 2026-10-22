import { useSyncExternalStore } from 'react';

/**
 * The game version and the loader the app is working with.
 *
 * Two screens need the same pair: Browse filters and installs with it, and
 * the queue installs with it. They used to disagree — Browse asked for 1.20.4
 * and the queue installed the 1.21.1 build — so the pair lives here and both
 * screens read it instead of each keeping a copy.
 *
 * The pair is not stored anywhere: it belongs to the session and follows the
 * instance it came from. An instance knows its own Minecraft version and its
 * own loader, so picking an instance is what sets the fields; the user may
 * override them, and the override stands until another instance is picked.
 *
 * shortcut: the pair is session state, not a persisted row; move it to core's
 * JSON settings file (core/src/settings.rs, under config_dir) when an instance
 * has to remember a pair the user chose rather than the one it was created
 * with.
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

/** What an instance says about itself. */
export interface InstanceTarget {
  id: string;
  /** Null when the instance was never scanned, or was created without one. */
  minecraftVersion: string | null;
  loader: string | null;
}

export interface GameTargetState extends GameTarget {
  /** The instance the values were taken from, null when they are only defaults. */
  instanceId: string | null;
  /** True when the user typed the values themselves. */
  overridden: boolean;
}

let current: GameTargetState = {
  gameVersion: DEFAULT_GAME_VERSION,
  loader: DEFAULT_LOADER,
  instanceId: null,
  overridden: false,
};

const listeners = new Set<() => void>();

export function getGameTarget(): GameTargetState {
  return current;
}

function commit(next: GameTargetState): void {
  current = next;
  for (const listener of listeners) {
    listener();
  }
}

/** The user typed a value: it is theirs until another instance is picked. */
export function setGameTarget(patch: Partial<GameTarget>): void {
  commit({ ...current, ...patch, overridden: true });
}

/**
 * The selected instance changed: the fields take the instance's own values.
 *
 * An instance that does not know its version or its loader leaves whatever
 * is in the field alone — an unknown value is not a reason to wipe a good
 * one. A value the user typed for this very instance stands: it is only
 * replaced when a different instance is picked.
 */
export function setTargetInstance(instance: InstanceTarget | null): void {
  if (!instance) {
    if (current.instanceId === null) {
      return;
    }
    commit({ ...current, instanceId: null });
    return;
  }
  if (current.instanceId === instance.id && current.overridden) {
    return;
  }
  const version = instance.minecraftVersion?.trim();
  const loader = instance.loader?.trim().toLowerCase();
  commit({
    ...current,
    gameVersion: version && version.length > 0 ? version : current.gameVersion,
    loader: loader && loader.length > 0 ? loader : current.loader,
    instanceId: instance.id,
    overridden: false,
  });
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Stable snapshot: `current` is only replaced on a write, so React never loops. */
export function useGameTarget(): GameTargetState {
  return useSyncExternalStore(subscribe, getGameTarget, getGameTarget);
}
