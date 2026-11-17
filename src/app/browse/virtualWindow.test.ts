import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { virtualWindow } from './virtualWindow.ts';

describe('the table virtualiser (F-165)', () => {
  it('spec_f165_the_virtualiser_renders_only_the_visible_window', () => {
    // A 500px pane of 40px rows shows ~13 rows; the window may not be the
    // whole catalogue.
    const w = virtualWindow(1000, 40, 0, 500, 0);
    assert.equal(w.start, 0);
    assert.ok(w.end <= 15, `end must stay small, got ${w.end}`);
  });

  it('spec_f165_scrolling_moves_the_window_and_keeps_an_overscan_band', () => {
    const w = virtualWindow(1000, 40, 4000, 500, 8);
    // Row 100 is at the top of the viewport; the band starts 8 rows earlier.
    assert.equal(w.start, 92);
    assert.equal(w.topPad, 92 * 40);
  });

  it('spec_f165_the_spacers_size_the_scrollbar_to_the_whole_list', () => {
    const w = virtualWindow(1000, 40, 4000, 500, 8);
    assert.equal(w.topPad + (w.end - w.start) * 40 + w.bottomPad, 1000 * 40);
  });

  it('spec_f165_a_short_list_renders_every_row_with_no_bottom_spacer', () => {
    const w = virtualWindow(5, 40, 0, 500, 8);
    assert.equal(w.start, 0);
    assert.equal(w.end, 5);
    assert.equal(w.bottomPad, 0);
  });

  it('spec_f165_the_window_starts_under_the_sticky_header', () => {
    // The header sits in flow above the rows, so the scroller's scrollTop
    // counts it too: with the inset, scrollTop 1040 means row 25 at the top
    // (effective scrollTop 1008), not row 26.
    const w = virtualWindow(1000, 40, 1040, 500, 8, 32);
    assert.equal(w.start, 17);
    assert.equal(w.topPad, 17 * 40);
  });
});
