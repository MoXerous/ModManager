//! Applying an update: plan, re-check, then write (F-121..F-124).
//!
//! Three things are deliberate here.
//!
//! **The plan is the confirmation.** [`plan_install`] is a read: it records
//! the lock's digest, the target it was built for, and the state of the files
//! it will replace. [`check_context`] re-reads all three immediately before
//! the first write and refuses with a typed [`UpdateApplyError::ContextChanged`]
//! when any of them moved (F-121). This is the same shape the `.mrpack` apply
//! (`PackChanged`) and the pack export (`InstanceChanged`) already use: the
//! plan carries what it was built from, and the write re-derives it.
//!
//! **The contract is data.** [`SafetyContractItem`] is the five promises the
//! old app showed before an install, each with the guard that keeps it. A
//! promise whose guard is not implemented is a lie, so the copy the UI renders
//! and the code that writes are one table (F-122), and every guard has a test
//! that proves it by reading bytes back off the disk.
//!
//! **Nothing is decided silently.** A [`BackupChoice`] of Yes runs the
//! full-profile backup of F-113 first; No is refused unless it was confirmed a
//! second time; Cancel stops before the journal record exists (F-123). A
//! download-only run writes one folder inside the instance and never reads
//! `mods/` for writing (F-124).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use sha1::Sha1;
use sha2::{Digest as _, Sha512};

use super::{UpdateCandidate, UpdatePreview, UpdateStatus};
use crate::downloader::{DownloadProgress, DownloadRequest, Downloader};
use crate::instances::backup::{self, BackupError, BackupRun, BackupSubject};
use crate::instances::lock::LOCK_FILE_NAME;
use crate::instances::lock::{read_lock, write_lock, LockEntry, LockError, LockFile};
use crate::instances::stamp::{backup_stamp, unique_path, utc_timestamp};
use crate::journal::{
    ItemState, JournalError, JournalOp, OperationItem, OperationRecord, OperationState,
};

/// The folder a download-only run writes into, inside the instance (F-124).
pub const DOWNLOAD_ONLY_DIRNAME: &str = "modrinth-update-downloads";
/// Where a replaced jar is moved before the new one lands. The name is the old
/// app's, so an instance that already has such a folder keeps one convention;
/// it is also in the backup's exclusion list, so a profile backup never copies
/// it.
pub const REPLACED_DIRNAME: &str = ".modrinth_bulk_downloader_backup";
/// Staging root, next to the instance content and excluded from backups.
pub const STAGING_DIRNAME: &str = ".mm-staging";

/// A promise the confirmation text makes about the write, and the guard that
/// keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyGuard {
    /// The replaced file is moved into a dated folder, never left nowhere.
    TimestampedBackup,
    /// The move is a rename: no pre-existing byte is ever removed.
    NoPermanentDelete,
    /// Only the files the plan names are touched.
    UnknownFilesUntouched,
    /// The recorded hash is re-read: a file that changed is skipped.
    ChangedSincePreviewSkipped,
    /// A file that is not the one being replaced is never overwritten.
    NoOverwriteUnlinkedFile,
}

impl SafetyGuard {
    /// The function that keeps this promise. `updates/apply_tests.rs` proves
    /// each one by its bytes on disk; this string is what the reviewer greps
    /// for, so it must stay a real path in this file.
    pub fn implemented_by(self) -> &'static str {
        match self {
            Self::TimestampedBackup => "updates::apply::move_to_backup",
            Self::NoPermanentDelete => "updates::apply::move_to_backup",
            Self::UnknownFilesUntouched => "updates::apply::apply_install",
            Self::ChangedSincePreviewSkipped => "updates::apply::install_one",
            Self::NoOverwriteUnlinkedFile => "updates::apply::install_one",
        }
    }
}

/// One line of the confirmation copy (F-122). `key` is the i18n key both
/// locales carry; `text` is the English copy the CLI prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyContractItem {
    /// Stable id, for tests and for the DTO.
    pub id: &'static str,
    pub key: &'static str,
    pub text: &'static str,
    pub guard: SafetyGuard,
}

/// The five promises, in the order the old app showed them.
pub const SAFETY_CONTRACT: [SafetyContractItem; 5] = [
    SafetyContractItem {
        id: "timestamped-backup",
        key: "updates.contract.backup",
        text: "Old local files are moved to a timestamped backup folder.",
        guard: SafetyGuard::TimestampedBackup,
    },
    SafetyContractItem {
        id: "no-delete",
        key: "updates.contract.noDelete",
        text: "No files are permanently deleted.",
        guard: SafetyGuard::NoPermanentDelete,
    },
    SafetyContractItem {
        id: "unknown-untouched",
        key: "updates.contract.unknownUntouched",
        text: "Unknown files are not touched.",
        guard: SafetyGuard::UnknownFilesUntouched,
    },
    SafetyContractItem {
        id: "changed-skipped",
        key: "updates.contract.changedSkipped",
        text: "If a local file changed since preview, that update is skipped.",
        guard: SafetyGuard::ChangedSincePreviewSkipped,
    },
    SafetyContractItem {
        id: "no-overwrite",
        key: "updates.contract.noOverwrite",
        text: "Existing unrelated target files are not overwritten.",
        guard: SafetyGuard::NoOverwriteUnlinkedFile,
    },
];

impl SafetyContractItem {
    pub fn all() -> &'static [SafetyContractItem] {
        &SAFETY_CONTRACT
    }
}

/// What a plan was built for. The folder is compared as a path: a plan is
/// about one instance, not about "whatever folder is open now".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTarget {
    pub instance_root: PathBuf,
    pub game_version: String,
    pub loader: String,
}

impl PlanTarget {
    pub fn new(instance_root: impl Into<PathBuf>, game_version: &str, loader: &str) -> Self {
        Self {
            instance_root: instance_root.into(),
            game_version: game_version.trim().to_string(),
            loader: loader.trim().to_string(),
        }
    }

    fn mods(&self) -> PathBuf {
        self.instance_root.join("mods")
    }

    fn is_concrete(&self) -> bool {
        !self.game_version.is_empty()
            && !self.loader.is_empty()
            && !self.loader.eq_ignore_ascii_case("any")
            && !self
                .loader
                .eq_ignore_ascii_case(crate::db::instances::UNKNOWN)
    }
}

/// One update this plan will install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlanEntry {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub old_filename: String,
    /// The hash the preview read from the lock. An empty value means the lock
    /// recorded none, and then only the file's presence can be checked.
    pub old_sha1: String,
    pub version_id: String,
    pub version_number: String,
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub sha1: String,
    pub sha512: String,
    pub game_versions: Vec<String>,
}

/// One replaced file, as the plan found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFact {
    pub filename: String,
    pub exists: bool,
    pub size: u64,
}

/// Everything the plan was built from, plus the token a caller can carry
/// across a process boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanContext {
    pub target: PlanTarget,
    pub lock_sha512: String,
    pub folder: Vec<FolderFact>,
    pub token: String,
}

/// Which half of the context moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextField {
    Lock,
    Target,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInstallPlan {
    pub entries: Vec<UpdatePlanEntry>,
    pub context: PlanContext,
}

impl UpdateInstallPlan {
    pub fn token(&self) -> &str {
        &self.context.token
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateApplyError {
    /// F-121: the plan describes a state that no longer exists.
    #[error("the update plan's context changed ({field:?}); nothing was written")]
    ContextChanged { field: ContextField },
    #[error("the update plan has nothing to install")]
    EmptyPlan,
    #[error("installing an update needs a concrete Minecraft version and loader")]
    NeedsTarget,
    /// F-123: No, without the second explicit confirmation.
    #[error("continuing without a profile backup was not confirmed a second time")]
    BackupNotConfirmed,
    /// F-123: Cancel.
    #[error("the update was cancelled before anything was written")]
    Cancelled,
    #[error(transparent)]
    Backup(#[from] BackupError),
    #[error(transparent)]
    Download(#[from] crate::downloader::client::DownloadError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The three-way answer to "create a full profile backup first?" (F-123).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupDecision {
    Yes,
    No,
    Cancel,
}

/// The decision plus the extra confirmation the No branch needs. A bare
/// `No` is not consent: proceeding without a copy is the user's explicit
/// second answer, not a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupChoice {
    pub decision: BackupDecision,
    pub confirmed_without_backup: bool,
}

impl BackupChoice {
    pub fn yes() -> Self {
        Self {
            decision: BackupDecision::Yes,
            confirmed_without_backup: false,
        }
    }

    /// No, chosen once. Refused by [`resolve_backup_choice`].
    pub fn no() -> Self {
        Self {
            decision: BackupDecision::No,
            confirmed_without_backup: false,
        }
    }

    /// No, then confirmed a second time.
    pub fn no_confirmed() -> Self {
        Self {
            decision: BackupDecision::No,
            confirmed_without_backup: true,
        }
    }

    pub fn cancel() -> Self {
        Self {
            decision: BackupDecision::Cancel,
            confirmed_without_backup: false,
        }
    }
}

/// Why a planned file was not replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The local file is gone.
    MissingOldFile,
    /// The local file no longer carries the hash the preview recorded.
    ChangedSincePreview,
    /// Something that is not the file being replaced already sits at the new
    /// name.
    TargetOccupied,
    /// The target appeared during the run, after the backup moved the old
    /// file away. The old file was restored.
    TargetAppeared,
}

impl SkipReason {
    /// The i18n code both locales carry (`updates.skip.<code>`).
    pub fn code(self) -> &'static str {
        match self {
            Self::MissingOldFile => "missingOldFile",
            Self::ChangedSincePreview => "changedSincePreview",
            Self::TargetOccupied => "targetOccupied",
            Self::TargetAppeared => "targetAppeared",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRow {
    pub project_id: String,
    pub filename: String,
    pub reason: SkipReason,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRow {
    pub project_id: String,
    pub filename: String,
    pub version_id: String,
    pub version_number: String,
    /// The replaced file inside the dated folder, when there was one.
    pub replaced: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInstallReport {
    pub operation_id: String,
    pub installed: Vec<InstalledRow>,
    pub skipped: Vec<SkippedRow>,
    /// The dated folder the replaced jars were moved into.
    pub replaced_backup_dir: PathBuf,
    /// The full-profile backup, when the user answered Yes.
    pub profile_backup: Option<BackupRun>,
}

/// The options of one install run. Borrowed, because the plan and the
/// downloader outlive the call.
pub struct InstallOptions<'a> {
    /// The target as the caller sees it now, not as the plan remembers it.
    pub target: &'a PlanTarget,
    pub subject: BackupSubject,
    pub backup: BackupChoice,
    pub downloader: &'a Downloader,
    /// Tests only, exactly like `DownloadRequest::allow_insecure_loopback`.
    pub allow_insecure_loopback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedFile {
    pub project_id: String,
    pub filename: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOnlyReport {
    pub operation_id: String,
    pub directory: PathBuf,
    pub prepared: Vec<PreparedFile>,
}

// --- the plan (read-only, F-121) -------------------------------------------

/// Builds the install plan from the preview the user is looking at.
///
/// `selected` names the projects to install; empty means every candidate the
/// preview marked `Available`. A candidate without a URL or a filename cannot
/// be downloaded, and is left out rather than planned into a failure.
///
/// Reads only: no folder is created, no lock is written, nothing is downloaded.
pub fn plan_install(
    instance_root: &Path,
    game_version: &str,
    loader: &str,
    preview: &UpdatePreview,
    selected: &[String],
) -> Result<UpdateInstallPlan, UpdateApplyError> {
    let target = PlanTarget::new(instance_root, game_version, loader);
    if !target.is_concrete() {
        return Err(UpdateApplyError::NeedsTarget);
    }
    let lock = match read_lock(&target.instance_root.join(LOCK_FILE_NAME)) {
        Ok(lock) => lock,
        Err(LockError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => LockFile::default(),
        Err(e) => return Err(e.into()),
    };

    let wants = |project_id: &str| {
        selected.is_empty() || selected.iter().any(|wanted| wanted == project_id)
    };
    let mut entries = Vec::new();
    for candidate in &preview.candidates {
        if candidate.status != UpdateStatus::Available || !wants(&candidate.project_id) {
            continue;
        }
        let Some(plan_entry) = plan_entry(candidate, &lock) else {
            continue;
        };
        entries.push(plan_entry);
    }
    if entries.is_empty() {
        return Err(UpdateApplyError::EmptyPlan);
    }
    let context = snapshot(&target, &entries);
    Ok(UpdateInstallPlan { entries, context })
}

fn plan_entry(candidate: &UpdateCandidate, lock: &LockFile) -> Option<UpdatePlanEntry> {
    let url = candidate.latest_url.clone()?;
    let filename = candidate.latest_filename.clone()?;
    if url.is_empty() || filename.is_empty() {
        return None;
    }
    let recorded = lock.entries.iter().find(|e| {
        e.project_id == candidate.project_id
            && (e.provider.is_empty() || e.provider.eq_ignore_ascii_case(&candidate.provider))
    });
    let old_filename = if candidate.filename.is_empty() {
        recorded.map(|e| e.filename.clone())?
    } else {
        candidate.filename.clone()
    };
    Some(UpdatePlanEntry {
        provider: candidate.provider.clone(),
        project_id: candidate.project_id.clone(),
        slug: recorded.map(|e| e.slug.clone()).unwrap_or_default(),
        old_filename,
        old_sha1: candidate.current_sha1.clone(),
        version_id: candidate.latest_version_id.clone().unwrap_or_default(),
        version_number: candidate.latest_version_number.clone().unwrap_or_default(),
        filename,
        url,
        size: candidate.latest_size.unwrap_or(0),
        sha1: candidate.latest_sha1.clone().unwrap_or_default(),
        sha512: candidate.latest_sha512.clone().unwrap_or_default(),
        game_versions: candidate.latest_game_versions.clone(),
    })
}

/// Re-derives the context and refuses when any part of it moved (F-121).
///
/// The order is the order of the failure: the wrong folder makes the rest of
/// the comparison meaningless, so `Target` is answered first.
pub fn check_context(
    plan: &UpdateInstallPlan,
    current: &PlanTarget,
) -> Result<(), UpdateApplyError> {
    if current.instance_root != plan.context.target.instance_root
        || current.game_version != plan.context.target.game_version
        || current.loader != plan.context.target.loader
    {
        return Err(UpdateApplyError::ContextChanged {
            field: ContextField::Target,
        });
    }
    let now = snapshot(&plan.context.target, &plan.entries);
    if now.lock_sha512 != plan.context.lock_sha512 {
        return Err(UpdateApplyError::ContextChanged {
            field: ContextField::Lock,
        });
    }
    if now.folder != plan.context.folder {
        return Err(UpdateApplyError::ContextChanged {
            field: ContextField::Folder,
        });
    }
    Ok(())
}

fn snapshot(target: &PlanTarget, entries: &[UpdatePlanEntry]) -> PlanContext {
    let mods = target.mods();
    let mut facts: BTreeMap<String, FolderFact> = BTreeMap::new();
    for entry in entries {
        facts.entry(entry.old_filename.clone()).or_insert_with(|| {
            let path = mods.join(&entry.old_filename);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_file() => FolderFact {
                    filename: entry.old_filename.clone(),
                    exists: true,
                    size: meta.len(),
                },
                _ => FolderFact {
                    filename: entry.old_filename.clone(),
                    exists: false,
                    size: 0,
                },
            }
        });
    }
    let folder: Vec<FolderFact> = facts.into_values().collect();
    let lock_sha512 = lock_digest(&target.instance_root.join(LOCK_FILE_NAME));
    let token = context_token(target, &lock_sha512, &folder);
    PlanContext {
        target: target.clone(),
        lock_sha512,
        folder,
        token,
    }
}

/// SHA-512 of the lock's bytes, empty when there is no readable lock. A
/// missing lock is a normal state (`EmptyLock` in the preview), and an
/// unreadable one is answered by [`read_lock`] when the plan is built.
fn lock_digest(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => sha512_hex(&bytes),
        Err(_) => String::new(),
    }
}

fn context_token(target: &PlanTarget, lock_sha512: &str, folder: &[FolderFact]) -> String {
    let mut text = String::from("mm.update.plan.v1\n");
    text.push_str(&format!(
        "root={}\ngame={}\nloader={}\nlock={}\n",
        target.instance_root.to_string_lossy(),
        target.game_version,
        target.loader,
        lock_sha512
    ));
    for fact in folder {
        text.push_str(&format!(
            "file={}={},{}\n",
            fact.filename, fact.exists, fact.size
        ));
    }
    sha512_hex(text.as_bytes())
}

// --- the backup decision (F-123) ------------------------------------------

/// Turns the three-way answer into either a real backup or an explicit
/// refusal. Nothing is written before this returns.
pub fn resolve_backup_choice(
    root: &Path,
    subject: &BackupSubject,
    choice: BackupChoice,
) -> Result<Option<BackupRun>, UpdateApplyError> {
    match choice.decision {
        BackupDecision::Cancel => Err(UpdateApplyError::Cancelled),
        BackupDecision::No if !choice.confirmed_without_backup => {
            Err(UpdateApplyError::BackupNotConfirmed)
        }
        BackupDecision::No => Ok(None),
        BackupDecision::Yes => {
            let plan = backup::build_backup_plan(root, subject, false, SystemTime::now())?;
            let cancel = AtomicBool::new(false);
            let run = backup::execute_backup(&plan, &cancel, &mut |_| {})?;
            Ok(Some(run))
        }
    }
}

// --- the install (F-121..F-123) -------------------------------------------

/// Applies the plan: re-check, backup decision, journal, then one file at a
/// time. Every skip is a row in the report and an item in the record; nothing
/// silently disappears.
pub async fn apply_install(
    plan: &UpdateInstallPlan,
    options: &InstallOptions<'_>,
    write: &mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
) -> Result<UpdateInstallReport, UpdateApplyError> {
    check_context(plan, options.target)?;
    let profile_backup = resolve_backup_choice(
        &options.target.instance_root,
        &options.subject,
        options.backup,
    )?;

    let mods = options.target.mods();
    fs::create_dir_all(&mods).map_err(|source| io("create mods dir", &mods, source))?;
    let stamp = backup_stamp(&utc_timestamp(SystemTime::now()));
    let replaced_backup_dir = options
        .target
        .instance_root
        .join(REPLACED_DIRNAME)
        .join(&stamp);
    let staging = options
        .target
        .instance_root
        .join(STAGING_DIRNAME)
        .join("update")
        .join(&stamp);

    let items = plan_items(plan, &staging, &mods);
    let record = write(JournalOp::Start {
        target: mods.to_string_lossy().into_owned(),
        items,
    })?;
    let operation_id = record.operation_id.clone();
    let fail = |write: &mut dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError>,
                error: UpdateApplyError|
     -> UpdateApplyError {
        let _ = write(JournalOp::Operation(OperationState::Failed));
        error
    };

    write(JournalOp::Operation(OperationState::Running))?;

    let mut installed: Vec<InstalledRow> = Vec::new();
    let mut skipped: Vec<SkippedRow> = Vec::new();
    for entry in &plan.entries {
        match install_one(entry, options, &staging, &replaced_backup_dir, write).await {
            Ok(Outcome::Installed(row)) => installed.push(row),
            Ok(Outcome::Skipped(row)) => skipped.push(row),
            Err(error) => return Err(fail(write, error)),
        }
    }

    if let Err(error) = update_lock(&options.target.instance_root, &installed, plan) {
        return Err(fail(write, error));
    }
    remove_empty_staging(&staging);
    write(JournalOp::Operation(OperationState::Completed))?;

    Ok(UpdateInstallReport {
        operation_id,
        installed,
        skipped,
        replaced_backup_dir,
        profile_backup,
    })
}

/// Applies one update and reports what happened to it.
///
/// The item states are the journal's own edges — queued, downloading,
/// verifying, committing — so a crash mid-file is legible in the record and
/// the recovery screen can tell a half-downloaded jar from a placed one.
async fn install_one(
    entry: &UpdatePlanEntry,
    options: &InstallOptions<'_>,
    staging: &Path,
    backup_dir: &Path,
    write: &mut JournalWrite<'_>,
) -> Result<Outcome, UpdateApplyError> {
    let mods = options.target.mods();
    let old_path = mods.join(&entry.old_filename);
    // Contract 4: the file the preview described is the file being replaced.
    let current_sha1 = match sha1_file(&old_path) {
        Ok(hash) => hash,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            set_item(write, &entry.project_id, ItemState::Skipped)?;
            return Ok(Outcome::Skipped(SkippedRow {
                project_id: entry.project_id.clone(),
                filename: entry.filename.clone(),
                reason: SkipReason::MissingOldFile,
                message: format!("{} is no longer in mods/", entry.old_filename),
            }));
        }
        Err(source) => return Err(io("hash old jar", &old_path, source)),
    };
    if !entry.old_sha1.is_empty() && !current_sha1.eq_ignore_ascii_case(&entry.old_sha1) {
        set_item(write, &entry.project_id, ItemState::Skipped)?;
        return Ok(Outcome::Skipped(SkippedRow {
            project_id: entry.project_id.clone(),
            filename: entry.filename.clone(),
            reason: SkipReason::ChangedSincePreview,
            message: format!(
                "{} changed since the preview; it was not replaced",
                entry.old_filename
            ),
        }));
    }

    let destination = mods.join(&entry.filename);
    let occupied = destination.is_file() && destination != old_path;
    if occupied {
        // Contract 5: an existing file is only acceptable when it already IS
        // the file being installed.
        let same = !entry.sha1.is_empty()
            && sha1_file(&destination)
                .map(|hash| hash.eq_ignore_ascii_case(&entry.sha1))
                .unwrap_or(false);
        if !same {
            set_item(write, &entry.project_id, ItemState::Skipped)?;
            return Ok(Outcome::Skipped(SkippedRow {
                project_id: entry.project_id.clone(),
                filename: entry.filename.clone(),
                reason: SkipReason::TargetOccupied,
                message: format!(
                    "{} already exists and is not this update; nothing was replaced",
                    entry.filename
                ),
            }));
        }
        let replaced = match move_to_backup(&old_path, backup_dir) {
            Ok(path) => path,
            Err(error) => {
                fail_item(write, &entry.project_id);
                return Err(error);
            }
        };
        // Its bytes are already the ones being installed, so nothing is
        // downloaded; the record still walks the canonical edges to reach a
        // terminal `Completed`.
        advance_to_commit(write, &entry.project_id)?;
        set_item(write, &entry.project_id, ItemState::Completed)?;
        return Ok(Outcome::Installed(InstalledRow {
            project_id: entry.project_id.clone(),
            filename: destination
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| entry.filename.clone()),
            version_id: entry.version_id.clone(),
            version_number: entry.version_number.clone(),
            replaced: Some(replaced.to_string_lossy().into_owned()),
        }));
    }

    fs::create_dir_all(staging).map_err(|source| io("create staging", staging, source))?;
    let staged = staging.join(&entry.filename);
    let request = DownloadRequest {
        url: entry.url.clone(),
        dest: staged,
        expected_size: if entry.size > 0 {
            Some(entry.size)
        } else {
            None
        },
        sha1_hex: non_empty(&entry.sha1),
        sha512_hex: non_empty(&entry.sha512),
        allow_insecure_loopback: options.allow_insecure_loopback,
    };
    set_item(write, &entry.project_id, ItemState::Downloading)?;
    let landed = match options
        .downloader
        .download(
            &request,
            &mut |_p: DownloadProgress| ControlFlow::Continue(()),
            &|| false,
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            // A record whose item is still "downloading" under a failed
            // operation reads as work in progress on the recovery screen.
            fail_item(write, &entry.project_id);
            return Err(error.into());
        }
    };
    if landed.status == crate::downloader::DownloadStatus::Cancelled {
        let _ = write(JournalOp::Item {
            item_id: entry.project_id.clone(),
            state: ItemState::Cancelled,
        });
        return Err(UpdateApplyError::Cancelled);
    }
    set_item(write, &entry.project_id, ItemState::Verifying)?;

    // Contract 1 and 2: the old file is moved, never removed.
    let replaced = match move_to_backup(&old_path, backup_dir) {
        Ok(path) => path,
        Err(error) => {
            fail_item(write, &entry.project_id);
            return Err(error);
        }
    };
    if destination.exists() {
        // Something appeared between the check and the commit. Put the old
        // file back rather than leaving a hole; never overwrite the newcomer.
        let _ = fs::rename(&replaced, &old_path);
        // The staged jar is not the plan's outcome any more: do not leave it
        // behind in a folder the backup excludes.
        let _ = fs::remove_file(&landed.path);
        // The record says failed, the report says skipped: the item did not
        // install, and no state edge leads from `verifying` to `skipped`.
        set_item(write, &entry.project_id, ItemState::Failed)?;
        return Ok(Outcome::Skipped(SkippedRow {
            project_id: entry.project_id.clone(),
            filename: entry.filename.clone(),
            reason: SkipReason::TargetAppeared,
            message: format!(
                "{} appeared during the update; the old file was restored",
                entry.filename
            ),
        }));
    }
    set_item(write, &entry.project_id, ItemState::Committing)?;
    if let Err(source) = fs::rename(&landed.path, &destination) {
        fail_item(write, &entry.project_id);
        return Err(io("place new jar", &destination, source));
    }
    set_item(write, &entry.project_id, ItemState::Completed)?;
    Ok(Outcome::Installed(InstalledRow {
        project_id: entry.project_id.clone(),
        filename: destination
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.filename.clone()),
        version_id: entry.version_id.clone(),
        version_number: entry.version_number.clone(),
        replaced: Some(replaced.to_string_lossy().into_owned()),
    }))
}

enum Outcome {
    Installed(InstalledRow),
    Skipped(SkippedRow),
}

/// Moves a replaced file into the dated folder. A rename, on the same volume,
/// so the bytes are never copied and never dropped: this is the whole of
/// contract 1 and contract 2.
fn move_to_backup(path: &Path, backup_dir: &Path) -> Result<PathBuf, UpdateApplyError> {
    fs::create_dir_all(backup_dir).map_err(|source| io("create backup dir", backup_dir, source))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "old.jar".to_string());
    let destination = unique_path(backup_dir.join(name));
    fs::rename(path, &destination).map_err(|source| io("move to backup", path, source))?;
    Ok(destination)
}

fn update_lock(
    root: &Path,
    installed: &[InstalledRow],
    plan: &UpdateInstallPlan,
) -> Result<(), UpdateApplyError> {
    if installed.is_empty() {
        return Ok(());
    }
    let path = root.join(LOCK_FILE_NAME);
    let mut entries = match read_lock(&path) {
        Ok(lock) => lock.entries,
        Err(LockError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    for row in installed {
        let Some(entry) = plan
            .entries
            .iter()
            .find(|candidate| candidate.project_id == row.project_id)
        else {
            continue;
        };
        entries.retain(|existing| {
            !(existing.provider.eq_ignore_ascii_case(&entry.provider)
                && existing.project_id == entry.project_id)
        });
        entries.push(LockEntry {
            provider: entry.provider.clone(),
            project_id: entry.project_id.clone(),
            slug: entry.slug.clone(),
            version_id: entry.version_id.clone(),
            filename: row.filename.clone(),
            sha1: entry.sha1.clone(),
            sha512: entry.sha512.clone(),
            game_versions: entry.game_versions.clone(),
        });
    }
    write_lock(&path, &LockFile { entries })?;
    Ok(())
}

fn plan_items(plan: &UpdateInstallPlan, staging: &Path, mods: &Path) -> Vec<OperationItem> {
    plan.entries
        .iter()
        .map(|entry| OperationItem {
            item_id: entry.project_id.clone(),
            project_id: entry.project_id.clone(),
            title: if entry.slug.is_empty() {
                entry.project_id.clone()
            } else {
                entry.slug.clone()
            },
            filename: entry.filename.clone(),
            state: ItemState::Queued,
            partial_path: staging.join(&entry.filename).to_string_lossy().into_owned(),
            final_path: mods.join(&entry.filename).to_string_lossy().into_owned(),
            message: String::new(),
        })
        .collect()
}

// --- download only (F-124) -------------------------------------------------

/// Prepares the update files without installing anything: one folder inside
/// the instance, one journal record, and `mods/` untouched.
pub async fn download_only(
    plan: &UpdateInstallPlan,
    target: &PlanTarget,
    downloader: &Downloader,
    allow_insecure_loopback: bool,
    write: &mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
) -> Result<DownloadOnlyReport, UpdateApplyError> {
    check_context(plan, target)?;
    let directory = target.instance_root.join(DOWNLOAD_ONLY_DIRNAME);
    let items: Vec<OperationItem> = plan
        .entries
        .iter()
        .map(|entry| OperationItem {
            item_id: entry.project_id.clone(),
            project_id: entry.project_id.clone(),
            title: entry.project_id.clone(),
            filename: entry.filename.clone(),
            state: ItemState::Queued,
            partial_path: String::new(),
            final_path: directory
                .join(&entry.filename)
                .to_string_lossy()
                .into_owned(),
            message: String::new(),
        })
        .collect();
    let record = write(JournalOp::Start {
        target: directory.to_string_lossy().into_owned(),
        items,
    })?;
    let operation_id = record.operation_id.clone();
    write(JournalOp::Operation(OperationState::Running))?;
    fs::create_dir_all(&directory)
        .map_err(|source| io("create download folder", &directory, source))?;

    let mut prepared = Vec::new();
    for entry in &plan.entries {
        let destination = unique_path(directory.join(&entry.filename));
        let request = DownloadRequest {
            url: entry.url.clone(),
            dest: destination,
            expected_size: if entry.size > 0 {
                Some(entry.size)
            } else {
                None
            },
            sha1_hex: non_empty(&entry.sha1),
            sha512_hex: non_empty(&entry.sha512),
            allow_insecure_loopback,
        };
        set_item(write, &entry.project_id, ItemState::Downloading)?;
        let landed = match downloader
            .download(
                &request,
                &mut |_p: DownloadProgress| ControlFlow::Continue(()),
                &|| false,
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                let _ = write(JournalOp::Item {
                    item_id: entry.project_id.clone(),
                    state: ItemState::Failed,
                });
                let _ = write(JournalOp::Operation(OperationState::Failed));
                return Err(error.into());
            }
        };
        if landed.status == crate::downloader::DownloadStatus::Cancelled {
            let _ = write(JournalOp::Item {
                item_id: entry.project_id.clone(),
                state: ItemState::Cancelled,
            });
            let _ = write(JournalOp::Operation(OperationState::Cancelled));
            return Err(UpdateApplyError::Cancelled);
        }
        let bytes = fs::metadata(&landed.path)
            .map(|m| m.len())
            .unwrap_or(entry.size);
        advance_to_commit(write, &entry.project_id)?;
        set_item(write, &entry.project_id, ItemState::Completed)?;
        prepared.push(PreparedFile {
            project_id: entry.project_id.clone(),
            filename: landed
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| entry.filename.clone()),
            path: landed.path.to_string_lossy().into_owned(),
            bytes,
        });
    }
    write(JournalOp::Operation(OperationState::Completed))?;
    Ok(DownloadOnlyReport {
        operation_id,
        directory,
        prepared,
    })
}

// --- helpers ---------------------------------------------------------------

/// The journal callback the apply and download-only paths share. Borrowed for
/// the length of one run: a long download must not hold the journal's lock.
type JournalWrite<'a> = dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send + 'a;

fn set_item(
    write: &mut JournalWrite<'_>,
    item_id: &str,
    state: ItemState,
) -> Result<(), UpdateApplyError> {
    write(JournalOp::Item {
        item_id: item_id.to_string(),
        state,
    })?;
    Ok(())
}

/// Best-effort "this item stopped here". The record must not keep claiming a
/// step that will never continue, and a journal write failing here must not
/// replace the real error the caller is about to report.
fn fail_item(write: &mut JournalWrite<'_>, item_id: &str) {
    let _ = write(JournalOp::Item {
        item_id: item_id.to_string(),
        state: ItemState::Failed,
    });
}

/// The edges an item must walk before it can be `Completed`.
fn advance_to_commit(write: &mut JournalWrite<'_>, item_id: &str) -> Result<(), UpdateApplyError> {
    for state in [
        ItemState::Downloading,
        ItemState::Verifying,
        ItemState::Committing,
    ] {
        set_item(write, item_id, state)?;
    }
    Ok(())
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn io(op: &'static str, path: &Path, source: std::io::Error) -> UpdateApplyError {
    UpdateApplyError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

fn sha1_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn sha512_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha512::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Removes the staging folder when the run left it empty. Best effort: a
/// leftover file is safer than a delete that guessed wrong.
fn remove_empty_staging(staging: &Path) {
    if let Ok(entries) = fs::read_dir(staging) {
        if entries.flatten().next().is_none() {
            let _ = fs::remove_dir(staging);
        }
    }
}
