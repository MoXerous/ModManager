//! Instance backup and restore over IPC (F-113, F-114, F-115, F-118).
//!
//! `mm_core::instances::backup` owns every decision: what a backup covers, which
//! groups a restore may touch, when a manifest is refused, and what a world
//! preview may read. This module is the bridge and only the bridge: it gates the
//! instance root through the registry, takes the per-root lock a pack apply also
//! takes, runs the blocking filesystem work off the async runtime, and streams
//! progress through the one throttle in `progress.rs` (AGENTS.md §5.7).
//!
//! # Plan first, then the typed word
//!
//! `plan_instance_restore` writes nothing and returns the manifest digest. The
//! confirm carries that digest back; the execute command re-plans server-side —
//! a plan body from the webview is never trusted, the same rule
//! `execute_toggle` and `execute_modpack_install` follow — and refuses a digest
//! that moved (`BackupChanged`), the code ADR 0007 §6-2 names for "the manifest
//! changed since it was written", while `RestoreMismatch` is kept for the one
//! thing it names there: a manifest that belongs to another instance.
//!
//! # The root is a registry question
//!
//! A backup copies out of a folder and a restore writes into it. "An absolute
//! path the webview sent" is not authority for either: the root must be a row
//! this app registered, which is the gate `open_instance_folder` and the pack
//! apply already apply.
//!
//! The content root is the row's `minecraft_root` when it has one (an instance
//! nested inside a game folder) and the row's own path otherwise. `mods/`,
//! `saves/` and `profile.json` live there, so that is what a backup covers.

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use tauri::ipc::Channel;

use mm_core::db::instances::InstanceRow;
use mm_core::instances::backup::manifest::ManifestError;
use mm_core::instances::backup::restore::{self, PendingRestore, RestoreError, RestoreProgress};
use mm_core::instances::backup::worlds;
use mm_core::instances::backup::{
    self as core_backup, BackupError, BackupProgress, BackupSubject, BACKUP_DIRNAME,
};

use crate::dto::{
    BackupCandidateDto, BackupEventDto, BackupListDto, BackupRequest, BackupResultDto, ErrorCode,
    IpcError, PendingRestoreDto, RestoreEntryDto, RestoreEventDto, RestoreExecuteRequest,
    RestorePlanDto, RestorePreviewRequest, RestoreResultEntryDto, RestoreRunDto, WorldDto,
    WorldsPreviewDto,
};
use crate::progress::ProgressThrottle;
use crate::state::{AppState, TransferGuard};

use super::pack::apply_lock_for;

/// Refuse a directory the registry does not own, and answer with what a backup
/// needs to describe itself.
///
/// `resolve_dir` runs first so a missing folder keeps its own answer (`io`)
/// instead of being reclassified as "not registered": the two are different
/// problems for the user.
async fn registered_instance(
    state: &AppState,
    raw: &str,
) -> Result<(PathBuf, BackupSubject), IpcError> {
    let root = super::resolve_dir(raw)?;
    let queried = root.clone();
    let row = state
        .with_db(move |db| {
            mm_core::db::instances::find_by_path(db, &queried).map_err(|e| super::row_error(&e))
        })
        .await?;
    let Some(row) = row else {
        return Err(IpcError::invalid_input(
            "that folder is not a registered instance",
        ));
    };
    let content_root = content_root_of(&row);
    Ok((
        content_root,
        BackupSubject {
            instance_id: row.id,
            name: row.name,
            minecraft_version: row.minecraft_version,
            loader: row.loader,
        },
    ))
}

/// The subject a backup needs, for a command that is not a backup itself
/// (F-123: the Yes branch of the update install).
pub(super) async fn instance_subject(
    state: &AppState,
    raw: &str,
) -> Result<BackupSubject, IpcError> {
    Ok(registered_instance(state, raw).await?.1)
}

/// Where an instance's content actually lives (`mods/`, `saves/`,
/// `profile.json`): the detected game root when the row has one, its own path
/// otherwise — the same fallback `metadata::mirror_of` uses.
fn content_root_of(row: &InstanceRow) -> PathBuf {
    let declared = if row.minecraft_root.as_os_str().is_empty() {
        row.path.clone()
    } else {
        row.minecraft_root.clone()
    };
    std::fs::canonicalize(&declared).unwrap_or(declared)
}

/// One `BackupError` in one place: writing a backup can fail as a missing root,
/// a link in the way, a cancel, or a disk error. A manifest error here comes
/// from writing (nothing is read on this path), so it is a disk failure, not
/// malformed input.
pub(super) fn backup_error(e: &BackupError) -> IpcError {
    match e {
        BackupError::RootMissing => {
            IpcError::new(ErrorCode::NotFound, "the instance content root is gone")
        }
        BackupError::RootIsLink => IpcError::new(ErrorCode::LinkOrReparse, e),
        BackupError::Cancelled => IpcError::new(ErrorCode::Cancelled, e),
        BackupError::Manifest(ManifestError::NewerSchema { .. }) => {
            IpcError::new(ErrorCode::NewerSchema, e)
        }
        BackupError::Manifest(_) => IpcError::io(e),
        BackupError::Atomic(mm_core::safety::atomic::AtomicError::LinkOrReparse(_)) => {
            IpcError::new(ErrorCode::LinkOrReparse, e)
        }
        BackupError::Atomic(_) | BackupError::Io(_) => IpcError::io(e),
    }
}

/// One `RestoreError` in one place, so preview and execute cannot disagree.
///
/// `BackupMissing` covers a folder with no manifest and a legacy manifest: both
/// are listed, neither is an input. `InstanceMismatch` is `RestoreMismatch` —
/// the user asked to restore a backup of another instance, which is a refusal
/// about the pair, not a malformed request. `ManifestChanged` is
/// `BackupChanged`: the disk moved between the preview and the confirm.
pub(super) fn restore_error(e: &RestoreError) -> IpcError {
    match e {
        RestoreError::BackupOutside => IpcError::invalid_input(e),
        // A plan whose safety folder is outside the instance is a plan this
        // bridge never builds: it is a malformed request, not a disk failure.
        RestoreError::SafetyOutside => IpcError::invalid_input(e),
        RestoreError::BackupMissing => IpcError::new(ErrorCode::NotFound, e),
        RestoreError::InstanceMismatch => IpcError::new(ErrorCode::RestoreMismatch, e),
        RestoreError::ManifestChanged => IpcError::new(ErrorCode::BackupChanged, e),
        RestoreError::ConfirmationRequired => IpcError::invalid_input(e),
        RestoreError::Manifest(ManifestError::NewerSchema { .. }) => {
            IpcError::new(ErrorCode::NewerSchema, e)
        }
        RestoreError::Manifest(_) => IpcError::invalid_input(e),
        RestoreError::Atomic(mm_core::safety::atomic::AtomicError::LinkOrReparse(_)) => {
            IpcError::new(ErrorCode::LinkOrReparse, e)
        }
        RestoreError::Atomic(_) | RestoreError::Io(_) => IpcError::io(e),
    }
}

fn backup_event(p: &BackupProgress) -> BackupEventDto {
    BackupEventDto {
        phase: if p.terminal {
            "backup.phase.done"
        } else {
            "backup.phase.copying"
        }
        .to_string(),
        copied_files: p.copied_files,
        total_files: p.total_files,
        copied_bytes: p.copied_bytes,
        total_bytes: p.total_bytes,
        terminal: p.terminal,
    }
}

fn restore_event(p: &RestoreProgress) -> RestoreEventDto {
    RestoreEventDto {
        phase: if p.terminal {
            "restore.phase.done"
        } else {
            "restore.phase.restoring"
        }
        .to_string(),
        restored_files: p.restored_files,
        total_files: p.total_files,
        restored_bytes: p.restored_bytes,
        total_bytes: p.total_bytes,
        terminal: p.terminal,
    }
}

/// The event a refused run sends before returning its error.
///
/// The screen follows the channel, so a refusal that sends nothing leaves the
/// progress view on its last event forever — the same rule
/// `abort_before_apply` follows for a pack.
fn refused_backup_event() -> BackupEventDto {
    BackupEventDto {
        phase: "backup.phase.failed".to_string(),
        copied_files: 0,
        total_files: 0,
        copied_bytes: 0,
        total_bytes: 0,
        terminal: true,
    }
}

fn refused_restore_event() -> RestoreEventDto {
    RestoreEventDto {
        phase: "restore.phase.failed".to_string(),
        restored_files: 0,
        total_files: 0,
        restored_bytes: 0,
        total_bytes: 0,
        terminal: true,
    }
}

/// Dated backups of one instance, newest first. Read-only.
#[tauri::command]
pub async fn list_instance_backups(
    state: tauri::State<'_, AppState>,
    instance_root: String,
) -> Result<BackupListDto, IpcError> {
    list_instance_backups_with(state.inner(), instance_root).await
}

/// The body of [`list_instance_backups`], with the state as a parameter.
pub async fn list_instance_backups_with(
    state: &AppState,
    instance_root: String,
) -> Result<BackupListDto, IpcError> {
    let (root, subject) = registered_instance(state, &instance_root).await?;
    let listing_root = root.clone();
    let found = tauri::async_runtime::spawn_blocking(move || {
        core_backup::list_backups(&listing_root, &subject.instance_id)
    })
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?;
    Ok(BackupListDto {
        backup_root: root.join(BACKUP_DIRNAME).display().to_string(),
        backups: found.iter().map(BackupCandidateDto::from).collect(),
    })
}

/// Copies an instance into a new dated folder and streams progress.
#[tauri::command]
pub async fn execute_instance_backup(
    state: tauri::State<'_, AppState>,
    req: BackupRequest,
    on_event: Channel<BackupEventDto>,
) -> Result<BackupResultDto, IpcError> {
    execute_instance_backup_with(state.inner(), req, on_event).await
}

/// The body of [`execute_instance_backup`], with the state as a parameter.
///
/// The plan is built inside this call and never leaves it: `BackupRequest`
/// carries no plan and no token, because a backup only reads the live instance
/// and writes a new folder. Its failure modes are the disk's.
pub async fn execute_instance_backup_with(
    state: &AppState,
    req: BackupRequest,
    on_event: Channel<BackupEventDto>,
) -> Result<BackupResultDto, IpcError> {
    let (root, subject) = registered_instance(state, &req.instance_root).await?;
    let include_saves = req.include_saves.unwrap_or(false);

    // The same per-root lock an apply holds: a backup reading `mods/` while an
    // apply replaces it would copy a tree that never fully existed.
    let lock = apply_lock_for(&root);
    let _guard = lock.lock().await;

    // A cancel the screen sends while the plan walks the tree must be observed,
    // so the flag is registered before the plan is built and dropped on every
    // exit (the guard), never at the end of a happy path only.
    let cancel = state.begin_transfer(&root);
    let _transfer = TransferGuard::new(state, root.clone(), cancel.clone());

    let plan_root = root.clone();
    let plan = tauri::async_runtime::spawn_blocking(move || {
        core_backup::build_backup_plan(&plan_root, &subject, include_saves, SystemTime::now())
    })
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?
    .map_err(|e| backup_error(&e))?;

    let channel = on_event.clone();
    let run = tauri::async_runtime::spawn_blocking(move || {
        let mut throttle = ProgressThrottle::new();
        let mut emit = |p: BackupProgress| {
            if !throttle.allows(p.terminal, Instant::now()) {
                return;
            }
            let _ = channel.send(backup_event(&p));
        };
        core_backup::execute_backup(&plan, &cancel, &mut emit)
    })
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?;

    match run {
        Ok(run) => Ok(BackupResultDto::from(&run)),
        Err(e) => {
            let _ = on_event.send(refused_backup_event());
            Err(backup_error(&e))
        }
    }
}

/// Stops the backup or restore running on this instance, if one is.
///
/// One command for both, because the key is the same: the resolved content root,
/// and the per-root lock allows one copy at a time. The copy polls the flag
/// between groups and between files, so a stop lands within a file rather than at
/// the end of the run. `true` means a run was there to stop; `false` is the
/// ordinary race between the click and the run's own end — data, not an error.
#[tauri::command]
pub async fn cancel_instance_transfer(
    state: tauri::State<'_, AppState>,
    instance_root: String,
) -> Result<bool, IpcError> {
    cancel_instance_transfer_with(state.inner(), instance_root).await
}

/// The body of [`cancel_instance_transfer`], with the state as a parameter.
pub async fn cancel_instance_transfer_with(
    state: &AppState,
    instance_root: String,
) -> Result<bool, IpcError> {
    // The same registry gate as the run it stops: a folder this app does not own
    // is not something a cancel can address.
    let (root, _subject) = registered_instance(state, &instance_root).await?;
    Ok(state.cancel_transfer(&root))
}

/// What a restore would do, without writing anything.
#[tauri::command]
pub async fn plan_instance_restore(
    state: tauri::State<'_, AppState>,
    req: RestorePreviewRequest,
) -> Result<RestorePlanDto, IpcError> {
    plan_instance_restore_with(state.inner(), req).await
}

/// The body of [`plan_instance_restore`], with the state as a parameter.
pub async fn plan_instance_restore_with(
    state: &AppState,
    req: RestorePreviewRequest,
) -> Result<RestorePlanDto, IpcError> {
    let (root, subject) = registered_instance(state, &req.instance_root).await?;
    let include_saves = req.include_saves.unwrap_or(false);
    let backup_dir = chosen_backup_dir(&req.backup_dir)?;
    let plan = tauri::async_runtime::spawn_blocking(move || {
        restore::build_restore_plan(
            &root,
            &subject,
            &backup_dir,
            include_saves,
            SystemTime::now(),
        )
    })
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?
    .map_err(|e| restore_error(&e))?;
    Ok(RestorePlanDto::from(&plan))
}

/// The chosen folder as a path. Containment, existence and identity are core's
/// decisions; this only refuses an empty string before the filesystem sees it.
fn chosen_backup_dir(raw: &str) -> Result<PathBuf, IpcError> {
    let path = PathBuf::from(raw.trim());
    if path.as_os_str().is_empty() {
        return Err(IpcError::invalid_input("empty backup path"));
    }
    Ok(path)
}

/// Applies the restore the user confirmed with the typed word.
#[tauri::command]
pub async fn execute_instance_restore(
    state: tauri::State<'_, AppState>,
    req: RestoreExecuteRequest,
    on_event: Channel<RestoreEventDto>,
) -> Result<RestoreRunDto, IpcError> {
    execute_instance_restore_with(state.inner(), req, on_event).await
}

/// The body of [`execute_instance_restore`], with the state as a parameter.
///
/// Order, and why: the registry gate and the per-root lock come first (nothing
/// is read from a folder this app does not own, and a restore must not race an
/// apply), then a fresh plan, then the digest check, and only then the writes.
/// A mismatch stops before the first rename.
pub async fn execute_instance_restore_with(
    state: &AppState,
    req: RestoreExecuteRequest,
    on_event: Channel<RestoreEventDto>,
) -> Result<RestoreRunDto, IpcError> {
    let (root, subject) = registered_instance(state, &req.instance_root).await?;
    let backup_dir = chosen_backup_dir(&req.backup_dir)?;
    let include_saves = req.include_saves;

    let lock = apply_lock_for(&root);
    let _guard = lock.lock().await;

    // F-168: the restore writes into the instance — the watcher stays quiet.
    state.note_own_write(&root);
    // Registered before the plan, like the backup's: the safety folder is not
    // created until execution, and a cancel between the click and the copy is
    // still a cancel.
    let cancel = state.begin_transfer(&root);
    let _transfer = TransferGuard::new(state, root.clone(), cancel.clone());

    let plan_root = root.clone();
    let plan = tauri::async_runtime::spawn_blocking(move || {
        restore::build_restore_plan(
            &plan_root,
            &subject,
            &backup_dir,
            include_saves,
            SystemTime::now(),
        )
    })
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?
    .map_err(|e| restore_error(&e))?;

    // The token ties the confirm to the plan the screen read. It is the
    // manifest's own digest, so a folder swapped under the same name is refused
    // here rather than restored.
    if !plan
        .manifest_sha256
        .eq_ignore_ascii_case(req.expected_manifest_sha256.trim())
    {
        // `BackupChanged`, not `RestoreMismatch` (ADR 0007 §6-2): the folder is
        // still this instance's backup, but its manifest moved after the preview
        // the user read, so the answer is "show the plan again" — while
        // `RestoreMismatch` answers "this is another instance's backup".
        return Err(IpcError::new(
            ErrorCode::BackupChanged,
            format!(
                "{} changed after the plan was shown",
                plan.backup_dir.display()
            ),
        ));
    }

    let confirmation = req.confirmation;
    let channel = on_event.clone();
    let run = tauri::async_runtime::spawn_blocking(move || {
        let mut throttle = ProgressThrottle::new();
        let mut emit = |p: RestoreProgress| {
            if !throttle.allows(p.terminal, Instant::now()) {
                return;
            }
            let _ = channel.send(restore_event(&p));
        };
        restore::execute_restore(&plan, &confirmation, &cancel, &mut emit)
    })
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?;

    match run {
        Ok(run) => Ok(RestoreRunDto {
            outcome: run.outcome.as_str().to_string(),
            safety_dir: run.safety_dir.display().to_string(),
            manifest_path: run.manifest_path.display().to_string(),
            entries: run
                .entries
                .iter()
                .map(RestoreResultEntryDto::from)
                .collect(),
            blocked: run.blocked.iter().map(RestoreEntryDto::from).collect(),
        }),
        Err(e) => {
            let _ = on_event.send(refused_restore_event());
            Err(restore_error(&e))
        }
    }
}

/// The worlds under `saves/`, read-only (F-118).
#[tauri::command]
pub async fn preview_instance_worlds(
    state: tauri::State<'_, AppState>,
    instance_root: String,
) -> Result<WorldsPreviewDto, IpcError> {
    preview_instance_worlds_with(state.inner(), instance_root).await
}

/// The body of [`preview_instance_worlds`], with the state as a parameter.
pub async fn preview_instance_worlds_with(
    state: &AppState,
    instance_root: String,
) -> Result<WorldsPreviewDto, IpcError> {
    let (root, _subject) = registered_instance(state, &instance_root).await?;
    let preview = tauri::async_runtime::spawn_blocking(move || worlds::preview_worlds(&root))
        .await
        .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?;
    Ok(WorldsPreviewDto {
        saves_dir: preview.saves_dir.display().to_string(),
        saves_exists: preview.saves_exists,
        worlds: preview.worlds.iter().map(WorldDto::from).collect(),
        message_code: preview.message_code.to_string(),
    })
}

/// Safety folders a crashed restore left behind, newest first. Read-only.
#[tauri::command]
pub async fn pending_instance_restores(
    state: tauri::State<'_, AppState>,
    instance_root: String,
) -> Result<Vec<PendingRestoreDto>, IpcError> {
    pending_instance_restores_with(state.inner(), instance_root).await
}

/// The body of [`pending_instance_restores`], with the state as a parameter.
///
/// Advisory: the scan reports what it sees and repairs nothing, so a failure
/// here is a banner that does not appear, never a broken instance view.
pub async fn pending_instance_restores_with(
    state: &AppState,
    instance_root: String,
) -> Result<Vec<PendingRestoreDto>, IpcError> {
    let (root, _subject) = registered_instance(state, &instance_root).await?;
    let found: Vec<PendingRestore> =
        tauri::async_runtime::spawn_blocking(move || restore::pending_restores(&root))
            .await
            .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?;
    Ok(found.iter().map(PendingRestoreDto::from).collect())
}
