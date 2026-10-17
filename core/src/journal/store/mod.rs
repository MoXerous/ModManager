//! Journal store: atomic per-operation JSON files + read-only discovery.
//!
//! Write path: refuse symlinked root (before **and** after mkdir) → refuse
//! symlinked target → exclusive temp in the *same* directory (cross-device
//! rename would lose atomicity) → write + fsync → chmod 0600 → rename →
//! best-effort directory fsync (skipped on Windows).
//!
//! `discover` is a pure leaf: it never creates the root, never follows a
//! symlink, sorts casefold by name, and reports per-file failures without
//! aborting the scan.

use super::{unfinished, JournalError, OperationRecord};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Read cap, applied on read only (spec §4-1).
pub const MAX_OPERATION_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryIssue {
    /// Basename only — full paths are privacy leakage (spec §4-7).
    pub filename: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryOutcome {
    Record(Box<OperationRecord>),
    Issue(DiscoveryIssue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub outcomes: Vec<DiscoveryOutcome>,
}

impl Discovery {
    pub fn records(&self) -> impl Iterator<Item = &OperationRecord> {
        self.outcomes.iter().filter_map(|o| match o {
            DiscoveryOutcome::Record(r) => Some(r.as_ref()),
            DiscoveryOutcome::Issue(_) => None,
        })
    }

    pub fn issues(&self) -> impl Iterator<Item = &DiscoveryIssue> {
        self.outcomes.iter().filter_map(|o| match o {
            DiscoveryOutcome::Issue(i) => Some(i),
            DiscoveryOutcome::Record(_) => None,
        })
    }

    pub fn unfinished(&self) -> impl Iterator<Item = &OperationRecord> {
        self.records().filter(|r| unfinished(r.state))
    }
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn io_err(context: &str, e: std::io::Error) -> JournalError {
    JournalError::Io(format!("{context}: {e}"))
}

/// Refuse a symlinked journal root and create it if missing. The check
/// runs before *and* after mkdir: a swapped-in link in between would
/// otherwise be followed (spec §4-4 step 1).
fn ensure_root_for_write(root: &Path) -> Result<(), JournalError> {
    if is_symlink(root) {
        return Err(JournalError::Io(
            "operation journal root is a symlink".into(),
        ));
    }
    if !root.exists() {
        std::fs::create_dir_all(root).map_err(|e| io_err("create journal root", e))?;
    }
    if is_symlink(root) {
        return Err(JournalError::Io(
            "operation journal root is a symlink".into(),
        ));
    }
    if !root.is_dir() {
        return Err(JournalError::Io(
            "operation journal root is not a folder".into(),
        ));
    }
    Ok(())
}

fn record_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.json"))
}

pub struct Journal {
    root: PathBuf,
}

impl Journal {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn read_record(&self, id: &str) -> Result<OperationRecord, JournalError> {
        let path = record_path(&self.root, id);
        let meta = std::fs::metadata(&path).map_err(|e| io_err("stat record", e))?;
        if meta.len() > MAX_OPERATION_JOURNAL_BYTES {
            return Err(JournalError::Io(
                "operation journal record is too large".into(),
            ));
        }
        let mut text = String::new();
        std::fs::File::open(&path)
            .map_err(|e| io_err("open record", e))?
            .read_to_string(&mut text)
            .map_err(|e| io_err("read record", e))?;
        let record: OperationRecord = serde_json::from_str(&text)
            .map_err(|e| JournalError::Io(format!("parse record: {e}")))?;
        if record.operation_id != id {
            return Err(JournalError::Io(
                "journal filename does not match operation ID".into(),
            ));
        }
        record.verify()?;
        Ok(record)
    }

    /// Commit one mutation: load, apply, bump, re-derive counters, atomic
    /// write. `expected` is the optimistic revision check; `None` skips.
    fn mutate(
        &self,
        id: &str,
        expected: Option<u64>,
        apply: impl FnOnce(&mut OperationRecord) -> Result<(), JournalError>,
    ) -> Result<OperationRecord, JournalError> {
        let mut record = self.read_record(id)?;
        if let Some(want) = expected {
            if record.revision != want {
                return Err(JournalError::Io(format!(
                    "operation revision conflict: expected {want}, found {}",
                    record.revision
                )));
            }
        }
        apply(&mut record)?;
        record.sync_counters();
        self.write_record(&record)?;
        Ok(record)
    }

    fn write_record(&self, record: &OperationRecord) -> Result<(), JournalError> {
        ensure_root_for_write(&self.root)?;
        let target = record_path(&self.root, &record.operation_id);
        if is_symlink(&target) {
            return Err(JournalError::Io(
                "operation journal record cannot replace a symlink".into(),
            ));
        }
        let temp = self.root.join(format!(
            ".{}.{}.tmp",
            record.operation_id,
            uuid::Uuid::new_v4().simple()
        ));
        let text = serde_json::to_string_pretty(record)
            .map_err(|e| JournalError::Io(format!("serialize record: {e}")))?;
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|e| io_err("create temp record", e))?;
            use std::io::Write;
            f.write_all(text.as_bytes())
                .map_err(|e| io_err("write temp record", e))?;
            f.write_all(b"\n")
                .map_err(|e| io_err("write temp record", e))?;
            f.sync_all().map_err(|e| io_err("fsync temp record", e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600));
            }
        }
        if let Err(e) = crate::safety::atomic::commit_part(&temp, &target) {
            let _ = std::fs::remove_file(&temp);
            return Err(JournalError::Io(format!("commit record: {e}")));
        }
        sync_dir_best_effort(&self.root);
        Ok(())
    }

    /// Fresh planned record. `operation_id` is regenerated while the
    /// filename already exists (spec §4-3).
    pub fn start(
        &self,
        kind: super::OperationKind,
        profile_id: &str,
        target: &str,
        items: Vec<super::OperationItem>,
    ) -> Result<OperationRecord, JournalError> {
        ensure_root_for_write(&self.root)?;
        let mut record = super::OperationRecord::new(kind, profile_id, target, items);
        while record_path(&self.root, &record.operation_id).exists() {
            record.operation_id = uuid::Uuid::new_v4().to_string();
        }
        self.write_record(&record)?;
        Ok(record)
    }

    pub fn read(&self, id: &str) -> Result<OperationRecord, JournalError> {
        self.read_record(id)
    }

    pub fn transition_operation(
        &self,
        id: &str,
        to: super::OperationState,
        expected: Option<u64>,
    ) -> Result<OperationRecord, JournalError> {
        self.mutate(id, expected, |r| r.transition_operation(to))
    }

    pub fn transition_item(
        &self,
        id: &str,
        item_id: &str,
        to: super::ItemState,
        expected: Option<u64>,
    ) -> Result<OperationRecord, JournalError> {
        self.mutate(id, expected, |r| r.transition_item(item_id, to))
    }

    pub fn replace_items(
        &self,
        id: &str,
        items: Vec<super::OperationItem>,
        expected: Option<u64>,
    ) -> Result<OperationRecord, JournalError> {
        self.mutate(id, expected, |r| r.replace_items(items))
    }

    pub fn bind_profile(
        &self,
        id: &str,
        profile_id: &str,
        expected: Option<u64>,
    ) -> Result<OperationRecord, JournalError> {
        self.mutate(id, expected, |r| r.bind_profile(profile_id))
    }

    /// Read-only scan. Creates nothing, follows no symlink, and never
    /// aborts on a single bad record (spec §4-6).
    pub fn discover(&self) -> Discovery {
        let mut outcomes: Vec<DiscoveryOutcome> = Vec::new();
        if is_symlink(&self.root) {
            return Discovery {
                outcomes: vec![DiscoveryOutcome::Issue(DiscoveryIssue {
                    filename: String::new(),
                    message: "operation journal root is a symlink and was not inspected.".into(),
                })],
            };
        }
        let rd = match std::fs::read_dir(&self.root) {
            Ok(rd) => rd,
            Err(_) => return Discovery { outcomes },
        };
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("json"))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort_by_key(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        });
        for path in paths {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if is_symlink(&path) {
                outcomes.push(DiscoveryOutcome::Issue(DiscoveryIssue {
                    filename,
                    message: "journal record is a symlink and was not read.".into(),
                }));
                continue;
            }
            let id = filename.trim_end_matches(".json");
            match self.read_record(id) {
                Ok(record) => outcomes.push(DiscoveryOutcome::Record(Box::new(record))),
                Err(e) => outcomes.push(DiscoveryOutcome::Issue(DiscoveryIssue {
                    filename,
                    message: safe_message(&e),
                })),
            }
        }
        Discovery { outcomes }
    }
}

/// Issue messages stay short and never contain a path (spec §4-6).
fn safe_message(e: &JournalError) -> String {
    let text = e.to_string();
    let mut out: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > 500 {
        out = out.chars().take(500).collect();
    }
    out
}

#[cfg(not(windows))]
fn sync_dir_best_effort(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

#[cfg(windows)]
fn sync_dir_best_effort(_dir: &Path) {}

#[cfg(test)]
pub(crate) mod tests;
