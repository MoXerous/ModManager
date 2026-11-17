import assert from 'node:assert/strict';
import { describe, it, mock } from 'node:test';

import type { ProjectDto } from '../../bindings/ProjectDto.ts';
import {
  PAGING_DEBOUNCE_MS,
  RequestGate,
  SearchCache,
  debounce,
  nearEnd,
  queryKeyOf,
} from './paging.ts';

function hit(id: string): ProjectDto {
  return {
    id,
    slug: id,
    title: id,
    description: 'd',
    author: 'a',
    projectType: 'mod',
    categories: [],
    downloads: 1,
    followers: 1,
    iconUrl: null,
    published: '2024-01-01T00:00:00Z',
    clientSide: 'required',
    serverSide: 'optional',
    latestVersion: '1.0.0',
    webUrl: null,
    providerId: 'modrinth',
  };
}

describe('scroll-triggered paging (F-166)', () => {
  it('spec_f166_a_visible_window_near_the_end_asks_for_the_next_page', () => {
    // The virtualiser reports the visible window; near its end is where the
    // old app preloaded the next page.
    assert.equal(nearEnd(90, 100), true);
    assert.equal(nearEnd(100, 100), true);
    assert.equal(nearEnd(50, 100), false);
  });

  it('spec_f166_a_burst_of_scroll_events_coalesces_to_the_old_apps_120ms', () => {
    mock.timers.enable({ apis: ['setTimeout'] });
    try {
      let calls = 0;
      const debounced = debounce(() => {
        calls += 1;
      }, PAGING_DEBOUNCE_MS);
      assert.equal(PAGING_DEBOUNCE_MS, 120);
      debounced();
      debounced();
      debounced();
      debounced();
      assert.equal(calls, 0, 'a burst must not start four requests');
      mock.timers.tick(119);
      assert.equal(calls, 0, 'the burst fires only after the 120ms window');
      mock.timers.tick(1);
      assert.equal(calls, 1, 'the burst collapses to one call');
    } finally {
      mock.timers.reset();
    }
  });

  it('spec_f166_a_superseded_request_never_renders', () => {
    // Tauri invoke() has no AbortSignal, so the guard is a generation: a
    // response whose request was replaced must be dropped, not rendered.
    const gate = new RequestGate();
    const first = gate.next();
    const second = gate.next();
    assert.equal(gate.isCurrent(first), false, 'a superseded request is not current');
    assert.equal(gate.isCurrent(second), true, 'the newest request is current');
    assert.equal(gate.isCurrent(gate.next()), true, 'a newer generation replaces the last');
    assert.equal(gate.isCurrent(second), false, 'the previous generation is now stale');
  });

  it('spec_f166_a_cache_hit_or_a_reset_invalidates_the_in_flight_response', () => {
    // The race: a page-2 request is in flight when the user switches provider
    // (or edits a facet, or hits a cached page — every path that resets the
    // rows without sending a request). The reset alone does not end the
    // in-flight request, so it must be invalidated explicitly or its late
    // answer renders over the new results.
    const gate = new RequestGate();
    const inFlight = gate.next();
    gate.invalidate();
    assert.equal(
      gate.isCurrent(inFlight),
      false,
      'a response arriving after a reset must be dropped',
    );
  });

  it('spec_f166_returning_to_the_same_query_key_reads_the_cache', () => {
    const cache = new SearchCache();
    const key = queryKeyOf({
      query: 'fabric',
      provider: 'modrinth',
      gameVersion: '1.21',
      loader: 'fabric',
    });
    cache.put(key, 0, { hits: [hit('a')], total: 1, loaded: 1 });
    const entry = cache.get(key, 0);
    assert.equal(entry?.total, 1, 'a cache hit answers without a network round trip');
    assert.deepEqual(entry?.hits.map((h) => h.id), ['a']);
  });

  it('spec_f166_the_page_is_part_of_the_cache_key', () => {
    const base = queryKeyOf({ query: 'fabric', provider: 'modrinth' });
    const cache = new SearchCache();
    cache.put(base, 0, { hits: [hit('a')], total: 2, loaded: 1 });
    cache.put(base, 1, { hits: [hit('b')], total: 2, loaded: 2 });
    assert.deepEqual(cache.get(base, 0)?.hits.map((h) => h.id), ['a']);
    assert.deepEqual(cache.get(base, 1)?.hits.map((h) => h.id), ['b']);
  });

  it('spec_f166_a_changed_facet_is_a_different_query_key', () => {
    const fabric = queryKeyOf({ query: 'sodium', provider: 'modrinth', loader: 'fabric' });
    const forge = queryKeyOf({ query: 'sodium', provider: 'modrinth', loader: 'forge' });
    assert.notEqual(fabric, forge, 'a changed filter is a new question');
    // The provider is part of it too: one catalogue's answers are not the
    // other's.
    assert.notEqual(
      queryKeyOf({ query: 'sodium', provider: 'modrinth' }),
      queryKeyOf({ query: 'sodium', provider: 'curseforge' }),
    );
  });

  it('spec_f166_a_modpack_search_never_collides_with_a_mod_search', () => {
    // F-166: the project type is part of the question. The page states it
    // explicitly on both doors — the wire and this key — and this lock keeps
    // the key's own default aligned with the wire's: an omitted type and an
    // explicit "mod" must be one question, and "modpack" another.
    const mod = queryKeyOf({
      query: 'sodium',
      provider: 'modrinth',
      projectType: 'mod',
    });
    const modpack = queryKeyOf({
      query: 'sodium',
      provider: 'modrinth',
      projectType: 'modpack',
    });
    assert.notEqual(mod, modpack, 'a future modpack search must not read the mod cache');
    assert.equal(
      queryKeyOf({ query: 'sodium', provider: 'modrinth' }),
      mod,
      'an omitted type is the mod search, never something else',
    );
  });

  it('spec_f166_a_manual_refresh_invalidates_the_cache', () => {
    const cache = new SearchCache();
    const key = queryKeyOf({ query: 'fabric', provider: 'modrinth' });
    cache.put(key, 0, { hits: [hit('a')], total: 1, loaded: 1 });
    cache.clear();
    assert.equal(cache.get(key, 0), null, 'a refresh must not serve the old page');
  });
});
