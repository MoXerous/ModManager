//! F-145: the unified, read-only operations view across kinds.
//!
//! The bridge is thin on purpose: the listing, the per-kind assessment and
//! every reason/action key are `mm_core::operations`'. What lives here is the
//! DTO mapping, the typed error for a request the journal cannot answer, and
//! the rule that nothing on this path mutates anything.
//!
//! Two rules inherited from the per-kind surfaces:
//!
//! - **Only an id goes back.** The assessor takes the operation id the table
//!   showed; the target, the instance and the disk evidence all come from the
//!   record, never from the webview.
//! - **No mutation lives here.** Where a recovery exists it is offered as a
//!   key from the surface that owns the kind, and the screen links to that
//!   surface. This module never resumes, dismisses or settles anything.

use mm_core::journal::OperationKind;
use mm_core::operations::{self, AssessError, OperationAssessment, OperationSummary};

use crate::dto::{ErrorCode, IpcError, OperationAssessmentDto, OperationSummaryDto};
use crate::state::AppState;

use super::pack::operation_state_word;

/// Every record the journal holds, newest first (F-145).
///
/// A scan, not a query: terminal records are listed too, because the state
/// column is what separates live work from history on this screen.
#[tauri::command]
pub async fn list_operations(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<OperationSummaryDto>, IpcError> {
    list_operations_with(state.inner()).await
}

/// The body of [`list_operations`], with the state as a parameter.
pub async fn list_operations_with(state: &AppState) -> Result<Vec<OperationSummaryDto>, IpcError> {
    let rows = state
        .with_db_and_journal(|db, journal| Ok(operations::list_operations(db, journal)))
        .await?;
    Ok(rows.into_iter().map(summary_dto).collect())
}

/// Assesses one arbitrary operation id, read-only.
///
/// The id is the only token the UI may send: a row can vanish between the
/// table read and the click, and that is answered as `notFound`, not as a
/// failing disk.
#[tauri::command]
pub async fn assess_operation(
    state: tauri::State<'_, AppState>,
    operation_id: String,
) -> Result<OperationAssessmentDto, IpcError> {
    assess_operation_with(state.inner(), operation_id).await
}

/// The body of [`assess_operation`], with the state as a parameter.
pub async fn assess_operation_with(
    state: &AppState,
    operation_id: String,
) -> Result<OperationAssessmentDto, IpcError> {
    let assessed = state
        .with_db_and_journal(move |db, journal| {
            operations::assess_operation(db, journal, &operation_id).map_err(|e| assess_error(&e))
        })
        .await?;
    Ok(assessment_dto(assessed))
}

/// The words are the record's own, not a second spelling the UI must learn.
fn kind_word(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::DownloadBatch => "download_batch",
        OperationKind::UpdateDownload => "update_download",
        OperationKind::UpdateInstall => "update_install",
        OperationKind::MrpackInstall => "mrpack_install",
        OperationKind::InstanceCreate => "instance_create",
    }
}

/// Splits the caller's mistakes from ours, one variant at a time.
///
/// A path-shaped id is malformed input, refused before it could become a
/// path; a well-formed id the journal does not hold is a stale row the user
/// fixes with Refresh; a journal that cannot be read is a real failure and
/// keeps its own code.
fn assess_error(e: &AssessError) -> IpcError {
    match e {
        AssessError::MalformedId(_) => IpcError::invalid_input(e),
        AssessError::Unknown(_) => IpcError::new(ErrorCode::NotFound, e),
        AssessError::Journal(j) => super::journal_error(j),
    }
}

fn summary_dto(row: OperationSummary) -> OperationSummaryDto {
    OperationSummaryDto {
        operation_id: row.operation_id,
        kind: kind_word(row.kind).to_string(),
        state: operation_state_word(row.state).to_string(),
        created_at: row.created_at,
        updated_at: row.updated_at,
        profile_id: row.profile_id,
        target_path: row.target_path,
        planned_count: row.planned_count,
        completed_count: row.completed_count,
        failed_count: row.failed_count,
        cancelled_count: row.cancelled_count,
        skipped_count: row.skipped_count,
        pack_name: row.pack_name,
        reason: row.reason,
    }
}

fn assessment_dto(assessed: OperationAssessment) -> OperationAssessmentDto {
    OperationAssessmentDto {
        operation_id: assessed.operation_id,
        kind: kind_word(assessed.kind).to_string(),
        state: operation_state_word(assessed.state).to_string(),
        target_path: assessed.target_path,
        staging_present: assessed.staging_present,
        backup_present: assessed.backup_present,
        item_count: assessed.item_count,
        final_present: assessed.final_present,
        part_present: assessed.part_present,
        action: assessed.action,
        reason: assessed.reason,
    }
}
