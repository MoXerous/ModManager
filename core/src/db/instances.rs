//! Instance rows: the `instances` table as typed Rust values.
//!
//! `core::db` owns the schema; this module owns reading and writing rows.
//! Nothing here touches the filesystem except `insert`, which must resolve the
//! caller's path before storing it: SQLite's `UNIQUE` is case-sensitive, so
//! `C:\Users\a` and `c:\users\A` are two spellings of one NTFS folder and would
//! otherwise register it twice.

use std::path::PathBuf;

use thiserror::Error;

use super::{Db, DbError};

/// Sentinel for "the user never said and we could not detect it".
///
/// A string rather than NULL because the columns are `NOT NULL` (SQLite cannot
/// relax that without a table rebuild, whose `DROP TABLE` would cascade into
/// `installed_mods` and `downloads`) and because the reference app already
/// reserves string values for undetectable loaders.
pub const UNKNOWN: &str = "unknown";

#[derive(Debug, Error)]
pub enum InstanceRowError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("cannot resolve instance path {path}: {source}")]
    UnresolvablePath {
        path: String,
        source: std::io::Error,
    },
    #[error("this folder is already registered as an instance: {0}")]
    DuplicatePath(PathBuf),
    #[error("unknown instance source {0:?}")]
    InvalidSource(String),
}

/// F-104: where the instance came from. Replaces the reference app's
/// `ownership_policy`, which no mutation path ever read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceSource {
    /// Created and managed by us.
    Managed,
    /// A folder the user pointed at that we did not create.
    Manual,
    /// A folder we took over and now manage.
    Adopted,
}

impl InstanceSource {
    pub const ALL: [InstanceSource; 3] = [
        InstanceSource::Managed,
        InstanceSource::Manual,
        InstanceSource::Adopted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            InstanceSource::Managed => "managed",
            InstanceSource::Manual => "manual",
            InstanceSource::Adopted => "adopted",
        }
    }

    pub fn parse(value: &str) -> Result<Self, InstanceRowError> {
        match value {
            "managed" => Ok(InstanceSource::Managed),
            "manual" => Ok(InstanceSource::Manual),
            "adopted" => Ok(InstanceSource::Adopted),
            other => Err(InstanceRowError::InvalidSource(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceRow {
    pub id: String,
    pub name: String,
    /// Resolved, canonical instance root.
    pub path: PathBuf,
    /// The Minecraft directory inside it (`<root>/.minecraft` for Prism-style
    /// layouts). Equals `path` for a plain folder.
    pub minecraft_root: PathBuf,
    /// `None` when the user never said and nothing detected it.
    ///
    /// Stored as the `UNKNOWN` sentinel rather than SQL NULL because the
    /// columns are `NOT NULL` and SQLite cannot relax that without a table
    /// rebuild whose `DROP TABLE` cascades into `installed_mods` and
    /// `downloads`. The translation happens here and only here, so callers
    /// still get an honest `Option` and never see the fake value.
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
    pub source: InstanceSource,
    pub created_at: i64,
    pub scanned_at: Option<i64>,
}

/// `None` for the sentinel, so no caller can pass it to the resolver, which
/// requires a real loader.
fn decode(value: String) -> Option<String> {
    if value == UNKNOWN {
        None
    } else {
        Some(value)
    }
}

fn encode(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| UNKNOWN.to_string())
}

/// Rows ordered by name, case-insensitively, so the screen shows them the same
/// way on every platform. Reads only the database: a path deleted on disk is
/// still returned, because the row is the user's registration.
pub fn list(db: &Db) -> Result<Vec<InstanceRow>, InstanceRowError> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, path, minecraft_root, minecraft_version, loader, source,
                    created_at, scanned_at
             FROM instances
             ORDER BY name COLLATE NOCASE, id",
        )
        .map_err(DbError::from)?;
    let rows = stmt
        .query_map([], |r| {
            let source: String = r.get(6)?;
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                source,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<i64>>(8)?,
            ))
        })
        .map_err(DbError::from)?;

    let mut out = Vec::new();
    for row in rows {
        let (id, name, path, minecraft_root, mc, loader, source, created_at, scanned_at) =
            row.map_err(DbError::from)?;
        out.push(InstanceRow {
            id,
            name,
            path: PathBuf::from(path),
            minecraft_root: PathBuf::from(minecraft_root),
            minecraft_version: decode(mc),
            loader: decode(loader),
            source: InstanceSource::parse(&source)?,
            created_at,
            scanned_at,
        });
    }
    Ok(out)
}

pub fn get(db: &Db, id: &str) -> Result<Option<InstanceRow>, InstanceRowError> {
    Ok(list(db)?.into_iter().find(|r| r.id == id))
}

/// Inserts a row, resolving `path` first so that two spellings of one folder
/// cannot both land in the table.
pub fn insert(db: &Db, row: &InstanceRow) -> Result<(), InstanceRowError> {
    let path =
        std::fs::canonicalize(&row.path).map_err(|source| InstanceRowError::UnresolvablePath {
            path: row.path.display().to_string(),
            source,
        })?;
    if !path.is_dir() {
        return Err(InstanceRowError::UnresolvablePath {
            path: path.display().to_string(),
            source: std::io::Error::other("not a folder"),
        });
    }
    let minecraft_root = if row.minecraft_root.as_os_str().is_empty() {
        path.clone()
    } else {
        std::fs::canonicalize(&row.minecraft_root).map_err(|source| {
            InstanceRowError::UnresolvablePath {
                path: row.minecraft_root.display().to_string(),
                source,
            }
        })?
    };

    let conn = db.conn();
    let clash: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM instances WHERE path = ?1 AND id <> ?2",
            rusqlite::params![path.to_string_lossy(), row.id],
            |r| r.get(0),
        )
        .map_err(DbError::from)?;
    if clash > 0 {
        return Err(InstanceRowError::DuplicatePath(path));
    }

    conn.execute(
        "INSERT INTO instances
           (id, name, path, minecraft_root, minecraft_version, loader, source, created_at, scanned_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            row.id,
            row.name,
            path.to_string_lossy(),
            minecraft_root.to_string_lossy(),
            encode(&row.minecraft_version),
            encode(&row.loader),
            row.source.as_str(),
            row.created_at,
            row.scanned_at,
        ],
    )
    .map_err(DbError::from)?;
    Ok(())
}

/// Stamps a successful scan so the screen can show the board's
/// "Scanned 2 min ago" without stat-ing anything at list time.
pub fn mark_scanned(db: &Db, id: &str, at: i64) -> Result<bool, InstanceRowError> {
    let changed = db
        .conn()
        .execute(
            "UPDATE instances SET scanned_at = ?2 WHERE id = ?1",
            rusqlite::params![id, at],
        )
        .map_err(DbError::from)?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::db::open;

    fn folder(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(&p).expect("mkdir");
        p
    }

    fn row(id: &str, path: &Path, source: InstanceSource) -> InstanceRow {
        InstanceRow {
            id: id.to_string(),
            name: id.to_uppercase(),
            path: path.to_path_buf(),
            minecraft_root: PathBuf::new(),
            minecraft_version: Some("1.21.1".to_string()),
            loader: Some("fabric".to_string()),
            source,
            created_at: 1,
            scanned_at: None,
        }
    }

    #[test]
    fn spec_f104_source_round_trips_all_three_values() {
        for s in InstanceSource::ALL {
            assert_eq!(InstanceSource::parse(s.as_str()).expect("valid source"), s);
        }
        assert!(matches!(
            InstanceSource::parse("linked_modpack"),
            Err(InstanceRowError::InvalidSource(_))
        ));
    }

    #[test]
    fn spec_f102_insert_then_list_preserves_row() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let root = folder(dir.path(), "Survival");
        insert(&db, &row("i1", &root, InstanceSource::Managed)).expect("insert");

        let rows = list(&db).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, InstanceSource::Managed);
        assert_eq!(rows[0].minecraft_version.as_deref(), Some("1.21.1"));
        assert_eq!(rows[0].scanned_at, None);
        // An empty minecraft_root falls back to the instance root.
        assert_eq!(
            rows[0].minecraft_root,
            std::fs::canonicalize(&root).expect("canon")
        );
    }

    #[test]
    fn spec_f103_link_refuses_a_second_row_for_the_same_folder() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let root = folder(dir.path(), "Linked");
        insert(&db, &row("i1", &root, InstanceSource::Manual)).expect("first");
        let err = insert(&db, &row("i2", &root, InstanceSource::Manual)).expect_err("duplicate");
        assert!(
            matches!(err, InstanceRowError::DuplicatePath(_)),
            "got {err:?}"
        );
        assert_eq!(list(&db).expect("list").len(), 1);
    }

    #[test]
    fn spec_f103_link_is_idempotent_across_case_spellings() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let root = folder(dir.path(), "CaseFolder");
        insert(&db, &row("i1", &root, InstanceSource::Manual)).expect("first");

        // A differently-cased spelling of the same NTFS folder.
        let flipped: String = root
            .to_string_lossy()
            .chars()
            .map(|c| {
                if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c
                }
            })
            .collect();
        let spelled = PathBuf::from(flipped);
        if spelled == root {
            // The path had no cased letters; the assertion is vacuous.
            return;
        }
        let err = insert(&db, &row("i2", &spelled, InstanceSource::Manual)).expect_err("duplicate");
        assert!(
            matches!(err, InstanceRowError::DuplicatePath(_)),
            "got {err:?}"
        );
        assert_eq!(list(&db).expect("list").len(), 1, "one folder, one row");
    }

    #[test]
    fn spec_f102_insert_rejects_a_path_that_does_not_exist() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let missing = dir.path().join("nope");
        let err = insert(&db, &row("i1", &missing, InstanceSource::Managed)).expect_err("missing");
        assert!(
            matches!(err, InstanceRowError::UnresolvablePath { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn spec_f102_insert_rejects_a_file() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let file = dir.path().join("a.jar");
        std::fs::write(&file, b"x").expect("write");
        let err =
            insert(&db, &row("i1", &file, InstanceSource::Managed)).expect_err("not a folder");
        assert!(
            matches!(err, InstanceRowError::UnresolvablePath { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn spec_f101_list_orders_by_name_case_insensitively() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        for (id, name) in [("i1", "zulu"), ("i2", "Alpha"), ("i3", "mike")] {
            let root = folder(dir.path(), name);
            let mut r = row(id, &root, InstanceSource::Managed);
            r.name = name.to_string();
            insert(&db, &r).expect("insert");
        }
        let names: Vec<String> = list(&db)
            .expect("list")
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, vec!["Alpha", "mike", "zulu"]);
    }

    #[test]
    fn spec_f103_link_stores_unknown_as_none_on_the_way_out() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let root = folder(dir.path(), "Undetected");
        let mut r = row("i1", &root, InstanceSource::Manual);
        r.minecraft_version = None;
        r.loader = None;
        insert(&db, &r).expect("insert");

        let back = get(&db, "i1").expect("get").expect("row");
        assert_eq!(
            back.minecraft_version, None,
            "sentinel never reaches callers"
        );
        assert_eq!(back.loader, None);

        // And the stored value really is the sentinel, not NULL or ''.
        let raw: (String, String) = db
            .conn()
            .query_row(
                "SELECT minecraft_version, loader FROM instances WHERE id = 'i1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("raw");
        assert_eq!(raw, (UNKNOWN.to_string(), UNKNOWN.to_string()));
    }

    #[test]
    fn spec_f101_mark_scanned_stamps_and_targets_one_row() {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let root = folder(dir.path(), "Scan");
        insert(&db, &row("i1", &root, InstanceSource::Managed)).expect("insert");

        assert!(mark_scanned(&db, "i1", 1700).expect("stamp"));
        assert!(!mark_scanned(&db, "missing", 1700).expect("no such row"));
        assert_eq!(
            get(&db, "i1").expect("get").expect("row").scanned_at,
            Some(1700)
        );
    }
}
