import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { ProjectDto } from '../../bindings/ProjectDto.ts';
import {
  clearSelection,
  getBrowseSelection,
  removeFavourite,
  removeSelected,
  toggleFavourite,
  toggleSelected,
} from './selectionStore.ts';
import { projectKey } from './columns.ts';

function project(id: string, providerId = 'modrinth'): ProjectDto {
  return {
    id,
    slug: id,
    title: `Title ${id}`,
    description: 'd',
    author: 'modmuss50',
    projectType: 'mod',
    categories: ['fabric'],
    downloads: 100,
    followers: 42,
    iconUrl: null,
    published: '2024-03-04T00:00:00Z',
    clientSide: 'required',
    serverSide: 'optional',
    latestVersion: '1.0.0',
    webUrl: null,
    providerId,
  };
}

/** The store is module state shared by the file's tests; reset it through
 *  its own public API so each test starts from empty lists. */
function resetStore(): void {
  clearSelection();
  for (const entry of [...getBrowseSelection().favourites]) {
    removeFavourite(entry.key);
  }
}

describe('selection and favourites across searches (F-165, F-164)', () => {
  it('spec_f165_toggling_a_row_selects_it_and_toggling_again_releases_it', () => {
    clearSelection();
    toggleSelected(project('P7dR8mSH'));
    assert.deepEqual(
      getBrowseSelection().selected.map((entry) => entry.key),
      ['modrinth:P7dR8mSH'],
    );
    toggleSelected(project('P7dR8mSH'));
    assert.deepEqual(getBrowseSelection().selected, []);
  });

  it('spec_f165_clear_selection_releases_every_row_at_once', () => {
    clearSelection();
    toggleSelected(project('a'));
    toggleSelected(project('b', 'curseforge'));
    toggleSelected(project('c'));
    clearSelection();
    assert.deepEqual(getBrowseSelection().selected, []);
  });

  it('spec_f165_the_favourite_toggle_is_independent_of_the_selection', () => {
    clearSelection();
    toggleFavourite(project('P7dR8mSH'));
    assert.deepEqual(getBrowseSelection().selected, []);
    assert.deepEqual(
      getBrowseSelection().favourites.map((entry) => entry.key),
      ['modrinth:P7dR8mSH'],
    );
    // The old table kept the two check states in separate columns with
    // separate signals; favouriting must not select.
  });

  it('spec_f164_the_two_providers_never_share_an_identity', () => {
    clearSelection();
    toggleSelected(project('238222', 'modrinth'));
    toggleSelected(project('238222', 'curseforge'));
    assert.equal(getBrowseSelection().selected.length, 2);
    assert.equal(projectKey(project('238222', 'curseforge')), 'curseforge:238222');
  });

  it('spec_f164_a_favourite_can_be_removed_from_the_favourites_tab', () => {
    resetStore();
    toggleFavourite(project('a'));
    toggleFavourite(project('b'));
    removeFavourite('modrinth:a');
    assert.deepEqual(
      getBrowseSelection().favourites.map((entry) => entry.key),
      ['modrinth:b'],
    );
  });

  it('spec_f164_a_selected_row_can_be_removed_from_the_selected_tab', () => {
    resetStore();
    toggleSelected(project('a'));
    toggleSelected(project('b'));
    removeSelected('modrinth:a');
    assert.deepEqual(
      getBrowseSelection().selected.map((entry) => entry.key),
      ['modrinth:b'],
    );
  });

  it('spec_f164_the_lists_keep_their_rows_across_a_new_search', () => {
    // "Selected compatible projects are preserved across searches and pages"
    // (the old app's promise on its Selected tab) — the store is module
    // state, not component state, so a BrowsePage unmount cannot wipe it.
    resetStore();
    toggleSelected(project('a'));
    toggleFavourite(project('z'));
    toggleSelected(project('a'));
    assert.deepEqual(getBrowseSelection().selected, []);
    assert.deepEqual(
      getBrowseSelection().favourites.map((entry) => entry.key),
      ['modrinth:z'],
    );
  });

  it('spec_f164_a_selected_row_keeps_the_project_it_was_ticked_from', () => {
    // A row preserved across searches must still render its title even after
    // its row has left the results, so the entry carries the snapshot.
    clearSelection();
    toggleSelected(project('gone'));
    const [entry] = getBrowseSelection().selected;
    assert.equal(entry.project.title, 'Title gone');
  });
});
