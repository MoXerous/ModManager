//! F-181: read the old application's data without writing to it.
//!
//! The reader is **explicit only**: nothing calls it at startup. It resolves
//! the old roots, takes a page-consistent copy of `app.sqlite3` through
//! SQLite's backup API, reads that copy read-only, and reports facts. The old
//! root is never opened for writing, and the copy is removed before returning
//! - including on failure.
//!
//! Two shapes must be tolerated because a stale database may be at either:
//! v1 (23 columns, no provider/profile-root columns) and v2 (34 columns).
//! Columns are discovered with `PRAGMA table_info`, never assumed.

pub mod db;
pub mod discover;
pub mod json;

#[cfg(test)]
mod tests;

pub use db::{LegacyDbScan, LegacyProfileRow};
pub use discover::{discover_roots, LegacyRootSource, LegacyRoots};
pub use json::{read_installed_mods, read_profile_json, InstalledModsFact, JsonFact};

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Newest old schema this reader understands (`CURRENT_SCHEMA_VERSION = 2`
/// in the reference implementation).
pub const SUPPORTED_DB_VERSION: u32 = 2;

/// Per-file ceiling. `profile.json` and `installed_mods.json` are a handful
/// of kilobytes in practice; the cap keeps a wrong path from reading a huge
/// file into memory.
pub const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;

/// Environment override the old application honoured for its whole home.
pub const ENV_HOME: &str = "MODRINTH_BULK_DOWNLOADER_HOME";

/// Directory name the old application used on every platform.
pub const APP_DIR_NAME: &str = "ModrinthBulkDownloader";

/// Database file name inside `<home>/data`.
pub const DB_FILE_NAME: &str = "app.sqlite3";

#[derive(Debug, Error)]
pub enum LegacyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// Stable codes for the warnings a scan can report. The UI translates the
/// code; `detail` carries paths and parser reasons for the diagnostic line.
pub mod warning {
    pub const ROOT_NOT_FOUND: &str = "root_not_found";
    pub const DATABASE_MISSING: &str = "database_missing";
    pub const DATABASE_UNREADABLE: &str = "database_unreadable";
    pub const PROFILES_TABLE_MISSING: &str = "profiles_table_missing";
    pub const DB_VERSION_UNSUPPORTED: &str = "db_version_unsupported";
    pub const PROFILE_ROOT_MISSING: &str = "profile_root_missing";
    pub const PROFILE_JSON_UNREADABLE: &str = "profile_json_unreadable";
    pub const INSTALLED_MODS_UNREADABLE: &str = "installed_mods_unreadable";
}

/// One fact the scan could not establish. `profile` is the profile name when
/// the warning belongs to one row, otherwise `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyWarning {
    pub code: &'static str,
    pub profile: Option<String>,
    pub detail: String,
}

impl LegacyWarning {
    fn new(code: &'static str) -> Self {
        Self {
            code,
            profile: None,
            detail: String::new(),
        }
    }

    fn for_profile(code: &'static str, profile: &str, detail: impl Into<String>) -> Self {
        Self {
            code,
            profile: Some(profile.to_string()),
            detail: detail.into(),
        }
    }

    fn with_detail(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            profile: None,
            detail: detail.into(),
        }
    }
}

/// What one profile row looks like after the scan. `root` is resolved with
/// the old application's own ladder (see [`LegacyProfileRow::root`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyProfileScan {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub loader: String,
    /// `profile_kind` on a v2 database; empty on v1.
    pub kind: String,
    /// `ownership_policy` on a v2 database; empty on v1.
    pub ownership: String,
    pub root: PathBuf,
    pub root_exists: bool,
    pub profile_json: JsonFact,
    pub installed_mods: InstalledModsFact,
}

/// The result of one explicit scan. No field is a promise that an import can
/// succeed: this slice only reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyScan {
    pub roots: LegacyRoots,
    pub database: Option<LegacyDbScan>,
    pub profiles: Vec<LegacyProfileScan>,
    pub warnings: Vec<LegacyWarning>,
}

/// Scan the old home. `scratch` is ours: the snapshot copy is written there
/// and deleted before the function returns.
pub fn scan(roots: &LegacyRoots, scratch: &Path) -> Result<LegacyScan, LegacyError> {
    let mut warnings = Vec::new();
    if roots.source == LegacyRootSource::None {
        warnings.push(LegacyWarning::new(warning::ROOT_NOT_FOUND));
        return Ok(LegacyScan {
            roots: roots.clone(),
            database: None,
            profiles: Vec::new(),
            warnings,
        });
    }
    if !roots.db_file.is_file() {
        warnings.push(LegacyWarning::with_detail(
            warning::DATABASE_MISSING,
            roots.db_file.display().to_string(),
        ));
        return Ok(LegacyScan {
            roots: roots.clone(),
            database: None,
            profiles: Vec::new(),
            warnings,
        });
    }

    let (database, rows) = match db::scan_database(&roots.db_file, scratch) {
        Ok(found) => found,
        Err(error) => {
            warnings.push(LegacyWarning::with_detail(
                warning::DATABASE_UNREADABLE,
                error.to_string(),
            ));
            return Ok(LegacyScan {
                roots: roots.clone(),
                database: None,
                profiles: Vec::new(),
                warnings,
            });
        }
    };

    if !database.supported {
        warnings.push(LegacyWarning::with_detail(
            warning::DB_VERSION_UNSUPPORTED,
            format!(
                "found={} supported={SUPPORTED_DB_VERSION}",
                database.user_version
            ),
        ));
    } else if !database.has_profiles_table {
        warnings.push(LegacyWarning::new(warning::PROFILES_TABLE_MISSING));
    }

    let mut profiles = Vec::with_capacity(rows.len());
    for row in rows {
        let (root, root_exists, profile_json, installed_mods) = match row.root() {
            Some(root) => {
                let root_exists = root.is_dir();
                if !root_exists {
                    warnings.push(LegacyWarning::for_profile(
                        warning::PROFILE_ROOT_MISSING,
                        &row.name,
                        root.display().to_string(),
                    ));
                }
                let profile_json = if root_exists {
                    read_profile_json(&root)
                } else {
                    JsonFact::missing()
                };
                let installed_mods = if root_exists {
                    read_installed_mods(&root)
                } else {
                    InstalledModsFact::missing()
                };
                (root, root_exists, profile_json, installed_mods)
            }
            None => {
                warnings.push(LegacyWarning::for_profile(
                    warning::PROFILE_ROOT_MISSING,
                    &row.name,
                    "no path column holds a root",
                ));
                (
                    PathBuf::new(),
                    false,
                    JsonFact::missing(),
                    InstalledModsFact::missing(),
                )
            }
        };

        if profile_json.present && !profile_json.ok {
            warnings.push(LegacyWarning::for_profile(
                warning::PROFILE_JSON_UNREADABLE,
                &row.name,
                profile_json.detail.clone(),
            ));
        }
        if installed_mods.present && !installed_mods.ok {
            warnings.push(LegacyWarning::for_profile(
                warning::INSTALLED_MODS_UNREADABLE,
                &row.name,
                installed_mods.detail.clone(),
            ));
        }

        profiles.push(LegacyProfileScan {
            id: row.id,
            name: row.name,
            game_version: row.game_version,
            loader: row.loader,
            kind: row.kind,
            ownership: row.ownership,
            root,
            root_exists,
            profile_json,
            installed_mods,
        });
    }

    Ok(LegacyScan {
        roots: roots.clone(),
        database: Some(database),
        profiles,
        warnings,
    })
}
