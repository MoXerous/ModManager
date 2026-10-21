import type { FocusEvent, KeyboardEvent } from 'react';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Command as CommandPrimitive } from 'cmdk';
import { Check, ChevronDown, CornerDownLeft } from 'lucide-react';
import { cn } from 'cn';

/**
 * An editable dropdown: the list suggests, it never constrains.
 *
 * A version ("1.21.1") or a loader ("fabric") is not a closed set we can own —
 * Modrinth ships new ones and the user may know one we do not — so the field
 * keeps whatever is typed and offers the list as a shortcut. `cmdk` is the
 * keyboard/listbox engine; the popup looks nothing like its palette shell.
 *
 * The field renders no visible label: callers put one beside it with `htmlFor`
 * and `id`, because the field is one cell in a row, not a stacked form row.
 * `label` is the field's accessible name; the listbox announces itself as
 * suggestions.
 */
export interface ComboboxProps {
  /** The text in the field. Free text is kept; a suggestion replaces it. */
  value: string;
  /** Every keystroke and every pick land here. */
  onValueChange: (value: string) => void;
  /** Offered rows. A value outside this list is still a valid value. */
  options: readonly string[];
  /** Accessible name for the field and the popup. */
  label: string;
  /** Ties an external `<label htmlFor>` to the input. */
  id?: string;
  placeholder?: string;
  disabled?: boolean;
  /** Direction of the field itself: versions and loaders are Latin tokens. */
  dir?: 'ltr' | 'rtl';
  /** Classes for the positioning root, e.g. a width. */
  className?: string;
  /** Classes for the popup, e.g. a width wider than a narrow field. */
  listClassName?: string;
}

export function Combobox({
  value,
  onValueChange,
  options,
  label,
  id,
  placeholder,
  disabled = false,
  dir = 'ltr',
  className,
  listClassName,
}: ComboboxProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  /**
   * Opening is not editing. A field already holding "1.21.1" must still show
   * the other versions, otherwise the popup would offer the value that is
   * already in the box and nothing else; the narrowing starts with a keystroke.
   */
  const [editing, setEditing] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  /** Read by the observer below, which outlives every open/close render. */
  const openRef = useRef(open);

  const typed = value.trim();
  const query = typed.toLowerCase();
  // `<select>` options are unique by contract; a free-typed list is not, and a
  // duplicate would make cmdk's item map and React's keys disagree.
  const unique = Array.from(new Set(options.filter((option) => option.length > 0)));
  const visible =
    editing && query.length > 0
      ? unique.filter((option) => option.toLowerCase().includes(query))
      : unique;
  // "Use what I typed" is an action, not a suggestion: it appears only while
  // editing and only when no row already says exactly that.
  const showCustom =
    editing && typed.length > 0 && !unique.some((option) => option.toLowerCase() === query);

  /**
   * cmdk is a palette primitive: its input hardcodes `aria-expanded="true"`
   * because a palette's list is always on screen, and it overwrites `id` with
   * its own generated one. This popup closes and sits beside a visible
   * `<label htmlFor={id}>`, so both attributes are corrected after every
   * commit — React will not touch them again while its props stay the same.
   */
  useLayoutEffect(() => {
    openRef.current = open;
    const input = inputRef.current;
    if (!input) {
      return;
    }
    input.setAttribute('aria-expanded', open ? 'true' : 'false');
    const root = input.closest('[cmdk-root]');
    if (id) {
      input.id = id;
      // cmdk's own screen-reader label still names this field; point it at the
      // real id so its `for` and the caller's visible `<label for>` agree.
      const ownLabel = root?.querySelector<HTMLLabelElement>('label[cmdk-label]');
      if (ownLabel) {
        ownLabel.htmlFor = id;
      }
    }
    if (!open) {
      input.removeAttribute('aria-activedescendant');
    }
  });

  /**
   * cmdk highlights a row by writing `aria-selected` straight into that row;
   * the surrounding component does not re-render, so no effect here would see
   * the change, and cmdk only publishes the active row on arrow keys. A
   * combobox must announce what Enter would pick as soon as the list filters,
   * so the highlight is mirrored onto the field. The observer also runs while
   * closed, because a pick makes cmdk write the id it just selected into a
   * field that no longer shows a list.
   */
  useEffect(() => {
    const input = inputRef.current;
    const list = listRef.current;
    if (!input || !list) {
      return;
    }
    const sync = () => {
      if (!openRef.current) {
        if (input.hasAttribute('aria-activedescendant')) {
          input.removeAttribute('aria-activedescendant');
        }
        return;
      }
      const active = list.querySelector<HTMLElement>('[role="option"][aria-selected="true"]');
      if (active?.id) {
        if (input.getAttribute('aria-activedescendant') !== active.id) {
          input.setAttribute('aria-activedescendant', active.id);
        }
      } else if (input.hasAttribute('aria-activedescendant')) {
        input.removeAttribute('aria-activedescendant');
      }
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(list, { attributes: true, attributeFilter: ['aria-selected'], subtree: true });
    // React owns the attribute too (cmdk renders it from its own state), so a
    // render that follows one of our writes must not leave the field pointing
    // at nothing.
    observer.observe(input, { attributes: true, attributeFilter: ['aria-activedescendant'] });
    return () => observer.disconnect();
  }, []);

  const close = () => {
    setOpen(false);
    setEditing(false);
  };

  const choose = (next: string) => {
    onValueChange(next);
    close();
  };

  const type = (next: string) => {
    setEditing(true);
    setOpen(true);
    onValueChange(next);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // An IME composition is still typing: arrows and Enter belong to it.
    if (event.nativeEvent.isComposing) {
      return;
    }

    if (event.key === 'Escape') {
      if (!open) {
        // Nothing to dismiss here; a surrounding dialog still gets its Escape.
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }

    if (event.key === 'Tab') {
      // Not prevented: Tab keeps moving focus, it only leaves the popup behind.
      close();
      return;
    }

    if (event.key === 'Home' || event.key === 'End') {
      // cmdk reads Home/End as "first/last option". In an editable field the
      // caret has to win, so the list jump is prevented and the caret moved by
      // hand (shift keeps extending a selection).
      event.preventDefault();
      const input = inputRef.current;
      if (!input) {
        return;
      }
      const at = event.key === 'Home' ? 0 : input.value.length;
      const start = input.selectionStart ?? at;
      const end = input.selectionEnd ?? at;
      if (event.shiftKey) {
        if (event.key === 'Home') {
          input.setSelectionRange(at, end);
        } else {
          input.setSelectionRange(start, at);
        }
      } else {
        input.setSelectionRange(at, at);
      }
      return;
    }

    if (!open && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      // Nothing is rendered to move over yet: open, and let the next press move.
      // Preventing the default also stops cmdk from selecting off-screen rows.
      event.preventDefault();
      setOpen(true);
      return;
    }

    if (!open && event.key === 'Enter') {
      // The typed value is already committed. Without this, cmdk would pick the
      // row that was active when the popup last closed.
      event.preventDefault();
    }
  };

  /** Focus leaving the whole widget, not just moving inside it. */
  const onBlur = (event: FocusEvent<HTMLDivElement>) => {
    const next = event.relatedTarget instanceof Node ? event.relatedTarget : null;
    if (next && event.currentTarget.contains(next)) {
      return;
    }
    close();
  };

  return (
    <div className={cn('relative', className)} onBlur={onBlur}>
      <CommandPrimitive
        label={label}
        shouldFilter={false}
        // Filtering is done above, because it must start only after a keystroke
        // and must include "what I typed" as a row. cmdk keeps the listbox,
        // selection and keyboard model; it does not decide what is visible.
        // Ctrl+K is the app's palette shortcut; cmdk's vim bindings would also
        // move the list under it.
        vimBindings={false}
        onKeyDown={onKeyDown}
        className="w-full"
      >
        <div dir={dir} className="relative">
          <CommandPrimitive.Input
            ref={inputRef}
            id={id}
            value={value}
            onValueChange={type}
            onFocus={() => {
              setOpen(true);
              setEditing(false);
            }}
            // A second click on an already focused field must reopen what
            // Escape closed, and focus does not fire again for that.
            onClick={() => setOpen(true)}
            disabled={disabled}
            placeholder={placeholder}
            aria-label={label}
            className="h-8 w-full min-w-0 rounded-control border border-input bg-surface-sunken ps-2.5 pe-8 font-mono text-body text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-60"
          />
          <ChevronDown
            aria-hidden
            strokeWidth={1.75}
            className="pointer-events-none absolute end-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
          />
        </div>

        {/* Kept mounted so cmdk's item registry and selection survive a close;
            `hidden` takes the whole listbox out of the layout and a11y tree. */}
        <div hidden={!open} dir={dir} className="absolute start-0 top-full z-50 mt-1 w-44 min-w-full">
          <CommandPrimitive.List
            ref={listRef}
            label={t('combobox.suggestions')}
            // A pointer press must not blur the input: the click that follows
            // is what selects, and blur would close the list first.
            onMouseDown={(event) => event.preventDefault()}
            className={cn(
              'max-h-64 overflow-y-auto rounded-card border border-border bg-popover p-1 text-popover-foreground shadow-md',
              listClassName,
            )}
          >
            {visible.map((option) => (
              <CommandPrimitive.Item
                key={option}
                value={option}
                onSelect={() => choose(option)}
                className="flex h-8 cursor-default items-center gap-2.5 rounded-control px-2.5 font-mono text-body text-foreground outline-none select-none data-[selected=true]:bg-accent data-[selected=true]:text-accent-foreground"
              >
                <span className="min-w-0 flex-1 truncate">{option}</span>
                {option === value && (
                  <Check aria-hidden strokeWidth={1.75} className="size-4 shrink-0 text-primary" />
                )}
              </CommandPrimitive.Item>
            ))}

            {/* Our own row, not cmdk's `Empty`: the "use what I typed" item is
                an item, so cmdk would always consider the list non-empty. */}
            {visible.length === 0 && (
              <div className="px-2.5 py-2 text-center text-small text-muted-foreground">
                {t('combobox.empty')}
              </div>
            )}

            {showCustom && (
              <>
                <CommandPrimitive.Separator alwaysRender className="my-1 h-px bg-border" />
                <CommandPrimitive.Item
                  // Not a value anyone could type: cmdk keys its registry by it,
                  // and it must not collide with a real option.
                  value="__combobox_use_typed__"
                  onSelect={() => choose(typed)}
                  className="flex min-h-8 cursor-default items-center gap-2.5 rounded-control px-2.5 py-1.5 text-body text-muted-foreground outline-none select-none data-[selected=true]:bg-accent data-[selected=true]:text-foreground"
                >
                  <CornerDownLeft
                    aria-hidden
                    strokeWidth={1.75}
                    className="size-4 shrink-0"
                  />
                  <span className="min-w-0 flex-1">
                    {t('combobox.useTyped', { value: typed })}
                  </span>
                </CommandPrimitive.Item>
              </>
            )}
          </CommandPrimitive.List>
        </div>
      </CommandPrimitive>
    </div>
  );
}
