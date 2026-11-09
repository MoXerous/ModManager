//! Recording an install: the instance lock and the `installed_mods` table.
//!
//! The lock is the durable file record of what sits next to the instance; the
//! table is the queryable copy. Each record carries the same identity —
//! `(provider, project_id)` — and neither side invents one: a record with no
//! project id can pin a filename and a hash in the lock, but it does not own a
//! row in a table whose key is that identity (ADR 0004 §5-4).
//!
//! Recording is idempotent per filename in the lock and per identity in the
//! table, so re-running an install that already landed replaces its entries
//! instead of appending duplicates.

use super::lock::{read_lock, write_lock, LockEntry, LockError, LockFile, LOCK_FILE_NAME};
use crate::db::instances;
use crate::db::Db;
use std::path::Path;

/// One file an install wrote, in the shape both records need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRecord {
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
    pub file_name: String,
    pub sha1: String,
    pub sha512: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("this folder is not a registered instance: {0}")]
    Unregistered(String),
    #[error(transparent)]
    Instance(#[from] instances::InstanceRowError),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

/// Records `records` for the registered instance at `instance_root`.
///
/// The instance must be registered first: a lock is never written into a
/// folder this app does not own. Then the lock is written, then the table.
///
/// Returns how many records were written down. Every record reaches the lock;
/// only those with a project id reach the table.
pub fn record_install(
    db: &Db,
    instance_root: &Path,
    records: &[InstalledRecord],
) -> Result<usize, RecordError> {
    let instance = instances::find_by_path(db, instance_root)?
        .ok_or_else(|| RecordError::Unregistered(instance_root.display().to_string()))?;

    let lock_path = instance.path.join(LOCK_FILE_NAME);
    let mut lock = match read_lock(&lock_path) {
        Ok(lock) => lock,
        // Only a missing file means "no lock yet". Every other read failure is
        // a refusal: silently rewriting a lock we could not read would erase
        // the record it holds, and a newer schema is refused above all (§5.10).
        Err(LockError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => LockFile::default(),
        Err(e) => return Err(e.into()),
    };

    for record in records {
        let entry = LockEntry {
            provider: record.provider.clone(),
            project_id: record.project_id.clone(),
            slug: String::new(),
            version_id: record.version_id.clone(),
            filename: record.file_name.clone(),
            sha1: record.sha1.clone(),
            sha512: record.sha512.clone(),
        };
        match lock
            .entries
            .iter_mut()
            .find(|e| e.filename.eq_ignore_ascii_case(&record.file_name))
        {
            Some(existing) => *existing = entry,
            None => lock.entries.push(entry),
        }
    }
    // Case-insensitive by filename, byte order as the tie-break: two runs must
    // produce the same file, and Windows cannot hold `A.jar` next to `a.jar`
    // in one folder anyway.
    lock.entries.sort_by(|a, b| {
        a.filename
            .to_ascii_lowercase()
            .cmp(&b.filename.to_ascii_lowercase())
            .then_with(|| a.filename.cmp(&b.filename))
    });
    write_lock(&lock_path, &lock)?;

    write_rows(db, &instance.id, records)?;
    Ok(records.len())
}

/// The table half.
///
/// `BEGIN IMMEDIATE` takes the write lock up front, so two concurrent writers
/// serialise at the start instead of failing halfway through a delete-insert
/// pair under a deferred transaction.
fn write_rows(
    db: &Db,
    instance_id: &str,
    records: &[InstalledRecord],
) -> Result<usize, RecordError> {
    if !records.iter().any(|r| !r.project_id.is_empty()) {
        // Nothing is keyed: the lock is the whole record. This is the pack
        // case, whose index carries names and hashes but no project ids.
        return Ok(0);
    }
    let conn = db.conn();
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let now = epoch_seconds();
    let result = (|| -> Result<usize, rusqlite::Error> {
        let mut written = 0usize;
        for record in records {
            if record.project_id.is_empty() {
                continue;
            }
            conn.execute(
                "DELETE FROM installed_mods
                 WHERE instance_id = ?1 AND provider = ?2 AND project_id = ?3",
                rusqlite::params![instance_id, record.provider, record.project_id],
            )?;
            conn.execute(
                "INSERT INTO installed_mods
                   (id, instance_id, slug, version_id, file_name, sha512, source,
                    installed_at, provider, project_id, sha1)
                 VALUES (?1, ?2, '', ?3, ?4, ?5, 'managed', ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    instance_id,
                    record.version_id,
                    record.file_name,
                    empty_as_null(&record.sha512),
                    now,
                    record.provider,
                    record.project_id,
                    empty_as_null(&record.sha1),
                ],
            )?;
            written += 1;
        }
        Ok(written)
    })();
    match result {
        Ok(written) => {
            conn.execute_batch("COMMIT")?;
            Ok(written)
        }
        Err(e) => {
            // The original error is the one worth reporting; a rollback that
            // also fails must not replace it.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e.into())
        }
    }
}

/// An absent hash is SQL NULL, not `''`: the column is nullable on purpose and
/// an empty string would read as a hash of zero bytes.
fn empty_as_null(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Unix seconds, the unit every `*_at` column in this database uses.
fn epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::instances::{InstanceRow, InstanceSource};
    use crate::db::Db;
    use std::path::Path;

    fn open(dir: &Path) -> Db {
        crate::db::open(&dir.join("test.db")).expect("db")
    }

    fn register(db: &Db, root: &Path, id: &str) {
        std::fs::create_dir_all(root.join("mods")).expect("mods");
        instances::insert(
            db,
            &InstanceRow {
                id: id.to_string(),
                name: "Test instance".to_string(),
                path: root.to_path_buf(),
                minecraft_root: root.to_path_buf(),
                minecraft_version: Some("1.20.1".to_string()),
                loader: Some("fabric".to_string()),
                source: InstanceSource::Managed,
                created_at: 0,
                scanned_at: None,
            },
        )
        .expect("instance row");
    }

    fn record(name: &str, project: &str) -> InstalledRecord {
        InstalledRecord {
            provider: "modrinth".to_string(),
            project_id: project.to_string(),
            version_id: format!("v-{project}"),
            file_name: name.to_string(),
            sha1: format!("sha1-{project}"),
            sha512: format!("sha512-{project}"),
        }
    }

    type Row = (
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
    );

    /// `(provider, project_id, version_id, file_name, sha512, source, sha1)`.
    fn db_rows(db: &Db) -> Vec<Row> {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT provider, project_id, version_id, file_name, sha512, source, sha1
                 FROM installed_mods
                 ORDER BY file_name",
            )
            .expect("prepare");
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    }

    #[test]
    fn spec_f128_record_install_writes_lock_and_db() {
        let dir = crate::test_tempdir();
        let db = open(dir.path());
        let root = dir.path().join("instance");
        register(&db, &root, "i1");

        let written = record_install(&db, &root, &[record("a.jar", "proj-a")]).expect("records");
        assert_eq!(written, 1);

        let lock = read_lock(&root.join(LOCK_FILE_NAME)).expect("lock");
        assert_eq!(lock.entries.len(), 1);
        let entry = &lock.entries[0];
        assert_eq!(entry.provider, "modrinth");
        assert_eq!(entry.project_id, "proj-a");
        assert_eq!(entry.version_id, "v-proj-a");
        assert_eq!(entry.filename, "a.jar");
        assert_eq!(entry.sha1, "sha1-proj-a");
        assert_eq!(entry.sha512, "sha512-proj-a");

        assert_eq!(
            db_rows(&db),
            vec![(
                "modrinth".to_string(),
                "proj-a".to_string(),
                "v-proj-a".to_string(),
                "a.jar".to_string(),
                Some("sha512-proj-a".to_string()),
                "managed".to_string(),
                Some("sha1-proj-a".to_string()),
            )],
            "the row must carry the same identity the lock does"
        );
    }

    #[test]
    fn spec_f128_record_install_is_idempotent() {
        let dir = crate::test_tempdir();
        let db = open(dir.path());
        let root = dir.path().join("instance");
        register(&db, &root, "i1");

        let first = record("a.jar", "proj-a");
        record_install(&db, &root, &[first]).expect("first");
        let updated = InstalledRecord {
            version_id: "v-proj-a-2".to_string(),
            sha1: "sha1-2".to_string(),
            sha512: "sha512-2".to_string(),
            ..record("A.JAR", "proj-a")
        };
        record_install(&db, &root, &[updated]).expect("second");

        let lock = read_lock(&root.join(LOCK_FILE_NAME)).expect("lock");
        assert_eq!(
            lock.entries.len(),
            1,
            "the same file must not be listed twice: {:?}",
            lock.entries
        );
        assert_eq!(lock.entries[0].version_id, "v-proj-a-2");
        assert_eq!(lock.entries[0].sha512, "sha512-2");

        let rows = db_rows(&db);
        assert_eq!(rows.len(), 1, "one identity owns one row: {rows:?}");
        assert_eq!(rows[0].2, "v-proj-a-2");
    }

    #[test]
    fn spec_f128_record_install_keeps_unknown_project_in_lock_only() {
        let dir = crate::test_tempdir();
        let db = open(dir.path());
        let root = dir.path().join("instance");
        register(&db, &root, "i1");

        record_install(&db, &root, &[record("pack-member.jar", "")]).expect("records");

        let lock = read_lock(&root.join(LOCK_FILE_NAME)).expect("lock");
        assert_eq!(lock.entries.len(), 1);
        assert_eq!(lock.entries[0].project_id, "");
        assert_eq!(lock.entries[0].sha512, "sha512-");
        assert!(
            db_rows(&db).is_empty(),
            "a row keyed by an empty project id would be a made-up identity"
        );
    }

    #[test]
    fn spec_f128_record_install_refuses_unregistered_instance() {
        let dir = crate::test_tempdir();
        let db = open(dir.path());
        let root = dir.path().join("not-registered");
        std::fs::create_dir_all(&root).expect("folder");

        let err = record_install(&db, &root, &[record("a.jar", "proj-a")])
            .expect_err("an unregistered folder must be refused");
        assert!(matches!(err, RecordError::Unregistered(_)), "got {err:?}");
        assert!(
            !root.join(LOCK_FILE_NAME).exists(),
            "a refused record must not leave a lock behind"
        );
        assert!(db_rows(&db).is_empty());
    }
}
