import type { ComponentType, ReactNode } from 'react';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Download, Layers, Search, SlidersHorizontal, type LucideProps } from 'lucide-react';

import { cn } from '../lib/cn';
import { watchSystemTheme } from './preferences';

export const PAGE_IDS = ['instances', 'browse', 'downloads', 'settings'] as const;
export type PageId = (typeof PAGE_IDS)[number];

interface NavItem {
  id: PageId;
  icon: ComponentType<LucideProps>;
  labelKey: string;
}

const NAV: readonly NavItem[] = [
  { id: 'instances', icon: Layers, labelKey: 'nav.instances' },
  { id: 'browse', icon: Search, labelKey: 'nav.browse' },
  { id: 'downloads', icon: Download, labelKey: 'nav.downloads' },
  { id: 'settings', icon: SlidersHorizontal, labelKey: 'nav.settings' },
];

export interface ShellProps {
  page: PageId;
  onNavigate: (page: PageId) => void;
  children: ReactNode;
}

export function Shell({ page, onNavigate, children }: ShellProps) {
  const { t } = useTranslation();

  useEffect(() => watchSystemTheme(), []);

  return (
    <div className="grid h-screen grid-cols-[56px_1fr] overflow-hidden">
      <nav
        aria-label={t('app.primaryNav')}
        className="flex flex-col items-center gap-1 border-e border-border bg-surface-sunken px-2 py-3.5"
      >
        <div className="pb-3.5 font-semibold" aria-hidden>
          M
        </div>
        {NAV.map((item) => (
          <button
            key={item.id}
            type="button"
            onClick={() => onNavigate(item.id)}
            aria-label={t(item.labelKey)}
            title={t(item.labelKey)}
            aria-current={item.id === page ? 'page' : undefined}
            className={cn(
              'flex h-9 w-full items-center justify-center rounded-control text-muted-foreground',
              item.id === page && 'bg-accent text-foreground',
            )}
          >
            <item.icon aria-hidden className="size-[18px]" strokeWidth={1.75} />
          </button>
        ))}
      </nav>

      <div className="flex min-w-0 flex-col">
        <header className="flex h-11 shrink-0 items-center gap-3 border-b border-border px-5 text-muted-foreground">
          <span className="text-foreground">{t(`nav.${page}`)}</span>
          {/*
            shortcut: the palette hint is static until cmdk lands in 5b; rendered
            as a div so it does not present a dead button.
          */}
          <div className="ms-auto flex h-7 items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5">
            <Search aria-hidden className="size-[18px]" strokeWidth={1.75} />
            <span className="text-small">{t('palette.placeholder')}</span>
            <kbd className="rounded-[4px] border border-border px-1 font-mono text-[11px]" dir="ltr">
              {t('palette.shortcut')}
            </kbd>
          </div>
        </header>
        <main className="min-h-0 flex-1 overflow-auto">{children}</main>
      </div>
    </div>
  );
}