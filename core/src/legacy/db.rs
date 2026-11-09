//! The database side of the scan: a page-consistent copy, then read-only
//! facts. Nothing here opens the old file for writing.
//!
//! `PRAGMA table_info(profiles)` decides which columns exist. The reference
//! has two real shapes - v1 with 23 columns and v2 with 34 - and a stale
//! database could be at either, so selecting by name is the only safe way to
//! read it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

use super::{LegacyError, SUPPORTED_DB_VERSION};

pub const PROFILES_TABLE: &str = "profiles";

/// Facts about the copied database itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyDbScan {
    pub path: PathBuf,
    pub bytes: u64,
    pub user_version: u32,
    pub supported: bool,
    pub has_profiles_table: bool,
    pub profile_count: usize,
    pub columns: Vec<String>,
}

/// One `profiles` row, with every column optional because a v1 database does
/// not have the v2 ones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyProfileRow {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub loader: String,
    pub kind: String,
    pub ownership: String,
    pub profile_root: String,
    pub instance_root: String,
    pub minecraft_root: String,
    pub mods_dir: String,
    pub download_dir: String,
}

impl LegacyProfileRow {
    /// The old application's own ladder (`_profile_root_from_existing_state`):
    /// `profile_root` then `instance_root` then `minecraft_root`, then the
    /// parent of a `mods` directory, then `download_dir`.
    pub fn root(&self) -> Option<PathBuf> {
        if let Some(value) = trimmed(&self.profile_root) {
            return Some(PathBuf::from(value));
        }
        if let Some(value) = trimmed(&self.instance_root) {
            return Some(PathBuf::from(value));
        }
        if let Some(value) = trimmed(&self.minecraft_root) {
            return Some(PathBuf::from(value));
        }
        if let Some(value) = trimmed(&self.mods_dir) {
            return mods_parent_or_self(value);
        }
        if let Some(value) = trimmed(&self.download_dir) {
            return mods_parent_or_self(value);
        }
        None
    }
}

pub(crate) fn scan_database(
    source: &Path,
    scratch: &Path,
) -> Result<(LegacyDbScan, Vec<LegacyProfileRow>), LegacyError> {
    std::fs::create_dir_all(scratch)?;
    let dest = scratch.join(format!("legacy-{}.sqlite3", uuid::Uuid::new_v4()));
    let _cleanup = SnapshotFiles::new(dest.clone());
    let bytes = std::fs::metadata(source)?.len();

    let src = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    {
        let mut dst = Connection::open(&dest)?;
        let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
        backup.run_to_completion(5, Duration::from_millis(50), None)?;
    }
    drop(src);

    let snapshot = Connection::open_with_flags(&dest, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let user_version: u32 = snapshot.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let supported = user_version <= SUPPORTED_DB_VERSION;
    let has_profiles_table: bool = snapshot.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [PROFILES_TABLE],
        |row| row.get(0),
    )?;
    let columns = if has_profiles_table {
        table_columns(&snapshot)?
    } else {
        Vec::new()
    };
    let rows = if supported && has_profiles_table {
        read_profiles(&snapshot)?
    } else {
        Vec::new()
    };

    Ok((
        LegacyDbScan {
            path: source.to_path_buf(),
            bytes,
            user_version,
            supported,
            has_profiles_table,
            profile_count: rows.len(),
            columns,
        },
        rows,
    ))
}

fn table_columns(connection: &Connection) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = connection.prepare("PRAGMA table_info(profiles)")?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    names.collect()
}

fn read_profiles(connection: &Connection) -> Result<Vec<LegacyProfileRow>, rusqlite::Error> {
    let mut stmt = connection.prepare("SELECT * FROM profiles")?;
    let names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let index = |name: &str| names.iter().position(|n| n.eq_ignore_ascii_case(name));
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let text = |name: &str| text_at(row, index(name));
        out.push(LegacyProfileRow {
            id: text("id"),
            name: text("name"),
            game_version: text("game_version"),
            loader: text("loader"),
            kind: text("profile_kind"),
            ownership: text("ownership_policy"),
            profile_root: text("profile_root"),
            instance_root: text("instance_root"),
            minecraft_root: text("minecraft_root"),
            mods_dir: text("mods_dir"),
            download_dir: text("download_dir"),
        });
    }
    Ok(out)
}

/// Column values are coerced to text because a v1 row and a v2 row disagree
/// on type only for `id`, and every consumer wants a string.
fn text_at(row: &rusqlite::Row<'_>, index: Option<usize>) -> String {
    match index.map(|i| row.get_ref(i)) {
        Some(Ok(ValueRef::Text(text))) => String::from_utf8_lossy(text).into_owned(),
        Some(Ok(ValueRef::Integer(value))) => value.to_string(),
        Some(Ok(ValueRef::Real(value))) => value.to_string(),
        _ => String::new(),
    }
}

fn trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// `.../mods` means the instance root is its parent; any other path is used
/// as-is (the reference's `_profile_root_from_existing_state`).
fn mods_parent_or_self(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    let is_mods = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("mods"));
    if is_mods {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && parent != Path::new(".") {
                return Some(parent.to_path_buf());
            }
        }
    }
    Some(path.to_path_buf())
}

/// Removes the snapshot and its sidecars even when the scan fails.
struct SnapshotFiles(PathBuf);

impl SnapshotFiles {
    fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Drop for SnapshotFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let mut sidecar = self.0.as_os_str().to_os_string();
        sidecar.push("-wal");
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
        let mut sidecar = self.0.as_os_str().to_os_string();
        sidecar.push("-shm");
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
}
