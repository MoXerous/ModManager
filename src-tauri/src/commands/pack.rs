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
//! # The instance root is a registry question
//!
//! A pack install mutates a folder, so "an absolute path the webview sent" is
//! not enough authority for it: the root must be one this app registered, the
//! same gate `open_instance_folder` applies. [`registered_root`] is the one
//! place that decides, and both commands go through it.
//!
//! # The applied archive is the planned archive
//!
//! [`read_plan`] hashes the exact bytes it parses (SHA-512, the algorithm the
//! format and `instance.lock.json` already use) and the plan returns that
//! digest. The apply must send it back; [`read_plan`] recomputes it for the
//! archive it is about to read and refuses when it differs. A file replaced
//! between the preview and the confirm therefore never writes anything the user
//! was not shown.
//!
//! # Staging
//!
//! The extract root is `<instance_root>/.mm-staging`, the same directory the
//! executor stages downloads in. One directory for both keeps every rename on
//! the instance's own filesystem and therefore atomic (AGENTS.md §5.5).
//!
//! Because that directory is shared by every apply into one instance, applies
//! against the same root are serialized here (see [`apply_lock_for`]): two
//! concurrent runs would otherwise reset, extract into and delete each other's
//! tree.
//!
//! Core's executor deliberately skips its "remove staging first" step when the
//! caller extracted into that same tree, so the reset belongs here: this
//! module removes a stale `.mm-staging` before reading the archive, and removes
//! it again if the archive is refused before the executor ever runs. Core owns
//! the end of a run, success or failure.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
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
use crate::state::AppState;

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

fn open_archive(pack_path: &Path) -> Result<(ZipArchive<std::fs::File>, String), IpcError> {
    let mut file = std::fs::File::open(pack_path)
        .map_err(|e| IpcError::io(format!("cannot open pack {}: {e}", pack_path.display())))?;
    // Hashed from the handle the archive is built on, not by reopening the
    // path: that keeps "the bytes this digest names" and "the bytes the plan
    // was derived from" the same read.
    let digest = sha512_of(&mut file)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| IpcError::io(format!("cannot rewind pack {}: {e}", pack_path.display())))?;
    let archive = ZipArchive::new(file).map_err(|e| {
        // A file that is not a zip (or is truncated) is the wrong file, not a
        // failing disk: `invalidInput` says exactly that.
        IpcError::invalid_input(format!(
            "{} is not a readable .mrpack archive: {e}",
            pack_path.display()
        ))
    })?;
    Ok((archive, digest))
}

/// SHA-512 of everything left in `reader`, lowercase hex.
///
/// The same algorithm and spelling the modpack format pins for its own files
/// and `instance.lock.json` records for installed ones, so one digest
/// vocabulary covers the whole install path.
fn sha512_of(reader: &mut impl Read) -> Result<String, IpcError> {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha512::new();
    let mut buf = [0u8; 65536];
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|e| IpcError::io(format!("cannot read pack: {e}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Reads the archive, plans it, and gates the plan against the instance.
///
/// Both commands call this, which is the whole reason it exists: the apply's
/// claim is that it decided exactly what the screen was shown, and two copies
/// of this sequence would be free to drift.
///
/// The returned digest is the SHA-512 of the archive these writes were read
/// from. `expected_digest` is the one the plan was shown with: when it is set
/// and does not match, the archive changed under the plan and nothing is
/// planned or extracted.
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
    expected_digest: Option<&str>,
) -> Result<(String, Pack, InstallPlan, Vec<exec::PlannedWrite>), IpcError> {
    let limits = ArchiveLimits::default();
    let (mut archive, digest) = open_archive(pack_path)?;
    if let Some(expected) = expected_digest {
        if !digest.eq_ignore_ascii_case(expected) {
            return Err(IpcError::invalid_input(format!(
                "{} is not the archive the plan was built from (planned {expected}, found {digest})",
                pack_path.display()
            )));
        }
    }
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
    Ok((digest, pack, plan, writes))
}

fn plan_dto(digest: &str, pack: &Pack, writes: &[exec::PlannedWrite]) -> ModpackPlanDto {
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
        // The token that must come back on apply: the screen can only send
        // back a digest it was given, so a pack swapped afterwards cannot be
        // applied under this plan's name.
        pack_sha512: digest.to_string(),
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

/// Canonicalizes a directory the webview named and requires the registry to
/// own it.
///
/// `/opt`, a home directory or any other folder on the machine is a directory
/// like any other, and a pack install mutates what is inside it. The registry
/// is this app's answer to "which folders do we manage", and it is the same
/// gate `open_instance_folder` uses: the row is read by path through
/// `find_by_path`, which canonicalizes both sides, so two spellings of one
/// folder cannot disagree.
///
/// `resolve_dir` runs first so a missing folder keeps its own answer (`io`,
/// as every command reports it) instead of being reclassified as "not
/// registered" — the two are different problems for the user.
async fn registered_root(state: &AppState, raw: &str) -> Result<PathBuf, IpcError> {
    let root = super::resolve_dir(raw)?;
    let queried = root.clone();
    state
        .with_db(move |db| {
            let row = mm_core::db::instances::find_by_path(db, &queried)
                .map_err(|e| super::row_error(&e))?;
            if row.is_none() {
                return Err(IpcError::invalid_input(
                    "that folder is not a registered instance",
                ));
            }
            Ok(())
        })
        .await?;
    Ok(root)
}

/// One async mutex per instance root, held for the whole apply.
///
/// `.mm-staging` is a single directory per instance, so two applies into one
/// root reset it, extract into it and delete it under each other's feet: one
/// run's cleanup removes overrides the other has not copied yet. Applies into
/// *different* roots stay parallel, which is why this is keyed by root.
///
/// In-process, not a file lock: every apply in this app arrives through this
/// module, and the app is a single process per instance folder. `core`'s
/// `instances::lock` is `instance.lock.json`, pinned install *state*, not a
/// mutual-exclusion lock, and there is no cross-process helper there to reuse.
/// The key is the canonical root, so two spellings of one folder wait for each
/// other.
static APPLY_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tauri::async_runtime::Mutex<()>>>>> =
    OnceLock::new();

fn apply_lock_for(root: &Path) -> Arc<tauri::async_runtime::Mutex<()>> {
    let registry = APPLY_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    // A poisoned registry means a panic while holding this table; the table
    // itself is still consistent (the only work under the lock is an insert),
    // so recover instead of failing an install for it.
    let mut map = registry.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(root.to_path_buf())
        .or_insert_with(|| Arc::new(tauri::async_runtime::Mutex::new(())))
        .clone()
}

/// Ends a run that stopped before core's executor existed.
///
/// Two things have to happen or the failure is silent and leaves garbage: the
/// terminal event (the screen follows the channel, not the promise, so a
/// refusal that sends nothing freezes the progress view on its last event) and
/// the staging reset (nothing this run extracted may survive it). Core owns
/// both once `apply` starts; until then they are this module's.
async fn abort_before_apply(
    on_event: &Channel<crate::dto::PackEventDto>,
    staging: PathBuf,
    e: IpcError,
) -> IpcError {
    let _ = on_event.send(refused_pack_event());
    reset_staging(staging).await;
    e
}

/// What a `.mrpack` would write into an instance, without writing any of it.
///
/// The dry-run half of AGENTS.md §5.4: nothing is extracted, no staging
/// directory is created, and no instance file is touched. `overwrites` is
/// answered from the filesystem, so the screen can show replace vs new before
/// the user confirms.
///
/// The returned `pack_sha512` is part of the answer, not decoration: it is the
/// only token that lets the apply prove it is installing the archive this plan
/// described.
#[tauri::command]
pub async fn plan_modpack_install(
    state: tauri::State<'_, AppState>,
    req: ModpackInstallRequest,
) -> Result<ModpackPlanDto, IpcError> {
    plan_modpack_install_with(state.inner(), req).await
}

/// The body of `plan_modpack_install`, with the state as a parameter.
///
/// The seam exists for the same reason `execute_install_with` does: a test can
/// drive the real registry gate without a Tauri app.
pub async fn plan_modpack_install_with(
    state: &AppState,
    req: ModpackInstallRequest,
) -> Result<ModpackPlanDto, IpcError> {
    let pack_path = resolve_file(&req.pack_path)?;
    let instance_root = registered_root(state, &req.instance_root).await?;
    // The plan itself stays here: it is what the writes were derived from, and
    // the apply rebuilds it rather than accepting one from the webview. The
    // digest of those bytes is the one thing the webview gets to keep.
    let (digest, pack, _plan, writes) = tauri::async_runtime::spawn_blocking(move || {
        read_plan(&pack_path, &instance_root, None, None)
    })
    .await
    .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))??;
    Ok(plan_dto(&digest, &pack, &writes))
}

/// Applies a `.mrpack` to an instance, streaming progress on a channel.
///
/// Production never sets `allow_insecure_loopback`: the executor's flag exists
/// only so a test can serve wiremock on loopback, exactly as
/// `execute_install` does.
#[tauri::command]
pub async fn execute_modpack_install(
    state: tauri::State<'_, AppState>,
    req: ModpackInstallRequest,
    on_event: Channel<crate::dto::PackEventDto>,
) -> Result<ModpackApplyResultDto, IpcError> {
    use mm_core::providers::ModrinthClient;
    let client = ModrinthClient::new().map_err(|e| IpcError::new(ErrorCode::Internal, e))?;
    execute_modpack_install_with(state.inner(), &client, req, on_event, false).await
}

/// The body of `execute_modpack_install`, with the provider as a parameter.
///
/// The seam exists so the whole command — read, plan, extract, download, hash,
/// atomic rename, cleanup and every `Channel` event — runs against a wiremock
/// server in a test. It is the same seam, for the same reason, as
/// `execute_install_with`.
pub async fn execute_modpack_install_with(
    state: &AppState,
    provider: &dyn mm_core::providers::Provider,
    req: ModpackInstallRequest,
    on_event: Channel<crate::dto::PackEventDto>,
    allow_insecure_loopback: bool,
) -> Result<ModpackApplyResultDto, IpcError> {
    use mm_core::downloader::Downloader;
    run_apply(
        state,
        provider,
        req,
        on_event,
        allow_insecure_loopback,
        Downloader::new(3),
    )
    .await
}

/// The apply itself, with the downloader's construction as a parameter.
///
/// `Downloader::new` fails when the HTTP client cannot be built, and that
/// failure is the first thing an apply does. It must not be the first thing it
/// *loses*: the run still owes the screen a terminal event and the instance a
/// clean staging tree, so the failure is handled inside this function like
/// every other pre-apply refusal. The parameter is the seam that lets a test
/// exercise that branch, which no real environment can be made to produce on
/// demand. Production passes exactly `Downloader::new(3)`.
pub(super) async fn run_apply(
    state: &AppState,
    provider: &dyn mm_core::providers::Provider,
    req: ModpackInstallRequest,
    on_event: Channel<crate::dto::PackEventDto>,
    allow_insecure_loopback: bool,
    downloader: Result<mm_core::downloader::Downloader, mm_core::downloader::client::DownloadError>,
) -> Result<ModpackApplyResultDto, IpcError> {
    let pack_path = resolve_file(&req.pack_path)?;
    // The registry gate comes before any staging work: a folder this app does
    // not own is never touched, not even to remove a stale tree inside it.
    let root = registered_root(state, &req.instance_root).await?;
    let staging = root.join(STAGING_DIR);

    // Serialized before the first reset: two applies on one root share the
    // staging tree, and one run's cleanup would delete the other's overrides.
    // The guard is held for the whole run, and dropped on every return path
    // because it is a local.
    let apply_lock = apply_lock_for(&root);
    let _apply_guard = apply_lock.lock().await;

    // No digest, no apply: the plan the screen showed is the only thing that
    // authorizes these writes, and an absent token means there is no such plan.
    let Some(expected) = req
        .pack_sha512
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return Err(abort_before_apply(
            &on_event,
            staging,
            IpcError::invalid_input(
                "no pack digest: the archive must be planned before it is applied",
            ),
        )
        .await);
    };

    let downloader = match downloader {
        Ok(downloader) => downloader,
        Err(e) => {
            return Err(
                abort_before_apply(&on_event, staging, IpcError::internal(e.to_string())).await,
            );
        }
    };

    // `.mm-staging` exists only while an apply runs. A tree left by a killed
    // run is never resumed, and core's executor keeps it when it is also the
    // extraction root, so the reset is this module's job.
    reset_staging(staging.clone()).await;

    let extracted = {
        let pack_path = pack_path.clone();
        let staging = staging.clone();
        let root = root.clone();
        let expected = expected.clone();
        tauri::async_runtime::spawn_blocking(move || {
            read_plan(&pack_path, &root, Some(&staging), Some(&expected))
        })
    }
    .await;

    let (_digest, pack, plan, _writes) = match extracted {
        Ok(Ok(v)) => v,
        // A refused apply must end the stream on a terminal event and leave no
        // extracted tree: the screen follows the channel, and a promise
        // rejection alone would leave it on the last event forever.
        Ok(Err(e)) => return Err(abort_before_apply(&on_event, staging, e).await),
        Err(e) => {
            return Err(abort_before_apply(
                &on_event,
                staging,
                IpcError::internal(format!("blocking task failed: {e}")),
            )
            .await);
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
