import { useTranslation } from 'react-i18next';

import { EmptyState } from '../../components/state';

export interface PlaceholderPageProps {
  /** i18n key under `nav.` for the page title. */
  titleKey: string;
  emptyTitleKey: string;
  emptyBodyKey: string;
}

/**
 * Page frame for the screens 5b/5c fill in. It is the real header + empty
 * state, not a mock: every list page starts empty exactly like this.
 */
export function PlaceholderPage({ titleKey, emptyTitleKey, emptyBodyKey }: PlaceholderPageProps) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-5 pt-4">
        <h1 className="text-page-title font-semibold">{t(titleKey)}</h1>
      </div>
      <EmptyState title={t(emptyTitleKey)} body={t(emptyBodyKey)} />
    </div>
  );
}