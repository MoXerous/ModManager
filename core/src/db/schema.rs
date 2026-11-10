//! Numbered migrations as inline SQL (F-133).
//!
//! M1: the four tables. M2: the `api_cache.fetched_at` index (TTL lookups).
//! M3: F-104 `source`, plus `minecraft_root` and `scanned_at` for the
//! Instances screen (F-101/F-103). M4: provider identity for installed mods
//! (ADR 0004 §5-4). M5: the download history (F-152/F-153).

pub const M1: &str = "
CREATE TABLE instances (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    minecraft_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE installed_mods (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
    slug TEXT NOT NULL,
    version_id TEXT NOT NULL,
    file_name TEXT NOT NULL,
    sha512 TEXT,
    source TEXT NOT NULL DEFAULT 'managed',
    installed_at INTEGER NOT NULL
);
CREATE TABLE downloads (
    id TEXT PRIMARY KEY,
    instance_id TEXT REFERENCES instances(id) ON DELETE SET NULL,
    url TEXT NOT NULL,
    dest TEXT NOT NULL,
    expected_size INTEGER,
    sha512 TEXT,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE api_cache (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);
";

pub const M2: &str = "
CREATE INDEX idx_api_cache_fetched ON api_cache(fetched_at);
CREATE INDEX idx_mods_instance ON installed_mods(instance_id);
";

/// F-104 `source`, plus what the Instances screen needs.
///
/// `ADD COLUMN` accepts NOT NULL with a non-null default *and* a CHECK
/// constraint, and validates it against every existing row — so the table
/// keeps its `minecraft_version`/`loader` NOT NULL guarantees without the
/// 12-step rebuild whose `DROP TABLE` would cascade into `installed_mods` and
/// `downloads`.
///
/// Unknown game version or loader is the sentinel `'unknown'`, not NULL: the
/// reference app already reserves string values for "cannot be detected"
/// (`NON_PROFILE_LOADER_VALUES`), and one sentinel keeps the Rust side to
/// `&str` instead of `Option` in every query.
pub const M3: &str = "
ALTER TABLE instances ADD COLUMN source TEXT NOT NULL DEFAULT 'managed'
    CHECK (source IN ('managed','manual','adopted'));
ALTER TABLE instances ADD COLUMN minecraft_root TEXT NOT NULL DEFAULT '';
ALTER TABLE instances ADD COLUMN scanned_at INTEGER;
CREATE INDEX idx_instances_source ON instances(source);
";

/// Provider identity for installed mods (ADR 0004 §5-4).
///
/// `slug` stays: it is `NOT NULL` today, and the ADR leaves dropping or
/// renaming it explicitly unverified. Existing rows keep `''`/`''` — there is
/// no backfill, because `provider` must never be filled from `slug` later and
/// nothing else is a reliable source.
///
/// The unique index is **partial** on purpose. Legacy rows all carry
/// `''`/`''`; a full unique index would make this migration fail on any
/// instance that already holds two legacy rows. The predicate enforces the
/// identity for every new row (`project_id` is always written) while letting
/// old rows survive.
///
/// `sha1` stays nullable: a legacy row has no verified SHA-1 recorded.
pub const M4: &str = "
ALTER TABLE installed_mods ADD COLUMN provider TEXT NOT NULL DEFAULT '';
ALTER TABLE installed_mods ADD COLUMN project_id TEXT NOT NULL DEFAULT '';
ALTER TABLE installed_mods ADD COLUMN sha1 TEXT;
CREATE UNIQUE INDEX idx_mods_identity
    ON installed_mods(instance_id, provider, project_id)
    WHERE project_id <> '';
";

/// F-152/F-153: one row per install outcome, and the indexes that make the
/// screen's two most common questions cheap.
///
/// The reference schema has no foreign key to the profile, and this one has
/// none to the instance either: deleting an instance must not erase what was
/// downloaded into it, which is exactly what a `ON DELETE CASCADE` would do
/// (the reference app's own data-model notes call that out as a defect).
///
/// `created_at` is an epoch second like every other timestamp here, not the
/// reference's `TEXT DEFAULT CURRENT_TIMESTAMP`: one time type per database
/// beats matching a spelling.
///
/// The indexes cover `ORDER BY created_at DESC, id DESC` (the default screen)
/// and `WHERE status = ?` within that order. The text search is a `LIKE`
/// by construction, so no b-tree can serve it; what keeps it bounded is that
/// the walk starts at the newest row and stops at the requested limit, so a
/// search reads the newest matches rather than the whole table.
pub const M5: &str = "
CREATE TABLE download_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL,
    project_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    version_id TEXT,
    version_number TEXT,
    game_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    filename TEXT NOT NULL,
    file_sha1 TEXT,
    destination_path TEXT,
    status TEXT NOT NULL
        CHECK (status IN ('success','skipped','failed','cancelled')),
    message TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_download_history_created
    ON download_history(created_at DESC, id DESC);
CREATE INDEX idx_download_history_status
    ON download_history(status, created_at DESC);
";

pub fn migrations() -> rusqlite_migration::Migrations<'static> {
    use rusqlite_migration::M;
    rusqlite_migration::Migrations::new(vec![M::up(M1), M::up(M2), M::up(M3), M::up(M4), M::up(M5)])
}
