//! Instance rows: the `instances` table as typed Rust values.
//!
//! `core::db` owns the schema; this module owns reading and writing rows.
//! Nothing here touches the filesystem except `insert`, which must resolve the
//! caller's path before storing it: SQLite's `UNIQUE` is case-sensitive, so
//! `C:\Users\a` and `c:\users\A` are two spellings of one NTFS folder and would
//! otherwise register it twice.

use std::path::{Path, PathBuf};

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
    let rows = stmt.query_map([], row_to_tuple).map_err(DbError::from)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(assemble(row.map_err(DbError::from)?)?);
    }
    Ok(out)
}

/// The row with this id, if any.
///
/// Queried by id directly, not by filtering [`list`]: `list` decodes *every*
/// row's `source`, so one hand-edited or corrupt value anywhere in the table
/// would make this fail. `execute_create` calls it on the rollback path, where
/// a failure means removing folders and leaving the row behind.
pub fn get(db: &Db, id: &str) -> Result<Option<InstanceRow>, InstanceRowError> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, path, minecraft_root, minecraft_version, loader, source,
                    created_at, scanned_at
             FROM instances
             WHERE id = ?1",
        )
        .map_err(DbError::from)?;
    let mut rows = stmt.query_map([id], row_to_tuple).map_err(DbError::from)?;
    match rows.next() {
        None => Ok(None),
        Some(row) => Ok(Some(assemble(row.map_err(DbError::from)?)?)),
    }
}

/// `list` and `get` must agree on the shape, so both go through these two.
type RowTuple = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
);

fn row_to_tuple(r: &rusqlite::Row<'_>) -> rusqlite::Result<RowTuple> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
    ))
}

fn assemble(t: RowTuple) -> Result<InstanceRow, InstanceRowError> {
    let (id, name, path, minecraft_root, mc, loader, source, created_at, scanned_at) = t;
    Ok(InstanceRow {
        id,
        name,
        path: PathBuf::from(path),
        minecraft_root: PathBuf::from(minecraft_root),
        minecraft_version: decode(mc),
        loader: decode(loader),
        source: InstanceSource::parse(&source)?,
        created_at,
        scanned_at,
    })
}

/// The Windows verbatim prefix `canonicalize` writes in front of an absolute
/// path (`\\?\C:\...`) — and therefore in front of what [`insert`] stores.
const VERBATIM_PREFIX: &str = "\\\\?\\";

/// Drops the verbatim prefix from a stored spelling, if it has one.
///
/// It must be handled here, on the stored side: rows already in the database
/// carry the prefix, and no migration rewrites stored paths to compare one
/// spelling. Comparing the prefix itself would mean comparing the four
/// characters against a path, which `Path::starts_with` cannot do portably
/// (`\\?\` parses as a component only on Windows).
fn strip_verbatim_prefix(p: &Path) -> &Path {
    p.to_str()
        .and_then(|s| s.strip_prefix(VERBATIM_PREFIX))
        .map(Path::new)
        .unwrap_or(p)
}

/// The row registered for this exact folder, if any.
///
/// A direct query rather than a scan of `list`, for two reasons: `list` decodes
/// *every* row's `source`, so one hand-edited value would make it fail and
/// take this with it; and this is called on the rollback path, where guessing
/// wrong means deleting a folder another instance owns.
///
/// The argument is normalised the same way [`insert`] stores it: an existing
/// folder resolves to the spelling `canonicalize` wrote, and the two meet
/// exactly. Comparing the caller's own spelling instead would answer "nobody
/// owns this folder" for a folder that is registered — on Windows
/// `canonicalize` adds the `\\?\` verbatim prefix, so the two spellings differ
/// in the common case.
///
/// A path that cannot be normalised is queried as given. That is sound rather
/// than merely a best effort: `insert` refuses a path that does not exist, so
/// no row can point at one *whenever it was written* — but a row's folder can
/// be deleted *afterwards*, and the row is still the user's registration (F-101
/// lists it). A caller asking about a registered folder that is gone cannot
/// resolve anything, so the exact match is followed by a spelling comparison
/// ([`find_by_spelling`]): the stored spelling may still name the folder the
/// caller holds, verbatim prefix and all. Without it, `open_instance_folder`
/// answers "that folder is not a registered instance" for a row this same
/// command listed a moment ago, while the pack gate answers "cannot open" for
/// the same bytes — one folder, three stories.
pub fn find_by_path(db: &Db, path: &Path) -> Result<Option<InstanceRow>, InstanceRowError> {
    let stored_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let conn = db.conn();
    let mut id: Option<String> = conn
        .query_row(
            "SELECT id FROM instances WHERE path = ?1",
            [stored_path.to_string_lossy().as_ref()],
            |r| r.get(0),
        )
        .ok();
    if id.is_none() {
        id = find_by_spelling(conn, &stored_path)?;
    }
    let Some(id) = id else {
        return Ok(None);
    };
    get(db, &id)
}

/// The exact match missed: does a stored spelling name this folder after all?
///
/// Only the shape of the stored string can differ here — every row is
/// canonicalized at insert, so both sides are absolute — and on Windows the
/// stored one may wear the verbatim prefix a caller asking about a deleted
/// folder can no longer produce. The comparison is the stored path, then the
/// same path without the prefix. It folds no case: the caller asks with the
/// spelling it got from the registry (`list`), and case-folding here would
/// make two case-different spellings of one NTFS folder collide (F-103).
///
/// Only `id` and `path` are read, so a corrupt `source` value anywhere in the
/// table cannot make this fail and turn a registered folder into an
/// unregistered one — the reason a scan of [`list`] was refused in the first
/// place.
fn find_by_spelling(
    conn: &rusqlite::Connection,
    asked: &Path,
) -> Result<Option<String>, InstanceRowError> {
    let mut stmt = conn
        .prepare("SELECT id, path FROM instances")
        .map_err(DbError::from)?;
    let mut rows = stmt.query([]).map_err(DbError::from)?;
    while let Some(row) = rows.next().map_err(DbError::from)? {
        let id: String = row.get(0).map_err(DbError::from)?;
        let stored: String = row.get(1).map_err(DbError::from)?;
        let stored = Path::new(&stored);
        if stored == asked || strip_verbatim_prefix(stored) == asked {
            return Ok(Some(id));
        }
    }
    Ok(None)
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

        // A differently-cased spelling of the same path. Whether that names the
        // *same* folder depends on the filesystem, and the test must not
        // assume Windows: this crate is built on Linux in CI, where the flipped
        // path is a different folder that has to exist in its own right.
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
        let spelled = PathBuf::from(&flipped);
        if spelled == root {
            // The path had no cased letters; the assertion is vacuous.
            return;
        }
        std::fs::create_dir_all(&spelled).expect("mkdir the flipped spelling");

        // Both sides canonicalised: `insert` compares canonical forms, so a raw path
        // against a canonical one would never match and the test would take the
        // wrong branch on every platform.
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).expect("canon");
        let same_folder = canon(&spelled) == canon(&root);
        let outcome = insert(&db, &row("i2", &spelled, InstanceSource::Manual));
        if same_folder {
            assert!(
                matches!(outcome, Err(InstanceRowError::DuplicatePath(_))),
                "one folder must not become two rows: {outcome:?}"
            );
            assert_eq!(list(&db).expect("list").len(), 1, "one folder, one row");
        } else {
            // Case-sensitive: these really are two folders, and collapsing them
            // would register an instance at a path the user never named.
            outcome.expect("a different folder is a different instance");
            assert_eq!(list(&db).expect("list").len(), 2, "two folders, two rows");
        }
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
    fn spec_f102_get_survives_a_corrupt_row_elsewhere_in_the_table() {
        // A `source` value the CHECK constraint cannot normally admit only gets
        // in by disabling enforcement. It must still not break `get`, which the
        // rollback path calls: failing there would remove folders and leave
        // the row behind.
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        for (id, name) in [("good", "Good"), ("bad", "Bad")] {
            let p = folder(dir.path(), name);
            let mut r = row(id, &p, InstanceSource::Managed);
            r.name = name.to_string();
            insert(&db, &r).expect("insert");
        }
        db.conn()
            .execute_batch(
                "PRAGMA ignore_check_constraints = ON;
                 UPDATE instances SET source = 'linked_modpack' WHERE id = 'bad';",
            )
            .expect("corrupt one row");

        assert!(list(&db).is_err(), "list reports the bad value");
        let got = get(&db, "good").expect("get must not fail").expect("row");
        assert_eq!(got.id, "good");
        assert_eq!(got.name, "Good");
    }

    #[test]
    fn spec_f102_find_by_path_matches_the_spelling_insert_stores() {
        // `insert` stores `std::fs::canonicalize(..)`, which on Windows carries
        // the `\\?\` verbatim prefix. `execute_create` asks about a path that
        // does not exist yet — `plan_create` permits an absent root — so it can
        // only offer the spelling it was handed. If the query does not
        // normalise its own argument, the two never meet and the caller is told
        // nobody owns the folder when somebody does.
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let p = folder(dir.path(), "Plain");
        insert(&db, &row("i1", &p, InstanceSource::Managed)).expect("insert");

        let raw = dir.path().join("Plain");
        assert!(
            !raw.to_string_lossy().starts_with("\\\\?\\"),
            "premise: the caller holds no verbatim prefix, got {:?}",
            raw.to_string_lossy()
        );
        let found = find_by_path(&db, &raw).expect("query");
        assert!(found.is_some(), "the caller's own spelling missed the row");
    }

    #[test]
    fn spec_f102_find_by_path_matches_a_registered_row_whose_folder_is_gone() {
        // The row is the user's registration, not a filesystem index: a folder
        // that was registered and then deleted keeps its row in `list`, and
        // commands that answer for folders (`open_instance_folder`, the pack
        // gate) must still be able to say "that folder is ours" about it. The
        // stored spelling carries the Windows verbatim prefix, which a caller
        // holding a deleted folder can never canonicalize into — so the
        // comparison must handle the stored spelling, not the query's.
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        let root = folder(dir.path(), "Gone");
        insert(&db, &row("i1", &root, InstanceSource::Managed)).expect("insert");

        // Hold the spelling the registry itself reported (what `list` hands
        // back is exactly this), while the folder is still there to ask about.
        let held = std::fs::canonicalize(&root).expect("canon");
        std::fs::remove_dir_all(&root).expect("delete the folder");

        // The stored row really is the spelling the deletion invalidated:
        // on Windows `insert` stored the verbatim-prefixed form, and this is
        // the case the exact match above can never produce.
        let stored_path: String = db
            .conn()
            .query_row("SELECT path FROM instances WHERE id = 'i1'", [], |r| {
                r.get(0)
            })
            .expect("stored path");
        #[cfg(windows)]
        assert!(
            stored_path.starts_with("\\\\?\\"),
            "premise: rows are stored verbatim-prefixed on Windows, got {stored_path:?}"
        );

        let found = find_by_path(&db, &held).expect("query");
        assert!(
            found.is_some(),
            "a registered folder that the disk deleted now looks unregistered"
        );
        let got = found.expect("row");
        assert_eq!(got.id, "i1");

        // And the registry question stays a registry question: nothing was
        // ever registered at this spelling, deleted or not.
        let stranger = dir.path().join("Stranger");
        std::fs::create_dir_all(&stranger).expect("mkdir stranger");
        std::fs::remove_dir_all(&stranger).expect("delete the stranger too");
        assert!(
            find_by_path(&db, &stranger).expect("query").is_none(),
            "a folder nobody registered has no row to find"
        );
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
