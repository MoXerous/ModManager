import { useSyncExternalStore } from 'react';

export const LOCALES = ['en', 'ar'] as const;
export type Locale = (typeof LOCALES)[number];

export const THEMES = ['system', 'dark', 'light'] as const;
export type Theme = (typeof THEMES)[number];

/** Western 0-9 or Arabic-Indic ٠-٩ (design board 17). */
export type Numerals = 'latn' | 'arab';

export interface Preferences {
  theme: Theme;
  locale: Locale;
  numerals: Numerals;
}

/** Dark ships as the default (board 17); Arabic is the owner's default language. */
const DEFAULTS: Preferences = { theme: 'dark', locale: 'ar', numerals: 'latn' };

const DIRECTION: Record<Locale, 'ltr' | 'rtl'> = { en: 'ltr', ar: 'rtl' };

// shortcut: preferences live in localStorage because three UI flags do not
// deserve a database table yet; move them to core's SQLite settings table when
// the Settings screen persists anything the app must remember per install.
const STORAGE_KEY = 'modmanager.preferences.v1';

function isOneOf<T extends string>(allowed: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value);
}

/** Storage is untrusted and may be unavailable (private mode), so every field is checked. */
function load(): Preferences {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(STORAGE_KEY);
  } catch {
    return DEFAULTS;
  }
  if (!raw) return DEFAULTS;
  try {
    const parsed: Partial<Preferences> = JSON.parse(raw);
    return {
      theme: isOneOf(THEMES, parsed.theme) ? parsed.theme : DEFAULTS.theme,
      locale: isOneOf(LOCALES, parsed.locale) ? parsed.locale : DEFAULTS.locale,
      numerals: parsed.numerals === 'arab' ? 'arab' : DEFAULTS.numerals,
    };
  } catch {
    return DEFAULTS;
  }
}

let current: Preferences = load();
const listeners = new Set<() => void>();

export function isDark(theme: Theme = current.theme): boolean {
  return theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
}

export function applyToDocument(prefs: Preferences): void {
  const root = document.documentElement;
  root.classList.toggle('dark', isDark(prefs.theme));
  root.lang = prefs.locale;
  root.dir = DIRECTION[prefs.locale];
}

export function getPreferences(): Preferences {
  return current;
}

export function setPreferences(patch: Partial<Preferences>): void {
  current = { ...current, ...patch };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(current));
  } catch {
    // Storage unavailable: the session still honours the change.
  }
  applyToDocument(current);
  for (const listener of listeners) listener();
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Stable snapshot: `current` is only replaced on a write, so React never loops. */
export function usePreferences(): Preferences {
  return useSyncExternalStore(subscribe, getPreferences, getPreferences);
}

/** Re-applies the document when the OS theme flips under a `system` preference. */
export function watchSystemTheme(): () => void {
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  const onChange = (): void => {
    if (current.theme === 'system') applyToDocument(current);
  };
  media.addEventListener('change', onChange);
  return () => {
    media.removeEventListener('change', onChange);
  };
}