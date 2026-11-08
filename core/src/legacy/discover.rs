//! Find the old application's home, in the old application's own order.
//!
//! The order is the reference implementation's `_resolve_roots`
//! (`app/config.py`), minus portable mode: our app cannot know where the old
//! executable lived, so a `portable.flag` is not discoverable. An explicit
//! path covers that case.

use std::path::{Path, PathBuf};

use super::{APP_DIR_NAME, DB_FILE_NAME, ENV_HOME};

/// Where the roots came from, as a stable code the UI can translate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyRootSource {
    Explicit,
    Environment,
    LegacyDotDir,
    Platform,
    None,
}

impl LegacyRootSource {
    pub fn code(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Environment => "environment",
            Self::LegacyDotDir => "legacy_dot_dir",
            Self::Platform => "platform",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRoots {
    pub source: LegacyRootSource,
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub db_file: PathBuf,
}

impl LegacyRoots {
    /// Nothing was found. Every path is empty; [`super::scan`] reports
    /// [`super::warning::ROOT_NOT_FOUND`] and reads nothing.
    pub fn none() -> Self {
        Self {
            source: LegacyRootSource::None,
            data: PathBuf::new(),
            config: PathBuf::new(),
            cache: PathBuf::new(),
            logs: PathBuf::new(),
            db_file: PathBuf::new(),
        }
    }

    fn from_home(home: &Path, source: LegacyRootSource) -> Self {
        let data = home.join("data");
        Self {
            source,
            db_file: data.join(DB_FILE_NAME),
            data,
            config: home.join("config"),
            cache: home.join("cache"),
            logs: home.join("logs"),
        }
    }

    fn from_platform(dirs: &PlatformDirs) -> Self {
        Self {
            source: LegacyRootSource::Platform,
            data: dirs.data.clone(),
            config: dirs.config.clone(),
            cache: dirs.cache.clone(),
            logs: dirs.logs.clone(),
            db_file: dirs.data.join(DB_FILE_NAME),
        }
    }
}

/// Platform bases under the old directory name, computed once from
/// `directories` (the same crate the rest of core uses).
#[derive(Debug, Clone)]
pub(crate) struct PlatformDirs {
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
}

impl PlatformDirs {
    fn from_base_dirs(base: &directories::BaseDirs) -> Self {
        let data = base.data_local_dir().join(APP_DIR_NAME);
        let config = base.config_dir().join(APP_DIR_NAME);
        let cache = base.cache_dir().join(APP_DIR_NAME);
        Self {
            logs: logs_root(base, &data),
            data,
            config,
            cache,
        }
    }
}

#[cfg(target_os = "windows")]
fn logs_root(_base: &directories::BaseDirs, data: &Path) -> PathBuf {
    // The reference returns the same LocalAppData folder for data, cache and
    // logs on Windows.
    data.to_path_buf()
}

#[cfg(target_os = "macos")]
fn logs_root(base: &directories::BaseDirs, _data: &Path) -> PathBuf {
    base.home_dir()
        .join("Library")
        .join("Logs")
        .join(APP_DIR_NAME)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn logs_root(base: &directories::BaseDirs, _data: &Path) -> PathBuf {
    base.state_dir()
        .map(|p| p.join(APP_DIR_NAME))
        .unwrap_or_else(|| {
            base.home_dir()
                .join(".local")
                .join("state")
                .join(APP_DIR_NAME)
        })
}

#[cfg(not(any(windows, unix)))]
fn logs_root(_base: &directories::BaseDirs, data: &Path) -> PathBuf {
    data.join("logs")
}

/// Resolve the old roots. Pure: creates nothing.
pub fn discover_roots(explicit: Option<&Path>) -> LegacyRoots {
    let base = directories::BaseDirs::new();
    let home = base.as_ref().map(|b| b.home_dir().to_path_buf());
    let env_home = std::env::var_os(ENV_HOME)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let platform = base.as_ref().map(PlatformDirs::from_base_dirs);
    roots_from(
        explicit,
        env_home.as_deref(),
        home.as_deref(),
        platform.as_ref(),
    )
}

pub(crate) fn roots_from(
    explicit: Option<&Path>,
    env_home: Option<&Path>,
    home: Option<&Path>,
    platform: Option<&PlatformDirs>,
) -> LegacyRoots {
    if let Some(path) = explicit {
        return LegacyRoots::from_home(path, LegacyRootSource::Explicit);
    }
    if let Some(path) = env_home {
        return LegacyRoots::from_home(path, LegacyRootSource::Environment);
    }
    if let Some(home) = home {
        let legacy = home.join(".modrinth_bulk_downloader");
        if legacy.join("data").join(DB_FILE_NAME).is_file() {
            return LegacyRoots::from_home(&legacy, LegacyRootSource::LegacyDotDir);
        }
    }
    match platform {
        Some(dirs) => LegacyRoots::from_platform(dirs),
        None => LegacyRoots::none(),
    }
}
