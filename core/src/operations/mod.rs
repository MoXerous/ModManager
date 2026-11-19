//! F-145: the unified, read-only operations view.
//!
//! The journal already records every kind of operation this app runs; until
//! now only two per-kind recovery surfaces read it (`instances::recovery` for
//! creates, `modpack::recovery` for pack applies), and a download batch was
//! surfaced nowhere. This module is the single reader across kinds.
//!
//! Two rules it inherits from those modules:
//!
//! - **Read-only, always.** Nothing here writes a record, a file or a row;
//!   the listing is a scan and the assessment is a stat walk. Recovery and
//!   settling stay with the per-kind surfaces, which the view links to.
//! - **Keys, not sentences.** Every `reason`/`action` value is an i18n key
//!   the UI translates; core names what is true, never how to say it.
//!
//! Where an assessment already exists for a kind, it is REUSED, not
//! reimplemented: a create is assessed by `instances::recovery::assess_create`
//! and a pack apply by `modpack::recovery`'s own assessment. The one new
//! treatment is the generic one for kinds with no recovery surface (a
//! download batch), which reports only what the disk holds.

use crate::journal::{unfinished, Journal, OperationKind, OperationRecord, OperationState};

/// The reason key kinds with no recovery surface of their own carry: the
/// record never reached a terminal state, which after a restart means it was
/// interrupted.
pub const REASON_INTERRUPTED: &str = "operations.reason.interrupted";

/// One row of the unified operations table.
///
/// The record's own fields, plus nothing: kind, state, both clocks, the
/// instance/pack it belongs to, the five derived counters, and — only for an
/// unfinished record — a reason key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationSummary {
    pub operation_id: String,
    pub kind: OperationKind,
    pub state: OperationState,
    pub created_at: i64,
    pub updated_at: i64,
    /// The instance the operation belongs to (`profile_id` on the record).
    pub profile_id: String,
    /// Where the operation acts: the instance root, or the download target.
    pub target_path: String,
    pub planned_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub cancelled_count: usize,
    pub skipped_count: usize,
    /// The pack's display name for a pack apply, empty for every other kind.
    pub pack_name: String,
    /// An i18n reason key when the record is unfinished, empty when terminal.
    pub reason: String,
}

/// What one record's disk actually holds, read without writing anything.
///
/// `final_present` and `part_present` are evidence counts from the disk, in
/// the spirit of the pack assessment's `done_count`: the record's counters are
/// one write late, so the disk is what answers "how much is there".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAssessment {
    pub operation_id: String,
    pub kind: OperationKind,
    pub state: OperationState,
    pub target_path: String,
    /// Whether the record's staging directory exists (empty path: absent).
    pub staging_present: bool,
    /// Whether the record's backup directory exists (empty path: absent).
    pub backup_present: bool,
    /// How many items the record carries.
    pub item_count: usize,
    /// How many items' final destinations exist on disk.
    pub final_present: usize,
    /// How many items' `.part` files exist on disk.
    pub part_present: usize,
    /// i18n action key of the recovery surface that already owns this kind,
    /// empty when none exists (a batch has none — the view stays read-only).
    pub action: String,
    /// i18n reason key, empty when the record is terminal and needs nothing.
    pub reason: String,
}

/// Why an arbitrary record could not be assessed.
///
/// Two of these are about the REQUEST and one about the disk, kept apart
/// because they mean different things to the screen: a stale row is answered
/// with "refresh", a traversal-shaped id is a malformed request, and a
/// journal that cannot be read is a real failure.
#[derive(Debug, thiserror::Error)]
pub enum AssessError {
    /// The id is not the canonical uuid the journal writes; nothing was read.
    #[error("operation id is not canonical: {0}")]
    MalformedId(String),
    /// The id is well formed and no record carries it.
    #[error("no operation with id {0}")]
    Unknown(String),
    /// The journal could not be read.
    #[error(transparent)]
    Journal(#[from] crate::journal::JournalError),
}

/// Every record the journal holds, newest first.
///
/// Terminal records are listed too: the view is the table of everything the
/// journal recorded, and the state column is what separates live work from
/// history. A record that is unfinished carries its reason key, assessed the
/// way its own recovery surface would assess it.
pub fn list_operations(db: &crate::db::Db, journal: &Journal) -> Vec<OperationSummary> {
    let mut rows: Vec<OperationSummary> = journal
        .discover()
        .records()
        .map(|r| summarize(db, r))
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
    rows
}

/// One record, as the table shows it.
fn summarize(db: &crate::db::Db, r: &OperationRecord) -> OperationSummary {
    OperationSummary {
        operation_id: r.operation_id.clone(),
        kind: r.kind,
        state: r.state,
        created_at: r.created_at,
        updated_at: r.updated_at,
        profile_id: r.profile_id.clone(),
        target_path: r.target_path.clone(),
        planned_count: r.planned_count,
        completed_count: r.completed_count,
        failed_count: r.failed_count,
        cancelled_count: r.cancelled_count,
        skipped_count: r.skipped_count,
        pack_name: r
            .pack
            .as_ref()
            .map(|p| p.pack_name.clone())
            .unwrap_or_default(),
        reason: reason_of(db, r),
    }
}

/// The reason key for one record: nothing to say about a terminal one, the
/// per-kind assessment's answer where a recovery surface exists, and the
/// generic one where none does.
fn reason_of(db: &crate::db::Db, r: &OperationRecord) -> String {
    if !unfinished(r.state) {
        return String::new();
    }
    match r.kind {
        OperationKind::InstanceCreate => crate::instances::recovery::assess_create(db, r)
            .reason
            .key()
            .to_string(),
        OperationKind::MrpackInstall => crate::modpack::recovery::assess_record(r)
            .reason
            .key()
            .to_string(),
        OperationKind::DownloadBatch
        | OperationKind::UpdateDownload
        | OperationKind::UpdateInstall => REASON_INTERRUPTED.to_string(),
    }
}

/// Assesses one arbitrary record by id, read-only.
///
/// A record that exists is assessed by the logic that already owns its kind;
/// a record the journal does not know is the caller's error to translate.
/// Nothing here mutates anything: the create and pack assessments are
/// read-only, and the generic one only stats paths.
pub fn assess_operation(
    db: &crate::db::Db,
    journal: &Journal,
    operation_id: &str,
) -> Result<OperationAssessment, AssessError> {
    // The id is webview input. A non-canonical one is refused before it could
    // become a path, and a canonical one the journal does not hold is
    // `Unknown` — a stale table, not a failing disk.
    if !crate::journal::is_canonical_id(operation_id) {
        return Err(AssessError::MalformedId(operation_id.to_string()));
    }
    if !journal.knows(operation_id) {
        return Err(AssessError::Unknown(operation_id.to_string()));
    }
    let record = journal.read(operation_id)?;
    Ok(evidence_of(db, &record))
}

/// The disk walk, shared by every kind: what the paths the record names
/// actually hold right now.
fn disk_evidence(r: &OperationRecord) -> (bool, bool, usize, usize) {
    let staging_present =
        !r.staging_path.is_empty() && std::path::Path::new(&r.staging_path).exists();
    let backup_present = !r.backup_path.is_empty() && std::path::Path::new(&r.backup_path).exists();
    let mut final_present = 0usize;
    let mut part_present = 0usize;
    for item in &r.items {
        if !item.final_path.is_empty() && std::path::Path::new(&item.final_path).exists() {
            final_present += 1;
        }
        if !item.partial_path.is_empty() && std::path::Path::new(&item.partial_path).exists() {
            part_present += 1;
        }
    }
    (staging_present, backup_present, final_present, part_present)
}

/// Reuses the per-kind assessment where one exists; stats the disk where none
/// does.
fn evidence_of(db: &crate::db::Db, r: &OperationRecord) -> OperationAssessment {
    let (staging_present, backup_present, final_present, part_present) = disk_evidence(r);
    let mut out = OperationAssessment {
        operation_id: r.operation_id.clone(),
        kind: r.kind,
        state: r.state,
        target_path: r.target_path.clone(),
        staging_present,
        backup_present,
        item_count: r.items.len(),
        final_present,
        part_present,
        action: String::new(),
        reason: String::new(),
    };
    if !unfinished(r.state) {
        return out;
    }
    match r.kind {
        OperationKind::InstanceCreate => {
            let assessed = crate::instances::recovery::assess_create(db, r);
            out.action = assessed.action.key().to_string();
            out.reason = assessed.reason.key().to_string();
        }
        OperationKind::MrpackInstall => {
            let assessed = crate::modpack::recovery::assess_record(r);
            out.action = assessed.action.key().to_string();
            out.reason = assessed.reason.key().to_string();
        }
        OperationKind::DownloadBatch
        | OperationKind::UpdateDownload
        | OperationKind::UpdateInstall => {
            out.reason = REASON_INTERRUPTED.to_string();
        }
    }
    out
}

#[cfg(test)]
mod tests;
