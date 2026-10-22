//! Numbered migrations as inline SQL (F-133).
//!
//! M1: the four tables. M2: the `api_cache.fetched_at` index (TTL lookups).
//! M3: F-104 `source`, plus `minecraft_root` and `scanned_at` for the
//! Instances screen (F-101/F-103).

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

pub fn migrations() -> rusqlite_migration::Migrations<'static> {
    use rusqlite_migration::M;
    rusqlite_migration::Migrations::new(vec![M::up(M1), M::up(M2), M::up(M3)])
}
