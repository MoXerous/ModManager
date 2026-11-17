import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { afterRescan, isForInstance, markStale } from './staleness.ts';

describe('the content staleness hint (F-168)', () => {
  it('spec_f168_a_stale_event_for_the_open_instance_shows_the_hint', () => {
    assert.equal(
      isForInstance('D:/instances/fabric', 'D:/instances/fabric'),
      true,
    );
    // The event's root is what Rust canonicalized; casing on Windows is not
    // identity.
    assert.equal(isForInstance('D:/Instances/Fabric', 'D:/instances/fabric'), true);
  });

  it('spec_f168_a_stale_event_for_another_instance_is_ignored', () => {
    assert.equal(isForInstance('D:/instances/other', 'D:/instances/fabric'), false);
  });

  it('spec_f168_a_refresh_clears_the_hint', () => {
    const state = markStale({ stale: false });
    assert.equal(state.stale, true, 'an event marks the content stale');
    assert.equal(afterRescan(state, true).stale, false, 'a refresh clears the hint');
  });

  it('spec_f168_a_failed_rescan_keeps_the_stale_hint', () => {
    // The hint is cleared by a SUCCESSFUL rescan only: a scan that failed has
    // not re-read the disk, so the hint keeps its grounds — and the screen
    // shows the failure next to it.
    const stale = markStale({ stale: false });
    assert.equal(
      afterRescan(stale, false).stale,
      true,
      'a failed rescan must not clear the hint',
    );
    assert.equal(
      afterRescan(stale, true).stale,
      false,
      'a successful rescan still clears it',
    );
  });
});
