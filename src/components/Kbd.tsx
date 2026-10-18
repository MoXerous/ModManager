import type { ReactNode } from 'react';

import { cn } from 'cn';

/** The board's `.k`: bordered, mono 11px, always Latin even in Arabic UI. */
export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd
      dir="ltr"
      className={cn(
        'rounded-[4px] border border-border px-1 font-mono text-[11px] text-muted-foreground',
        className,
      )}
    >
      {children}
    </kbd>
  );
}