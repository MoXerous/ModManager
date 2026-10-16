//! Numbered migrations as inline SQL (F-133).
//!
//! M1: the four tables. M2: the `api_cache.fetched_at` index (TTL lookups).

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

pub fn migrations() -> rusqlite_migration::Migrations<'static> {
    use rusqlite_migration::M;
    rusqlite_migration::Migrations::new(vec![M::up(M1), M::up(M2)])
}
