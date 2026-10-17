//! Operation journal: pure model + transition rules (no filesystem).
//!
//! Records are crash-auditable JSON per operation (F-141). State moves
//! only along the documented edges (F-142), and `discover` classifies
//! unfinished operations for startup reporting (F-143).

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
    MrpackInstall,
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

/// 18 top-level keys, strict on both sides.
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
}

pub const SCHEMA: &str = "modmanager.operation_journal.v1";

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
        }
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
        self.verify_id()?;
        self.verify_item_ids()?;
        self.verify_counters()
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
        self.sync_counters();
        self.touch();
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests;
