//! F-181: read the old application's data, and register its profiles as
//! instances of this app.
//!
//! This module is a bridge: it validates the one optional path the webview may
//! send, chooses a scratch directory the app owns, and hands the work to
//! `mm_core::legacy`. It never opens the old home for writing. The scan is a
//! report; the import registers folders into **our** database and writes
//! nothing into an imported folder.
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
//!
//! # Why the plan travels, not the truth
//!
//! The webview sends profile ids and nothing else. `execute_legacy_import`
//! re-scans and rebuilds the plan in Rust (AGENTS.md §5.4), so a plan shown on
//! screen can never be edited into one that registers a folder the user never
//! saw.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::dto::{IpcError, LegacyImportPlanDto, LegacyImportReportDto, LegacyScanDto};
use crate::state::AppState;

/// Shown when the user typed a path that is not absolute. The UI translates
/// `error.invalidInput`; this message is for the log.
const RELATIVE_HOME: &str = "the old application's home must be an absolute path";

/// Shown when the request names no profile. A confirm with nothing selected is
/// a malformed request, not an empty import.
const EMPTY_SELECTION: &str = "no legacy profiles selected";

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

/// F-181: what an import of `selected` would do. Writes nothing.
#[tauri::command]
pub async fn plan_legacy_import(
    state: tauri::State<'_, AppState>,
    override_home: Option<String>,
    selected: Vec<String>,
) -> Result<LegacyImportPlanDto, IpcError> {
    let explicit = explicit_home(override_home)?;
    let selected = normalize_selection(selected)?;
    let scratch_base = scratch_base();
    let scan = tauri::async_runtime::spawn_blocking(move || scan_core(explicit, &scratch_base))
        .await
        .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))??;
    state
        .with_db(move |db| plan_dto(db, &scan, &selected))
        .await
}

/// F-181: registers the selected profiles in our database, after backing it up.
#[tauri::command]
pub async fn execute_legacy_import(
    state: tauri::State<'_, AppState>,
    override_home: Option<String>,
    selected: Vec<String>,
) -> Result<LegacyImportReportDto, IpcError> {
    let explicit = explicit_home(override_home)?;
    let selected = normalize_selection(selected)?;
    let scratch_base = scratch_base();
    let scan = tauri::async_runtime::spawn_blocking(move || scan_core(explicit, &scratch_base))
        .await
        .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))??;
    let now = epoch_now();
    state
        .with_db(move |db| execute_dto(db, &scan, &selected, now))
        .await
}

/// The seam tests drive: resolve the roots, scan them, and remove the scratch
/// tree whatever the outcome.
pub(crate) fn scan_with(
    explicit: Option<PathBuf>,
    scratch_base: &Path,
) -> Result<LegacyScanDto, IpcError> {
    scan_core(explicit, scratch_base).map(|scan| LegacyScanDto::from(&scan))
}

/// The core scan, without the DTO: shared by the report, the plan and the
/// execute, so all three describe the same old home.
pub(crate) fn scan_core(
    explicit: Option<PathBuf>,
    scratch_base: &Path,
) -> Result<mm_core::legacy::LegacyScan, IpcError> {
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

    result.map_err(|e| {
        use mm_core::legacy::LegacyError;
        match &e {
            LegacyError::Io(_) => IpcError::io(&e),
            LegacyError::Sql(_) => IpcError::internal(&e),
        }
    })
}

/// Test seam: the real scan and the real core plan, with no Tauri app.
#[cfg(test)]
pub(crate) fn plan_import_with(
    db: &mm_core::db::Db,
    explicit: Option<PathBuf>,
    scratch: &Path,
    selected: Vec<String>,
) -> Result<LegacyImportPlanDto, IpcError> {
    let selected = normalize_selection(selected)?;
    let scan = scan_core(explicit, scratch)?;
    plan_dto(db, &scan, &selected)
}

/// Test seam: the real scan and the real core execute, with no Tauri app.
#[cfg(test)]
pub(crate) fn execute_import_with(
    db: &mm_core::db::Db,
    explicit: Option<PathBuf>,
    scratch: &Path,
    selected: Vec<String>,
) -> Result<LegacyImportReportDto, IpcError> {
    let selected = normalize_selection(selected)?;
    let scan = scan_core(explicit, scratch)?;
    execute_dto(db, &scan, &selected, epoch_now())
}

fn plan_dto(
    db: &mm_core::db::Db,
    scan: &mm_core::legacy::LegacyScan,
    selected: &[String],
) -> Result<LegacyImportPlanDto, IpcError> {
    mm_core::legacy::plan_import(db, scan, selected)
        .map(|plan| LegacyImportPlanDto::from(&plan))
        .map_err(import_error)
}

fn execute_dto(
    db: &mm_core::db::Db,
    scan: &mm_core::legacy::LegacyScan,
    selected: &[String],
    now: i64,
) -> Result<LegacyImportReportDto, IpcError> {
    mm_core::legacy::execute_import(db, scan, selected, now)
        .map(|report| LegacyImportReportDto::from(&report))
        .map_err(import_error)
}

/// Trim, drop blanks and de-duplicate while keeping the caller's order.
pub(crate) fn normalize_selection(selected: Vec<String>) -> Result<Vec<String>, IpcError> {
    let mut out: Vec<String> = Vec::new();
    for raw in selected {
        let id = raw.trim();
        if id.is_empty() || out.iter().any(|seen| seen == id) {
            continue;
        }
        out.push(id.to_string());
    }
    if out.is_empty() {
        return Err(IpcError::invalid_input(EMPTY_SELECTION));
    }
    Ok(out)
}

/// A filesystem failure is `io`; a database or row failure is `internal`.
fn import_error(error: mm_core::legacy::ImportError) -> IpcError {
    use mm_core::legacy::ImportError;
    match &error {
        ImportError::Io(_) => IpcError::io(&error),
        ImportError::Db(_) | ImportError::Row(_) => IpcError::internal(&error),
    }
}

/// `now()` for the rows this module writes.
fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
