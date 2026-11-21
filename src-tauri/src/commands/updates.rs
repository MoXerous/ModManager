//! Applying updates over IPC (F-121..F-124).
//!
//! The shape is the bridge's usual one: a read-only plan, then a write that
//! **rebuilds the plan server-side** and refuses when the token the webview
//! carries back is not the token just computed (F-121). A plan body from the
//! webview is never trusted — the same rule `.mrpack` export and apply follow.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mm_core::downloader::Downloader;
use mm_core::instances::backup::BackupSubject;
use mm_core::instances::metadata::{read_mirror, PROFILE_FILE_NAME};
use mm_core::journal::{Journal, JournalError, JournalOp, OperationKind, OperationRecord};
use mm_core::providers::Provider;
use mm_core::updates::apply::{
    apply_install, download_only, plan_install, BackupChoice, BackupDecision, InstallOptions,
    SafetyContractItem, UpdateApplyError, UpdateInstallPlan,
};

use crate::dto::{
    ApplyUpdateRequest, BackupDecisionDto, ErrorCode, InstalledUpdateDto, IpcError,
    PreparedUpdateDto, SafetyContractItemDto, SkippedUpdateDto, UpdateDownloadResultDto,
    UpdateInstallPlanDto, UpdateInstallPlanRequest, UpdateInstallResultDto, UpdatePlanEntryDto,
};
use crate::state::AppState;

use super::pack::apply_lock_for;

/// The plan a confirm was built from, plus where it lives.
struct Built {
    root: PathBuf,
    plan: UpdateInstallPlan,
}

/// Re-read the instance and re-plan. Reads only: no download, no write.
async fn build(
    provider: &dyn Provider,
    instance_root: &str,
    game_version: &str,
    loader: &str,
    selected: &[String],
) -> Result<Built, IpcError> {
    let root = super::resolve_dir(instance_root)?;
    let preview = mm_core::updates::check_updates(provider, &root, game_version, loader)
        .await
        .map_err(update_error)?;
    let plan =
        plan_install(&root, game_version, loader, &preview, selected).map_err(apply_error)?;
    Ok(Built { root, plan })
}

/// F-121: a read-only install plan, carrying the token and the five promises.
#[tauri::command]
pub async fn preview_update_install(
    state: tauri::State<'_, AppState>,
    req: UpdateInstallPlanRequest,
) -> Result<UpdateInstallPlanDto, IpcError> {
    let provider = crate::providers::build(state.inner(), &req.provider)?;
    let built = build(
        provider.as_ref(),
        &req.instance_root,
        &req.game_version,
        &req.loader,
        &req.selected,
    )
    .await?;
    Ok(UpdateInstallPlanDto {
        token: built.plan.token().to_string(),
        entries: built.plan.entries.iter().map(entry_dto).collect(),
        // F-122: the contract travels with the plan, from core's own table, so
        // the copy the dialog shows and the guards that run cannot drift.
        contract: SafetyContractItem::all()
            .iter()
            .map(|item| SafetyContractItemDto {
                id: item.id.to_string(),
                key: item.key.to_string(),
                text: item.text.to_string(),
            })
            .collect(),
    })
}

/// F-121..F-123: apply the plan the user confirmed.
#[tauri::command]
pub async fn execute_update_install(
    state: tauri::State<'_, AppState>,
    req: ApplyUpdateRequest,
) -> Result<UpdateInstallResultDto, IpcError> {
    let provider = crate::providers::build(state.inner(), &req.provider)?;
    let built = build(
        provider.as_ref(),
        &req.instance_root,
        &req.game_version,
        &req.loader,
        &req.selected,
    )
    .await?;
    if built.plan.token() != req.expected_token {
        return Err(IpcError::new(
            ErrorCode::UpdateContextChanged,
            "the instance changed after the update plan was shown",
        ));
    }
    let choice = match req.backup {
        BackupDecisionDto::Yes => BackupChoice::yes(),
        BackupDecisionDto::No if req.confirm_no_backup => BackupChoice::no_confirmed(),
        BackupDecisionDto::No => BackupChoice::no(),
        BackupDecisionDto::Cancel => BackupChoice::cancel(),
    };
    // The subject describes the profile a backup would copy; only the Yes
    // branch reads it. Resolving the registry row unconditionally would let a
    // Cancel (a refusal) fail for a reason that has nothing to do with it.
    let subject = if choice.decision == BackupDecision::Yes {
        super::backup::instance_subject(state.inner(), &req.instance_root).await?
    } else {
        profile_subject(&built.root)
    };

    // One apply per instance at a time, and the watcher stays quiet about the
    // writes this run makes (F-168).
    let lock = apply_lock_for(&built.plan.context.target.instance_root);
    let _guard = lock.lock().await;
    let mods = super::mods_dir_of(&built.root);
    state.note_own_write(&mods);

    let downloader = Downloader::new(3).map_err(IpcError::internal)?;
    let target = built.plan.context.target.clone();
    let options = InstallOptions {
        target: &target,
        subject,
        backup: choice,
        downloader: &downloader,
        allow_insecure_loopback: false,
    };
    let journal = state.journal_handle();
    let mut write = writer(journal, OperationKind::UpdateInstall);
    let report = apply_install(&built.plan, &options, &mut write)
        .await
        .map_err(apply_error)?;
    Ok(UpdateInstallResultDto {
        operation_id: report.operation_id,
        installed: report
            .installed
            .into_iter()
            .map(|row| InstalledUpdateDto {
                project_id: row.project_id,
                filename: row.filename,
                version_number: row.version_number,
                replaced: row.replaced,
            })
            .collect(),
        skipped: report
            .skipped
            .into_iter()
            .map(|row| SkippedUpdateDto {
                project_id: row.project_id,
                filename: row.filename,
                reason_code: row.reason.code().to_string(),
                message: row.message,
            })
            .collect(),
        replaced_backup_dir: report.replaced_backup_dir.to_string_lossy().into_owned(),
        profile_backup_dir: report
            .profile_backup
            .map(|run| run.backup_dir.to_string_lossy().into_owned()),
    })
}

/// F-124: prepare the files in `modrinth-update-downloads/`, touch nothing
/// installed, and journal what was prepared.
#[tauri::command]
pub async fn execute_update_download_only(
    state: tauri::State<'_, AppState>,
    req: ApplyUpdateRequest,
) -> Result<UpdateDownloadResultDto, IpcError> {
    let provider = crate::providers::build(state.inner(), &req.provider)?;
    let built = build(
        provider.as_ref(),
        &req.instance_root,
        &req.game_version,
        &req.loader,
        &req.selected,
    )
    .await?;
    if built.plan.token() != req.expected_token {
        return Err(IpcError::new(
            ErrorCode::UpdateContextChanged,
            "the instance changed after the update plan was shown",
        ));
    }
    let downloader = Downloader::new(3).map_err(IpcError::internal)?;
    let target = built.plan.context.target.clone();
    let journal = state.journal_handle();
    let mut write = writer(journal, OperationKind::UpdateDownload);
    let report = download_only(&built.plan, &target, &downloader, false, &mut write)
        .await
        .map_err(apply_error)?;
    Ok(UpdateDownloadResultDto {
        operation_id: report.operation_id,
        directory: report.directory.to_string_lossy().into_owned(),
        prepared: report
            .prepared
            .into_iter()
            .map(|file| PreparedUpdateDto {
                project_id: file.project_id,
                filename: file.filename,
                path: file.path,
                bytes: file.bytes,
            })
            .collect(),
    })
}

fn entry_dto(entry: &mm_core::updates::apply::UpdatePlanEntry) -> UpdatePlanEntryDto {
    UpdatePlanEntryDto {
        project_id: entry.project_id.clone(),
        old_filename: entry.old_filename.clone(),
        filename: entry.filename.clone(),
        version_number: entry.version_number.clone(),
        size: entry.size,
    }
}

/// Who a profile backup would be about, without asking the registry. Used on
/// the branches that never take a backup (No/Cancel), where the subject is
/// only carried, not read; the instance's own `profile.json` names it, and a
/// folder ModManager did not create falls back to its directory name.
fn profile_subject(root: &Path) -> BackupSubject {
    let fallback = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "instance".to_string());
    match read_mirror(&root.join(PROFILE_FILE_NAME)) {
        Ok(meta) => BackupSubject {
            instance_id: meta.id,
            name: meta.name,
            minecraft_version: meta.minecraft_version,
            loader: meta.loader,
        },
        Err(_) => BackupSubject {
            instance_id: fallback.clone(),
            name: fallback,
            minecraft_version: None,
            loader: None,
        },
    }
}

/// One lock per write, like the rest of the app: the download can take
/// minutes and must not hold the journal.
fn writer(
    journal: Arc<Mutex<Journal>>,
    kind: OperationKind,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send {
    let mut id = String::new();
    move |op: JournalOp| {
        let journal = journal
            .lock()
            .map_err(|e| JournalError::Io(e.to_string()))?;
        match op {
            JournalOp::Start { target, items } => {
                let record = journal.start(kind, "", &target, items)?;
                id = record.operation_id.clone();
                Ok(record)
            }
            JournalOp::Operation(state) => journal.transition_operation(&id, state, None),
            JournalOp::Item { item_id, state } => {
                journal.transition_item(&id, &item_id, state, None)
            }
            JournalOp::Items { transitions } => journal.transition_items(&id, &transitions, None),
            JournalOp::PackStart { .. } => Err(JournalError::ItemMissing(
                "an update is not a pack apply".to_string(),
            )),
        }
    }
}

fn update_error(e: mm_core::updates::UpdateError) -> IpcError {
    match &e {
        mm_core::updates::UpdateError::NeedsConcreteTarget => IpcError::invalid_input(e),
        mm_core::updates::UpdateError::Provider(p) => IpcError::from_provider(p),
        mm_core::updates::UpdateError::Lock(l) => IpcError::from_lock(l),
    }
}

fn apply_error(e: UpdateApplyError) -> IpcError {
    match &e {
        // F-121: the disk moved, the request was fine.
        UpdateApplyError::ContextChanged { .. } => {
            IpcError::new(ErrorCode::UpdateContextChanged, e)
        }
        // F-123: no silent default.
        UpdateApplyError::BackupNotConfirmed => {
            IpcError::new(ErrorCode::UpdateBackupNotConfirmed, e)
        }
        UpdateApplyError::Cancelled => IpcError::new(ErrorCode::Cancelled, e),
        UpdateApplyError::EmptyPlan | UpdateApplyError::NeedsTarget => IpcError::invalid_input(e),
        UpdateApplyError::Backup(b) => super::backup::backup_error(b),
        UpdateApplyError::Download(d) => IpcError::io(d),
        UpdateApplyError::Journal(j) => super::journal_error(j),
        UpdateApplyError::Lock(l) => IpcError::from_lock(l),
        UpdateApplyError::Io { .. } => IpcError::io(e),
    }
}
