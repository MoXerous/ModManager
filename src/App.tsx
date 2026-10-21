import { useState } from 'react';
import { flushSync } from 'react-dom';

import { Shell, type PageId } from './app/Shell';
import { usePreferences } from './app/preferences';
import { BrowsePage } from './app/pages/BrowsePage';
import { DownloadsPage } from './app/pages/DownloadsPage';
import { InstancesPage } from './app/pages/InstancesPage';
import { ModpackPage } from './app/pages/ModpackPage';
import { PlaceholderPage } from './app/pages/PlaceholderPage';
import { SettingsPage } from './app/pages/SettingsPage';

export default function App() {
  // shortcut: the page is component state until the screens need linkable
  // routes; Wouter comes with the screens that carry state in the URL.
  const [page, setPage] = useState<PageId>('instances');
  // The root subscribes to preference changes (theme, locale, numerals, rail)
  // so any change re-renders the whole tree; pages read digits and locale at
  // render time through the i18n format hook and formatNumber/formatBytes.
  usePreferences();

  /**
   * A page swap is the one transition with a real outgoing frame to fade from.
   * The View Transition API snapshots it without mounting two page trees —
   * mounting both would re-run every page effect, including its IPC calls.
   * Without the API, or under reduced motion, the swap is simply instant.
   */
  const navigate = (next: PageId): void => {
    if (next === page) {
      return;
    }
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (reduced || typeof document.startViewTransition !== 'function') {
      setPage(next);
      return;
    }
    document.startViewTransition(() => {
      flushSync(() => setPage(next));
    });
  };

  return (
    <Shell page={page} onNavigate={navigate}>
      {page === 'instances' ? (
        <InstancesPage />
      ) : page === 'browse' ? (
        <BrowsePage onInstalled={() => setPage('downloads')} />
      ) : page === 'modpack' ? (
        <ModpackPage />
      ) : page === 'downloads' ? (
        <DownloadsPage />
      ) : page === 'settings' ? (
        <SettingsPage />
      ) : (
        <PlaceholderPage
          titleKey={`nav.${page}`}
          emptyTitleKey={`page.${page}.emptyTitle`}
          emptyBodyKey={`page.${page}.emptyBody`}
        />
      )}
    </Shell>
  );
}
