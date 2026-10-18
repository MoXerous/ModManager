//! أوامر IPC. هذا الملف جسر رقيق فقط: لا منطق عمل هنا، المنطق كله في mm-core.
//! مسارات الملفات من الـwebview إدخال غير موثوق: تُحلَّل وتُتحقَّق قبل أي I/O.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::dto::{
    AppInfo, CreateInstancePlanDto, CreateInstanceRequest, CreateRecoveryDto, ErrorCode,
    InstallPlanDto, InstanceSnapshotDto, InstanceSummaryDto, IpcError, LinkInstanceRequest,
    LocalFactDto, LockEntryDto, PlanEntryDto, PlanSummaryDto, PreviewEntryDto, PreviewToggleDto,
    ProjectDto, SearchPageDto, SearchQueryDto, TogglePlanDto, ToggleResultDto, WarningDto,
};
use crate::state::AppState;

/// Walks the core error enums variant by variant: collapsing them into
/// `internal` would show the user a generic failure for something we can
/// explain precisely.
fn db_error(e: &mm_core::db::DbError) -> IpcError {
    use mm_core::db::DbError as D;
    match e {
        // The user must update or downgrade; retrying changes nothing.
        D::NewerSchema { .. } => IpcError::new(ErrorCode::NewerSchema, e),
        D::Io(_) => IpcError::io(e),
        // Migration and WAL failures are our setup, not the user's action.
        D::Wal(_) | D::Migration(_) | D::Sql(_) => IpcError::internal(e),
    }
}

fn metadata_error(e: &mm_core::instances::metadata::MetadataError) -> IpcError {
    use mm_core::instances::metadata::MetadataError as M;
    match e {
        M::NewerSchema { .. } => IpcError::new(ErrorCode::NewerSchema, e),
        M::Atomic(_) | M::Io(_) => IpcError::io(e),
        // A corrupt or truncated file is bad input for the reader, not a bug.
        M::NulByte | M::TooLarge { .. } => IpcError::invalid_input(e),
        M::Parse(_) | M::MissingField(_) => IpcError::internal(e),
    }
}

/// The instance-row errors in one place: `list` raises them directly and
/// `registry_error` wraps them, so the two cannot drift apart.
fn row_error(e: &mm_core::db::instances::InstanceRowError) -> IpcError {
    use mm_core::db::instances::InstanceRowError as R;
    match e {
        R::DuplicatePath(_) => IpcError::invalid_input(e),
        // A folder that is not there any more is a different answer from one
        // that was never valid, and the UI shows them differently.
        R::UnresolvablePath { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            IpcError::new(ErrorCode::NotFound, e)
        }
        R::UnresolvablePath { .. } => IpcError::invalid_input(e),
        R::InvalidSource(_) => IpcError::internal(e),
        R::Db(db) => db_error(db),
    }
}

fn registry_error(e: &mm_core::instances::registry::RegistryError) -> IpcError {
    use mm_core::instances::registry::RegistryError as E;
    match e {
        // The request cannot be satisfied as stated.
        E::RootNotEmpty(_) | E::AlreadyAnInstance(_) | E::RootIsFile(_) | E::UnsafeName(_) => {
            IpcError::invalid_input(e)
        }
        E::Io(_) => IpcError::io(e),
        E::Row(r) => row_error(r),
        E::Metadata(m) => metadata_error(m),
        E::Journal(j) => journal_error(j),
        // Rollback did not finish; the on-disk state is indeterminate.
        E::RolledBack(_, _) => IpcError::internal(e),
    }
}

fn journal_error(e: &mm_core::journal::JournalError) -> IpcError {
    use mm_core::journal::JournalError as J;
    match e {
        // The operation id was not one of ours, so the request was malformed.
        J::NotCanonicalId | J::ItemMissing(_) | J::DuplicateItem(_) => IpcError::invalid_input(e),
        // A state or counter that does not add up is our bookkeeping, not the
        // user's request — retrying the same action will hit it again.
        J::IllegalOperationTransition { .. }
        | J::IllegalItemTransition { .. }
        | J::CounterMismatch(_) => IpcError::internal(e),
        J::Io(_) => IpcError::io(e),
    }
}

fn summary(row: &mm_core::db::instances::InstanceRow) -> InstanceSummaryDto {
    use crate::dto::InstanceSourceDto;
    InstanceSummaryDto {
        id: row.id.clone(),
        name: row.name.clone(),
        path: row.path.to_string_lossy().into_owned(),
        minecraft_root: row.minecraft_root.to_string_lossy().into_owned(),
        minecraft_version: row.minecraft_version.clone(),
        loader: row.loader.clone(),
        source: match row.source {
            mm_core::db::instances::InstanceSource::Managed => InstanceSourceDto::Managed,
            mm_core::db::instances::InstanceSource::Manual => InstanceSourceDto::Manual,
            mm_core::db::instances::InstanceSource::Adopted => InstanceSourceDto::Adopted,
        },
        // One cheap stat per row, inside spawn_blocking. The row is listed
        // either way; this only tells the screen to show it as needing
        // attention.
        path_missing: !row.path.is_dir(),
        scanned_at: row.scanned_at,
    }
}

/// `now()` in one place so a create and its plan cannot disagree.
fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    mm_core::app_info().into()
}

#[tauri::command]
pub async fn search_projects(query: SearchQueryDto) -> Result<SearchPageDto, IpcError> {
    use mm_core::providers::{ModrinthClient, Provider};
    let client = ModrinthClient::new().map_err(|e| IpcError::new(ErrorCode::Internal, e))?;
    let page = client
        .search(&query.to_core())
        .await
        .map_err(|e| IpcError::from_provider(&e))?;
    Ok(SearchPageDto {
        hits: page
            .hits
            .iter()
            .map(|p| ProjectDto::from_core(p, "modrinth"))
            .collect(),
        total: page.total,
    })
}

/// Resolve-only preview: never writes, never downloads (F-063, dry-run by
/// construction). `existing_files` feeds the F-065 info warning.
#[tauri::command]
pub async fn build_install_plan(
    slugs: Vec<String>,
    game_version: String,
    loader: String,
    instance_root: Option<String>,
) -> Result<InstallPlanDto, IpcError> {
    use mm_core::providers::ModrinthClient;
    use mm_core::resolver::resolve_and_plan;
    if slugs.is_empty() {
        return Err(IpcError::invalid_input("no projects given"));
    }
    let client = ModrinthClient::new().map_err(|e| IpcError::new(ErrorCode::Internal, e))?;
    let existing = existing_filenames(instance_root)?;
    let plan = resolve_and_plan(&client, &slugs, &game_version, &loader, &existing)
        .await
        .map_err(|e| IpcError::from_provider(&e))?;
    Ok(InstallPlanDto {
        entries: plan.entries.iter().map(PlanEntryDto::from_core).collect(),
        summary: PlanSummaryDto {
            selected_count: plan.summary.selected_count,
            dependency_count: plan.summary.dependency_count,
            total_count: plan.summary.total_count,
            downloadable_count: plan.summary.downloadable_count,
            blocked_count: plan.summary.blocked_count,
            game_version: plan.summary.game_version.clone(),
            loader: plan.summary.loader.clone(),
            requires_confirmation: mm_core::resolver::requires_confirmation(&plan.summary),
        },
        warnings: plan.warnings.iter().map(WarningDto::from_core).collect(),
    })
}

/// Read-only instance inventory: mods facts + locked entries + loaders.
#[tauri::command]
pub async fn scan_instance(instance_root: String) -> Result<InstanceSnapshotDto, IpcError> {
    let root = resolve_dir(&instance_root)?;
    tauri::async_runtime::spawn_blocking(move || {
        use mm_core::instances::{read_lock, scan_mods, Instance};
        use mm_core::jar_reader::{detect_loaders, ScanCache};
        let mods = scan_mods(&Instance {
            root: root.clone(),
            minecraft_root: root.clone(),
        });
        let locked = match read_lock(&root.join("instance.lock.json")) {
            Ok(l) => l
                .entries
                .iter()
                .map(|e| LockEntryDto {
                    provider: e.provider.clone(),
                    project_id: e.project_id.clone(),
                    slug: e.slug.clone(),
                    version_id: e.version_id.clone(),
                    filename: e.filename.clone(),
                    sha1: e.sha1.clone(),
                })
                .collect(),
            // A newer lock schema is a typed refusal, never an empty list
            // (§5.10) — this is the only place that surfaces it.
            Err(e @ mm_core::instances::LockError::NewerSchema { .. }) => {
                return Err(IpcError::from_lock(&e));
            }
            Err(_) => Vec::new(),
        };
        // Authoritative launchers are empty for now; jar scan fills it
        // (F-092/F-093 sources land with profile support in M2).
        let mut cache = ScanCache::new();
        let mods_dir = root.join("mods");
        let loaders = detect_loaders(&[], &mods_dir, &mut cache);
        let source = if loaders.is_empty() {
            "unknown"
        } else {
            "jars"
        };
        Ok(InstanceSnapshotDto {
            loaders: loaders.iter().map(|l| l.as_str().to_string()).collect(),
            loader_source: source.to_string(),
            mods: mods
                .iter()
                .map(|f| LocalFactDto {
                    filename: f.filename.clone(),
                    size: f.size,
                    sha1: f.sha1.clone(),
                    resource_type: format!("{:?}", f.resource_type).to_lowercase(),
                    disabled: f.disabled,
                    project_id: locked
                        .iter()
                        .find(|l| l.filename == f.filename)
                        .map(|l| l.project_id.clone()),
                })
                .collect(),
            locked,
        })
    })
    .await
    .map_err(IpcError::internal)?
}

#[tauri::command]
pub async fn preview_toggle(req: PreviewToggleDto) -> Result<TogglePlanDto, IpcError> {
    let mods_dir = resolve_dir(&req.mods_dir)?;
    let files = filenames(&req.files);
    tauri::async_runtime::spawn_blocking(move || {
        use mm_core::instances::toggle::preview_toggle;
        let plan = preview_toggle(&mods_dir, req.action.into(), &files);
        Ok(TogglePlanDto {
            entries: plan
                .entries
                .iter()
                .map(|e| PreviewEntryDto {
                    filename: file_name(&e.source),
                    dest_filename: file_name(&e.dest),
                    allowed: e.allowed,
                    reason: e.reason.clone(),
                })
                .collect(),
        })
    })
    .await
    .map_err(IpcError::internal)?
}

/// Rename-only mutation. The plan is **recomputed server-side** from the
/// same inputs — a plan body from the webview is never trusted.
#[tauri::command]
pub async fn execute_toggle(req: PreviewToggleDto) -> Result<Vec<ToggleResultDto>, IpcError> {
    let mods_dir = resolve_dir(&req.mods_dir)?;
    let files = filenames(&req.files);
    tauri::async_runtime::spawn_blocking(move || {
        use mm_core::instances::toggle::{execute_toggle, preview_toggle};
        let action = req.action.into();
        let plan = preview_toggle(&mods_dir, action, &files);
        let results = execute_toggle(&plan);
        Ok(results
            .iter()
            .map(|r| ToggleResultDto {
                filename: file_name(&r.source),
                dest_filename: file_name(&r.dest),
                state: format!("{:?}", r.state),
                reason: r.reason.clone(),
            })
            .collect())
    })
    .await
    .map_err(IpcError::internal)?
}

// --- shared guards ------------------------------------------------------

/// Canonicalize a webview-supplied directory: missing or non-dir → typed
/// error. Symlinks resolve through canonicalize, so the caller gets the
/// real target.
fn resolve_dir(raw: &str) -> Result<PathBuf, IpcError> {
    let p = PathBuf::from(raw.trim());
    if p.as_os_str().is_empty() {
        return Err(IpcError::invalid_input("empty path"));
    }
    let canon = std::fs::canonicalize(&p)
        .map_err(|e| IpcError::io(format!("cannot open {}: {e}", p.display())))?;
    if !canon.is_dir() {
        return Err(IpcError::invalid_input("not a folder"));
    }
    Ok(canon)
}

/// Filenames only: a traversing, nested, or blank name is dropped here, and
/// core re-checks containment under `mods_dir` anyway. The check is on the
/// **raw** string: `file_name()` alone would hide `../escape.jar`.
fn filenames(raw: &[String]) -> Vec<PathBuf> {
    raw.iter()
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty()
                || trimmed.contains('/')
                || trimmed.contains('\\')
                || trimmed == ".."
                || trimmed.contains(':')
            {
                return None;
            }
            Some(PathBuf::from(trimmed))
        })
        .collect()
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

/// Filenames already present in the instance mods dir (F-065 info warning).
fn existing_filenames(instance_root: Option<String>) -> Result<HashSet<String>, IpcError> {
    let root = match instance_root {
        Some(r) => resolve_dir(&r)?,
        None => return Ok(HashSet::new()),
    };
    let mut out = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(root.join("mods")) {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                out.insert(n.to_string());
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn list_instances(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstanceSummaryDto>, IpcError> {
    let rows = state
        .with_db(|db| mm_core::db::instances::list(db).map_err(|e| row_error(&e)))
        .await
        // Never `unwrap` into an empty list: a failure the screen cannot see is
        // worse than an error (spec §5.10).
        ?;
    Ok(rows.iter().map(summary).collect())
}

#[tauri::command]
pub async fn plan_create_instance(
    req: CreateInstanceRequest,
) -> Result<CreateInstancePlanDto, IpcError> {
    let plan = mm_core::instances::registry::plan_create(
        &PathBuf::from(req.root),
        &req.name,
        req.minecraft_version,
        req.loader,
        epoch_now(),
    )
    .map_err(|e| registry_error(&e))?;
    Ok(CreateInstancePlanDto {
        root: plan.root.to_string_lossy().into_owned(),
        name: plan.name.clone(),
        directories: plan
            .dirs
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect(),
        relative_directories: plan
            .relative_dirs()
            .into_iter()
            .map(str::to_string)
            .collect(),
    })
}

#[tauri::command]
pub async fn execute_create_instance(
    state: tauri::State<'_, AppState>,
    req: CreateInstanceRequest,
) -> Result<InstanceSummaryDto, IpcError> {
    // The plan is rebuilt here, never read back from the webview: a folder
    // taken between preview and confirm makes `plan_create` refuse, and that
    // refusal is the drift detection.
    let plan = mm_core::instances::registry::plan_create(
        &PathBuf::from(req.root),
        &req.name,
        req.minecraft_version,
        req.loader,
        epoch_now(),
    )
    .map_err(|e| registry_error(&e))?;

    let row = state
        .with_db_and_journal(move |db, journal| {
            mm_core::instances::registry::execute_create(db, journal, &plan)
                .map_err(|e| registry_error(&e))
        })
        .await?;
    Ok(summary(&row))
}

#[tauri::command]
pub async fn link_instance(
    state: tauri::State<'_, AppState>,
    req: LinkInstanceRequest,
) -> Result<InstanceSummaryDto, IpcError> {
    let root = PathBuf::from(req.root);
    let name = req.name;
    let row = state
        .with_db(move |db| {
            mm_core::instances::registry::link_instance(db, &root, &name, epoch_now())
                .map_err(|e| registry_error(&e))
        })
        .await?;
    Ok(summary(&row))
}

#[tauri::command]
pub async fn list_pending_creates(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CreateRecoveryDto>, IpcError> {
    use mm_core::instances::recovery;
    let rows = state
        .with_db_and_journal(|db, journal| Ok(recovery::pending_creates(db, journal)))
        .await?;
    Ok(rows
        .into_iter()
        .map(|c| CreateRecoveryDto {
            operation_id: c.operation_id,
            root: c.root.to_string_lossy().into_owned(),
            instance_id: c.instance_id,
            action: c.action.key().to_string(),
            reason: c.reason.key().to_string(),
        })
        .collect())
}

/// Finishes a crashed create. Takes an operation id only — the root comes from
/// the record, so the webview never names a folder for core to act on.
#[tauri::command]
pub async fn finish_create(
    state: tauri::State<'_, AppState>,
    operation_id: String,
) -> Result<InstanceSummaryDto, IpcError> {
    use mm_core::instances::recovery;
    let row = state
        .with_db_and_journal(move |db, journal| {
            recovery::finish(db, journal, &operation_id).map_err(|e| registry_error(&e))
        })
        .await?;
    Ok(summary(&row))
}

/// Abandons a crashed create: removes the standard folders that are still
/// empty, keeps the root, and closes the record. Returns how many went.
#[tauri::command]
pub async fn abandon_create(
    state: tauri::State<'_, AppState>,
    operation_id: String,
) -> Result<usize, IpcError> {
    use mm_core::instances::recovery;
    state
        .with_db_and_journal(move |_db, journal| {
            recovery::abandon(journal, &operation_id).map_err(|e| registry_error(&e))
        })
        .await
}

#[cfg(test)]
pub(crate) mod tests;
