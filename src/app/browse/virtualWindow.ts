/**
 * F-165: the window of table rows the viewport needs.
 *
 * The old table was a `QTableView`: thousands of catalogue rows at a fixed
 * row height. A plain `.map()` over a page that grows would mount every cell
 * and make scrolling the thing the user feels. This is the whole virtualiser
 * — a fixed row height and a window over the scroller — which is smaller
 * than a dependency and has no behaviour of its own to get wrong.
 *
 * Rows are fixed height by design: a two-line cell would need measurement,
 * and the columns are single-line on purpose (the old table truncated too).
 */
export interface VirtualWindow {
  /** First row to render (inclusive). */
  start: number;
  /** Last row to render (exclusive). */
  end: number;
  /** Height of the spacer above the window. */
  topPad: number;
  /** Height of the spacer below it, so the scrollbar tells the truth. */
  bottomPad: number;
}

export function virtualWindow(
  count: number,
  rowHeight: number,
  scrollTop: number,
  height: number,
  overscan: number,
  /** Flow height above the rows (a sticky header); the scroller counts it. */
  topInset = 0,
): VirtualWindow {
  const effectiveScroll = Math.max(0, scrollTop - topInset);
  const effectiveHeight = Math.max(0, height - topInset);
  const first = Math.floor(effectiveScroll / rowHeight);
  const visible = effectiveHeight > 0 ? Math.ceil(effectiveHeight / rowHeight) + 1 : 0;
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, start + visible + overscan * 2);
  return {
    start,
    end,
    topPad: start * rowHeight,
    bottomPad: Math.max(0, (count - end) * rowHeight),
  };
}
