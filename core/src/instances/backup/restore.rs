//! Restore (F-114) and the `saves` flag's restore half (F-115).
//!
//! Three gates before a single write: containment (the chosen folder is inside
//! this instance's backup root), identity (the manifest names this instance),
//! freshness (the manifest is the one the preview showed). Then, per group, the
//! live target is moved aside and the backup is copied back — never moved, so
//! one backup can be restored more than once — and every copied byte is hashed
//! against the manifest while it is written.
//!
//! The target of every write comes from [`EntrySource`] alone (ADR 0007 §6-5):
//! the manifest's `path` is display and cross-check, never routing.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use super::manifest::{EntrySource, ManifestError};
use super::BackupSubject;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreState {
    Restored,
    Failed,
    RolledBack,
    Cancelled,
    DependencyFailed,
}

impl RestoreState {
    pub fn as_str(self) -> &'static str {
        match self {
            RestoreState::Restored => "restored",
            RestoreState::Failed => "failed",
            RestoreState::RolledBack => "rolled-back",
            RestoreState::Cancelled => "cancelled",
            RestoreState::DependencyFailed => "dependency-failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    Complete,
    Partial,
    Cancelled,
}

impl RestoreOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            RestoreOutcome::Complete => "complete",
            RestoreOutcome::Partial => "partial",
            RestoreOutcome::Cancelled => "cancelled",
        }
    }
}

/// One planned group. Blocked rows never reach execution but are reported, so
/// the user is not told "complete" over something that was left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreEntry {
    pub source: EntrySource,
    pub backup_path: PathBuf,
    pub target_path: PathBuf,
    pub safety_path: PathBuf,
    pub allowed: bool,
    pub reason_code: &'static str,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePlan {
    pub root: PathBuf,
    pub backup_dir: PathBuf,
    pub safety_dir: PathBuf,
    pub created_at: String,
    pub include_saves: bool,
    pub manifest_sha256: String,
    pub entries: Vec<RestoreEntry>,
}

impl RestorePlan {
    pub fn allowed_entries(&self) -> impl Iterator<Item = &RestoreEntry> {
        self.entries.iter().filter(|e| e.allowed)
    }

    pub fn blocked_entries(&self) -> impl Iterator<Item = &RestoreEntry> {
        self.entries.iter().filter(|e| !e.allowed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreResultEntry {
    pub source: EntrySource,
    pub backup_path: PathBuf,
    pub target_path: PathBuf,
    pub safety_path: PathBuf,
    pub state: RestoreState,
    pub reason_code: String,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRun {
    pub outcome: RestoreOutcome,
    pub safety_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub entries: Vec<RestoreResultEntry>,
    pub blocked: Vec<RestoreEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreProgress {
    pub restored_files: u64,
    pub total_files: u64,
    pub restored_bytes: u64,
    pub total_bytes: u64,
    pub terminal: bool,
}

/// A safety folder with no completed manifest: a restore that died between
/// moving the target aside and copying it back (ADR 0007 §6-6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRestore {
    pub stamp: String,
    pub safety_dir: PathBuf,
    pub has_current: bool,
    pub groups: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("the selected folder is outside this instance's backup root")]
    BackupOutside,
    #[error("the selected folder is not a usable backup of this instance")]
    BackupMissing,
    #[error("the backup belongs to another instance")]
    InstanceMismatch,
    #[error("the backup changed after the plan was shown")]
    ManifestChanged,
    #[error("the typed confirmation word is required")]
    ConfirmationRequired,
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Atomic(#[from] crate::safety::atomic::AtomicError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Read-only plan: containment, identity, freshness, and one verdict per group.
pub fn build_restore_plan(
    root: &Path,
    subject: &BackupSubject,
    backup_dir: &Path,
    include_saves: bool,
    now: SystemTime,
) -> Result<RestorePlan, RestoreError> {
    let canonical_backup = canonical_backup_dir(root, backup_dir)?;
    let manifest_path = canonical_backup.join(super::BACKUP_MANIFEST_NAME);
    let manifest = read_restore_manifest(&manifest_path)?;
    // Identity is the row's id, never the name (editable) or the path (moves).
    if manifest.instance_id != subject.instance_id {
        return Err(RestoreError::InstanceMismatch);
    }

    let manifest_sha256 = super::manifest::manifest_digest(&manifest_path)?;
    let created_at = crate::instances::stamp::utc_timestamp(now);
    let safety_dir = crate::instances::stamp::unique_path(
        root.join(super::SAFETY_DIRNAME)
            .join(crate::instances::stamp::backup_stamp(&created_at)),
    );
    let entries = EntrySource::restore_order()
        .into_iter()
        .map(|source| {
            plan_entry(
                root,
                &canonical_backup,
                &safety_dir,
                &manifest,
                source,
                include_saves,
            )
        })
        .collect();
    Ok(RestorePlan {
        root: root.to_path_buf(),
        backup_dir: canonical_backup,
        safety_dir,
        created_at,
        include_saves,
        manifest_sha256,
        entries,
    })
}

/// Resolves the chosen folder and proves it is inside this instance's backup
/// root. Both sides are canonicalised, so a relative path, a spelled-out `..`
/// or a link cannot rename its way into scope.
pub(super) fn canonical_backup_dir(
    root: &Path,
    backup_dir: &Path,
) -> Result<PathBuf, RestoreError> {
    let requested = std::fs::canonicalize(backup_dir).map_err(|_| RestoreError::BackupMissing)?;
    let backup_root = root.join(super::BACKUP_DIRNAME);
    let backup_root =
        std::fs::canonicalize(&backup_root).map_err(|_| RestoreError::BackupMissing)?;
    if requested == backup_root || !requested.starts_with(&backup_root) {
        return Err(RestoreError::BackupOutside);
    }
    if !requested.is_dir() {
        return Err(RestoreError::BackupMissing);
    }
    Ok(requested)
}

pub(super) fn read_restore_manifest(
    manifest_path: &Path,
) -> Result<super::manifest::BackupManifest, RestoreError> {
    match super::manifest::read_manifest(manifest_path) {
        Ok(manifest) => Ok(manifest),
        // A folder with no manifest is not a backup, and a legacy manifest is
        // listed but refused: both answer the same way to a restore request.
        Err(ManifestError::Missing) | Err(ManifestError::Legacy) => {
            Err(RestoreError::BackupMissing)
        }
        Err(error) => Err(RestoreError::Manifest(error)),
    }
}

fn plan_entry(
    root: &Path,
    backup_dir: &Path,
    safety_dir: &Path,
    manifest: &super::manifest::BackupManifest,
    source: EntrySource,
    include_saves: bool,
) -> RestoreEntry {
    let target_path = source.target_of(root);
    let backup_path = backup_dir.join(source.group_name());
    let safety_path = safety_dir.join("current").join(source.group_name());
    let blocked = |reason_code: &'static str| RestoreEntry {
        source,
        backup_path: backup_path.clone(),
        target_path: target_path.clone(),
        safety_path: safety_path.clone(),
        allowed: false,
        reason_code,
        file_count: 0,
        total_bytes: 0,
    };

    // The row is in the plan so the user sees it, but no byte of it is ever
    // written back (ADR 0007 §6-3).
    if source == EntrySource::ProfileJson {
        return blocked("profile-not-restored");
    }
    let Ok(meta) = std::fs::symlink_metadata(&backup_path) else {
        return blocked("source-missing");
    };
    if crate::safety::guard::is_link_or_reparse(&backup_path) {
        return blocked("entry-is-link");
    }
    if source == EntrySource::Saves && !include_saves {
        return blocked("saves-blocked-by-default");
    }
    // The manifest is untrusted input: one row recorded outside its own group
    // blocks the whole group, because a half-copied directory is worse than a
    // refusal the user can read.
    if !rows_within_group(manifest, source) {
        return blocked("entry-outside-group");
    }
    if safety_path.exists() {
        return blocked("safety-exists");
    }
    if crate::safety::guard::link_at_or_inside(root, &target_path).is_some() {
        return blocked("target-under-link");
    }
    let (file_count, total_bytes) = if meta.is_dir() {
        count_tree(&backup_path)
    } else {
        (1, meta.len())
    };
    RestoreEntry {
        source,
        backup_path,
        target_path,
        safety_path,
        allowed: true,
        reason_code: "allowed",
        file_count,
        total_bytes,
    }
}

/// Is every manifest row for `source` recorded under that group's directory?
///
/// `..`, an absolute path, a drive prefix, a backslash and a link component are
/// all refusals. The check never produces a path: it only says yes or no.
pub(super) fn rows_within_group(
    manifest: &super::manifest::BackupManifest,
    source: EntrySource,
) -> bool {
    manifest
        .entries
        .iter()
        .filter(|entry| entry.source == source)
        .all(|entry| {
            let group = source.group_name();
            if entry.path == group {
                return true;
            }
            let Some(rest) = entry
                .path
                .strip_prefix(group)
                .and_then(|r| r.strip_prefix('/'))
            else {
                return false;
            };
            if !source.is_dir() {
                // A file-sourced group has exactly one name, no children.
                return false;
            }
            rest.split('/').all(valid_component)
        })
}

fn valid_component(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains(':')
        && !part.contains('\\')
        && !part.contains('\0')
}

fn count_tree(path: &Path) -> (u64, u64) {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let child = entry.path();
            match std::fs::symlink_metadata(&child) {
                Ok(meta) if meta.is_dir() => stack.push(child),
                Ok(meta) if meta.is_file() => {
                    files += 1;
                    bytes += meta.len();
                }
                _ => {}
            }
        }
    }
    (files, bytes)
}

/// Re-check everything, then move-aside + copy each allowed group.
pub fn execute_restore(
    plan: &RestorePlan,
    confirmation: &str,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(RestoreProgress),
) -> Result<RestoreRun, RestoreError> {
    // The typed word is the user's answer to a plan they read. Nothing has
    // been written when this fails.
    if confirmation.trim() != super::CONFIRMATION_WORD {
        return Err(RestoreError::ConfirmationRequired);
    }
    let backup_dir = canonical_backup_dir(&plan.root, &plan.backup_dir)?;
    let manifest_path = backup_dir.join(super::BACKUP_MANIFEST_NAME);
    let manifest = read_restore_manifest(&manifest_path)?;
    // Freshness: the manifest must be the bytes whose plan the user read. A
    // changed one is refused before the first rename, not repaired.
    if super::manifest::manifest_digest(&manifest_path)? != plan.manifest_sha256 {
        return Err(RestoreError::ManifestChanged);
    }
    let total_files: u64 = plan.allowed_entries().map(|e| e.file_count).sum();
    let total_bytes: u64 = plan.allowed_entries().map(|e| e.total_bytes).sum();
    let mut restored_files = 0u64;
    let mut restored_bytes = 0u64;
    let mut cancelled = false;
    let mut first_failure = false;
    let mut mods_failed = false;
    let mut results: Vec<RestoreResultEntry> = Vec::new();

    let mut progress = |restored_files: u64, restored_bytes: u64, terminal: bool| {
        on_progress(RestoreProgress {
            restored_files,
            total_files,
            restored_bytes,
            total_bytes,
            terminal,
        });
    };

    for entry in plan.allowed_entries() {
        if cancelled || cancel.load(Ordering::Relaxed) {
            cancelled = true;
            results.push(result_of(entry, RestoreState::Cancelled, "cancelled"));
            continue;
        }
        // The ownership claim goes last: restoring the lock before the files
        // would make it vouch for bytes that never arrived.
        if entry.source == EntrySource::Lock && mods_failed {
            results.push(result_of(
                entry,
                RestoreState::DependencyFailed,
                "dependency-failed",
            ));
            continue;
        }
        if let Some(code) = recheck_entry(plan, entry, &manifest) {
            if entry.source == EntrySource::Mods {
                mods_failed = true;
            }
            results.push(result_of(entry, RestoreState::Failed, code));
            first_failure = true;
            continue;
        }
        match restore_group(
            entry,
            &manifest,
            cancel,
            &mut restored_files,
            &mut restored_bytes,
            &mut progress,
        ) {
            GroupEnd::Restored => {
                results.push(result_of(entry, RestoreState::Restored, "restored"));
            }
            GroupEnd::Cancelled => {
                cancelled = true;
                results.push(result_of(entry, RestoreState::Cancelled, "cancelled"));
            }
            GroupEnd::RolledBack(code) => {
                if entry.source == EntrySource::Mods {
                    mods_failed = true;
                }
                first_failure = true;
                results.push(result_of(entry, RestoreState::RolledBack, code));
            }
            GroupEnd::Failed(code) => {
                if entry.source == EntrySource::Mods {
                    mods_failed = true;
                }
                first_failure = true;
                results.push(result_of(entry, RestoreState::Failed, code));
            }
        }
    }

    let outcome = if cancelled {
        RestoreOutcome::Cancelled
    } else if first_failure {
        RestoreOutcome::Partial
    } else {
        RestoreOutcome::Complete
    };
    progress(restored_files, restored_bytes, true);

    let blocked: Vec<RestoreEntry> = plan.blocked_entries().cloned().collect();
    let manifest_path = write_restore_manifest(plan, outcome, &results, &blocked)?;
    Ok(RestoreRun {
        outcome,
        safety_dir: plan.safety_dir.clone(),
        manifest_path,
        entries: results,
        blocked,
    })
}

fn result_of(entry: &RestoreEntry, state: RestoreState, code: &str) -> RestoreResultEntry {
    RestoreResultEntry {
        source: entry.source,
        backup_path: entry.backup_path.clone(),
        target_path: entry.target_path.clone(),
        safety_path: entry.safety_path.clone(),
        state,
        reason_code: code.to_string(),
        file_count: entry.file_count,
        total_bytes: entry.total_bytes,
    }
}

/// The plan is not trusted at execution either: a hand-made one could name a
/// target outside the instance, or point at another group's backup.
fn recheck_entry(
    plan: &RestorePlan,
    entry: &RestoreEntry,
    manifest: &super::manifest::BackupManifest,
) -> Option<&'static str> {
    if entry.target_path != entry.source.target_of(&plan.root) {
        return Some("entry-outside-group");
    }
    if !rows_within_group(manifest, entry.source) {
        return Some("entry-outside-group");
    }
    if entry.source == EntrySource::Saves && !plan.include_saves {
        return Some("saves-blocked-by-default");
    }
    if crate::safety::guard::is_link_or_reparse(&entry.backup_path) {
        return Some("entry-is-link");
    }
    if !entry.backup_path.starts_with(&plan.backup_dir) {
        return Some("backup-outside");
    }
    if crate::safety::guard::link_at_or_inside(&plan.root, &entry.target_path).is_some() {
        return Some("target-under-link");
    }
    if entry.safety_path.exists() {
        return Some("safety-exists");
    }
    None
}

enum GroupEnd {
    Restored,
    Cancelled,
    RolledBack(&'static str),
    Failed(&'static str),
}

fn restore_group(
    entry: &RestoreEntry,
    manifest: &super::manifest::BackupManifest,
    cancel: &AtomicBool,
    restored_files: &mut u64,
    restored_bytes: &mut u64,
    on_progress: &mut dyn FnMut(u64, u64, bool),
) -> GroupEnd {
    let had_target = std::fs::symlink_metadata(&entry.target_path).is_ok();
    if had_target {
        let Some(parent) = entry.safety_path.parent() else {
            return GroupEnd::Failed("safety-unwritable");
        };
        if std::fs::create_dir_all(parent).is_err() {
            return GroupEnd::Failed("safety-unwritable");
        }
        if std::fs::rename(&entry.target_path, &entry.safety_path).is_err() {
            return GroupEnd::Failed("move-aside-failed");
        }
    }

    let expected = expected_hashes(manifest, entry.source);
    match copy_group(
        entry,
        &expected,
        cancel,
        restored_files,
        restored_bytes,
        on_progress,
    ) {
        Ok(()) => GroupEnd::Restored,
        Err("cancelled") => {
            rollback(entry, had_target);
            GroupEnd::Cancelled
        }
        Err(code) => {
            if rollback(entry, had_target) {
                GroupEnd::RolledBack(code)
            } else {
                GroupEnd::Failed(code)
            }
        }
    }
}

/// Puts the previous target back after a failed or cancelled group, keeping
/// the half-copied tree for inspection rather than deleting it.
fn rollback(entry: &RestoreEntry, had_target: bool) -> bool {
    let Some(current) = entry.safety_path.parent() else {
        return false;
    };
    let Some(safety_dir) = current.parent() else {
        return false;
    };
    let failed = crate::instances::stamp::unique_path(
        safety_dir
            .join("failed_restore")
            .join(entry.source.group_name()),
    );
    if std::fs::symlink_metadata(&entry.target_path).is_ok() {
        if let Some(parent) = failed.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::rename(&entry.target_path, &failed);
    }
    if had_target {
        let _ = std::fs::rename(&entry.safety_path, &entry.target_path);
        return true;
    }
    false
}

fn expected_hashes(
    manifest: &super::manifest::BackupManifest,
    source: EntrySource,
) -> std::collections::HashMap<String, String> {
    manifest
        .entries
        .iter()
        .filter(|entry| entry.source == source && entry.status.has_bytes())
        .map(|entry| (entry.path.clone(), entry.sha256.clone()))
        .collect()
}

fn copy_group(
    entry: &RestoreEntry,
    expected: &std::collections::HashMap<String, String>,
    cancel: &AtomicBool,
    restored_files: &mut u64,
    restored_bytes: &mut u64,
    on_progress: &mut dyn FnMut(u64, u64, bool),
) -> Result<(), &'static str> {
    if entry.source.is_dir() {
        std::fs::create_dir_all(&entry.target_path).map_err(|_| "write-failed")?;
        let mut stack = vec![(entry.backup_path.clone(), entry.target_path.clone())];
        while let Some((dir, dest_dir)) = stack.pop() {
            let read = std::fs::read_dir(&dir).map_err(|_| "read-failed")?;
            for child in read.flatten() {
                if cancel.load(Ordering::Relaxed) {
                    return Err("cancelled");
                }
                let child_path = child.path();
                let meta = std::fs::symlink_metadata(&child_path).map_err(|_| "read-failed")?;
                // A link inside the backup is a refusal, not a copy: it could
                // point outside the backup entirely.
                if crate::safety::guard::is_link_or_reparse(&child_path) {
                    return Err("entry-is-link");
                }
                let dest_path = dest_dir.join(child.file_name());
                if meta.is_dir() {
                    std::fs::create_dir_all(&dest_path).map_err(|_| "write-failed")?;
                    stack.push((child_path, dest_path));
                } else if meta.is_file() {
                    let rel = child_path
                        .strip_prefix(&entry.backup_path)
                        .map(|rel| format!("{}/{}", entry.source.group_name(), rel_display(rel)))
                        .map_err(|_| "read-failed")?;
                    let Some(want) = expected.get(&rel) else {
                        return Err("unrecorded-file");
                    };
                    copy_verified(&child_path, &dest_path, want)?;
                    *restored_files += 1;
                    *restored_bytes += meta.len();
                    on_progress(*restored_files, *restored_bytes, false);
                }
            }
        }
        Ok(())
    } else {
        let rel = entry.source.group_name().to_string();
        let Some(want) = expected.get(&rel) else {
            return Err("unrecorded-file");
        };
        let len = std::fs::metadata(&entry.backup_path)
            .map_err(|_| "read-failed")?
            .len();
        copy_verified(&entry.backup_path, &entry.target_path, want)?;
        *restored_files += 1;
        *restored_bytes += len;
        on_progress(*restored_files, *restored_bytes, false);
        Ok(())
    }
}

/// Copies one file while hashing it: the bytes compared to the manifest are
/// the bytes written, so a second read can never disagree with the first.
fn copy_verified(from: &Path, to: &Path, want: &str) -> Result<(), &'static str> {
    use sha2::Digest as _;
    let mut input = std::fs::File::open(from).map_err(|_| "read-failed")?;
    // `create_new` refuses an existing destination instead of replacing a file
    // that appeared between the rename and the copy.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .map_err(|_| "write-failed")?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut input, &mut buf).map_err(|_| "read-failed")?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        std::io::Write::write_all(&mut output, &buf[..read]).map_err(|_| "write-failed")?;
    }
    let got = super::manifest::hex(hasher.finalize());
    if got != want {
        return Err("hash-mismatch");
    }
    Ok(())
}

fn rel_display(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(serde::Serialize)]
struct RestoreDoc<'a> {
    schema: &'a str,
    created_at: &'a str,
    backup_dir: String,
    safety_dir: String,
    manifest_sha256: &'a str,
    include_saves: bool,
    outcome: &'a str,
    entries: Vec<RestoreDocEntry>,
    blocked: Vec<RestoreDocEntry>,
}

#[derive(serde::Serialize)]
struct RestoreDocEntry {
    source: String,
    backup_path: String,
    target_path: String,
    safety_path: String,
    state: String,
    reason_code: String,
    file_count: u64,
    total_bytes: u64,
}

fn write_restore_manifest(
    plan: &RestorePlan,
    outcome: RestoreOutcome,
    entries: &[RestoreResultEntry],
    blocked: &[RestoreEntry],
) -> Result<PathBuf, RestoreError> {
    std::fs::create_dir_all(&plan.safety_dir)?;
    let row = |source: EntrySource,
               backup_path: &Path,
               target_path: &Path,
               safety_path: &Path,
               state: &str,
               reason_code: &str,
               file_count: u64,
               total_bytes: u64| RestoreDocEntry {
        source: source.as_str().to_string(),
        backup_path: backup_path.to_string_lossy().into_owned(),
        target_path: target_path.to_string_lossy().into_owned(),
        safety_path: safety_path.to_string_lossy().into_owned(),
        state: state.to_string(),
        reason_code: reason_code.to_string(),
        file_count,
        total_bytes,
    };
    let doc = RestoreDoc {
        schema: super::RESTORE_SCHEMA,
        created_at: &plan.created_at,
        backup_dir: plan.backup_dir.to_string_lossy().into_owned(),
        safety_dir: plan.safety_dir.to_string_lossy().into_owned(),
        manifest_sha256: &plan.manifest_sha256,
        include_saves: plan.include_saves,
        outcome: outcome.as_str(),
        entries: entries
            .iter()
            .map(|e| {
                row(
                    e.source,
                    &e.backup_path,
                    &e.target_path,
                    &e.safety_path,
                    e.state.as_str(),
                    &e.reason_code,
                    e.file_count,
                    e.total_bytes,
                )
            })
            .collect(),
        blocked: blocked
            .iter()
            .map(|e| {
                row(
                    e.source,
                    &e.backup_path,
                    &e.target_path,
                    &e.safety_path,
                    "blocked",
                    e.reason_code,
                    e.file_count,
                    e.total_bytes,
                )
            })
            .collect(),
    };
    let mut bytes = serde_json::to_vec_pretty(&doc).map_err(|_| RestoreError::BackupMissing)?;
    bytes.push(b'\n');
    let path = plan.safety_dir.join(super::RESTORE_MANIFEST_NAME);
    crate::safety::atomic::write_atomic_unique(&path, &mut bytes.as_slice())?;
    Ok(path)
}
/// Read-only scan of safety folders that never got a manifest.
///
/// A folder under the safety root without [`super::RESTORE_MANIFEST_NAME`] is a
/// restore that died mid-run; it is reported so the user can look at it, and
/// nothing here writes, moves or deletes. Directory links are not followed, so a
/// link planted in the safety folder cannot pull the scan elsewhere.
pub fn pending_restores(root: &Path) -> Vec<PendingRestore> {
    let safety_root = root.join(super::SAFETY_DIRNAME);
    let Ok(entries) = std::fs::read_dir(&safety_root) else {
        return Vec::new();
    };
    let mut pending: Vec<PendingRestore> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .filter(|dir| !dir.join(super::RESTORE_MANIFEST_NAME).exists())
        .map(|safety_dir| {
            let current = safety_dir.join("current");
            let has_current = current.is_dir();
            let mut groups = if has_current {
                std::fs::read_dir(&current)
                    .map(|children| {
                        children
                            .flatten()
                            .filter(|child| child.file_type().map(|t| t.is_dir()).unwrap_or(false))
                            .map(|child| child.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            groups.sort();
            PendingRestore {
                stamp: safety_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                safety_dir,
                has_current,
                groups,
            }
        })
        .collect();
    pending.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    pending
}
