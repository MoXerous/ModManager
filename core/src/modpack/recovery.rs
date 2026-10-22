//! F-076: what the app may offer after a crashed or failed pack apply.
//!
//! A pack apply writes into a pre-existing, registered instance, in place,
//! with no backup. A crash — or a plain failure — therefore leaves the
//! instance HALF APPLIED: some files are in place, some are not, and
//! nothing on the disk says so. Stage 1 made that state *describable* (the
//! journal record); this module makes it *recoverable*.
//!
//! Three decisions shape everything here, and they are not up for debate
//! again:
//!
//! 1. **Resume, never rollback.** A rollback would delete the user's own
//!    files — the worst possible guess. The filesystem is never torn (every
//!    write is `.part` → fsync → atomic rename), so there is no partial
//!    state a rollback would repair. The resume is idempotent and
//!    convergent: running it twice writes the same bytes.
//!
//! 2. **A failed apply leaves the operation RUNNING**, not `failed`. A
//!    failed apply leaves the instance half applied exactly like a crash,
//!    so the app must be able to describe it and offer the recovery. That
//!    is the convention `downloader::execute_batch` already follows.
//!
//! 3. **The disk decides, not the counters.** A record's item states are
//!    written one file late (the ending of a file rides in the next file's
//!    write, so the record is one write per file), so after a crash the
//!    last item may still read `committing` although it reached the disk.
//!    The recovery measures every destination on the disk and compares it
//!    with the hash the plan pinned.
//!
//! One deliberate asymmetry, stated here so it is not mistaken for a bug
//! later: for a TERMINAL item the JOURNAL wins (the file is skipped, so a
//! user's post-crash edit to a finished file survives), and for a
//! `committing` item the DISK wins (the rename may or may not have landed,
//! so the disk decides).
//!
//! The assessment ([`pending_pack_applies`]) is read-only, and nothing is
//! written until the user chose [`PackRecoveryAction::Resume`]. Even then
//! the recovery takes an OPERATION ID, never a path: the root, the pack and
//! the staging tree all come from the record, so the webview cannot name a
//! folder for core to act on — the same rule `instances::recovery::finish`
//! follows.

use std::io::{Seek, SeekFrom};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use super::archive::{plan_install, read_pack, InstallOp, InstallPlan, PackError};
use super::exec::{
    self, checked_relative, sha512_of_file, sha512_of_reader, ExecError, PackEvent, PackIdentity,
    PlanExecutor,
};
use super::mrpack::{Pack, STAGING_DIR};
use crate::downloader::Downloader;
use crate::journal::{
    pack_item, ItemState, Journal, JournalError, JournalOp, OperationItem, OperationKind,
    OperationRecord, OperationState, PackApplyMeta,
};
use crate::providers::Provider;
use crate::safety::atomic::part_path;
use crate::safety::extract::{extract_safely, ExtractError};
use crate::safety::inspect::ArchiveLimits;

/// What the user can be offered about an interrupted pack apply.
/// Never applied without a deliberate choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackRecoveryAction {
    /// Finish the apply: the files that are missing are written.
    Resume,
    /// Close the case without touching the instance.
    Dismiss,
}

impl PackRecoveryAction {
    /// Stable key for the UI to translate, never a sentence.
    pub fn key(self) -> &'static str {
        match self {
            Self::Resume => "recovery.pack.action.resume",
            Self::Dismiss => "recovery.pack.action.dismiss",
        }
    }
}

/// Why, as data. The UI translates it; core does not write copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackRecoveryReason {
    /// Not a pack apply, or already completed.
    NotApplicable,
    /// The apply died before its first file.
    NothingWritten,
    /// Some files are in place, some are not.
    PartiallyApplied,
    /// Every file is in place; only the commit is missing.
    FullyApplied,
    /// The .mrpack is no longer where the record says it was.
    PackMissing,
    /// The .mrpack is there but is not the archive the plan was built from.
    PackChanged,
    /// The instance folder is gone.
    InstanceMissing,
}

impl PackRecoveryReason {
    /// Stable key for the UI to translate, never a sentence.
    ///
    /// The stage that wires this into the UI adds the matching entries to
    /// both locales; core only names the key.
    pub fn key(self) -> &'static str {
        match self {
            Self::NotApplicable => "recovery.pack.notApplicable",
            Self::NothingWritten => "recovery.pack.nothingWritten",
            Self::PartiallyApplied => "recovery.pack.partiallyApplied",
            Self::FullyApplied => "recovery.pack.fullyApplied",
            Self::PackMissing => "recovery.pack.packMissing",
            Self::PackChanged => "recovery.pack.packChanged",
            Self::InstanceMissing => "recovery.pack.instanceMissing",
        }
    }
}

/// One interrupted pack apply, described.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackRecovery {
    pub operation_id: String,
    pub instance_root: PathBuf,
    pub pack_name: String,
    pub pack_version_id: String,
    /// The digest of the archive the plan was built from.
    pub pack_sha512: String,
    /// How many destinations the plan names.
    pub planned_count: usize,
    /// How many of them are in place — the evidence, not the record's own
    /// counters: a `committing` item whose rename landed is in place even
    /// though the record does not say so yet.
    pub done_count: usize,
    pub action: PackRecoveryAction,
    pub reason: PackRecoveryReason,
}

/// What a recovery did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackRecoveryReport {
    pub operation_id: String,
    /// Files this run wrote.
    pub written: usize,
    /// Files that were already in place and were left alone.
    pub skipped: usize,
    /// Files that could not be written.
    pub failed: usize,
    /// The record's state when the run ended.
    pub state: OperationState,
}

/// Why a recovery could not run.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error(transparent)]
    Pack(#[from] PackError),
    #[error(transparent)]
    Extract(#[from] ExtractError),
    /// The record is not a pack apply. Both mutating entry points are
    /// reached by operation id, so a download batch's id must not be able
    /// to make the recovery touch an instance.
    #[error("operation {0} is not a pack apply")]
    NotAPackApply(String),
    /// The case is already closed: nothing to do, and nothing is written.
    #[error("operation {0} has already finished")]
    AlreadyFinished(String),
    /// The instance folder is gone. The recovery does not recreate it.
    #[error("the instance folder {0} is gone")]
    InstanceMissing(String),
    /// The archive is gone. Without it there are no bytes to resume with.
    #[error("the pack archive {0} is gone")]
    PackMissing(String),
    /// The archive is not the one the plan was built from.
    #[error("the pack archive is not the one the plan was built from")]
    PackChanged,
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the blocking task failed: {0}")]
    Join(String),
}

/// Crashed or failed pack applies that still need a decision. Read-only.
///
/// Nothing here writes, and nothing here mutates the disk: the assessment
/// looks at the record and at the filesystem and says what it found.
pub fn pending_pack_applies(journal: &Journal) -> Vec<PackRecovery> {
    journal
        .discover()
        .unfinished()
        .filter(|record| record.kind == OperationKind::MrpackInstall)
        .map(assess)
        .collect()
}

/// Inspects one pack-apply record. Never mutates anything.
///
/// The order of the questions is the order of the recovery's own: the
/// instance folder first (a folder the user deleted is not ours to
/// recreate), then the archive (without it there are no bytes to resume
/// with), then the archive's digest (the plan-digest binding), and only
/// then what is already in place.
fn assess(record: &OperationRecord) -> PackRecovery {
    let root = PathBuf::from(&record.target_path);
    let mut out = PackRecovery {
        operation_id: record.operation_id.clone(),
        instance_root: root.clone(),
        pack_name: String::new(),
        pack_version_id: String::new(),
        pack_sha512: String::new(),
        planned_count: record.items.len(),
        done_count: 0,
        action: PackRecoveryAction::Dismiss,
        reason: PackRecoveryReason::NotApplicable,
    };
    let Some(meta) = record.pack.as_ref() else {
        return out;
    };
    out.pack_name = meta.pack_name.clone();
    out.pack_version_id = meta.pack_version_id.clone();
    out.pack_sha512 = meta.archive_sha512.clone();

    if !root.is_dir() {
        out.reason = PackRecoveryReason::InstanceMissing;
        return out;
    }
    let archive = Path::new(&meta.archive_path);
    if !archive.is_file() {
        out.reason = PackRecoveryReason::PackMissing;
        return out;
    }
    // The plan-digest binding. The record holds the digest the plan was
    // built from, so the recovery needs no token from the webview — and a
    // pack that changed since cannot be resumed against.
    match sha512_of_file(archive) {
        Ok(digest) if digest.eq_ignore_ascii_case(&meta.archive_sha512) => {}
        Ok(_) => {
            out.reason = PackRecoveryReason::PackChanged;
            return out;
        }
        // Unreadable is not "changed": it is not there as far as the
        // recovery is concerned.
        Err(_) => {
            out.reason = PackRecoveryReason::PackMissing;
            return out;
        }
    }

    // How much of the pack is in place. The evidence is the disk: a
    // terminal item says the file landed (and the journal wins there, so
    // a user's edit survives), and every other item is asked of the
    // filesystem.
    let mut in_place = 0usize;
    for item in &record.items {
        if item.state.is_terminal() || Path::new(&item.final_path).is_file() {
            in_place += 1;
        }
    }
    out.done_count = in_place;
    out.reason = if record.items.is_empty() || in_place == 0 {
        PackRecoveryReason::NothingWritten
    } else if in_place == record.items.len() {
        PackRecoveryReason::FullyApplied
    } else {
        PackRecoveryReason::PartiallyApplied
    };
    out.action = PackRecoveryAction::Resume;
    out
}

/// Finishes an interrupted pack apply.
///
/// Takes an OPERATION ID, never a path: the root, the pack and the staging
/// tree all come from the record, so the webview cannot name a folder for
/// core to act on - the same rule `instances::recovery::finish` follows.
///
/// The `write` callback must write to the record `operation_id` names: it
/// is the executor's journal, and the recovery checks that every write
/// landed in that record and nowhere else.
pub async fn recover(
    provider: &dyn Provider,
    downloader: &Downloader,
    journal: &Journal,
    operation_id: &str,
    write: &mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
    on_event: &mut (dyn FnMut(&PackEvent) + Send),
) -> Result<PackRecoveryReport, RecoveryError> {
    recover_with(
        provider,
        downloader,
        journal,
        operation_id,
        write,
        on_event,
        false,
    )
    .await
}

/// [`recover`], with the loopback switch the tests need.
///
/// Production leaves the switch off, so the downloader stays https-only.
/// The seam exists for the reason `execute_modpack_install_with` has one:
/// a test can serve wiremock over loopback.
pub async fn recover_with(
    provider: &dyn Provider,
    downloader: &Downloader,
    journal: &Journal,
    operation_id: &str,
    write: &mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
    on_event: &mut (dyn FnMut(&PackEvent) + Send),
    allow_insecure_loopback: bool,
) -> Result<PackRecoveryReport, RecoveryError> {
    let record = journal.read(operation_id)?;
    let meta = require_pack(&record)?;
    if record.state.is_terminal() {
        // Nothing to do: the case is closed. No file is written and no
        // second record is created.
        return Ok(PackRecoveryReport {
            operation_id: operation_id.to_string(),
            written: 0,
            skipped: 0,
            failed: 0,
            state: record.state,
        });
    }
    let instance_root = PathBuf::from(&record.target_path);
    if !instance_root.is_dir() {
        // The folder the user deleted is not ours to recreate. The case is
        // settled: there is nothing left to resume, and a record left
        // running is offered again on every launch, forever.
        journal.transition_operation(operation_id, OperationState::Failed, None)?;
        return Err(RecoveryError::InstanceMissing(
            instance_root.to_string_lossy().into_owned(),
        ));
    }
    let archive_path = PathBuf::from(&meta.archive_path);
    if !archive_path.is_file() {
        // Without the archive there are no bytes to resume with. The
        // recovery says so; it does not guess.
        return Err(RecoveryError::PackMissing(
            archive_path.to_string_lossy().into_owned(),
        ));
    }

    // The archive is opened once and kept open: the digest is computed
    // from the same handle the plan and the extraction read, so the bytes
    // that are resumed against are the bytes the digest named.
    let limits = ArchiveLimits::default();
    let expected_digest = meta.archive_sha512.clone();
    let for_task = archive_path.clone();
    let items_for_task = record.items.clone();
    let root_for_task = instance_root.clone();
    let (pack, plan, work, mut archive) = tokio::task::spawn_blocking(
        move || -> Result<(Pack, InstallPlan, Work, ZipArchive<std::fs::File>), RecoveryError> {
            let mut file =
                std::fs::File::open(&for_task).map_err(|e| io_err("open", &for_task, e))?;
            let digest = sha512_of_reader(&mut file).map_err(|e| io_err("read", &for_task, e))?;
            if !digest.eq_ignore_ascii_case(&expected_digest) {
                return Err(RecoveryError::PackChanged);
            }
            file.seek(SeekFrom::Start(0))
                .map_err(|e| io_err("rewind", &for_task, e))?;
            let mut archive = ZipArchive::new(file)
                .map_err(|e| RecoveryError::Pack(PackError::Zip(e.to_string())))?;
            let (pack, inventory) = read_pack(&mut archive, &limits)?;
            let plan = plan_install(&pack, &inventory)?;
            // The plan is never read back from the record: the archive says
            // what the pack wants, the record says what was attempted.
            let work = classify(&plan, &items_for_task, &root_for_task);
            Ok((pack, plan, work, archive))
        },
    )
    .await
    .map_err(|e| RecoveryError::Join(e.to_string()))??;

    if work.missing.is_empty() && work.ambiguous.is_empty() {
        // Every file is in place; only the record's ending is missing. The
        // recovery writes nothing at all here.
        close_completed(journal, operation_id)?;
        on_event(&PackEvent::operation(exec::phase::DONE));
        return Ok(PackRecoveryReport {
            operation_id: operation_id.to_string(),
            written: 0,
            skipped: work.finished.len(),
            failed: 0,
            state: OperationState::Completed,
        });
    }

    // The staging tree must be empty before the extraction: `extract_safely`
    // writes every member with the atomic writer, and the atomic writer
    // refuses a `.part` it cannot attribute — which is exactly what the
    // tree of a killed run holds. The reset is the apply's own path, not a
    // second one.
    exec::remove_staging(&instance_root).await;
    let staging = instance_root.join(STAGING_DIR);
    let root_for_extract = instance_root.clone();
    let staging_for_extract = staging.clone();
    let ambiguous_for_extract = work.ambiguous.clone();
    let resolved = tokio::task::spawn_blocking(move || -> Result<Resolved, RecoveryError> {
        extract_safely(&mut archive, &staging_for_extract, &limits, &mut |_| {
            ControlFlow::Continue(())
        })?;
        resolve_ambiguous(
            &ambiguous_for_extract,
            &root_for_extract,
            &staging_for_extract,
        )
    })
    .await
    .map_err(|e| RecoveryError::Join(e.to_string()))??;

    // The work list: the destinations that are simply not in place, plus the
    // ambiguous ones whose rename had not landed.
    let mut redo = work.missing.clone();
    redo.extend(resolved.redo.iter().cloned());

    if redo.is_empty() {
        // Every file was in place after all: the ambiguous ones had landed.
        close_completed(journal, operation_id)?;
        on_event(&PackEvent::operation(exec::phase::DONE));
        return Ok(PackRecoveryReport {
            operation_id: operation_id.to_string(),
            written: 0,
            skipped: work.finished.len() + resolved.landed.len(),
            failed: 0,
            state: OperationState::Completed,
        });
    }

    // The `.part` an interrupted commit of THIS pack left behind is the
    // only one that may be removed: the journal is the only proof that a
    // file on the disk is ours, and a `.part` no record names is somebody
    // else's file. The write refuses a `.part` it cannot attribute, so this
    // is what keeps the pack installable.
    //
    // This runs BEFORE the item list is repaired: the repair can add an
    // item for a destination the record never mentioned, and a part at
    // such a destination is not ours to remove — the honest answer for it
    // is the `StalePart` refusal the write gives.
    let identity = PackIdentity {
        name: meta.pack_name.clone(),
        version_id: meta.pack_version_id.clone(),
        archive_sha512: meta.archive_sha512.clone(),
        archive_path: meta.archive_path.clone(),
    };
    exec::clear_interrupted_parts(journal, &identity, &instance_root);

    // The record's items, rebuilt against the plan. The items the redo will
    // move go back to `queued`: the state machine has no edge out of
    // `failed` or `committing` back to the start (a commit is irreversible
    // by design), and the redo has to walk the same chain every other file
    // walks. The destinations whose rename had landed become `completed`:
    // the disk says so, and the record follows the disk.
    let items = repair_items(
        &record,
        &plan,
        &instance_root,
        &meta.pack_name,
        &redo,
        &resolved.landed,
    )?;
    journal.replace_items(operation_id, items, None)?;

    let filtered = InstallPlan { ops: redo };
    let executor = PlanExecutor::new(
        provider,
        downloader,
        &pack,
        &filtered,
        &staging,
        &instance_root,
        identity,
    )
    .allow_insecure_loopback(allow_insecure_loopback)
    .resuming(operation_id);

    // Every write has to land in the record this recovery is about: the
    // executor's transitions name item ids, and a write into another
    // record would leave this one lying about its state.
    let mut guarded = |op: JournalOp| -> Result<OperationRecord, JournalError> {
        let written = write(op)?;
        if written.operation_id != operation_id {
            return Err(JournalError::Io(
                "the journal callback wrote to a different operation".into(),
            ));
        }
        Ok(written)
    };

    let outcome = executor.apply(&mut guarded, on_event).await;
    match outcome {
        Ok(report) => {
            let final_record = journal.read(operation_id)?;
            Ok(PackRecoveryReport {
                operation_id: operation_id.to_string(),
                written: report.written.len(),
                skipped: report.skipped.len() + work.finished.len() + resolved.landed.len(),
                failed: final_record
                    .items
                    .iter()
                    .filter(|item| item.state == ItemState::Failed)
                    .count(),
                state: final_record.state,
            })
        }
        // A failed recovery leaves the operation RUNNING with the failing
        // item `failed` (F-076): the instance is half applied, exactly like
        // a crash, and the case is still open.
        Err(e) => Err(RecoveryError::Exec(e)),
    }
}

/// Closes the case without touching the instance.
///
/// The user chose this, so the record ends `failed` - the same convention
/// as `instances::recovery::abandon`. Nothing on the disk is touched: the
/// half-applied files stay, because a rollback would delete the user's own
/// files.
pub fn dismiss(journal: &Journal, operation_id: &str) -> Result<(), RecoveryError> {
    let record = journal.read(operation_id)?;
    require_pack(&record)?;
    if record.state.is_terminal() {
        return Err(RecoveryError::AlreadyFinished(operation_id.to_string()));
    }
    journal.transition_operation(operation_id, OperationState::Failed, None)?;
    Ok(())
}

/// What the record says about the plan, split by what has to be done.
#[derive(Debug, Default)]
struct Work {
    /// The destinations that are not in place.
    missing: Vec<InstallOp>,
    /// The destinations whose rename may or may not have landed.
    ambiguous: Vec<InstallOp>,
    /// The destinations the record says are finished.
    finished: Vec<String>,
}

/// What the disk says about the ambiguous destinations.
#[derive(Debug, Default)]
struct Resolved {
    /// The destinations to write.
    redo: Vec<InstallOp>,
    /// The destinations whose rename had landed.
    landed: Vec<String>,
}

/// Splits the plan against the record.
///
/// The record's word is the rule for a terminal item: the file is left
/// alone, so a user's post-crash edit to a finished file survives. For a
/// `committing` item the record has nothing to say — the part was verified
/// and the rename was pending — so the disk decides, later.
fn classify(plan: &InstallPlan, items: &[OperationItem], instance_root: &Path) -> Work {
    let mut work = Work::default();
    for op in &plan.ops {
        let safe_path = op_path(op);
        match item_for(items, instance_root, safe_path) {
            Some(item) if matches!(item.state, ItemState::Completed | ItemState::Skipped) => {
                work.finished.push(safe_path.to_string());
            }
            Some(item) if item.state == ItemState::Committing => {
                work.ambiguous.push(op.clone());
            }
            // `queued`, `downloading`, `verifying` and `failed` all say the
            // file is not in place. So does a destination the record does
            // not mention at all: the plan says it belongs there.
            _ => work.missing.push(op.clone()),
        }
    }
    work
}

/// Decides what a `committing` item means, by measuring the disk.
///
/// The record says the part was verified and the rename was pending. The
/// disk knows whether the rename happened: the destination either holds
/// the bytes the plan pinned, or it does not.
fn resolve_ambiguous(
    ambiguous: &[InstallOp],
    instance_root: &Path,
    staging: &Path,
) -> Result<Resolved, RecoveryError> {
    let mut resolved = Resolved::default();
    for op in ambiguous {
        let safe_path = op_path(op);
        let expected = expected_hash(op, staging)?;
        let dest = instance_root.join(safe_path);
        let landed = dest.is_file()
            && sha512_of_file(&dest)
                .map(|actual| actual.eq_ignore_ascii_case(&expected))
                .unwrap_or(false);
        if landed {
            resolved.landed.push(safe_path.to_string());
        } else {
            resolved.redo.push(op.clone());
        }
    }
    Ok(resolved)
}

/// The hash a destination has to hold for this operation.
fn expected_hash(op: &InstallOp, staging: &Path) -> Result<String, RecoveryError> {
    match op {
        InstallOp::Download { sha512, .. } => Ok(sha512.to_lowercase()),
        InstallOp::CopyOverride {
            safe_path,
            source_layer,
        } => {
            // An override's bytes live in the archive, and the extraction
            // put them in the staging tree: the copy the executor will
            // make reads exactly those bytes.
            let layer = source_layer.strip_suffix('/').unwrap_or(source_layer);
            let src = staging.join(layer).join(safe_path);
            sha512_of_file(&src).map_err(|e| io_err("read", &src, e))
        }
    }
}

/// The record's item for one destination, if it has one.
///
/// The destination is matched by the path it lands at, and by the name the
/// plan gave it: either one identifies the item, and a record that spells
/// neither does not mention the destination at all.
fn item_for<'a>(
    items: &'a [OperationItem],
    instance_root: &Path,
    safe_path: &str,
) -> Option<&'a OperationItem> {
    let wanted = instance_root.join(safe_path).to_string_lossy().into_owned();
    items
        .iter()
        .find(|item| item.final_path == wanted)
        .or_else(|| items.iter().find(|item| item.item_id == safe_path))
}

/// The record's item list, rebuilt against the plan.
///
/// Three things can be wrong with the list a crashed apply left. A
/// destination the plan names and the record does not mention gets an item,
/// so the executor has something to write under. A destination the record
/// mentions under another name is renamed to the plan's spelling, because
/// that is the name the executor writes under. And an item whose state
/// cannot legally move again — `failed` or `committing` — is put back at
/// `queued`, because the redo has to walk the same chain every other file
/// walks and the state machine has no edge back to the start.
///
/// The list is rewritten in one write: a record is a single document, and
/// the recovery is not the hot path the apply is.
fn repair_items(
    record: &OperationRecord,
    plan: &InstallPlan,
    instance_root: &Path,
    pack_name: &str,
    redo: &[InstallOp],
    landed: &[String],
) -> Result<Vec<OperationItem>, RecoveryError> {
    let mut items = record.items.clone();
    for op in &plan.ops {
        let safe_path = op_path(op);
        let rel = checked_relative(safe_path)?;
        let dest = instance_root.join(&rel);
        let dest_text = dest.to_string_lossy().into_owned();
        if items.iter().any(|item| item.item_id == safe_path) {
            continue;
        }
        match items.iter_mut().find(|item| item.final_path == dest_text) {
            Some(item) => {
                item.item_id = safe_path.to_string();
            }
            None => {
                items.push(pack_item(
                    safe_path,
                    pack_name,
                    &part_path(&dest).to_string_lossy(),
                    &dest_text,
                ));
            }
        }
    }
    for op in redo {
        let safe_path = op_path(op);
        if let Some(item) = items.iter_mut().find(|item| item.item_id == safe_path) {
            item.state = ItemState::Queued;
        }
    }
    for path in landed {
        if let Some(item) = items.iter_mut().find(|item| item.item_id == path.as_str()) {
            item.state = ItemState::Completed;
        }
    }
    Ok(items)
}

/// Closes a record whose files are all in place.
///
/// `planned → completed` is not an edge, so a record that never started
/// running is moved to `running` first. The state machine refuses anything
/// else, which is the point: a record is closed by a legal move.
fn close_completed(journal: &Journal, operation_id: &str) -> Result<(), RecoveryError> {
    let record = journal.read(operation_id)?;
    if record.state == OperationState::Planned {
        journal.transition_operation(operation_id, OperationState::Running, None)?;
    }
    journal.transition_operation(operation_id, OperationState::Completed, None)?;
    Ok(())
}

/// The pack a record is about, or the refusal.
///
/// Both mutating entry points are reached by operation id, so each must
/// confirm what kind of record it was handed: a download batch's id must
/// not be able to make the recovery touch an instance.
fn require_pack(record: &OperationRecord) -> Result<&PackApplyMeta, RecoveryError> {
    match (record.kind, record.pack.as_ref()) {
        (OperationKind::MrpackInstall, Some(meta)) => Ok(meta),
        _ => Err(RecoveryError::NotAPackApply(record.operation_id.clone())),
    }
}

/// The destination an operation names.
fn op_path(op: &InstallOp) -> &str {
    match op {
        InstallOp::CopyOverride { safe_path, .. } => safe_path,
        InstallOp::Download { safe_path, .. } => safe_path,
    }
}

fn io_err(op: &'static str, path: &Path, source: std::io::Error) -> RecoveryError {
    RecoveryError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}
