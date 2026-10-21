import type { ComponentType, ReactNode } from 'react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Download,
  Layers,
  Package,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  Search,
  SlidersHorizontal,
  type LucideProps,
} from 'lucide-react';
import { cn } from 'cn';

import { Kbd } from '@/components/Kbd';

import { CommandPalette } from './CommandPalette';
import { isRtl, setPreferences, usePreferences, watchSystemTheme } from './preferences';

export const PAGE_IDS = ['instances', 'browse', 'modpack', 'downloads', 'settings'] as const;
export type PageId = (typeof PAGE_IDS)[number];

interface NavItem {
  id: PageId;
  icon: ComponentType<LucideProps>;
  labelKey: string;
}

export const NAV: readonly NavItem[] = [
  { id: 'instances', icon: Layers, labelKey: 'nav.instances' },
  { id: 'browse', icon: Search, labelKey: 'nav.browse' },
  { id: 'modpack', icon: Package, labelKey: 'nav.modpack' },
  { id: 'downloads', icon: Download, labelKey: 'nav.downloads' },
  { id: 'settings', icon: SlidersHorizontal, labelKey: 'nav.settings' },
];

/** The 56px rail from the boards, and a width that fits the longest label. */
const RAIL_COLLAPSED = 56;
const RAIL_EXPANDED = 208;

/**
 * The rail's icon sits 11px into the 56px rail, which is its centre (8px pad +
 * 11 + half of 18). Keeping that padding in both states means the icons never
 * jump sideways while the rail grows; only the label area appears.
 */
const ITEM = 'flex h-9 w-full shrink-0 items-center gap-2.5 overflow-hidden rounded-control ps-[11px] pe-2';

/**
 * Labels are clipped, never wrapped: a wrapping label would change the nav's
 * height mid-animation. The timing is asymmetric so the words stay out of the
 * way while collapsing and only arrive once there is room while expanding.
 *
 * The label is hidden from the accessibility tree while it is invisible: a
 * screen reader must not read out a name the eye cannot see. The button
 * carries the name of the action anyway.
 */
function labelClass(collapsed: boolean): string {
  return cn(
    'min-w-0 truncate whitespace-nowrap text-body transition-opacity ease-out will-change-[opacity]',
    collapsed ? 'opacity-0 duration-100' : 'opacity-100 duration-150 delay-75',
  );
}

export interface ShellProps {
  page: PageId;
  onNavigate: (page: PageId) => void;
  children: ReactNode;
}

export function Shell({ page, onNavigate, children }: ShellProps) {
  const { t } = useTranslation();
  // Reads the persisted flag through the same store as theme and locale, so a
  // change from anywhere (this button today, Settings later) repaints the rail.
  const { sidebarCollapsed, locale } = usePreferences();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const rtl = isRtl(locale);

  useEffect(() => watchSystemTheme(), []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'k' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  // The rail is the first grid column, so in RTL it sits on the right; the
  // panel glyph and its arrow have to point at the edge the rail is on.
  const ToggleIcon = sidebarCollapsed
    ? rtl
      ? PanelRightOpen
      : PanelLeftOpen
    : rtl
      ? PanelRightClose
      : PanelLeftClose;
  const toggleLabel = t(sidebarCollapsed ? 'sidebar.action.expand' : 'sidebar.action.collapse');

  return (
    <div
      // The rail's track changes in one step, it is not animated: the second
      // track is `1fr`, so every frame of a track animation would lay the
      // whole page out again — twelve layouts for one click, all of them
      // thrown away. `grid-template-columns` cannot be composited either, so
      // a `will-change` hint on it would promise a compositor that does not
      // exist. The width therefore changes once, and the only motion left is
      // the labels' opacity, which the compositor animates on its own layer.
      // Reduced motion is handled once, globally, in theme.css.
      className="grid h-screen overflow-hidden"
      style={{
        gridTemplateColumns: `${sidebarCollapsed ? RAIL_COLLAPSED : RAIL_EXPANDED}px 1fr`,
      }}
    >
      <nav
        id="app-sidebar"
        aria-label={t('app.primaryNav')}
        className="flex min-w-0 flex-col gap-1 overflow-hidden border-e border-border bg-surface-sunken px-2 py-3.5"
      >
        <div
          aria-hidden
          className="mb-3.5 flex h-[18px] w-full shrink-0 items-center ps-[11px] font-semibold leading-none"
        >
          <span className="w-[18px] text-center">M</span>
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
              ITEM,
              'text-muted-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50',
              item.id === page && 'bg-accent text-foreground',
            )}
          >
            <item.icon aria-hidden className="size-[18px] shrink-0" strokeWidth={1.75} />
            <span aria-hidden={sidebarCollapsed || undefined} className={labelClass(sidebarCollapsed)}>
              {t(item.labelKey)}
            </span>
          </button>
        ))}

        <div className="mt-auto w-full border-t border-border pt-2">
          <button
            type="button"
            onClick={() => setPreferences({ sidebarCollapsed: !sidebarCollapsed })}
            aria-label={toggleLabel}
            title={toggleLabel}
            // The rail never leaves the page: only its labels come and go, so
            // there is nothing here to disclose and nothing to collapse.
            className={cn(
              ITEM,
              'text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50',
            )}
          >
            <ToggleIcon aria-hidden className="size-[18px] shrink-0" strokeWidth={1.75} />
            <span aria-hidden={sidebarCollapsed || undefined} className={labelClass(sidebarCollapsed)}>
              {toggleLabel}
            </span>
          </button>
        </div>
      </nav>

      <div className="flex min-w-0 flex-col">
        <header className="flex h-11 shrink-0 items-center gap-3 border-b border-border px-5 text-muted-foreground">
          <span className="text-foreground">{t(`nav.${page}`)}</span>
          <button
            type="button"
            onClick={() => setPaletteOpen(true)}
            className="ms-auto flex h-7 items-center gap-2 rounded-control border border-border bg-surface-sunken px-2.5 text-muted-foreground hover:text-foreground"
          >
            <Search aria-hidden className="size-[18px]" strokeWidth={1.75} />
            <span className="text-small">{t('palette.placeholder')}</span>
            <Kbd>{t('palette.shortcut')}</Kbd>
          </button>
        </header>
        {/* The named region is what App cross-fades between page changes. */}
        <main className="min-h-0 flex-1 overflow-auto [view-transition-name:page-content]">
          {children}
        </main>
      </div>

      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        onNavigate={onNavigate}
      />
    </div>
  );
}
