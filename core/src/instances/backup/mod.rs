//! Full instance backup (F-113) and its opt-in `saves` flag (F-115).
//!
//! A backup is a dated folder inside the instance carrying a mirror of
//! `mods/`, `config/` and the instance's own metadata files, completed by
//! `backup_manifest.json`. It is copy-only: nothing in the live instance is
//! moved, renamed or deleted, and a folder without a manifest is not a backup.
//!
//! The plan is read-only ([`build_backup_plan`]); [`execute_backup`] writes.

pub mod manifest;
pub mod restore;
#[cfg(test)]
pub mod tests;
pub mod worlds;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;
use thiserror::Error;

use crate::instances::stamp::{backup_stamp, unique_path, utc_timestamp};
use crate::safety::guard;
use manifest::{
    read_manifest, write_manifest, BackupEntry, BackupManifest, EntryKind, EntrySource,
    EntryStatus, ManifestError, ManifestState, BACKUP_SCHEMA,
};

/// Dated backup root, relative to `content_root`. Kept byte-identical to the
/// old application: the name is a compatibility surface, its contents are not
/// (ADR 0007 §3, §5).
pub const BACKUP_DIRNAME: &str = ".modrinth_bulk_downloader_profile_backups";
/// Where an existing target is moved aside before the copy. Shared with the old
/// app and with `remove`.
pub const SAFETY_DIRNAME: &str = ".modrinth_bulk_downloader_restore_safety";
pub const BACKUP_MANIFEST_NAME: &str = "backup_manifest.json";
pub const RESTORE_MANIFEST_NAME: &str = "restore_manifest.json";
pub const RESTORE_SCHEMA: &str = "mm.instance_restore.v1";
/// The exact word a user types before a restore writes anything.
pub const CONFIRMATION_WORD: &str = "RESTORE";
/// Identity snapshot written into the backup when `profile.json` is absent.
pub const IDENTITY_SCHEMA: &str = "mm.instance_backup.identity.v1";

/// Third-party names that are never copied into a backup, at any depth: a
/// backup must not contain a backup, and app-owned scaffolding is not content.
pub const EXCLUDED_DIR_NAMES: [&str; 5] = [
    BACKUP_DIRNAME,
    SAFETY_DIRNAME,
    ".modrinth_bulk_downloader_removed_content",
    ".modrinth_bulk_downloader_backup",
    ".mm-staging",
];

/// Who the backup is about. `instance_id` is what a restore compares: a name is
/// editable and a path moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSubject {
    pub instance_id: String,
    pub name: String,
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPlan {
    pub root: PathBuf,
    pub backup_dir: PathBuf,
    pub created_at: String,
    pub subject: BackupSubject,
    pub include_saves: bool,
    pub total_files: u64,
    pub total_bytes: u64,
    /// Groups whose source folder was absent when the plan was built.
    pub missing_sources: Vec<EntrySource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupProgress {
    pub copied_files: u64,
    pub total_files: u64,
    pub copied_bytes: u64,
    pub total_bytes: u64,
    pub terminal: bool,
}

/// One thing the backup could not copy, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSkip {
    pub source: EntrySource,
    pub path: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRun {
    pub backup_dir: PathBuf,
    pub created_at: String,
    pub manifest_path: PathBuf,
    pub manifest_sha256: String,
    pub include_saves: bool,
    pub copied_files: u64,
    pub copied_bytes: u64,
    pub skipped: Vec<BackupSkip>,
}

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("the instance content root does not exist")]
    RootMissing,
    #[error("the instance content root is a link or reparse point")]
    RootIsLink,
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Atomic(#[from] crate::safety::atomic::AtomicError),
    #[error("the backup was cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What the backup list shows for one dated folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupCandidate {
    pub stamp: String,
    pub backup_dir: PathBuf,
    pub created_at: String,
    pub include_saves: bool,
    pub manifest_state: ManifestState,
    pub instance_matches: bool,
    pub entry_count: u64,
    pub total_bytes: u64,
}

/// Build a read-only plan: no folder is created, no file is opened for write.
pub fn build_backup_plan(
    root: &Path,
    subject: &BackupSubject,
    include_saves: bool,
    now: SystemTime,
) -> Result<BackupPlan, BackupError> {
    if !root.is_dir() {
        return Err(BackupError::RootMissing);
    }
    if guard::is_link_or_reparse(root) {
        return Err(BackupError::RootIsLink);
    }
    let created_at = utc_timestamp(now);
    let backup_dir = unique_path(root.join(BACKUP_DIRNAME).join(backup_stamp(&created_at)));
    let mut total_files = 0;
    let mut total_bytes = 0;
    let mut missing_sources = Vec::new();
    for source in sources(include_saves) {
        let path = source.target_of(root);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {
                collect_totals(&path, &mut total_files, &mut total_bytes);
            }
            Ok(meta) if meta.is_file() => {
                total_files += 1;
                total_bytes += meta.len();
            }
            _ => missing_sources.push(source),
        }
    }
    Ok(BackupPlan {
        root: root.to_path_buf(),
        backup_dir,
        created_at,
        subject: subject.clone(),
        include_saves,
        total_files,
        total_bytes,
        missing_sources,
    })
}

/// The groups a backup covers, in order. `saves` only when asked.
pub fn sources(include_saves: bool) -> Vec<EntrySource> {
    let mut out: Vec<EntrySource> = EntrySource::backup_order().to_vec();
    if include_saves {
        out.insert(4, EntrySource::Saves);
    }
    out
}

/// Read-only byte/file count for the plan. Links are not followed and not
/// counted: they are refused by the copy.
fn collect_totals(dir: &Path, files: &mut u64, bytes: &mut u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if is_excluded_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => collect_totals(&path, files, bytes),
            Ok(meta) if meta.is_file() => {
                *files += 1;
                *bytes += meta.len();
            }
            _ => {}
        }
    }
}

fn is_excluded_name(name: &str) -> bool {
    EXCLUDED_DIR_NAMES
        .iter()
        .any(|excluded| name.eq_ignore_ascii_case(excluded))
}

/// Copy the plan's groups into the dated folder, then write the manifest.
///
/// Order matters only for cancellation and progress here; nothing in the live
/// instance is written. A missing source folder is not an error: it is a
/// `skipped` manifest row with `source-missing`, so the backup says what it
/// does not contain.
pub fn execute_backup(
    plan: &BackupPlan,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(BackupProgress),
) -> Result<BackupRun, BackupError> {
    if !plan.root.is_dir() {
        return Err(BackupError::RootMissing);
    }
    if guard::is_link_or_reparse(&plan.root) {
        return Err(BackupError::RootIsLink);
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(BackupError::Cancelled);
    }
    std::fs::create_dir_all(&plan.backup_dir)?;

    let mut entries: Vec<BackupEntry> = Vec::new();
    let mut skipped: Vec<BackupSkip> = Vec::new();
    let mut copied_files = 0u64;
    let mut copied_bytes = 0u64;
    let mut progress = |copied_files: u64, copied_bytes: u64, terminal: bool| {
        on_progress(BackupProgress {
            copied_files,
            total_files: plan.total_files,
            copied_bytes,
            total_bytes: plan.total_bytes,
            terminal,
        });
    };

    for source in sources(plan.include_saves) {
        if cancel.load(Ordering::Relaxed) {
            return Err(BackupError::Cancelled);
        }
        let from = source.target_of(&plan.root);
        let to = plan.backup_dir.join(source.group_name());
        let meta = std::fs::symlink_metadata(&from);
        match meta {
            Err(_) => {
                skipped.push(BackupSkip {
                    source,
                    path: source.group_name().to_string(),
                    reason_code: "source-missing".to_string(),
                });
                entries.push(BackupEntry {
                    source,
                    path: source.group_name().to_string(),
                    kind: if source.is_dir() {
                        EntryKind::Dir
                    } else {
                        EntryKind::File
                    },
                    size: 0,
                    sha256: String::new(),
                    modified_at: 0,
                    version_id: String::new(),
                    status: EntryStatus::Skipped,
                    reason_code: "source-missing".to_string(),
                });
            }
            Ok(meta) if guard::is_link_or_reparse(&from) => {
                skipped.push(BackupSkip {
                    source,
                    path: source.group_name().to_string(),
                    reason_code: "symlink".to_string(),
                });
                entries.push(BackupEntry {
                    source,
                    path: source.group_name().to_string(),
                    kind: if meta.is_dir() {
                        EntryKind::Dir
                    } else {
                        EntryKind::File
                    },
                    size: 0,
                    sha256: String::new(),
                    modified_at: 0,
                    version_id: String::new(),
                    status: EntryStatus::Skipped,
                    reason_code: "symlink".to_string(),
                });
            }
            Ok(meta) if meta.is_file() => {
                if source == EntrySource::ProfileJson {
                    // Copied: an existing mirror is the instance's own words.
                    let digest = copy_file(&from, &to)?;
                    copied_files += 1;
                    copied_bytes += meta.len();
                    entries.push(file_entry(
                        source,
                        source.group_name(),
                        meta.len(),
                        digest,
                        meta,
                    ));
                } else {
                    let digest = copy_file(&from, &to)?;
                    copied_files += 1;
                    copied_bytes += meta.len();
                    entries.push(file_entry(
                        source,
                        source.group_name(),
                        meta.len(),
                        digest,
                        meta,
                    ));
                }
                progress(copied_files, copied_bytes, false);
            }
            Ok(meta) if meta.is_dir() => {
                if source == EntrySource::ProfileJson {
                    // Not a directory and never restored; treated as missing.
                    skipped.push(BackupSkip {
                        source,
                        path: source.group_name().to_string(),
                        reason_code: "source-missing".to_string(),
                    });
                    continue;
                }
                std::fs::create_dir_all(&to)?;
                let mut stack = vec![(from.clone(), to.clone())];
                while let Some((dir, dest_dir)) = stack.pop() {
                    let Ok(read) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for child in read.flatten() {
                        if cancel.load(Ordering::Relaxed) {
                            return Err(BackupError::Cancelled);
                        }
                        let name = child.file_name().to_string_lossy().into_owned();
                        if is_excluded_name(&name) {
                            continue;
                        }
                        let child_path = child.path();
                        let rel = child_path
                            .strip_prefix(&plan.root)
                            .map(relative_display)
                            .unwrap_or_else(|_| name.clone());
                        let Ok(child_meta) = std::fs::symlink_metadata(&child_path) else {
                            continue;
                        };
                        if guard::is_link_or_reparse(&child_path) {
                            skipped.push(BackupSkip {
                                source,
                                path: rel.clone(),
                                reason_code: "symlink".to_string(),
                            });
                            entries.push(BackupEntry {
                                source,
                                path: rel,
                                kind: EntryKind::File,
                                size: 0,
                                sha256: String::new(),
                                modified_at: 0,
                                version_id: String::new(),
                                status: EntryStatus::Skipped,
                                reason_code: "symlink".to_string(),
                            });
                            continue;
                        }
                        let dest = dest_dir.join(&name);
                        if child_meta.is_dir() {
                            std::fs::create_dir_all(&dest)?;
                            stack.push((child_path, dest));
                        } else if child_meta.is_file() {
                            let digest = copy_file(&child_path, &dest)?;
                            copied_files += 1;
                            copied_bytes += child_meta.len();
                            entries.push(file_entry(
                                source,
                                &rel,
                                child_meta.len(),
                                digest,
                                child_meta,
                            ));
                            progress(copied_files, copied_bytes, false);
                        }
                    }
                }
            }
            Ok(_) => {}
        }
    }

    // `saves` absent from the plan because the request did not ask for it is
    // still a fact the backup must state: a folder that silently omits a world
    // lies by omission.
    if !plan.include_saves
        && std::fs::symlink_metadata(plan.root.join(EntrySource::Saves.group_name()))
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
    {
        entries.push(BackupEntry {
            source: EntrySource::Saves,
            path: EntrySource::Saves.group_name().to_string(),
            kind: EntryKind::Dir,
            size: 0,
            sha256: String::new(),
            modified_at: 0,
            version_id: String::new(),
            status: EntryStatus::Skipped,
            reason_code: "saves-not-included".to_string(),
        });
        skipped.push(BackupSkip {
            source: EntrySource::Saves,
            path: EntrySource::Saves.group_name().to_string(),
            reason_code: "saves-not-included".to_string(),
        });
    }
    // `profile.json` absent: write an identity snapshot *inside the backup
    // only*. It is never restored, so it is not a second source of truth.
    if !plan
        .root
        .join(EntrySource::ProfileJson.group_name())
        .is_file()
    {
        let identity = identity_snapshot(&plan.subject, &plan.created_at);
        let mut bytes = serde_json::to_vec_pretty(&identity)
            .map_err(|_| BackupError::Manifest(ManifestError::Invalid))?;
        bytes.push(b'\n');
        let path = plan.backup_dir.join(EntrySource::ProfileJson.group_name());
        crate::safety::atomic::write_atomic_unique(&path, &mut bytes.as_slice())?;
        use sha2::Digest as _;
        entries.retain(|e| e.source != EntrySource::ProfileJson);
        entries.push(BackupEntry {
            source: EntrySource::ProfileJson,
            path: EntrySource::ProfileJson.group_name().to_string(),
            kind: EntryKind::File,
            size: bytes.len() as u64,
            sha256: manifest::hex(sha2::Sha256::digest(&bytes)),
            modified_at: 0,
            version_id: String::new(),
            status: EntryStatus::Generated,
            reason_code: String::new(),
        });
    }

    entries.sort_by(|a, b| {
        a.source
            .as_str()
            .cmp(b.source.as_str())
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut manifest = BackupManifest {
        schema: BACKUP_SCHEMA.to_string(),
        created_at: plan.created_at.clone(),
        instance_id: plan.subject.instance_id.clone(),
        instance_name: plan.subject.name.clone(),
        minecraft_version: plan.subject.minecraft_version.clone(),
        loader: plan.subject.loader.clone(),
        app_version: crate::app_info().version.to_string(),
        include_saves: plan.include_saves,
        entries_sha256: BackupManifest::digest_of(&entries),
        entries,
    };
    // The digest travels in the same document it describes; recomputing it
    // here means the value handed to the reader can never be the empty string
    // `digest_of` returns on a serialiser failure.
    manifest.entries_sha256 = BackupManifest::digest_of(&manifest.entries);

    let manifest_path = plan.backup_dir.join(BACKUP_MANIFEST_NAME);
    let manifest_sha256 = write_manifest(&manifest_path, &manifest)?;
    progress(copied_files, copied_bytes, true);

    Ok(BackupRun {
        backup_dir: plan.backup_dir.clone(),
        created_at: plan.created_at.clone(),
        manifest_path,
        manifest_sha256,
        include_saves: plan.include_saves,
        copied_files,
        copied_bytes,
        skipped,
    })
}

fn file_entry(
    source: EntrySource,
    path: &str,
    size: u64,
    digest: String,
    meta: std::fs::Metadata,
) -> BackupEntry {
    BackupEntry {
        source,
        path: path.to_string(),
        kind: EntryKind::File,
        size,
        sha256: digest,
        modified_at: modified_at(&meta),
        version_id: String::new(),
        status: EntryStatus::Copied,
        reason_code: String::new(),
    }
}

fn modified_at(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Copies one file and returns the SHA-256 of the bytes it wrote, hashing the
/// stream it reads (one pass, no second read).
fn copy_file(from: &Path, to: &Path) -> Result<String, BackupError> {
    use sha2::Digest;
    let mut input = std::fs::File::open(from)?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut input, &mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        std::io::Write::write_all(&mut output, &buf[..read])?;
    }
    Ok(manifest::hex(hasher.finalize()))
}

fn identity_snapshot(subject: &BackupSubject, created_at: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": IDENTITY_SCHEMA,
        "instance_id": subject.instance_id,
        "instance_name": subject.name,
        "minecraft_version": subject.minecraft_version,
        "loader": subject.loader,
        "created_at": created_at,
        "note": "identity snapshot for this backup only; never restored",
    })
}

/// Read-only listing of the dated folders inside the backup root.
///
/// Every folder is listed even when its manifest is missing or legacy: hiding
/// a folder the user can see in their file manager is the worse lie.
pub fn list_backups(root: &Path, instance_id: &str) -> Vec<BackupCandidate> {
    let backup_root = root.join(BACKUP_DIRNAME);
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(&backup_root) else {
        return out;
    };
    for child in read.flatten() {
        let path = child.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(BACKUP_MANIFEST_NAME);
        let state = manifest::manifest_state(&manifest_path);
        let manifest = read_manifest(&manifest_path).ok();
        let created_at = manifest
            .as_ref()
            .map(|m| m.created_at.clone())
            .or_else(|| {
                // A legacy manifest still carries the timestamp the old app
                // wrote; reading one key is not restoring it.
                legacy_created_at(&manifest_path)
            })
            .unwrap_or_default();
        let include_saves = manifest
            .as_ref()
            .map(|m| m.include_saves)
            .unwrap_or_else(|| path.join(EntrySource::Saves.group_name()).exists());
        let (entry_count, total_bytes) = manifest
            .as_ref()
            .map(|m| {
                let files = m
                    .entries
                    .iter()
                    .filter(|e| e.kind == EntryKind::File)
                    .count();
                let bytes = m.entries.iter().map(|e| e.size).sum();
                (files as u64, bytes)
            })
            .unwrap_or((0, 0));
        out.push(BackupCandidate {
            stamp: child.file_name().to_string_lossy().into_owned(),
            backup_dir: path,
            created_at,
            include_saves,
            manifest_state: state,
            instance_matches: manifest
                .as_ref()
                .map(|m| m.instance_id == instance_id)
                .unwrap_or(false),
            entry_count,
            total_bytes,
        });
    }
    out.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.stamp.cmp(&a.stamp))
    });
    out
}

fn legacy_created_at(manifest_path: &Path) -> Option<String> {
    let bytes = std::fs::read(manifest_path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("created_at")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// `/`-separated path for the manifest, independent of the platform separator.
fn relative_display(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}
