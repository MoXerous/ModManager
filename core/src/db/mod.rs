//! SQLite storage: WAL, numbered migrations, backup, version gate, api_cache.
//!
//! `Db::open` is the only entry point: pragmas, gate, backup, migrate.

pub mod cache;
pub mod download_history;
pub mod instances;
pub mod schema;

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Current schema version (`MIGRATIONS.len()`).
pub const CURRENT_VERSION: u32 = 5;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database schema v{found} is newer than supported v{supported}")]
    NewerSchema { found: u32, supported: u32 },
    #[error("wal mode not applied: {0}")]
    Wal(String),
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Db {
    conn: rusqlite::Connection,
    path: PathBuf,
}

impl Db {
    /// Filesystem path of this database (used for backup naming).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Schema version the file carries (`PRAGMA user_version`).
    ///
    /// `open()` refuses a newer file, so this is always `<= CURRENT_VERSION`;
    /// the settings screen shows it so a user can tell an upgraded profile
    /// from a fresh one when reporting a problem.
    pub fn user_version(&self) -> Result<u32, DbError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Read/write access for the row modules in this crate.
    pub(crate) fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }

    /// Page-consistent backup of the current database before a legacy import.
    ///
    /// The copy is taken *before* the first row of the import is written, so a
    /// failed or unwanted import can always be undone by hand. Named with the
    /// same no-replace discipline as the migration backup (F-136): a `.bak`
    /// that already exists is never overwritten.
    ///
    /// The backup goes to a temporary name that carries a UUID, so two calls
    /// cannot collide, and it is published by rename only after the copy has
    /// been flushed to disk. Any failure removes the temporary file.
    pub fn backup_before_legacy_import(&self) -> Result<PathBuf, DbError> {
        let temp = self
            .path
            .with_extension(format!("pre-legacy-import.{}.tmp", uuid::Uuid::new_v4()));

        let copied = (|| -> Result<(), DbError> {
            let mut dst = rusqlite::Connection::open(&temp)?;
            // The destination must be a rollback-journal database: in WAL mode
            // the copied pages could land in a `-wal` sidecar, and the rename
            // below only publishes the main file.
            dst.pragma_update(None, "journal_mode", "DELETE")?;
            let backup = rusqlite::backup::Backup::new(&self.conn, &mut dst)?;
            backup.run_to_completion(i32::MAX, std::time::Duration::from_millis(0), None)?;
            Ok(())
        })();
        if let Err(error) = copied {
            remove_sqlite_files(&temp);
            return Err(error);
        }

        // fsync through a write-capable handle: FlushFileBuffers fails on
        // read-only handles (Windows) — same rule as atomic::commit_part.
        let synced = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temp)
            .and_then(|file| file.sync_all());
        if let Err(error) = synced {
            remove_sqlite_files(&temp);
            return Err(DbError::Io(error));
        }

        let target = unique_legacy_bak(&self.path);
        if let Err(error) = std::fs::rename(&temp, &target) {
            remove_sqlite_files(&temp);
            return Err(DbError::Io(error));
        }
        Ok(target)
    }
}

/// Open (creating) `path`: pragmas, version gate, backup, migrate.
///
/// Order matters: `busy_timeout` first (even the WAL pragma can wait),
/// then the newer-schema gate **before any writing pragma**, then WAL
/// (asserted), the rest, backup of non-empty older DBs, then migrations.
pub fn open(path: &Path) -> Result<Db, DbError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut conn = rusqlite::Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > CURRENT_VERSION {
        return Err(DbError::NewerSchema {
            found: version,
            supported: CURRENT_VERSION,
        });
    }
    let journal: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    if journal.to_lowercase() != "wal" {
        return Err(DbError::Wal(journal));
    }
    conn.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
    if version < CURRENT_VERSION {
        // Backup only real upgrades (v1..CURRENT): a fresh v0 database has
        // no user content — the WAL pragma above already initialized the
        // file, so file size alone cannot gate the backup (spec §2-5).
        if version > 0 {
            backup_db(&conn, path, version)?;
        }
        schema::migrations().to_latest(&mut conn)?;
    }
    Ok(Db {
        conn,
        path: path.to_path_buf(),
    })
}

/// Page-consistent pre-migration backup via the SQLite backup API.
///
/// No outer `BEGIN IMMEDIATE`: `to_latest` owns its transaction, and a
/// nested begin would fail. Single-process callers are serialized by the
/// app layer; a racing second migrator fails closed on busy_timeout with
/// the database untouched (migrations are transactional).
fn backup_db(conn: &rusqlite::Connection, path: &Path, from: u32) -> Result<(), DbError> {
    let name = backup_name(path, from);
    let mut dst = rusqlite::Connection::open(&name)?;
    let backup = rusqlite::backup::Backup::new(conn, &mut dst)?;
    backup.run_to_completion(i32::MAX, std::time::Duration::from_millis(0), None)?;
    drop(backup);
    drop(dst);
    // fsync through a write-capable handle: FlushFileBuffers fails on
    // read-only handles (Windows) — same rule as atomic::commit_part.
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&name)?;
    f.sync_all()?;
    drop(f);
    let target = unique_bak(path, from);
    std::fs::rename(&name, &target)?;
    Ok(())
}

/// `.pre-v{from}-to-v{CURRENT}.{epoch}.{pid}.bak` next to the database.
fn backup_name(path: &Path, from: u32) -> PathBuf {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    path.with_extension(format!(
        "pre-v{from}-to-v{CURRENT_VERSION}.{epoch}.{}.tmp",
        std::process::id()
    ))
}

/// F-136 no-replace publish: never overwrite an existing `.bak`.
fn unique_bak(path: &Path, from: u32) -> PathBuf {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut n = 0u32;
    loop {
        let cand = path.with_extension(format!(
            "pre-v{from}-to-v{CURRENT_VERSION}.{epoch}.{}.{n}.bak",
            std::process::id()
        ));
        if !cand.exists() {
            return cand;
        }
        n += 1;
    }
}

/// `pre-legacy-import.<epoch>.<pid>.<n>.bak` next to the database.
fn unique_legacy_bak(path: &Path) -> PathBuf {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut n = 0u32;
    loop {
        let cand = path.with_extension(format!(
            "pre-legacy-import.{epoch}.{}.{n}.bak",
            std::process::id()
        ));
        if !cand.exists() {
            return cand;
        }
        n += 1;
    }
}

/// Removes a temporary database and the sidecars SQLite may have created next
/// to it. Best effort: the caller is already on a failure path.
fn remove_sqlite_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(db: &Db) -> Vec<String> {
        let mut stmt = db
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("tables");
        stmt.query_map([], |r| r.get(0))
            .expect("rows")
            .map(|r| r.expect("name"))
            .collect()
    }

    #[test]
    fn spec_f151_user_version_reports_the_schema_the_file_carries() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        assert_eq!(
            db.user_version().expect("version"),
            CURRENT_VERSION,
            "the file and the constant must agree or the settings screen lies"
        );
    }

    #[test]
    fn spec_f104_upgrades_v2_to_v3_preserving_rows() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let mut conn = rusqlite::Connection::open(&p).expect("raw open");
            schema::migrations()
                .to_version(&mut conn, 2)
                .expect("to v2");
            conn.execute(
                "INSERT INTO instances (id, name, path, minecraft_version, loader, created_at)
                 VALUES ('i1', 'n', 'p', '1.21', 'fabric', 1)",
                [],
            )
            .expect("seed");
        }
        let db = open(&p).expect("upgrade");
        let v: u32 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(v, CURRENT_VERSION, "open() always lands on the latest");
        let (name, source, minecraft_root, scanned_at): (String, String, String, Option<i64>) = db
            .conn
            .query_row(
                "SELECT name, source, minecraft_root, scanned_at FROM instances WHERE id = 'i1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("row preserved");
        assert_eq!(name, "n");
        assert_eq!(source, "managed", "pre-existing rows default to managed");
        assert_eq!(minecraft_root, "");
        assert_eq!(scanned_at, None, "never scanned is NULL, not 0");
    }

    #[test]
    fn spec_f133_upgrades_v3_to_current_preserving_rows() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let mut conn = rusqlite::Connection::open(&p).expect("raw open");
            schema::migrations()
                .to_version(&mut conn, 3)
                .expect("to v3");
            conn.execute(
                "INSERT INTO instances (id, name, path, minecraft_version, loader, created_at)
                 VALUES ('i1', 'n', 'p', '1.21', 'fabric', 1)",
                [],
            )
            .expect("seed instance");
            conn.execute(
                "INSERT INTO installed_mods
                 (id, instance_id, slug, version_id, file_name, installed_at)
                 VALUES ('m1', 'i1', 's', 'v', 'f.jar', 1)",
                [],
            )
            .expect("seed mod");
        }
        let db = open(&p).expect("upgrade");
        let v: u32 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(v, CURRENT_VERSION);
        let (id, provider, project_id, sha1): (String, String, String, Option<String>) = db
            .conn
            .query_row(
                "SELECT id, provider, project_id, sha1 FROM installed_mods WHERE id = 'm1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("row preserved");
        assert_eq!(id, "m1");
        assert_eq!(provider, "", "no backfill: provider stays empty");
        assert_eq!(project_id, "", "no backfill: project_id stays empty");
        assert_eq!(sha1, None, "no verified SHA-1 for a legacy row");
    }

    #[test]
    fn spec_f133_v4_identity_is_per_instance_and_provider() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        for id in ["i1", "i2"] {
            db.conn
                .execute(
                    "INSERT INTO instances (id, name, path, minecraft_version, loader, created_at)
                     VALUES (?1, 'n', ?1, '1.21', 'fabric', 1)",
                    rusqlite::params![id],
                )
                .expect("seed instance");
        }
        let insert = |id: &str, instance: &str, provider: &str, project: &str| {
            db.conn.execute(
                "INSERT INTO installed_mods
                 (id, instance_id, slug, version_id, file_name, installed_at, provider, project_id)
                 VALUES (?1, ?2, 's', 'v', 'f.jar', 1, ?3, ?4)",
                rusqlite::params![id, instance, provider, project],
            )
        };
        insert("m1", "i1", "modrinth", "P1").expect("first row");
        let err = insert("m2", "i1", "modrinth", "P1").expect_err("same identity");
        let text = err.to_string();
        assert!(
            text.contains("UNIQUE"),
            "expected a UNIQUE failure, got: {text}"
        );
        insert("m3", "i1", "curseforge", "P1").expect("same project under another provider");
        insert("m4", "i2", "modrinth", "P1").expect("same project in another instance");
    }

    #[test]
    fn spec_f133_v4_legacy_empty_identity_rows_are_not_blocked() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let mut conn = rusqlite::Connection::open(&p).expect("raw open");
            schema::migrations()
                .to_version(&mut conn, 3)
                .expect("to v3");
            conn.execute(
                "INSERT INTO instances (id, name, path, minecraft_version, loader, created_at)
                 VALUES ('i1', 'n', 'p', '1.21', 'fabric', 1)",
                [],
            )
            .expect("seed instance");
            for id in ["m1", "m2"] {
                conn.execute(
                    "INSERT INTO installed_mods
                     (id, instance_id, slug, version_id, file_name, installed_at)
                     VALUES (?1, 'i1', 's', 'v', 'f.jar', 1)",
                    rusqlite::params![id],
                )
                .expect("seed legacy mod");
            }
        }
        let db = open(&p).expect("the partial index must let legacy rows through");
        let n: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM installed_mods
                 WHERE instance_id = 'i1' AND provider = '' AND project_id = ''",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 2, "both empty-identity rows survive");
    }

    #[test]
    fn spec_f104_source_rejects_unknown_value() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        for good in ["managed", "manual", "adopted"] {
            db.conn
                .execute(
                    "INSERT INTO instances
                     (id, name, path, minecraft_version, loader, created_at, source)
                     VALUES (?1, 'n', ?1, '1.21', 'fabric', 1, ?2)",
                    rusqlite::params![format!("id-{good}"), good],
                )
                .expect("allowed source");
        }
        let err = db
            .conn
            .execute(
                "INSERT INTO instances
                 (id, name, path, minecraft_version, loader, created_at, source)
                 VALUES ('bad', 'n', 'bad', '1.21', 'fabric', 1, 'linked_modpack')",
                [],
            )
            .expect_err("CHECK must reject an unlisted source");
        let text = err.to_string();
        assert!(
            text.contains("CHECK") && text.contains("source"),
            "expected a CHECK failure naming source, got: {text}"
        );
    }

    #[test]
    fn spec_f104_path_unique_is_case_sensitive_so_writes_canonicalize() {
        // Documents why the repository canonicalizes before insert: SQLite's
        // UNIQUE is case-sensitive, so two spellings of one NTFS path would
        // otherwise register the same folder twice.
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let a = dir.path().join("Case");
        std::fs::create_dir_all(&a).expect("mkdir");
        for (id, spelling) in [
            ("i1", a.to_string_lossy().into_owned()),
            ("i2", "c:\\users\\A".to_string()),
        ] {
            db.conn
                .execute(
                    "INSERT INTO instances (id, name, path, minecraft_version, loader, created_at)
                     VALUES (?1, 'n', ?2, '1.21', 'fabric', 1)",
                    rusqlite::params![id, spelling],
                )
                .expect("both spellings are accepted by the database");
        }
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM instances", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 2, "the schema does not dedupe; the writer must");
    }

    #[test]
    fn spec_f133_creates_schema_then_latest() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let t = tables(&db);
        for want in ["instances", "installed_mods", "downloads", "api_cache"] {
            assert!(t.contains(&want.to_string()), "missing {want}");
        }
        let v: u32 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(v, CURRENT_VERSION);
        // Foreign keys hold on the fresh schema.
        db.conn
            .execute_batch("PRAGMA foreign_key_check")
            .expect("fk clean");
        // v4 provider identity, and `slug` kept (ADR 0004 §5-4).
        let mut stmt = db
            .conn
            .prepare("PRAGMA table_info(installed_mods)")
            .expect("table_info");
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .expect("rows")
            .map(|r| r.expect("column"))
            .collect();
        for want in ["slug", "provider", "project_id", "sha1"] {
            assert!(cols.contains(&want.to_string()), "missing column {want}");
        }
    }

    #[test]
    fn spec_f140_wal_busy_fk_set_and_persistent() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let db = open(&p).expect("open");
            let journal: String = db
                .conn
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .expect("journal");
            assert_eq!(journal, "wal");
            let timeout: i64 = db
                .conn
                .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
                .expect("timeout");
            assert_eq!(timeout, 5000);
            let fk: i64 = db
                .conn
                .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                .expect("fk");
            assert_eq!(fk, 1);
        }
        // WAL persists across reopen (it is a database property).
        let db = open(&p).expect("reopen");
        let journal: String = db
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("journal");
        assert_eq!(journal, "wal");
    }

    #[test]
    fn spec_f133_upgrades_v1_to_v2_preserving_rows() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let mut conn = rusqlite::Connection::open(&p).expect("raw open");
            schema::migrations()
                .to_version(&mut conn, 1)
                .expect("to v1");
            conn.execute(
                "INSERT INTO instances (id, name, path, minecraft_version, loader, created_at)
                 VALUES ('i1', 'n', 'p', '1.21', 'fabric', 1)",
                [],
            )
            .expect("seed");
        }
        let db = open(&p).expect("upgrade");
        let v: u32 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(v, CURRENT_VERSION, "open() always lands on the latest");
        let name: String = db
            .conn
            .query_row("SELECT name FROM instances WHERE id = 'i1'", [], |r| {
                r.get(0)
            })
            .expect("row preserved");
        assert_eq!(name, "n");
    }

    #[test]
    fn spec_f135_backup_created_before_migrate() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let mut conn = rusqlite::Connection::open(&p).expect("raw open");
            schema::migrations()
                .to_version(&mut conn, 1)
                .expect("to v1");
        }
        open(&p).expect("upgrade");
        let baks: Vec<_> = std::fs::read_dir(dir.path())
            .expect("ls")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert!(
            baks.iter().any(|n| n.to_string_lossy().ends_with(".bak")),
            "expected a .bak next to {p:?}, found {baks:?}"
        );
    }

    #[test]
    fn spec_f135_backup_created_before_v4_upgrade() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let mut conn = rusqlite::Connection::open(&p).expect("raw open");
            schema::migrations()
                .to_version(&mut conn, 3)
                .expect("to v3");
        }
        open(&p).expect("upgrade");
        let baks: Vec<_> = std::fs::read_dir(dir.path())
            .expect("ls")
            .map(|e| e.expect("entry").file_name())
            .collect();
        // The name carries the version being left and the one being reached;
        // spelling the reached version here would make every later migration
        // edit a test about v4.
        let expected = format!(".pre-v3-to-v{CURRENT_VERSION}.");
        assert!(
            baks.iter().any(|n| n.to_string_lossy().contains(&expected)),
            "expected a {expected} .bak next to {p:?}, found {baks:?}"
        );
    }

    #[test]
    fn spec_f134_refuses_newer_schema_byte_identical() {
        let dir = crate::test_tempdir();
        let p = dir.path().join("t.db");
        {
            let conn = rusqlite::Connection::open(&p).expect("raw open");
            conn.execute_batch("PRAGMA user_version = 99")
                .expect("stamp");
        }
        let before = std::fs::read(&p).expect("read");
        assert!(matches!(
            open(&p),
            Err(DbError::NewerSchema {
                found: 99,
                supported: CURRENT_VERSION
            })
        ));
        assert_eq!(std::fs::read(&p).expect("read"), before, "byte-identical");
        let baks: Vec<_> = std::fs::read_dir(dir.path())
            .expect("ls")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert!(
            !baks.iter().any(|n| n.to_string_lossy().ends_with(".bak")),
            "no backup on refusal"
        );
    }

    #[test]
    fn spec_f133_cascade_and_set_null() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        db.conn
            .execute(
                "INSERT INTO instances (id, name, path, minecraft_version, loader, created_at)
                 VALUES ('i1', 'n', 'p', '1.21', 'fabric', 1)",
                [],
            )
            .expect("seed");
        db.conn
            .execute(
                "INSERT INTO installed_mods (id, instance_id, slug, version_id, file_name, installed_at)
                 VALUES ('m1', 'i1', 's', 'v', 'f.jar', 1)",
                [],
            )
            .expect("seed");
        db.conn
            .execute(
                "INSERT INTO downloads (id, instance_id, url, dest, status, created_at)
                 VALUES ('d1', 'i1', 'u', 'd', 'done', 1)",
                [],
            )
            .expect("seed");
        db.conn
            .execute("DELETE FROM instances WHERE id = 'i1'", [])
            .expect("delete");
        let mods: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM installed_mods", [], |r| r.get(0))
            .expect("count");
        assert_eq!(mods, 0, "mods cascade");
        let inst: Option<String> = db
            .conn
            .query_row(
                "SELECT instance_id FROM downloads WHERE id = 'd1'",
                [],
                |r| r.get(0),
            )
            .expect("row");
        assert_eq!(inst, None, "downloads set null");
    }
}
