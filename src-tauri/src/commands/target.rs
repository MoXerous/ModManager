//! Target-version planner and metadata prune over IPC (F-125..F-127,
//! F-130..F-132).
//!
//! The planner is read-only by construction: core returns facts, this file
//! maps them. The prune is the one write here, and it follows the same rule as
//! every other mutation: the webview carries a token, the bridge rebuilds the
//! plan server-side, and a token that does not match the fresh one is refused
//! rather than trusted (F-121 shape).

use mm_core::providers::Provider;
use mm_core::updates::prune::PruneError;
use mm_core::updates::target::{plan_targets, TargetError, TargetRequest};

use crate::dto::{
    ExecutePruneRequest, IpcError, PrunePlanDto, PrunePlanRequest, PruneReportDto, TargetPlanDto,
    TargetPlanRequest,
};
use crate::state::AppState;

/// F-125: the eight-column plan, plus the F-126 suggestion and the F-127
/// cleanup report. No provider call is made for a missing concrete target.
#[tauri::command]
pub async fn plan_target_versions(
    state: tauri::State<'_, AppState>,
    req: TargetPlanRequest,
) -> Result<TargetPlanDto, IpcError> {
    let provider = crate::providers::build(state.inner(), &req.provider)?;
    plan_target_versions_with(provider.as_ref(), req).await
}

/// The body with the provider as a parameter: the bridge's own job — resolving
/// the folder and mapping the typed errors — is testable without a network.
pub(crate) async fn plan_target_versions_with(
    provider: &dyn Provider,
    req: TargetPlanRequest,
) -> Result<TargetPlanDto, IpcError> {
    let root = super::resolve_dir(&req.instance_root)?;
    let request = TargetRequest::new(&req.game_version, &req.loader);
    plan_targets(provider, &root, &request)
        .await
        .map(Into::into)
        .map_err(target_error)
}

fn target_error(error: TargetError) -> IpcError {
    match error {
        TargetError::NeedsConcreteTarget => IpcError::invalid_input(error.to_string()),
        TargetError::Lock(lock) => IpcError::from_lock(&lock),
    }
}

fn prune_error(error: PruneError) -> IpcError {
    match error {
        // A moved lock is a plan the user must look at again, not malformed
        // input (same code the update confirm uses for the same reason).
        PruneError::LockChanged => IpcError::new(
            crate::dto::ErrorCode::UpdateContextChanged,
            error.to_string(),
        ),
        PruneError::Lock(lock) => IpcError::from_lock(&lock),
    }
}

/// F-130: the mandatory dry run. Reads the scan and the lock; writes nothing.
#[tauri::command]
pub async fn preview_lock_prune(req: PrunePlanRequest) -> Result<PrunePlanDto, IpcError> {
    let root = super::resolve_dir(&req.instance_root)?;
    tauri::async_runtime::spawn_blocking(move || {
        mm_core::updates::prune::plan_prune(&root)
            .map(Into::into)
            .map_err(prune_error)
    })
    .await
    .map_err(IpcError::internal)?
}

/// F-130: remove only the confirmed entries whose file is still absent. The
/// plan is rebuilt here and must match `expected_digest`; a plan body from the
/// webview is never trusted. The digest covers the lock's bytes AND the
/// previewed candidate names, so a fresh plan that would remove a record the
/// user never saw cannot pass.
#[tauri::command]
pub async fn execute_lock_prune(
    state: tauri::State<'_, AppState>,
    req: ExecutePruneRequest,
) -> Result<PruneReportDto, IpcError> {
    let root = super::resolve_dir(&req.instance_root)?;
    // The lock is the app's own write: the watcher must not read it as staleness.
    state.note_own_write(&root);
    tauri::async_runtime::spawn_blocking(move || execute_lock_prune_at(&root, &req))
        .await
        .map_err(IpcError::internal)?
}

/// The body of [`execute_lock_prune`] with the folder already resolved, so the
/// token check is testable without a WebView.
pub(crate) fn execute_lock_prune_at(
    root: &std::path::Path,
    req: &ExecutePruneRequest,
) -> Result<PruneReportDto, IpcError> {
    let plan = mm_core::updates::prune::plan_prune(root).map_err(prune_error)?;
    if plan.digest() != req.expected_digest {
        return Err(IpcError::new(
            crate::dto::ErrorCode::UpdateContextChanged,
            "the instance lock changed since the dry run",
        ));
    }
    mm_core::updates::prune::execute_prune(root, plan.confirm())
        .map(Into::into)
        .map_err(prune_error)
}
