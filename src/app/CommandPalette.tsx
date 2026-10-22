import { useTranslation } from 'react-i18next';
import { Search } from 'lucide-react';

import { Kbd } from '@/components/Kbd';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog';

import { NAV, type PageId } from './Shell';

// Board 11, and nothing more: with no instances and no mod data behind it, the
// palette's honest contents are the four destinations. The board's data groups
// (mods, instances) and its chord hints arrive with the screens that fill them
// and with F-171 keyboard shortcuts — showing either now would be a lie.
export interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onNavigate: (page: PageId) => void;
}

export function CommandPalette({ open, onOpenChange, onNavigate }: CommandPaletteProps) {
  const { t } = useTranslation();

  const run = (page: PageId) => {
    onNavigate(page);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogHeader className="sr-only">
        <DialogTitle>{t('palette.title')}</DialogTitle>
        <DialogDescription>{t('palette.description')}</DialogDescription>
      </DialogHeader>
      <DialogContent
        showCloseButton={false}
        // The board pins the palette 100px from the top, inset-inline centred at
        // 580px — not the centred max-w-lg Dialog ships with.
        className="top-[100px] w-[580px] translate-y-0 gap-0 overflow-hidden p-0"
      >
        <Command>
          <CommandInput placeholder={t('palette.placeholder')}>
            <Search aria-hidden className="size-[18px] shrink-0" strokeWidth={1.75} />
            <Kbd className="ms-auto">{t('palette.escape')}</Kbd>
          </CommandInput>
          <CommandList>
            <CommandEmpty>{t('palette.empty')}</CommandEmpty>
            <CommandGroup heading={t('palette.group.goTo')}>
              {NAV.map((item) => (
                <CommandItem
                  key={item.id}
                  // cmdk filters on `value`, so both scripts are searchable.
                  value={`${item.id} ${t(item.labelKey)}`}
                  onSelect={() => run(item.id)}
                >
                  <item.icon aria-hidden strokeWidth={1.75} />
                  {t(item.labelKey)}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
          <div className="flex items-center gap-1.5 border-t border-border px-5 py-3 text-small text-muted-foreground">
            <Kbd>↑↓</Kbd>
            <span>{t('palette.hint.navigate')}</span>
            <Kbd>Enter</Kbd>
            <span>{t('palette.hint.run')}</span>
          </div>
        </Command>
      </DialogContent>
    </Dialog>
  );
}