import { useCallback, useEffect, useRef, useState } from 'react';

import { virtualWindow } from './virtualWindow.ts';

/**
 * F-165: the results table renders only the rows the viewport can show.
 *
 * The maths lives in `virtualWindow.ts` (tested); this hook is only the DOM
 * around it — the scroller ref, the scroll position and the pane height.
 */
export interface VirtualRows {
  /** Attach to the scrolling element. */
  ref: React.RefObject<HTMLDivElement | null>;
  onScroll: () => void;
  /** First row to render (inclusive). */
  start: number;
  /** Last row to render (exclusive). */
  end: number;
  /** Height of the spacer above the window. */
  topPad: number;
  /** Height of the spacer below it, so the scrollbar tells the truth. */
  bottomPad: number;
  /** Total rows the scroller believes exist. */
  total: number;
}

export function useVirtualRows(
  count: number,
  rowHeight: number,
  overscan = 8,
  /** Flow height above the rows, e.g. a sticky header. */
  topInset = 0,
): VirtualRows {
  const ref = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);

  useEffect(() => {
    const element = ref.current;
    if (!element) {
      return;
    }
    const measure = () => setHeight(element.clientHeight);
    measure();
    // The pane resizes with the window, and a stale height leaves a band of
    // rows unrendered at the bottom.
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const onScroll = useCallback(() => {
    const element = ref.current;
    if (element) {
      setScrollTop(element.scrollTop);
    }
  }, []);

  const window_ = virtualWindow(count, rowHeight, scrollTop, height, overscan, topInset);
  return {
    ref,
    onScroll,
    start: window_.start,
    end: window_.end,
    topPad: window_.topPad,
    bottomPad: window_.bottomPad,
    total: count,
  };
}
