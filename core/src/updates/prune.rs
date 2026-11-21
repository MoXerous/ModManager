//! Metadata-only prune of `instance.lock.json` (F-130).
//!
//! Pruning is a destructive act on the only durable record of what was
//! installed, so it has one shape: [`plan_prune`] is a mandatory dry run that
//! moves nothing, and [`execute_prune`] cannot run without the
//! [`PruneConfirmation`] that only a plan can mint. The execute step re-reads
//! the lock and re-scans the folder, so a file that came back between the plan
//! and the confirm keeps its entry (`lessons.md` §4.3: absence is a result, not
//! an error - and presence is a veto).
//!
//! Only metadata is touched. No `.jar` is renamed, moved or deleted, and an
//! entry is a candidate only when NO file on disk satisfies it (F-131). A file
//! that is present but not eligible for the target is not absence.

use std::path::Path;

use crate::instances::lock::{
    read_lock, write_lock, LockEntry, LockError, LockFile, LOCK_FILE_NAME,
};
use crate::instances::{scan_mods, Instance, LocalContentFact};

use super::target::entry_is_present;

/// One lock entry whose file is gone (F-130).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneCandidate {
    pub filename: String,
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
}

/// The dry run. Always read-only; `confirm` is the only way to ask for the
/// write, which is what makes the dry run mandatory rather than advisory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunePlan {
    pub candidates: Vec<PruneCandidate>,
    /// The dry run this plan is. A plan is never an execution.
    pub dry_run: bool,
    pub present_files: usize,
    pub lock_entries: usize,
    digest: String,
}

impl PrunePlan {
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// The token a confirm carries back; it is a hash of the lock's bytes, not
    /// the bytes themselves.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Consumes the dry run. There is no other constructor.
    pub fn confirm(self) -> PruneConfirmation {
        PruneConfirmation {
            digest: self.digest,
            filenames: self
                .candidates
                .into_iter()
                .map(|candidate| candidate.filename)
                .collect(),
        }
    }
}

/// Proof that the user saw a dry run. Fields are private and there is no
/// public constructor: it comes from [`PrunePlan::confirm`] and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneConfirmation {
    digest: String,
    filenames: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PruneReport {
    /// Entries removed from the lock (metadata only).
    pub removed: Vec<String>,
    /// Candidates whose file was back on disk at execute time; kept.
    pub kept_present: Vec<String>,
    pub lock_entries_before: usize,
    pub lock_entries_after: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum PruneError {
    #[error(transparent)]
    Lock(#[from] LockError),
    /// The lock moved between the dry run and the confirm. Nothing written;
    /// the user sees the plan again.
    #[error("the instance lock changed since the dry run")]
    LockChanged,
}

/// The mandatory dry run: scan, compare with the lock, name what would go.
pub fn plan_prune(instance_root: &Path) -> Result<PrunePlan, PruneError> {
    let path = instance_root.join(LOCK_FILE_NAME);
    let lock = match read_lock(&path) {
        Ok(lock) => lock,
        Err(LockError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            LockFile::default()
        }
        Err(error) => return Err(error.into()),
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(LockError::Io(error).into()),
    };
    let facts = scan_facts(instance_root);
    Ok(PrunePlan {
        candidates: candidates_for(&lock.entries, &facts),
        dry_run: true,
        present_files: facts.len(),
        lock_entries: lock.entries.len(),
        digest: lock_digest(&bytes),
    })
}

/// Remove the confirmed entries, but only those whose file is STILL absent.
///
/// A candidate whose file reappeared is kept and named in
/// [`PruneReport::kept_present`]. The write happens only when at least one
/// entry really goes, and it is the same atomic lock writer every other
/// mutation uses.
pub fn execute_prune(
    instance_root: &Path,
    confirmation: PruneConfirmation,
) -> Result<PruneReport, PruneError> {
    let path = instance_root.join(LOCK_FILE_NAME);
    let lock = match read_lock(&path) {
        Ok(lock) => lock,
        Err(LockError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            LockFile::default()
        }
        Err(error) => return Err(error.into()),
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(LockError::Io(error).into()),
    };
    if lock_digest(&bytes) != confirmation.digest {
        return Err(PruneError::LockChanged);
    }

    let facts = scan_facts(instance_root);
    let confirmed: Vec<String> = confirmation
        .filenames
        .iter()
        .map(|name| name.trim().to_lowercase())
        .collect();

    let before = lock.entries.len();
    let mut kept: Vec<LockEntry> = Vec::with_capacity(before);
    let mut removed = Vec::new();
    let mut kept_present = Vec::new();
    for entry in lock.entries {
        if confirmed.contains(&entry.filename.trim().to_lowercase()) {
            if entry_is_present(&entry, &facts) {
                kept_present.push(entry.filename.clone());
                kept.push(entry);
            } else {
                removed.push(entry.filename.clone());
            }
        } else {
            kept.push(entry);
        }
    }

    if !removed.is_empty() {
        write_lock(
            &path,
            &LockFile {
                entries: kept.clone(),
            },
        )?;
    }

    Ok(PruneReport {
        removed,
        kept_present,
        lock_entries_before: before,
        lock_entries_after: kept.len(),
    })
}

/// The scan the whole slice shares: the instance root is the Minecraft root
/// here, the same shape `scan_instance` uses.
fn scan_facts(instance_root: &Path) -> Vec<LocalContentFact> {
    scan_mods(&Instance {
        root: instance_root.to_path_buf(),
        minecraft_root: instance_root.to_path_buf(),
    })
}

/// The digest a confirm carries: the exact bytes the plan read.
pub(crate) fn lock_digest(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A recordable entry: the same files the update check and the planner judge.
pub(crate) fn is_recordable_entry(entry: &LockEntry) -> bool {
    let lower = entry.filename.to_ascii_lowercase();
    lower.ends_with(".jar") || lower.ends_with(".jar.disabled")
}

pub(crate) fn candidates_for(
    entries: &[LockEntry],
    facts: &[LocalContentFact],
) -> Vec<PruneCandidate> {
    entries
        .iter()
        .filter(|entry| is_recordable_entry(entry))
        .filter(|entry| !entry_is_present(entry, facts))
        .map(|entry| PruneCandidate {
            filename: entry.filename.clone(),
            provider: entry.provider.clone(),
            project_id: entry.project_id.clone(),
            version_id: entry.version_id.clone(),
        })
        .collect()
}
