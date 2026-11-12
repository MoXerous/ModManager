//! Safe content removal (F-111/F-112): move to a dated backup folder inside
//! `mods/`, never delete permanently.
//!
//! The plan is read-only: [`build_remove_preview`] only inspects the folder.
//! [`execute_remove`] re-checks every guard before moving a single file, so a
//! file that changed between preview and confirm is refused, not trusted.
//! External or unknown files demand the typed word [`REMOVE_CONFIRMATION_WORD`]
//! because removing one is a decision the app cannot infer from the lock file.

use crate::safety::atomic::write_atomic_unique;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Dated backup root, relative to the instance's `mods` folder.
pub const REMOVE_BACKUP_DIRNAME: &str = ".modrinth_bulk_downloader_removed_content";
/// The record written inside the dated folder after a run.
pub const REMOVE_MANIFEST_NAME: &str = "remove_manifest.json";
/// Manifest schema tag, kept byte-identical to the legacy app so an old
/// backup folder is still readable by either implementation.
pub const REMOVE_SCHEMA: &str = "modrinth_bulk_downloader.profile_content_remove.v1";
/// The exact word a user types before external/unknown files are moved.
pub const REMOVE_CONFIRMATION_WORD: &str = "REMOVE";

/// Why a candidate is allowed or refused.
///
/// The code is the stable value the UI translates; the message is the legacy
/// English wording kept for the manifest and for logs. Both travel together
/// because a translated string must never be the only record of a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveReason {
    Allowed,
    RootMissing,
    RootUnresolvable,
    SourceMissing,
    SourceUnresolvable,
    Symlink,
    Outside,
    BackupFolder,
    Unsupported,
}

impl RemoveReason {
    pub fn code(self) -> &'static str {
        match self {
            RemoveReason::Allowed => "allowed",
            RemoveReason::RootMissing => "root-missing",
            RemoveReason::RootUnresolvable => "root-unresolvable",
            RemoveReason::SourceMissing => "source-missing",
            RemoveReason::SourceUnresolvable => "source-unresolvable",
            RemoveReason::Symlink => "symlink",
            RemoveReason::Outside => "outside",
            RemoveReason::BackupFolder => "backup-folder",
            RemoveReason::Unsupported => "unsupported",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            RemoveReason::Allowed => {
                "Will remove the file from the active mods folder after preview and backup handling."
            }
            RemoveReason::RootMissing => "Mods folder does not exist.",
            RemoveReason::RootUnresolvable => "Mods folder cannot be resolved.",
            RemoveReason::SourceMissing => "Source file does not exist.",
            RemoveReason::SourceUnresolvable => "Source file cannot be resolved.",
            RemoveReason::Symlink => "Symlink source skipped for safety.",
            RemoveReason::Outside => {
                "Source file is outside the active profile mods folder."
            }
            RemoveReason::BackupFolder => "Remove backup folders cannot be selected for removal.",
            RemoveReason::Unsupported => {
                "Only local .jar or .jar.disabled files can be removed in this phase."
            }
        }
    }
}

/// One file the UI selected. `external` is ownership, not a path fact: the
/// bridge sets it when the lock file has no record for the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveCandidate {
    pub path: PathBuf,
    pub external: bool,
}

/// A guarded verdict for one candidate. Nothing has moved yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveEntry {
    pub source: PathBuf,
    /// Where the file would go; inside [`RemovePlan::backup_dir`].
    pub backup_path: PathBuf,
    pub allowed: bool,
    /// Allowed, but ownership is external/unknown: the typed word is required.
    pub requires_confirmation: bool,
    pub reason_code: &'static str,
    pub reason: String,
}

/// The whole plan: one dated folder shared by every entry in the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovePlan {
    pub mods_dir: PathBuf,
    pub backup_dir: PathBuf,
    /// UTC ISO-8601 at second precision, e.g. `2023-11-14T22:13:20Z`.
    pub created_at: String,
    pub entries: Vec<RemoveEntry>,
}

impl RemovePlan {
    /// True when at least one allowed entry is external/unknown.
    pub fn requires_confirmation(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.allowed && e.requires_confirmation)
    }

    /// Allowed entries; the only ones execution touches.
    pub fn allowed_entries(&self) -> impl Iterator<Item = &RemoveEntry> {
        self.entries.iter().filter(|e| e.allowed)
    }
}

/// Terminal state of one allowed entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveState {
    MovedToBackup,
    Failed,
}

impl RemoveState {
    pub fn as_str(self) -> &'static str {
        match self {
            RemoveState::MovedToBackup => "moved-to-backup",
            RemoveState::Failed => "failed",
        }
    }
}

/// What happened to one allowed entry. Refused entries are not repeated here:
/// the preview is their record, and the manifest only claims what it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveResult {
    pub source: PathBuf,
    pub backup_path: PathBuf,
    pub state: RemoveState,
    pub reason_code: String,
    pub message: String,
}

/// Execution outcome, including the record it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveRun {
    pub results: Vec<RemoveResult>,
    pub manifest_path: PathBuf,
    /// Empty when the manifest was written; otherwise the exact reason it was
    /// not. The moves still happened, so this is a warning, not an error.
    pub manifest_error: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RemoveError {
    #[error("typed confirmation is required before removing external or unknown files")]
    ConfirmationRequired,
}

/// Build a guarded plan. Reads only: the backup folder is created at execution.
///
/// `now` is an argument rather than a hidden call so the folder name and the
/// manifest timestamp are testable and the two calls of one request agree.
pub fn build_remove_preview(
    mods_dir: &Path,
    candidates: &[RemoveCandidate],
    now: SystemTime,
) -> RemovePlan {
    let created_at = utc_timestamp(now);
    let root = std::fs::canonicalize(mods_dir).ok();
    let base = root.as_deref().unwrap_or(mods_dir);
    let backup_dir = unique_path(
        base.join(REMOVE_BACKUP_DIRNAME)
            .join(backup_stamp(&created_at)),
    );
    let entries = candidates
        .iter()
        .map(|c| preview_one(root.as_deref(), mods_dir, &backup_dir, c))
        .collect();
    RemovePlan {
        mods_dir: mods_dir.to_path_buf(),
        backup_dir,
        created_at,
        entries,
    }
}

fn preview_one(
    root: Option<&Path>,
    mods_dir: &Path,
    backup_dir: &Path,
    candidate: &RemoveCandidate,
) -> RemoveEntry {
    let deny = |reason: RemoveReason| RemoveEntry {
        source: candidate.path.clone(),
        backup_path: candidate.path.clone(),
        allowed: false,
        requires_confirmation: false,
        reason_code: reason.code(),
        reason: reason.message().to_string(),
    };
    let root = match root {
        Some(r) if r.is_dir() => r,
        _ if !mods_dir.is_dir() => return deny(RemoveReason::RootMissing),
        _ => return deny(RemoveReason::RootUnresolvable),
    };
    if !candidate.path.is_file() {
        return deny(RemoveReason::SourceMissing);
    }
    // `is_file` follows links, so a symlink to a real file reaches this check.
    if is_link(&candidate.path) {
        return deny(RemoveReason::Symlink);
    }
    let source = match std::fs::canonicalize(&candidate.path) {
        Ok(p) => p,
        Err(_) => return deny(RemoveReason::SourceUnresolvable),
    };
    if !source.starts_with(root) {
        return deny(RemoveReason::Outside);
    }
    if in_backup_folder(&source) {
        return deny(RemoveReason::BackupFolder);
    }
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if !supported(name) {
        return deny(RemoveReason::Unsupported);
    }
    let relative = match source.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return deny(RemoveReason::Outside),
    };
    RemoveEntry {
        backup_path: unique_path(backup_dir.join("files").join(relative)),
        source,
        allowed: true,
        requires_confirmation: candidate.external,
        reason_code: RemoveReason::Allowed.code(),
        reason: RemoveReason::Allowed.message().to_string(),
    }
}

/// Re-check the plan, then move every allowed entry into its backup path.
///
/// When any allowed entry is external/unknown, a missing or wrong typed word
/// refuses the whole run *before* anything moves. Execution never deletes: the
/// only write to a source is a rename into the dated folder, and a destination
/// that appeared since the preview is refused rather than overwritten.
pub fn execute_remove(plan: &RemovePlan, confirmation: &str) -> Result<RemoveRun, RemoveError> {
    if plan.requires_confirmation() && confirmation.trim() != REMOVE_CONFIRMATION_WORD {
        return Err(RemoveError::ConfirmationRequired);
    }
    let root = std::fs::canonicalize(&plan.mods_dir).ok();
    let results: Vec<RemoveResult> = plan
        .allowed_entries()
        .map(|entry| execute_one(root.as_deref(), &plan.mods_dir, &plan.backup_dir, entry))
        .collect();
    let manifest_path = plan.backup_dir.join(REMOVE_MANIFEST_NAME);
    // A run with no allowed entries writes no record: a manifest that says
    // "nothing happened" would claim a backup folder that has no reason to
    // exist. Refusals live in the preview.
    let manifest_error = if results.is_empty() {
        String::new()
    } else {
        write_manifest(plan, &results).err().unwrap_or_default()
    };
    Ok(RemoveRun {
        results,
        manifest_path,
        manifest_error,
    })
}

fn execute_one(
    root: Option<&Path>,
    mods_dir: &Path,
    backup_dir: &Path,
    entry: &RemoveEntry,
) -> RemoveResult {
    let fail = |code: &str, message: &str| RemoveResult {
        source: entry.source.clone(),
        backup_path: entry.backup_path.clone(),
        state: RemoveState::Failed,
        reason_code: code.to_string(),
        message: message.to_string(),
    };
    let root = match root {
        Some(r) if r.is_dir() => r,
        _ if !mods_dir.is_dir() => {
            return fail(
                RemoveReason::RootMissing.code(),
                "Mods folder no longer exists.",
            )
        }
        _ => {
            return fail(
                RemoveReason::RootUnresolvable.code(),
                "Mods folder cannot be resolved.",
            )
        }
    };
    if !entry.source.is_file() {
        return fail(
            RemoveReason::SourceMissing.code(),
            "Source file no longer exists.",
        );
    }
    if is_link(&entry.source) {
        return fail(
            RemoveReason::Symlink.code(),
            "Source file is a symlink and was skipped for safety.",
        );
    }
    let source = match std::fs::canonicalize(&entry.source) {
        Ok(p) => p,
        Err(_) => {
            return fail(
                RemoveReason::SourceUnresolvable.code(),
                "Source file cannot be resolved.",
            )
        }
    };
    if !source.starts_with(root) {
        return fail(
            RemoveReason::Outside.code(),
            "Source file is outside the active profile mods folder.",
        );
    }
    if in_backup_folder(&source) {
        return fail(
            RemoveReason::BackupFolder.code(),
            "Remove backup folders cannot be selected for removal.",
        );
    }
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if !supported(name) {
        return fail(
            RemoveReason::Unsupported.code(),
            "Only local .jar or .jar.disabled files can be removed in this phase.",
        );
    }
    // The plan is built in Rust, but a hand-made plan is not trusted either:
    // a backup path outside the dated folder would be an unchecked move.
    if !entry.backup_path.starts_with(backup_dir) {
        return fail(
            "backup-outside",
            "Backup destination is outside the remove backup folder.",
        );
    }
    if entry.backup_path.exists() {
        return fail(
            "backup-exists",
            "Local backup destination already exists; nothing was overwritten.",
        );
    }
    if let Some(parent) = entry.backup_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return fail(
                "backup-unwritable",
                &format!("Could not create the backup folder: {err}"),
            );
        }
    }
    match std::fs::rename(&entry.source, &entry.backup_path) {
        Ok(()) => RemoveResult {
            source: entry.source.clone(),
            backup_path: entry.backup_path.clone(),
            state: RemoveState::MovedToBackup,
            reason_code: RemoveState::MovedToBackup.as_str().to_string(),
            message: "Moved file to the local remove backup instead of deleting it permanently."
                .to_string(),
        },
        Err(err) => fail("move-failed", &format!("Could not move the file: {err}")),
    }
}

#[derive(Serialize)]
struct ManifestDoc<'a> {
    schema: &'a str,
    created_at: &'a str,
    mods_dir: String,
    entries: Vec<ManifestEntry<'a>>,
}

#[derive(Serialize)]
struct ManifestEntry<'a> {
    source_path: String,
    backup_path: String,
    status: &'a str,
    message: &'a str,
}

fn write_manifest(plan: &RemovePlan, results: &[RemoveResult]) -> Result<(), String> {
    std::fs::create_dir_all(&plan.backup_dir)
        .map_err(|e| format!("Could not create the backup folder: {e}"))?;
    let doc = ManifestDoc {
        schema: REMOVE_SCHEMA,
        created_at: &plan.created_at,
        mods_dir: plan.mods_dir.to_string_lossy().into_owned(),
        entries: results
            .iter()
            .map(|r| ManifestEntry {
                source_path: r.source.to_string_lossy().into_owned(),
                backup_path: r.backup_path.to_string_lossy().into_owned(),
                status: r.state.as_str(),
                message: &r.message,
            })
            .collect(),
    };
    let mut text = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("Could not serialize {REMOVE_MANIFEST_NAME}: {e}"))?;
    text.push('\n');
    write_atomic_unique(
        &plan.backup_dir.join(REMOVE_MANIFEST_NAME),
        &mut text.as_bytes(),
    )
    .map(|_| ())
    .map_err(|e| format!("Could not write {REMOVE_MANIFEST_NAME}: {e}"))
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Any path component named like the backup root is off limits, at any depth:
/// a file moved out of one backup folder must not be re-removed.
fn in_backup_folder(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(REMOVE_BACKUP_DIRNAME)
    })
}

fn supported(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".jar") || lower.ends_with(".jar.disabled")
}

/// `stem`, then `stem-1`, `stem-2`, ... preserving the extension. Used for
/// both the dated folder and each file, so a second run in the same second
/// never lands on an existing destination.
fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_string);
    for i in 1..10_000 {
        let name = match &extension {
            Some(ext) => format!("{stem}-{i}.{ext}"),
            None => format!("{stem}-{i}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    // Exhausting ten thousand names means something is very wrong; returning
    // the original makes execution refuse an existing destination, never
    // overwrite one.
    path
}

/// Folder-safe form of an ISO timestamp, capped like the legacy app.
fn backup_stamp(created_at: &str) -> String {
    let cleaned: String = created_at
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "profile_content_remove".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

/// UTC ISO-8601 seconds. Done locally instead of pulling a date crate for one
/// folder name: the conversion is small, total, and tested at the epoch.
fn utc_timestamp(now: SystemTime) -> String {
    let epoch = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = epoch.div_euclid(86_400);
    let seconds = epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a proleptic
/// Gregorian date. Integer-only, so no locale or time zone can leak in.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests;
