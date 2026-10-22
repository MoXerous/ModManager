//! Operation journal: pure model + transition rules (no filesystem).
//!
//! Records are crash-auditable JSON per operation (F-141). State moves
//! only along the documented edges (F-142), and `discover` classifies
//! unfinished operations for startup reporting (F-143).
//!
//! A pack apply records the pack it applies (F-076): a record that says
//! "something was applied here" but not *what* cannot be resumed, and a
//! pack that changed between the plan and the apply cannot be recognised.
//! The pack block is additive — an operation that is not a pack apply has
//! none, and a record written before this block existed still reads.

pub mod store;

pub use store::{Discovery, DiscoveryOutcome, Journal};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    DownloadBatch,
    UpdateDownload,
    UpdateInstall,
    /// F-076: applying a `.mrpack` to an instance.
    ///
    /// The only kind that carries a [`PackApplyMeta`] block: a pack apply is
    /// the one operation whose subject is a *file* rather than a profile, and
    /// resuming it means knowing which archive the plan was built from.
    MrpackInstall,
    /// F-102: creating an instance folder, its `profile.json` and its row.
    ///
    /// Recorded with **zero items**. `OperationItem` is per-mod and its strict
    /// decoder rejects any shape change against every record already on disk,
    /// so a create is expressed through the record's own fields: `profile_id`
    /// is the instance id `plan_create` already minted, `target_path` is the
    /// root, and `staging_path`/`backup_path` stay empty because nothing is
    /// staged or backed up. All five derived counters read 0, which callers
    /// must treat as "no items" rather than a ratio.
    InstanceCreate,
}

/// The pack a record applies (F-076).
///
/// Four keys, strict on both sides, exactly like every other block in a
/// record. It is optional on the record: an operation that is not a pack
/// apply has none, and a record written before this block existed (schema
/// v1) has none either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackApplyMeta {
    /// The pack's display name, as the index spells it.
    pub pack_name: String,
    /// The pack's version id, as the index spells it.
    pub pack_version_id: String,
    /// SHA-512 of the archive the plan was built from, lowercase hex.
    ///
    /// This is the digest the plan showed the user. It is what ties a record
    /// to the plan it came from, and what tells a later run that the pack in
    /// front of the user is the pack this record is about.
    pub archive_sha512: String,
    /// Absolute path of the archive the bytes came from.
    pub archive_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Planned,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Every operation edge. `committing` is deliberately absent from item
/// edges — commit is uncancellable by design (spec §4-5).
pub const OPERATION_EDGES: &[(OperationState, OperationState)] = &[
    (OperationState::Planned, OperationState::Planned),
    (OperationState::Planned, OperationState::Running),
    (OperationState::Planned, OperationState::Failed),
    (OperationState::Planned, OperationState::Cancelled),
    (OperationState::Running, OperationState::Running),
    (OperationState::Running, OperationState::Completed),
    (OperationState::Running, OperationState::Failed),
    (OperationState::Running, OperationState::Cancelled),
];

pub const ITEM_EDGES: &[(ItemState, ItemState)] = &[
    (ItemState::Queued, ItemState::Queued),
    (ItemState::Queued, ItemState::Downloading),
    (ItemState::Queued, ItemState::Skipped),
    (ItemState::Queued, ItemState::Failed),
    (ItemState::Queued, ItemState::Cancelled),
    (ItemState::Downloading, ItemState::Downloading),
    (ItemState::Downloading, ItemState::Verifying),
    (ItemState::Downloading, ItemState::Failed),
    (ItemState::Downloading, ItemState::Cancelled),
    (ItemState::Verifying, ItemState::Verifying),
    (ItemState::Verifying, ItemState::Committing),
    (ItemState::Verifying, ItemState::Failed),
    (ItemState::Verifying, ItemState::Cancelled),
    // No `Committing -> Cancelled`: the commit is already irreversible.
    (ItemState::Committing, ItemState::Committing),
    (ItemState::Committing, ItemState::Completed),
    (ItemState::Committing, ItemState::Failed),
    (ItemState::Completed, ItemState::Completed),
    (ItemState::Skipped, ItemState::Skipped),
    (ItemState::Failed, ItemState::Failed),
    (ItemState::Cancelled, ItemState::Cancelled),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Queued,
    Downloading,
    Verifying,
    Committing,
    Completed,
    Skipped,
    Failed,
    Cancelled,
}

/// Exhaustive so a new variant cannot compile without deciding whether it
/// is terminal.
impl OperationState {
    pub fn is_terminal(self) -> bool {
        match self {
            OperationState::Planned | OperationState::Running => false,
            OperationState::Completed | OperationState::Failed | OperationState::Cancelled => true,
        }
    }

    pub fn may_go_to(self, to: Self) -> bool {
        OPERATION_EDGES.contains(&(self, to))
    }
}

impl ItemState {
    pub fn is_terminal(self) -> bool {
        match self {
            ItemState::Queued
            | ItemState::Downloading
            | ItemState::Verifying
            | ItemState::Committing => false,
            ItemState::Completed
            | ItemState::Skipped
            | ItemState::Failed
            | ItemState::Cancelled => true,
        }
    }

    pub fn may_go_to(self, to: Self) -> bool {
        ITEM_EDGES.contains(&(self, to))
    }
}

/// One unit of work. Exactly 8 keys: unknown **and** missing are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationItem {
    pub item_id: String,
    pub project_id: String,
    pub title: String,
    pub filename: String,
    pub state: ItemState,
    pub partial_path: String,
    pub final_path: String,
    pub message: String,
}

/// Builds the journal item for one pack destination (F-076).
///
/// The encoding is the eight-key one every `OperationItem` uses, and the
/// meaning of each key for a pack file is:
///
/// - `item_id`: the destination path. The plan has one op per destination,
///   so it is unique inside the record.
/// - `project_id`: empty. A pack file belongs to no project.
/// - `title`: the pack's name, so a record read on its own still says which
///   pack it is about.
/// - `filename`: the destination's file name.
/// - `state`: `queued` until the apply reaches it.
/// - `partial_path`: the `.part` this apply writes and owns. F-076: this is
///   the proof that the file is ours, and the only thing that lets a later
///   run remove it instead of refusing to install over it.
/// - `final_path`: the destination the file lands at.
/// - `message`: empty. The reason for a failure belongs to the operation's
///   own `message`, not to every item.
///
/// Both paths are absolute: a relative path would depend on the working
/// directory of whoever reads the record back, and the proof has to survive
/// the process that wrote it.
pub fn pack_item(
    safe_path: &str,
    pack_name: &str,
    partial_path: &str,
    final_path: &str,
) -> OperationItem {
    let filename = safe_path
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(safe_path)
        .to_string();
    OperationItem {
        item_id: safe_path.to_string(),
        project_id: String::new(),
        title: pack_name.to_string(),
        filename,
        state: ItemState::Queued,
        partial_path: partial_path.to_string(),
        final_path: final_path.to_string(),
        message: String::new(),
    }
}

/// The 5 counters are derived from `items`, never trusted from disk: the
/// decoder recomputes and compares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counters {
    pub planned_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub cancelled_count: usize,
    pub skipped_count: usize,
}

pub fn derive_counters(items: &[OperationItem]) -> Counters {
    let mut c = Counters {
        planned_count: items.len(),
        completed_count: 0,
        failed_count: 0,
        cancelled_count: 0,
        skipped_count: 0,
    };
    for i in items {
        match i.state {
            ItemState::Completed => c.completed_count += 1,
            ItemState::Failed => c.failed_count += 1,
            ItemState::Cancelled => c.cancelled_count += 1,
            ItemState::Skipped => c.skipped_count += 1,
            ItemState::Queued
            | ItemState::Downloading
            | ItemState::Verifying
            | ItemState::Committing => {}
        }
    }
    c
}

pub fn unfinished(state: OperationState) -> bool {
    !state.is_terminal()
}

/// 18 top-level keys, strict on both sides, plus the optional `pack` block
/// (F-076): 19 for a pack apply, 18 for everything else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub schema: String,
    pub operation_id: String,
    pub revision: u64,
    pub kind: OperationKind,
    pub state: OperationState,
    pub created_at: i64,
    pub updated_at: i64,
    pub profile_id: String,
    pub target_path: String,
    pub staging_path: String,
    pub backup_path: String,
    pub planned_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub cancelled_count: usize,
    pub skipped_count: usize,
    pub message: String,
    pub items: Vec<OperationItem>,
    /// F-076: the pack this operation applies.
    ///
    /// Optional and additive: absent for every operation that is not a pack
    /// apply, and absent from every record written before this block existed
    /// (schema v1). The `kind` and this block must agree — see
    /// [`OperationRecord::verify_pack`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<PackApplyMeta>,
}

/// The schema this build writes.
///
/// v2 added the optional `pack` block (F-076). v1 records are still read:
/// they are the records this app wrote before the block existed, and refusing
/// them would turn every one of them into a corrupt file.
pub const SCHEMA: &str = "modmanager.operation_journal.v2";

/// The schema this build still reads, and does not write.
pub const SCHEMA_V1: &str = "modmanager.operation_journal.v1";

/// Every schema this build accepts on read. Anything else is refused: a
/// record written by a newer build may have keys this one would misread.
pub const ACCEPTED_SCHEMAS: &[&str] = &[SCHEMA, SCHEMA_V1];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JournalError {
    #[error("illegal operation transition {from:?} -> {to:?}")]
    IllegalOperationTransition {
        from: OperationState,
        to: OperationState,
    },
    #[error("illegal item transition {from:?} -> {to:?}")]
    IllegalItemTransition { from: ItemState, to: ItemState },
    #[error("operation item is missing: {0}")]
    ItemMissing(String),
    #[error("duplicate item id: {0}")]
    DuplicateItem(String),
    #[error("counter mismatch for {0}")]
    CounterMismatch(&'static str),
    #[error("operation id must be a canonical lowercase uuid")]
    NotCanonicalId,
    /// The record's schema is not one this build reads. It was written by a
    /// newer build, and guessing its shape would be reading a file whose
    /// meaning this build does not know.
    #[error("unsupported operation journal schema: {found}")]
    UnsupportedSchema { found: String },
    /// F-076: the record's kind and its pack block disagree. A pack apply
    /// without its pack cannot be resumed — there is no "what" to resume — and
    /// a pack block on an operation that is not a pack apply is a field no
    /// other kind has any meaning for.
    #[error("operation kind {kind:?} and its pack block disagree (has_pack: {has_pack})")]
    KindPackMismatch { kind: OperationKind, has_pack: bool },
    #[error("io: {0}")]
    Io(String),
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl OperationRecord {
    /// Fresh planned record: revision 1, `created_at == updated_at`.
    pub fn new(
        kind: OperationKind,
        profile_id: &str,
        target: &str,
        items: Vec<OperationItem>,
    ) -> Self {
        let now = now_secs();
        let counters = derive_counters(&items);
        Self {
            schema: SCHEMA.to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            kind,
            state: OperationState::Planned,
            created_at: now,
            updated_at: now,
            profile_id: profile_id.to_string(),
            target_path: target.to_string(),
            staging_path: String::new(),
            backup_path: String::new(),
            planned_count: counters.planned_count,
            completed_count: counters.completed_count,
            failed_count: counters.failed_count,
            cancelled_count: counters.cancelled_count,
            skipped_count: counters.skipped_count,
            message: String::new(),
            items,
            pack: None,
        }
    }

    /// F-076: attaches the pack this operation applies.
    ///
    /// A builder rather than a parameter of [`OperationRecord::new`]: every
    /// other kind has no pack, and making them all pass `None` would be a
    /// way of saying "no pack" thirty times over.
    pub fn with_pack(mut self, pack: PackApplyMeta) -> Self {
        self.pack = Some(pack);
        self
    }

    pub fn counters(&self) -> Counters {
        derive_counters(&self.items)
    }

    /// Re-derive and compare; a mismatch means a hand-edited or corrupt file.
    pub fn verify_counters(&self) -> Result<(), JournalError> {
        let c = self.counters();
        let pairs: [(&'static str, usize, usize); 5] = [
            ("planned_count", self.planned_count, c.planned_count),
            ("completed_count", self.completed_count, c.completed_count),
            ("failed_count", self.failed_count, c.failed_count),
            ("cancelled_count", self.cancelled_count, c.cancelled_count),
            ("skipped_count", self.skipped_count, c.skipped_count),
        ];
        for (key, stored, derived) in pairs {
            if stored != derived {
                return Err(JournalError::CounterMismatch(key));
            }
        }
        Ok(())
    }

    pub fn verify_item_ids(&self) -> Result<(), JournalError> {
        let mut seen: HashSet<&str> = HashSet::new();
        for item in &self.items {
            if !seen.insert(item.item_id.as_str()) {
                return Err(JournalError::DuplicateItem(item.item_id.clone()));
            }
        }
        Ok(())
    }

    /// Full load-time validation: strict keys happen in serde, here we
    /// re-check identity, ids, and counters.
    pub fn verify(&self) -> Result<(), JournalError> {
        self.verify_schema()?;
        self.verify_id()?;
        self.verify_item_ids()?;
        self.verify_counters()?;
        self.verify_pack()
    }

    /// The schema gate (F-141).
    ///
    /// Checked first, before anything else: a record written by a newer
    /// build may carry keys this one would read as something else, and the
    /// honest answer to "I do not know this file" is to refuse it rather than
    /// to interpret it. v1 is accepted — those are the records this app wrote
    /// before the pack block existed, and they are still perfectly readable.
    pub fn verify_schema(&self) -> Result<(), JournalError> {
        if ACCEPTED_SCHEMAS.contains(&self.schema.as_str()) {
            return Ok(());
        }
        Err(JournalError::UnsupportedSchema {
            found: self.schema.clone(),
        })
    }

    /// F-076: the kind and the pack block must agree.
    ///
    /// A `mrpack_install` record without its pack cannot be resumed: there is
    /// no archive to resume *against*, and a record that names no archive
    /// could be resumed against any. A pack block on an operation that is not
    /// a pack apply is a field that kind has no meaning for, so it is a
    /// record this build did not write and does not understand.
    ///
    /// A v1 record has no pack block at all, and this build never wrote a
    /// v1 `mrpack_install` record — the pack apply was not journalled before
    /// this block existed — so refusing one costs nothing that was ever
    /// written.
    pub fn verify_pack(&self) -> Result<(), JournalError> {
        let consistent = match self.kind {
            OperationKind::MrpackInstall => self.pack.is_some(),
            _ => self.pack.is_none(),
        };
        if consistent {
            return Ok(());
        }
        Err(JournalError::KindPackMismatch {
            kind: self.kind,
            has_pack: self.pack.is_some(),
        })
    }

    /// Canonical lowercase UUID only: `parse_str` also accepts uppercase
    /// and brace forms, so the round-trip string compare is the gate.
    pub fn verify_id(&self) -> Result<(), JournalError> {
        match uuid::Uuid::parse_str(&self.operation_id) {
            Ok(u) if u.to_string() == self.operation_id => Ok(()),
            _ => Err(JournalError::NotCanonicalId),
        }
    }

    fn touch(&mut self) {
        self.revision += 1;
        self.updated_at = now_secs();
    }

    fn sync_counters(&mut self) {
        let c = self.counters();
        self.planned_count = c.planned_count;
        self.completed_count = c.completed_count;
        self.failed_count = c.failed_count;
        self.cancelled_count = c.cancelled_count;
        self.skipped_count = c.skipped_count;
    }

    pub fn replace_items(&mut self, items: Vec<OperationItem>) -> Result<(), JournalError> {
        self.items = items;
        self.verify_item_ids()?;
        self.sync_counters();
        self.touch();
        Ok(())
    }

    pub fn bind_profile(&mut self, profile_id: &str) -> Result<(), JournalError> {
        if self.profile_id == profile_id {
            return Ok(());
        }
        self.profile_id = profile_id.to_string();
        self.touch();
        Ok(())
    }

    /// Same-state transitions are legal and still bump (spec §4-3).
    pub fn transition_operation(&mut self, to: OperationState) -> Result<(), JournalError> {
        if !self.state.may_go_to(to) {
            return Err(JournalError::IllegalOperationTransition {
                from: self.state,
                to,
            });
        }
        self.state = to;
        self.touch();
        Ok(())
    }

    pub fn transition_item(&mut self, item_id: &str, to: ItemState) -> Result<(), JournalError> {
        self.apply_item_state(item_id, to)?;
        self.sync_counters();
        self.touch();
        Ok(())
    }

    /// Applies several item transitions and bumps the revision **once**.
    ///
    /// The state machine still gates every edge — each transition is checked
    /// against the item's own state — but the record is written once for the
    /// whole batch. That matters because a record is a single document: every
    /// write rewrites the whole file, and the file grows with the number of
    /// items. A pack apply that moved four states per file would rewrite the
    /// record four times per file, so the bytes written grow with the square
    /// of the file count.
    ///
    /// Listing the same item twice is how a caller walks an item along a
    /// chain (`queued → downloading → verifying → committing`): each step is
    /// still checked, and the item ends on the last one.
    pub fn transition_items(
        &mut self,
        transitions: &[(String, ItemState)],
    ) -> Result<(), JournalError> {
        for (item_id, to) in transitions {
            self.apply_item_state(item_id, *to)?;
        }
        self.sync_counters();
        self.touch();
        Ok(())
    }

    /// One item's state change, without touching the revision.
    fn apply_item_state(&mut self, item_id: &str, to: ItemState) -> Result<(), JournalError> {
        let item = self
            .items
            .iter_mut()
            .find(|i| i.item_id == item_id)
            .ok_or_else(|| JournalError::ItemMissing(item_id.to_string()))?;
        if !item.state.may_go_to(to) {
            return Err(JournalError::IllegalItemTransition {
                from: item.state,
                to,
            });
        }
        item.state = to;
        Ok(())
    }
}

/// One journal write the caller needs.
///
/// A callback rather than a `&Journal`, and that is the whole point: a
/// `&Journal` held across a multi-minute download means the mutex behind it
/// is held for minutes too, and every other command that touches the journal
/// — including the recovery reads on the Instances page — waits for the last
/// byte. This type lets the caller open and close the lock per write.
#[derive(Debug, Clone)]
pub enum JournalOp {
    Start {
        target: String,
        items: Vec<OperationItem>,
    },
    Operation(OperationState),
    Item {
        item_id: String,
        state: ItemState,
    },
    /// F-076: a pack apply, with the pack's identity attached.
    ///
    /// A pack apply is the one operation whose subject is a file rather than
    /// a profile, so its record has to name the archive: a record that names
    /// no archive cannot be resumed against one.
    PackStart {
        target: String,
        items: Vec<OperationItem>,
        pack: PackApplyMeta,
    },
    /// Several item transitions, applied in one write.
    ///
    /// The same edges are checked, one after the other; only the write is
    /// shared. A pack apply that moved four states per file would otherwise
    /// rewrite the whole record four times per file, and the record grows
    /// with the number of files.
    Items {
        transitions: Vec<(String, ItemState)>,
    },
}

#[cfg(test)]
pub(crate) mod tests;
