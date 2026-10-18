import { useState } from 'react';

import { Shell, type PageId } from './app/Shell';
import { PlaceholderPage } from './app/pages/PlaceholderPage';
import { SettingsPage } from './app/pages/SettingsPage';

// shortcut: navigation is component state until the screens have real routes to
// link to; Wouter comes in 5c, when deep links and the command palette exist.
export default function App() {
  const [page, setPage] = useState<PageId>('instances');

  return (
    <Shell page={page} onNavigate={setPage}>
      {page === 'settings' ? (
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