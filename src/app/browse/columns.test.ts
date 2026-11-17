import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { ProjectDto } from '../../bindings/ProjectDto.ts';
import {
  COLUMN_IDS,
  dateCell,
  envCell,
  projectKey,
  sortRows,
  statusCell,
} from './columns.ts';

function row(overrides: Partial<ProjectDto>): ProjectDto {
  return {
    id: 'P7dR8mSH',
    slug: 'fabric-api',
    title: 'Fabric API',
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
    webUrl: 'https://modrinth.com/mod/fabric-api',
    providerId: 'modrinth',
    ...overrides,
  };
}

describe('the 13-column results table (F-165)', () => {
  it('spec_f165_the_table_has_the_old_apps_thirteen_columns_in_order', () => {
    // The old `DISCOVER_RESULT_HEADERS`: Select, Fav, Type, Title, Slug,
    // Status, Version, Date, Env, Downloads, Followers, Categories, URL.
    assert.deepEqual(COLUMN_IDS, [
      'select',
      'fav',
      'type',
      'title',
      'slug',
      'status',
      'version',
      'date',
      'env',
      'downloads',
      'followers',
      'categories',
      'url',
    ]);
    assert.equal(COLUMN_IDS.length, 13);
  });

  it('spec_f165_sort_orders_the_rows_by_the_active_column_without_reloading', () => {
    const rows = [
      row({ id: 'a', followers: 5 }),
      row({ id: 'b', followers: 900 }),
      row({ id: 'c', followers: 120 }),
    ];
    const desc = sortRows(rows, 'followers', 'desc').map((r) => r.id);
    const asc = sortRows(rows, 'followers', 'asc').map((r) => r.id);
    assert.deepEqual(desc, ['b', 'c', 'a']);
    assert.deepEqual(asc, ['a', 'c', 'b']);
  });

  it('spec_f165_sort_is_stable_and_moves_empty_cells_last', () => {
    // A row CurseForge gave no date for must not land on top of a "newest
    // first" sort just because "" compares small.
    const rows = [
      row({ id: 'a', published: '' }),
      row({ id: 'b', published: '2025-01-01T00:00:00Z' }),
      row({ id: 'c', published: '2024-06-01T00:00:00Z' }),
    ];
    const newestFirst = sortRows(rows, 'date', 'desc').map((r) => r.id);
    assert.deepEqual(newestFirst, ['b', 'c', 'a']);
    const oldestFirst = sortRows(rows, 'date', 'asc').map((r) => r.id);
    assert.deepEqual(oldestFirst, ['c', 'b', 'a']);
  });

  it('spec_f165_the_env_cell_names_both_sides_or_stays_unknown', () => {
    assert.equal(envCell(row({})), 'C:required / S:optional');
    // CurseForge states no side; "unknown", never a guessed side.
    assert.equal(envCell(row({ clientSide: '', serverSide: '' })), null);
  });

  it('spec_f165_the_date_cell_is_the_day_of_the_published_timestamp', () => {
    assert.equal(dateCell(row({})), '2024-03-04');
    assert.equal(dateCell(row({ published: '' })), null);
  });

  it('spec_f165_the_status_cell_says_whether_a_version_resolved', () => {
    assert.equal(statusCell(row({})), 'resolved');
    assert.equal(statusCell(row({ latestVersion: null })), 'unknown');
  });

  it('spec_f165_a_row_identity_is_its_provider_and_its_id', () => {
    assert.equal(projectKey(row({})), 'modrinth:P7dR8mSH');
    assert.equal(
      projectKey(row({ providerId: 'curseforge', id: '238222' })),
      'curseforge:238222',
    );
  });
});
