import { useState } from 'react';

import { Shell, type PageId } from './app/Shell';
import { usePreferences } from './app/preferences';
import { BrowsePage } from './app/pages/BrowsePage';
import { DownloadsPage } from './app/pages/DownloadsPage';
import { InstancesPage } from './app/pages/InstancesPage';
import { ModpackPage } from './app/pages/ModpackPage';
import { PlaceholderPage } from './app/pages/PlaceholderPage';
import { SettingsPage } from './app/pages/SettingsPage';

// shortcut: navigation is component state until the screens have real routes to
// link to; Wouter comes with the screens that carry state in the URL.
export default function App() {
  const [page, setPage] = useState<PageId>('instances');
  // The root subscribes to preference changes (theme, locale, numerals) so any
  // change re-renders the whole tree; pages read digits and locale at render
  // time through the i18n format hook and formatNumber/formatBytes.
  usePreferences();

  return (
    <Shell page={page} onNavigate={setPage}>
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