import { useState } from 'react';

import { Shell, type PageId } from './app/Shell';
import { DownloadsPage } from './app/pages/DownloadsPage';
import { InstancesPage } from './app/pages/InstancesPage';
import { PlaceholderPage } from './app/pages/PlaceholderPage';
import { SettingsPage } from './app/pages/SettingsPage';

// shortcut: navigation is component state until the screens have real routes to
// link to; Wouter comes with the screens that carry state in the URL.
export default function App() {
  const [page, setPage] = useState<PageId>('instances');

  return (
    <Shell page={page} onNavigate={setPage}>
      {page === 'instances' ? (
        <InstancesPage />
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