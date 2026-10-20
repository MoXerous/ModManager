//! Modpack (`.mrpack`) install over IPC: plan first, apply second.
//!
//! `mm_core::modpack` already owns every decision the format makes — what the
//! index asks for, which override layer wins, which destinations are refused,
//! and how a verified file is committed. This module is the bridge and nothing
//! else: it validates the two paths the webview sent, runs the blocking archive
//! work off the async runtime, and streams `PackEvent`s through the one
//! throttle in `progress.rs` (AGENTS.md §5.7).
//!
//! Both commands take [`ModpackInstallRequest`]. The apply command re-reads and
//! re-plans the archive server-side; a plan body from the webview is never
//! trusted, the same rule `execute_toggle` and `execute_create_instance` follow.
//!
//! # Staging
//!
//! The extract root is `<instance_root>/.mm-staging`, the same directory the
//! executor stages downloads in. One directory for both keeps every rename on
//! the instance's own filesystem and therefore atomic (AGENTS.md §5.5).
//!
//! Core's executor deliberately skips its "remove staging first" step when the
//! caller extracted into that same tree, so the reset belongs here: this
//! module removes a stale `.mm-staging` before reading the archive, and removes
//! it again if the archive is refused before the executor ever runs. Core owns
//! the end of a run, success or failure.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tauri::ipc::Channel;
use zip::ZipArchive;

use mm_core::modpack::archive::{plan_install, read_pack, InstallPlan, PackError};
use mm_core::modpack::exec::{self, ExecError, PackEvent};
use mm_core::modpack::mrpack::{Pack, STAGING_DIR};
use mm_core::safety::extract::{extract_safely, ExtractError};
use mm_core::safety::inspect::{ArchiveLimits, InspectError};

use crate::dto::{
    ErrorCode, IpcError, ModpackApplyResultDto, ModpackInstallRequest, ModpackPlanDto,
    PackWriteDto, PackWriteKindDto,
};
use crate::progress::{
    is_decisive_pack_phase, pack_event_to_dto, refused_pack_event, ProgressThrottle,
};

/// Mirrors `modpack::exec`'s bounded retry: a scanner or an antivirus can hold
/// a fresh handle for a moment on Windows.
const STAGING_REMOVE_ATTEMPTS: u32 = 3;
const STAGING_REMOVE_WAIT_MS: u64 = 50;

/// Opens a pack archive. Every refusal `archive` raises is a property of the
/// archive, not of the disk: the format, the names, the limits and the
/// reserved destinations are input rules. That is why one code covers them —
/// the UI's `error.invalidInput` entry is exactly the right sentence, and the
/// variant's own `Display` stays in `message` for the log (AGENTS.md §5.3).
fn pack_error(e: &PackError) -> IpcError {
    IpcError::invalid_input(e)
}

/// Same reasoning as [`pack_error`], plus the three failure shapes that are
/// not the user's input at all.
fn extract_error(e: &ExtractError) -> IpcError {
    use mm_core::safety::extract::ExtractError as X;
    match e {
        X::Inspect(InspectError::Name(..))
        | X::Inspect(InspectError::TooManyMembers)
        | X::Inspect(InspectError::Duplicate(_))
        | X::Inspect(InspectError::Encrypted(_))
        | X::Inspect(InspectError::LinkOrSpecial(_))
        | X::Inspect(InspectError::MemberTooLarge(_))
        | X::Inspect(InspectError::TotalTooLarge)
        | X::Inspect(InspectError::BadRatio(_))
        | X::Inspect(InspectError::ReadLimit(_))
        | X::Inspect(InspectError::NotFound)
        | X::Inspect(InspectError::IsDirectory(_))
        | X::Escape(_)
        | X::TooLong(_)
        | X::Empty
        | X::ReservedPartSuffix(_) => IpcError::invalid_input(e),
        X::Atomic(_) | X::Io { .. } => IpcError::io(e),
        // Nobody cancels an extraction here, so reaching this is our bug.
        X::Cancelled => IpcError::internal(e),
    }
}

/// One `ExecError` in one place, so plan and apply cannot disagree.
fn exec_error(e: &ExecError) -> IpcError {
    use mm_core::modpack::exec::ExecError as E;
    match e {
        // Refusals: an unsafe name, a directory-shaped destination, a reserved
        // path, or a pre-existing link that would redirect the write. The
        // archive (or the instance) cannot be installed into as asked.
        E::UnsafePath { .. } | E::NotAFile { .. } | E::ReservedPath { .. } | E::Escape(_) => {
            IpcError::invalid_input(e)
        }
        // No source can supply the hash the pack pinned. "Not found" is the
        // honest code: the file exists in no place, and retrying the same
        // apply will ask the same question.
        E::NoDownloadSource { .. } => IpcError::new(ErrorCode::NotFound, e),
        // The plan names an override the extraction should have produced. The
        // tree is ours, so this is a bug in staging, not the user's pack.
        E::MissingOverride { .. } | E::Internal(_) => IpcError::internal(e),
        // Corrupt or tampered bytes, a failed rename, a failing disk.
        E::HashMismatch { .. } | E::Atomic(_) | E::Download(_) | E::Io { .. } => IpcError::io(e),
    }
}

/// Canonicalizes a webview-supplied file: missing or not a file → typed error.
///
/// No extension check: a `.mrpack` the user renamed to something else is still
/// a pack, and the content gate (`read_pack`) is the authority on that. The
/// suffix would only add a way to refuse a valid file.
fn resolve_file(raw: &str) -> Result<PathBuf, IpcError> {
    let p = PathBuf::from(raw.trim());
    if p.as_os_str().is_empty() {
        return Err(IpcError::invalid_input("empty path"));
    }
    let canon = std::fs::canonicalize(&p).map_err(|e| {
        IpcError::new(
            ErrorCode::NotFound,
            format!("cannot open {}: {e}", p.display()),
        )
    })?;
    if !canon.is_file() {
        return Err(IpcError::invalid_input("not a file"));
    }
    Ok(canon)
}

fn open_archive(pack_path: &Path) -> Result<ZipArchive<std::fs::File>, IpcError> {
    let file = std::fs::File::open(pack_path)
        .map_err(|e| IpcError::io(format!("cannot open pack {}: {e}", pack_path.display())))?;
    ZipArchive::new(file).map_err(|e| {
        // A file that is not a zip (or is truncated) is the wrong file, not a
        // failing disk: `invalidInput` says exactly that.
        IpcError::invalid_input(format!(
            "{} is not a readable .mrpack archive: {e}",
            pack_path.display()
        ))
    })
}

/// Reads the archive, plans it, and gates the plan against the instance.
///
/// Both commands call this, which is the whole reason it exists: the apply's
/// claim is that it decided exactly what the screen was shown, and two copies
/// of this sequence would be free to drift.
///
/// Writes nothing when `staging` is `None` — no extraction, no staging
/// directory, no instance file. When it is `Some`, the archive is extracted
/// there for the override copies, and only after the gate above: a refused
/// plan never lands a byte anywhere. Blocking: called through
/// `spawn_blocking`.
fn read_plan(
    pack_path: &Path,
    instance_root: &Path,
    staging: Option<&Path>,
) -> Result<(Pack, InstallPlan, Vec<exec::PlannedWrite>), IpcError> {
    let limits = ArchiveLimits::default();
    let mut archive = open_archive(pack_path)?;
    let (pack, inventory) = read_pack(&mut archive, &limits).map_err(|e| pack_error(&e))?;
    let plan = plan_install(&pack, &inventory).map_err(|e| pack_error(&e))?;
    // The plan's paths get the same gate the apply gives them, before anything
    // is extracted: an unsafe or reserved destination is refused while the
    // staging tree is still empty.
    let writes = exec::dry_run(&plan, instance_root).map_err(|e| exec_error(&e))?;
    if let Some(staging) = staging {
        extract_safely(&mut archive, staging, &limits, &mut |_| {
            ControlFlow::Continue(())
        })
        .map_err(|e| extract_error(&e))?;
    }
    Ok((pack, plan, writes))
}

fn plan_dto(pack: &Pack, writes: &[exec::PlannedWrite]) -> ModpackPlanDto {
    let mut download_count = 0;
    let mut override_count = 0;
    let mut replace_count = 0;
    let entries: Vec<PackWriteDto> = writes
        .iter()
        .map(|w| {
            if w.overwrites {
                replace_count += 1;
            }
            let (kind, size) = match &w.kind {
                exec::PlannedWriteKind::Override => {
                    override_count += 1;
                    (PackWriteKindDto::Override, None)
                }
                exec::PlannedWriteKind::Download { size, .. } => {
                    download_count += 1;
                    (PackWriteKindDto::Download, *size)
                }
            };
            PackWriteDto {
                safe_path: w.safe_path.clone(),
                kind,
                overwrites: w.overwrites,
                size,
            }
        })
        .collect();
    ModpackPlanDto {
        name: pack.name.clone(),
        version_id: pack.version_id.clone(),
        summary: pack.summary.clone(),
        writes: entries,
        download_count,
        override_count,
        replace_count,
    }
}

/// Removes `<instance_root>/.mm-staging` with bounded retries, off the async
/// runtime. Best effort, like core's cleanup: the next apply removes it first,
/// so a handle held by another process cannot accumulate staging trees.
async fn reset_staging(staging: PathBuf) {
    let _ = tauri::async_runtime::spawn_blocking(move || {
        for attempt in 1..=STAGING_REMOVE_ATTEMPTS {
            match std::fs::remove_dir_all(&staging) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) if attempt < STAGING_REMOVE_ATTEMPTS => {
                    std::thread::sleep(std::time::Duration::from_millis(
                        STAGING_REMOVE_WAIT_MS * u64::from(attempt),
                    ));
                }
                Err(_) => return,
            }
        }
    })
    .await;
}

/// What a `.mrpack` would write into an instance, without writing any of it.
///
/// The dry-run half of AGENTS.md §5.4: nothing is extracted, no staging
/// directory is created, and no instance file is touched. `overwrites` is
/// answered from the filesystem, so the screen can show replace vs new before
/// the user confirms.
#[tauri::command]
pub async fn plan_modpack_install(req: ModpackInstallRequest) -> Result<ModpackPlanDto, IpcError> {
    let pack_path = resolve_file(&req.pack_path)?;
    let instance_root = super::resolve_dir(&req.instance_root)?;
    // The plan itself stays here: it is what the writes were derived from, and
    // the apply rebuilds it rather than accepting one from the webview.
    let (pack, _plan, writes) =
        tauri::async_runtime::spawn_blocking(move || read_plan(&pack_path, &instance_root, None))
            .await
            .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))??;
    Ok(plan_dto(&pack, &writes))
}

/// Applies a `.mrpack` to an instance, streaming progress on a channel.
///
/// Production never sets `allow_insecure_loopback`: the executor's flag exists
/// only so a test can serve wiremock on loopback, exactly as
/// `execute_install` does.
#[tauri::command]
pub async fn execute_modpack_install(
    req: ModpackInstallRequest,
    on_event: Channel<crate::dto::PackEventDto>,
) -> Result<ModpackApplyResultDto, IpcError> {
    use mm_core::providers::ModrinthClient;
    let client = ModrinthClient::new().map_err(|e| IpcError::new(ErrorCode::Internal, e))?;
    execute_modpack_install_with(&client, req, on_event, false).await
}

/// The body of `execute_modpack_install`, with the provider as a parameter.
///
/// The seam exists so the whole command — read, plan, extract, download, hash,
/// atomic rename, cleanup and every `Channel` event — runs against a wiremock
/// server in a test. It is the same seam, for the same reason, as
/// `execute_install_with`.
pub async fn execute_modpack_install_with(
    provider: &dyn mm_core::providers::Provider,
    req: ModpackInstallRequest,
    on_event: Channel<crate::dto::PackEventDto>,
    allow_insecure_loopback: bool,
) -> Result<ModpackApplyResultDto, IpcError> {
    use mm_core::downloader::Downloader;

    let pack_path = resolve_file(&req.pack_path)?;
    let root = super::resolve_dir(&req.instance_root)?;
    let staging = root.join(STAGING_DIR);
    // Before the first byte is extracted: a failure here would otherwise leave
    // a full staging tree behind and a channel that never says why.
    let downloader = Downloader::new(3).map_err(|e| IpcError::internal(e.to_string()))?;

    // `.mm-staging` exists only while an apply runs. A tree left by a killed
    // run is never resumed, and core's executor keeps it when it is also the
    // extraction root, so the reset is this module's job.
    reset_staging(staging.clone()).await;

    let extracted = {
        let pack_path = pack_path.clone();
        let staging = staging.clone();
        let root = root.clone();
        tauri::async_runtime::spawn_blocking(move || read_plan(&pack_path, &root, Some(&staging)))
    }
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?;

    let (pack, plan, _writes) = match extracted {
        Ok(v) => v,
        Err(e) => {
            // A refused apply must end the stream on a terminal event: the
            // screen follows the channel, and a promise rejection alone would
            // leave it on the last progress event forever.
            let _ = on_event.send(refused_pack_event());
            // Nothing from the refused pack stays behind, index included.
            reset_staging(staging).await;
            return Err(e);
        }
    };

    let mut throttle = ProgressThrottle::new();
    let mut emit = |event: &PackEvent| {
        if !throttle.allows(is_decisive_pack_phase(event.phase), Instant::now()) {
            return;
        }
        // A closed channel means the window went away. The files are the point,
        // so the apply keeps going; nothing here could cancel it usefully.
        let _ = on_event.send(pack_event_to_dto(event));
    };

    let executor = exec::PlanExecutor::new(provider, &downloader, &pack, &plan, &staging, &root)
        .allow_insecure_loopback(allow_insecure_loopback);

    // Core removes the staging tree and emits the terminal event on both
    // paths, so a failure here has already finished narrating itself.
    let report: exec::ApplyReport = match executor.apply(&mut emit).await {
        Ok(report) => report,
        Err(e) => return Err(exec_error(&e)),
    };

    Ok(ModpackApplyResultDto {
        written: report.written.len(),
        skipped: report.skipped.len(),
        bytes: report.bytes,
    })
}
