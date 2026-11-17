import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { isForInstance, markStale, refreshed } from './staleness.ts';

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
    assert.equal(refreshed(state).stale, false, 'a refresh clears the hint');
  });
});
