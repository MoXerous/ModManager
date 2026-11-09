//! Platform directories (storage layout §1-8).
//!
//! Pure resolution: no mkdir, no env override, no portable flag (F-179 is
//! Drop). Database and WAL live under machine-local data — never roaming.
//! Tests inject roots; production resolves once at startup.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathsError {
    #[error("no platform directories resolve on this machine")]
    NoBaseDir,
}

/// Resolved application roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    data: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    logs: PathBuf,
}

impl Paths {
    /// Production resolution: once at startup. Pure read, creates nothing.
    pub fn resolve() -> Result<Self, PathsError> {
        Self::from_project_dirs(directories::ProjectDirs::from("", "", "ModManager"))
    }

    /// Test seam (and explicit roots): everything hangs under `root`.
    pub fn from_root(root: &Path) -> Self {
        Self {
            data: root.join("data"),
            config: root.join("config"),
            cache: root.join("cache"),
            logs: root.join("data").join("logs"),
        }
    }

    /// Injectable constructor over an optional platform value.
    pub(crate) fn from_project_dirs(
        dirs: Option<directories::ProjectDirs>,
    ) -> Result<Self, PathsError> {
        let dirs = dirs.ok_or(PathsError::NoBaseDir)?;
        Ok(Self {
            // Machine-local: a roaming-synced WAL corrupts (spec §1-8).
            data: dirs.data_local_dir().to_path_buf(),
            config: dirs.config_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
            logs: dirs.data_local_dir().join("logs"),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    pub fn logs_dir(&self) -> &Path {
        &self.logs
    }

    /// Operation journal root (journal.md §4-1).
    ///
    /// Pure, like `db_file`: `Journal`'s writer owns root creation, including
    /// the before/after symlink check, so pre-creating it here in `ensure`
    /// would bypass that guard.
    pub fn operations_dir(&self) -> PathBuf {
        self.data.join("operations")
    }

    /// Database file: always under machine-local data, never config/cache.
    pub fn db_file(&self) -> PathBuf {
        self.data.join("app.sqlite3")
    }

    /// The only mutator: created once by the bootstrap, never by accessors.
    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.data, &self.config, &self.cache, &self.logs] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, Paths) {
        let dir = crate::test_tempdir();
        let p = Paths::from_root(dir.path());
        (dir, p)
    }

    #[test]
    fn spec_storage_paths_layout_under_root() {
        let (_dir, p) = root();
        assert!(p.db_file().starts_with(p.data_dir()));
        assert!(p.logs_dir().starts_with(p.data_dir()));
        assert_ne!(p.data_dir(), p.config_dir());
        assert_ne!(p.data_dir(), p.cache_dir());
    }

    #[test]
    fn spec_storage_paths_db_not_in_config_or_cache() {
        let (_dir, p) = root();
        assert!(!p.db_file().starts_with(p.config_dir()));
        assert!(!p.db_file().starts_with(p.cache_dir()));
    }

    #[test]
    fn spec_storage_paths_resolve_creates_nothing() {
        let dir = crate::test_tempdir();
        let probe = dir.path().join("untouched");
        let _ = Paths::from_root(&probe);
        assert!(!probe.exists(), "resolution is pure");
    }

    #[test]
    fn spec_storage_paths_ensure_is_idempotent() {
        let (_dir, p) = root();
        p.ensure().expect("ensure");
        p.ensure().expect("ensure twice");
        assert!(p.data_dir().is_dir());
        assert!(p.config_dir().is_dir());
        assert!(p.cache_dir().is_dir());
        assert!(p.logs_dir().is_dir());
    }

    #[test]
    fn spec_storage_paths_none_is_typed_error() {
        assert_eq!(Paths::from_project_dirs(None), Err(PathsError::NoBaseDir));
    }
}
