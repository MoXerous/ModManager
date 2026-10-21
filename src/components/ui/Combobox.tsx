import type { FocusEvent, KeyboardEvent } from "react"
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"
import { createPortal } from "react-dom"
import { useTranslation } from "react-i18next"
import { Command as CommandPrimitive } from "cmdk"
import { Check, ChevronDown, CornerDownLeft } from "lucide-react"
import { cn } from "cn"

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
 *
 * The popup is a portal on `document.body`. The page scrolls inside `main`,
 * which clips whatever overflows it, so a list opened near the bottom of a
 * short window used to be cut off with its rows unreachable. The portal is
 * placed from the trigger's box, so it still lines up with the field.
 */

/** How long the leave animation runs before the popup leaves the DOM. */
const EXIT_MS = 200

/** The popup never grows past this, whatever the window says. */
const MAX_HEIGHT = 256

/** Space between the field and the popup, and the margin kept off the window. */
const GAP = 4
const MARGIN = 8

interface PopupBox {
  /** Distance from the top of the viewport, when the popup opens downwards. */
  top?: number
  /** Distance from the bottom of the viewport, when it opens upwards. */
  bottom?: number
  /** Distance from the inline start of the viewport. */
  start: number
  /** The popup is never narrower than the field that opened it. */
  minWidth: number
  maxHeight: number
}

export interface ComboboxProps {
  /** The text in the field. Free text is kept; a suggestion replaces it. */
  value: string
  /** Every keystroke and every pick land here. */
  onValueChange: (value: string) => void
  /** Offered rows. A value outside this list is still a valid value. */
  options: readonly string[]
  /** Accessible name for the field and the popup. */
  label: string
  /** Ties an external `<label htmlFor>` to the input. */
  id?: string
  placeholder?: string
  disabled?: boolean
  /** Direction of the field itself: versions and loaders are Latin tokens. */
  dir?: "ltr" | "rtl"
  /** Classes for the positioning root, e.g. a width. */
  className?: string
  /** Classes for the popup, e.g. a width wider than a narrow field. */
  listClassName?: string
}

export function Combobox({
  value,
  onValueChange,
  options,
  label,
  id,
  placeholder,
  disabled = false,
  dir = "ltr",
  className,
  listClassName,
}: ComboboxProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  /**
   * Opening is not editing. A field already holding "1.21.1" must still show
   * the other versions, otherwise the popup would offer the value that is
   * already in the box and nothing else; the narrowing starts with a keystroke.
   */
  const [editing, setEditing] = useState(false)
  /** The popup stays in the DOM while it fades out, then it is hidden. */
  const [present, setPresent] = useState(false)
  const [box, setBox] = useState<PopupBox | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const anchorRef = useRef<HTMLDivElement>(null)
  /**
   * The portalled popup. It is the widget's other half, but it is not its
   * child: the portal puts it on the body, so a focus check that only asks
   * about the field's own box reads a move into the popup as a departure.
   */
  const popupRef = useRef<HTMLDivElement>(null)
  /** The widget: the field's box, and through the portal the popup. */
  const widgetRef = useRef<HTMLDivElement>(null)
  /** Read by the observer below, which outlives every open/close render. */
  const openRef = useRef(open)
  /**
   * cmdk's own id for the input, read before ours replaces it. cmdk looks the
   * input up by that id to give the caret back whenever the list moved it
   * away, so the lookup has to keep working. The same id is recorded on
   * cmdk's root, which lives exactly as long as the cmdk instance that made
   * it: a render that comes back with a fresh set of refs still finds it.
   */
  const cmdkInputIdRef = useRef<string | null>(null)
  /**
   * The id we last patched onto the input. A caller that changes its `id`
   * leaves our own id behind on the input and on cmdk's label, and those are
   * not cmdk's id: the capture has to tell them apart.
   */
  const patchedIdRef = useRef<string | null>(null)

  const typed = value.trim()
  const query = typed.toLowerCase()
  // `<select>` options are unique by contract; a free-typed list is not, and a
  // duplicate would make cmdk's item map and React's keys disagree.
  const unique = Array.from(new Set(options.filter((option) => option.length > 0)))
  const visible =
    editing && query.length > 0
      ? unique.filter((option) => option.toLowerCase().includes(query))
      : unique
  // "Use what I typed" is an action, not a suggestion: it appears only while
  // editing and only when no row already says exactly that.
  const showCustom =
    editing && typed.length > 0 && !unique.some((option) => option.toLowerCase() === query)

  /**
   * The popup lines up with the field, not with the page. The page is RTL by
   * default, so the side the popup grows towards follows the document's
   * direction while the rows themselves keep the field's direction.
   */
  const pageDir = document.documentElement.dir === "rtl" ? "rtl" : "ltr"

  /**
   * cmdk highlights a row by writing `aria-selected` straight into that row;
   * the surrounding component does not re-render, so no effect here would see
   * the change, and cmdk only publishes the active row on arrow keys. A
   * combobox must announce what Enter would pick as soon as the list filters,
   * so the highlight is mirrored onto the field.
   *
   * The mirror runs from the layout effect below, so it is refreshed on every
   * render — reopening the popup republishes the row that is still active —
   * and from the observer below, which sees the rows change without a render.
   */
  const syncActiveDescendant = useCallback(() => {
    const input = inputRef.current
    const list = listRef.current
    if (!input || !list) {
      return
    }
    if (!openRef.current) {
      if (input.hasAttribute("aria-activedescendant")) {
        input.removeAttribute("aria-activedescendant")
      }
      return
    }
    const active = list.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')
    if (active?.id) {
      if (input.getAttribute("aria-activedescendant") !== active.id) {
        input.setAttribute("aria-activedescendant", active.id)
      }
    } else if (input.hasAttribute("aria-activedescendant")) {
      input.removeAttribute("aria-activedescendant")
    }
  }, [])

  /**
   * cmdk is a palette primitive: its input hardcodes `aria-expanded="true"`
   * because a palette's list is always on screen, and it overwrites `id` with
   * its own generated one. This popup closes and sits beside a visible
   * `<label htmlFor={id}>`, so both attributes are corrected after every
   * commit — React will not touch them again while its props stay the same.
   *
   * The id is the delicate part. cmdk looks the input up by the id it
   * generated whenever it wants to give the caret back, and falls back to the
   * listbox when that lookup misses. Ours replaces the id, so the lookup would
   * miss and the focus would land in the listbox: the field would lose the
   * caret in the middle of a word. The id is therefore parked on the field's
   * box, which is not focusable: cmdk's lookup lands on a dead end, the caret
   * stays in the field, and the listbox is never the focus target.
   *
   * Parking is an optimisation, not the mechanism. The filter survives a focus
   * that moved to the listbox anyway (see the blur handler below), so a
   * capture that comes back empty costs the caret, not the filter. It is
   * still read as carefully as it can be:
   *
   *  - cmdk's own label carries the id cmdk expects in its `for`, and React
   *    rewrites that attribute whenever cmdk renders a new id, so it is the
   *    one place the live id survives our own patch;
   *  - the input's own id, whenever it is not ours — a fresh input carries
   *    cmdk's id;
   *  - the id recorded on cmdk's root, which outlives the render that read it
   *    and outlives the refs of that render.
   *
   * The capture runs before the patch: afterwards the input and the label
   * both carry our id, and cmdk's is only in the root's record.
   */
  useLayoutEffect(() => {
    openRef.current = open
    const input = inputRef.current
    if (!input) {
      return
    }
    input.setAttribute("aria-expanded", open ? "true" : "false")
    const root = input.closest("[cmdk-root]")
    if (id) {
      const ownLabel = root?.querySelector<HTMLLabelElement>("label[cmdk-label]")
      const labelFor = ownLabel?.getAttribute("for") ?? null
      const inputFor = input.id || null
      const recorded = root?.getAttribute("data-cmdk-input-id") ?? null
      // Ours is not cmdk's: neither the id we use now nor the one we used
      // before are candidates.
      const ours = [id, patchedIdRef.current]
      const cmdkId = [labelFor, inputFor, recorded].find(
        (candidate) => candidate !== null && candidate !== "" && !ours.includes(candidate),
      )
      if (cmdkId) {
        cmdkInputIdRef.current = cmdkId
        // The record lives on cmdk's root, so it lives exactly as long as the
        // cmdk instance that generated the id.
        root?.setAttribute("data-cmdk-input-id", cmdkId)
      }
      const anchor = anchorRef.current
      if (anchor && cmdkInputIdRef.current && anchor.id !== cmdkInputIdRef.current) {
        anchor.id = cmdkInputIdRef.current
      }
      input.id = id
      patchedIdRef.current = id
      // cmdk's own screen-reader label still names this field; point it at the
      // real id so its `for` and the caller's visible `<label for>` agree.
      if (ownLabel) {
        ownLabel.htmlFor = id
      }
    }
    syncActiveDescendant()
  })

  useEffect(() => {
    const list = listRef.current
    if (!list) {
      return
    }
    const observer = new MutationObserver(() => syncActiveDescendant())
    observer.observe(list, {
      attributes: true,
      attributeFilter: ["aria-selected"],
      subtree: true,
    })
    return () => observer.disconnect()
  }, [syncActiveDescendant])

  /**
   * The listbox is never the focus target: the caret belongs in the field.
   * Should anything focus the list anyway — cmdk does it whenever its lookup
   * for the input misses — the focus goes back to the field, caret and all.
   * The field's own focus handler reads the return as a return, not as a
   * fresh visit, so the typed filter survives it (see the handler below).
   */
  useEffect(() => {
    const list = listRef.current
    const input = inputRef.current
    if (!list || !input) {
      return
    }
    const onListFocus = (event: Event) => {
      if (event.target !== list) {
        return
      }
      const start = input.selectionStart
      const end = input.selectionEnd
      input.focus()
      // `focus()` alone would drop the caret at the end of the word.
      if (start !== null && end !== null) {
        input.setSelectionRange(start, end)
      }
    }
    list.addEventListener("focus", onListFocus)
    return () => list.removeEventListener("focus", onListFocus)
  }, [])

  /** The popup rides the leave animation before it leaves the DOM. */
  useEffect(() => {
    if (open) {
      setPresent(true)
      return
    }
    const timer = window.setTimeout(() => setPresent(false), EXIT_MS)
    return () => window.clearTimeout(timer)
  }, [open])

  /**
   * The popup is placed from the field's box, because it lives on the body
   * and the body does not scroll with the page. It opens downwards when there
   * is room and upwards when there is not, and it never leaves the window.
   */
  useLayoutEffect(() => {
    if (!present) {
      return
    }
    const anchor = anchorRef.current
    const list = listRef.current
    if (!anchor) {
      return
    }
    const place = () => {
      const rect = anchor.getBoundingClientRect()
      const height = list?.scrollHeight ?? 0
      const width = list?.offsetWidth ?? rect.width
      const below = window.innerHeight - rect.bottom - GAP - MARGIN
      const above = rect.top - GAP - MARGIN
      const downwards = below >= height || below >= above
      const maxHeight = Math.max(96, Math.min(MAX_HEIGHT, downwards ? below : above))
      // The inline start of the field, in the direction the page reads in.
      const raw = pageDir === "rtl" ? window.innerWidth - rect.right : rect.left
      // The popup never leaves the window, whichever way it opens.
      const max = Math.max(MARGIN, window.innerWidth - width - MARGIN)
      const start = Math.min(Math.max(raw, MARGIN), max)
      setBox({
        top: downwards ? rect.bottom + GAP : undefined,
        bottom: downwards ? undefined : window.innerHeight - rect.top + GAP,
        start,
        minWidth: rect.width,
        maxHeight,
      })
    }
    place()
    window.addEventListener("scroll", place, true)
    window.addEventListener("resize", place)
    return () => {
      window.removeEventListener("scroll", place, true)
      window.removeEventListener("resize", place)
    }
  }, [present, pageDir])

  const close = () => {
    setOpen(false)
    setEditing(false)
  }

  const choose = (next: string) => {
    onValueChange(next)
    close()
  }

  const type = (next: string) => {
    setEditing(true)
    setOpen(true)
    onValueChange(next)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // An IME composition is still typing: arrows and Enter belong to it.
    if (event.nativeEvent.isComposing) {
      return
    }

    if (event.key === "Escape") {
      if (!open) {
        // Nothing to dismiss here; a surrounding dialog still gets its Escape.
        return
      }
      event.preventDefault()
      event.stopPropagation()
      close()
      return
    }

    if (event.key === "Tab") {
      // Not prevented: Tab keeps moving focus, it only leaves the popup behind.
      close()
      return
    }

    if (event.key === "Home" || event.key === "End") {
      // cmdk reads Home/End as "first/last option". In an editable field the
      // caret has to win, so the list jump is prevented and the caret moved by
      // hand (shift keeps extending a selection).
      event.preventDefault()
      const input = inputRef.current
      if (!input) {
        return
      }
      const at = event.key === "Home" ? 0 : input.value.length
      const start = input.selectionStart ?? at
      const end = input.selectionEnd ?? at
      if (event.shiftKey) {
        if (event.key === "Home") {
          input.setSelectionRange(at, end)
        } else {
          input.setSelectionRange(start, at)
        }
      } else {
        input.setSelectionRange(at, at)
      }
      return
    }

    if (!open && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      // Nothing is rendered to move over yet: open, and let the next press move.
      // Preventing the default also stops cmdk from selecting off-screen rows.
      event.preventDefault()
      setOpen(true)
      return
    }

    if (!open && event.key === "Enter") {
      // The typed value is already committed. Without this, cmdk would pick the
      // row that was active when the popup last closed.
      event.preventDefault()
    }
  }

  /**
   * Focus leaving the whole widget, not just moving inside it.
   *
   * The popup is portalled onto the body, so it is not a DOM child of the
   * field: a focus that moves into the popup looks like a departure, and the
   * widget would close and throw the typed filter away. The popup is part of
   * the widget, so both are asked, and the field keeps what was typed.
   */
  const onBlur = (event: FocusEvent<HTMLDivElement>) => {
    const next = event.relatedTarget instanceof Node ? event.relatedTarget : null
    if (next && (event.currentTarget.contains(next) || popupRef.current?.contains(next))) {
      return
    }
    close()
  }

  return (
    <div ref={widgetRef} className={cn("relative", className)} onBlur={onBlur}>
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
        {/*
          The field's box. It carries cmdk's own id for the input, which the
          layout effect above parks there: cmdk looks the input up by that id
          to give the caret back, and a box that cannot be focused is a dead
          end, so the focus never leaves the field.
        */}
        <div ref={anchorRef} dir={dir} className="relative">
          <CommandPrimitive.Input
            ref={inputRef}
            id={id}
            value={value}
            onValueChange={type}
            onFocus={(event) => {
              setOpen(true)
              // A focus that came back from the listbox is not a new visit:
              // the text the user typed, and the filter it narrowed the list
              // with, have to survive it. cmdk moves the focus there on its
              // own whenever its lookup for the input misses, and the widget
              // has to be ready for that.
              const from = event.relatedTarget instanceof Node ? event.relatedTarget : null
              if (!from || !popupRef.current?.contains(from)) {
                setEditing(false)
              }
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

        {/*
          Kept mounted so cmdk's item registry and selection survive a close;
          `hidden` takes the whole listbox out of the layout and the a11y tree.
          The portal puts it on the body, where the scrolling page cannot clip
          it, and the box places it under the field.
        */}
        {createPortal(
          <div
            ref={popupRef}
            dir={pageDir}
            data-slot="combobox-popup"
            data-state={open ? "open" : "closed"}
            hidden={!present}
            onBlur={(event) => {
              // The popup is the widget's other half, so a focus that leaves
              // it for the field is still inside the widget — and a focus that
              // leaves it for the rest of the page is not.
              const next = event.relatedTarget instanceof Node ? event.relatedTarget : null
              if (next && (event.currentTarget.contains(next) || widgetRef.current?.contains(next))) {
                return
              }
              close()
            }}
            style={{
              insetInlineStart: box?.start,
              top: box?.top,
              bottom: box?.bottom,
              minWidth: box?.minWidth,
            }}
            className={cn(
              "fixed z-50 w-44 origin-top",
              "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
              "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
              "duration-200",
            )}
          >
            <CommandPrimitive.List
              ref={listRef}
              dir={dir}
              label={t("combobox.suggestions")}
              // A pointer press must not blur the input: the click that follows
              // is what selects, and blur would close the list first.
              onMouseDown={(event) => event.preventDefault()}
              style={{ maxHeight: box?.maxHeight }}
              className={cn(
                "max-h-64 w-full overflow-y-auto rounded-card border border-border bg-popover p-1 text-popover-foreground shadow-md",
                listClassName,
              )}
            >
              {visible.map((option) => (
                <CommandPrimitive.Item
                  key={option}
                  value={option}
                  onSelect={() => choose(option)}
                  // The selected row is a solid --primary, not a tint of it:
                  // a tint of the one accent that differs from the popover
                  // measured 1.5:1 in dark and 1.3:1 in light, and a boundary
                  // below 3:1 is not a boundary (WCAG 1.4.11). The check mark
                  // follows the row's own colour, or it would vanish into it.
                  className="group flex h-8 cursor-default items-center gap-2.5 rounded-control px-2.5 font-mono text-body text-foreground outline-none select-none data-[selected=true]:bg-primary data-[selected=true]:text-primary-foreground"
                >
                  <span className="min-w-0 flex-1 truncate">{option}</span>
                  {option === value && (
                    <Check
                      aria-hidden
                      strokeWidth={1.75}
                      className="size-4 shrink-0 text-primary group-data-[selected=true]:text-primary-foreground"
                    />
                  )}
                </CommandPrimitive.Item>
              ))}

              {/* Our own row, not cmdk's `Empty`: the "use what I typed" item is
                  an item, so cmdk would always consider the list non-empty. */}
              {visible.length === 0 && (
                <div className="px-2.5 py-2 text-center text-small text-muted-foreground">
                  {t("combobox.empty")}
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
                    className="flex min-h-8 cursor-default items-center gap-2.5 rounded-control px-2.5 py-1.5 text-body text-muted-foreground outline-none select-none data-[selected=true]:bg-primary data-[selected=true]:text-primary-foreground"
                  >
                    <CornerDownLeft
                      aria-hidden
                      strokeWidth={1.75}
                      className="size-4 shrink-0"
                    />
                    <span className="min-w-0 flex-1">
                      {t("combobox.useTyped", { value: typed })}
                    </span>
                  </CommandPrimitive.Item>
                </>
              )}
            </CommandPrimitive.List>
          </div>,
          document.body,
        )}
      </CommandPrimitive>
    </div>
  )
}
