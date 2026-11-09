//! F-181: preview the old application's data over IPC.
//!
//! This module is a bridge: it validates the one optional path the webview may
//! send, chooses a scratch directory the app owns, and hands the work to
//! `mm_core::legacy`. It never opens the old home for writing, and it never
//! imports anything - the answer is a report the Settings screen renders.
//!
//! # Why a scratch directory at all
//!
//! SQLite's backup API needs a destination file. The scan copies the old
//! database there, reads the copy, and deletes it before returning. The
//! destination is under the app's own cache directory, not the old home and
//! not the system temp: the cache is already created by the bootstrap and
//! lives on the same volume as the rest of the app's data.
//!
//! # The one input
//!
//! `override_home` is for portable old installs whose home the app cannot
//! discover (AGENTS.md §6: ask, do not guess). It must be absolute when given;
//! a relative path would silently mean "relative to our process directory",
//! which is not what the user typed. An empty string means "discover".

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::dto::{IpcError, LegacyScanDto};

/// Shown when the user typed a path that is not absolute. The UI translates
/// `error.invalidInput`; this message is for the log.
const RELATIVE_HOME: &str = "the old application's home must be an absolute path";

/// Serial number so two scans in the same nanosecond cannot share a snapshot
/// name. The counter is process-wide and wraps harmlessly.
static SCRATCH_SERIAL: AtomicU64 = AtomicU64::new(0);

#[tauri::command]
pub async fn scan_legacy_import(override_home: Option<String>) -> Result<LegacyScanDto, IpcError> {
    let explicit = explicit_home(override_home)?;
    let scratch_base = scratch_base();
    tauri::async_runtime::spawn_blocking(move || scan_with(explicit, &scratch_base))
        .await
        .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?
}

/// The seam tests drive: resolve the roots, scan them, and remove the scratch
/// tree whatever the outcome.
pub(crate) fn scan_with(
    explicit: Option<PathBuf>,
    scratch_base: &Path,
) -> Result<LegacyScanDto, IpcError> {
    let roots = mm_core::legacy::discover_roots(explicit.as_deref());
    let scratch = scratch_base.join(format!(
        "scan-{}-{}-{}",
        std::process::id(),
        SCRATCH_SERIAL.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));

    let result = mm_core::legacy::scan(&roots, &scratch);

    // The snapshot is a copy of somebody's database: it never outlives the
    // report, including when the scan failed partway through.
    let _ = std::fs::remove_dir_all(&scratch);

    result.map(|scan| LegacyScanDto::from(&scan)).map_err(|e| {
        use mm_core::legacy::LegacyError;
        match &e {
            LegacyError::Io(_) => IpcError::io(&e),
            LegacyError::Sql(_) => IpcError::internal(&e),
        }
    })
}

/// Validates the optional home without touching the disk.
pub(crate) fn explicit_home(raw: Option<String>) -> Result<Option<PathBuf>, IpcError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(IpcError::invalid_input(RELATIVE_HOME));
    }
    Ok(Some(path))
}

/// Where the snapshot copy goes. Production has a cache directory; the
/// fallback exists so a broken platform resolution cannot turn a read-only
/// preview into an error before the scan even runs.
fn scratch_base() -> PathBuf {
    match mm_core::paths::Paths::resolve() {
        Ok(paths) => paths.cache_dir().join("legacy-scan"),
        Err(_) => std::env::temp_dir().join("mm-legacy-scan"),
    }
}
