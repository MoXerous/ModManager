//! Process-wide application state.
//!
//! The database handle lives here rather than being opened per call: opening
//! runs the version gate, the pre-migration backup and the migrations, so it
//! must happen once. Every command that touches it goes through
//! `spawn_blocking`, because a `rusqlite::Connection` blocks and `Db` is
//! `Send` but not `Sync`.
//!
//! The operation journal is held beside it for the same reason, and is opened
//! once against the resolved platform directories.

use std::sync::{Arc, Mutex};

use mm_core::db::Db;
use mm_core::journal::Journal;
use mm_core::paths::Paths;

use crate::dto::IpcError;

/// `Mutex`, not `RwLock`: the point is to serialise access to a connection that
/// is not `Sync`, not to allow concurrent reads.
#[derive(Clone)]
pub struct AppState {
    db: Arc<Mutex<Db>>,
    journal: Arc<Mutex<Journal>>,
    /// Test seam: the private journal store this state owns.
    ///
    /// `None` in production, where the journal is the app's own directory.
    /// That one has to outlive the state: the app must find its own
    /// unfinished operations after a restart, so it is never a random
    /// per-state directory.
    #[cfg(test)]
    owned_journal_root: Option<Arc<tempfile::TempDir>>,
}

impl AppState {
    /// Resolves the platform directories, creates them, and opens the database
    /// at its standard location. Fails loudly at startup rather than per call.
    ///
    /// The journal root is deliberately *not* created here: `Journal`'s writer
    /// owns root creation, including its before/after symlink check.
    ///
    /// The root is the app's own stable directory and nothing else: a random
    /// per-state root would make the journal useless, because the app could
    /// not find its own unfinished operations after a restart.
    pub fn open() -> Result<Self, String> {
        let paths = Paths::resolve().map_err(|e| e.to_string())?;
        paths
            .ensure()
            .map_err(|e| format!("cannot create data directories: {e}"))?;
        let db = mm_core::db::open(&paths.db_file()).map_err(|e| e.to_string())?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            journal: Arc::new(Mutex::new(Journal::new(paths.operations_dir()))),
            // Production owns no private store: the journal is the app's own
            // directory, and it outlives the state.
            #[cfg(test)]
            owned_journal_root: None,
        })
    }

    /// Test seam: injects an already-open database so IPC tests can drive the
    /// real handle without touching the machine's data directory.
    ///
    /// The journal store is private to this state: a directory of its own,
    /// named uniquely, removed with the state that owns it. A shared store
    /// let one test's record answer another's lookup - the failure CI caught,
    /// where a lookup by archive digest came back with a record that belonged
    /// to a different test's temp directory.
    #[cfg(test)]
    pub fn from_db(db: Db) -> Self {
        let root = tempfile::TempDir::new().expect("a private journal store");
        Self {
            db: Arc::new(Mutex::new(db)),
            journal: Arc::new(Mutex::new(Journal::new(root.path()))),
            owned_journal_root: Some(Arc::new(root)),
        }
    }

    /// The private journal store this state owns.
    ///
    /// Test seam: the isolation test reads it to prove that two states in one
    /// process do not share a store.
    #[cfg(test)]
    fn journal_root(&self) -> &std::path::Path {
        self.owned_journal_root
            .as_ref()
            .expect("a test state owns a private journal store")
            .path()
    }

    /// The journal handle, for an operation that must write to it repeatedly
    /// over a long period.
    ///
    /// Not a `with_*` helper on purpose. Those lock for the length of the
    /// closure, and a download takes minutes — holding that lock would stall
    /// every other command that touches the journal, recovery reads included.
    /// The caller locks per write instead, which is what
    /// `execute_batch`'s callback is for.
    pub fn journal_handle(&self) -> Arc<Mutex<Journal>> {
        self.journal.clone()
    }

    /// Runs `f` against the database on a blocking thread.
    ///
    /// The error channel is `IpcError`, not `String`: it is what carries the
    /// code the UI translates, and flattening it here would report every
    /// failure as `io` — including "that folder is not empty", which is the
    /// user's input, not a failing disk.
    pub async fn with_db<T, F>(&self, f: F) -> Result<T, IpcError>
    where
        T: Send + 'static,
        F: FnOnce(&Db) -> Result<T, IpcError> + Send + 'static,
    {
        let db = self.db.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let guard = db.lock().map_err(|e| IpcError::internal(e.to_string()))?;
            f(&guard)
        })
        .await
        .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?
    }

    /// For an operation that touches both the database and the journal. The two
    /// locks are always taken in this order, so they cannot deadlock.
    pub async fn with_db_and_journal<T, F>(&self, f: F) -> Result<T, IpcError>
    where
        T: Send + 'static,
        F: FnOnce(&Db, &Journal) -> Result<T, IpcError> + Send + 'static,
    {
        let db = self.db.clone();
        let journal = self.journal.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let db_guard = db.lock().map_err(|e| IpcError::internal(e.to_string()))?;
            let journal_guard = journal
                .lock()
                .map_err(|e| IpcError::internal(e.to_string()))?;
            f(&db_guard, &journal_guard)
        })
        .await
        .map_err(|e| IpcError::internal(format!("blocking task failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-076: two states in one process keep a journal store of their own.
    ///
    /// The store used to be named after the process id, so every test in the
    /// binary wrote into the same folder and one test's record could answer
    /// another's lookup. That is how CI came to fail with a record from a
    /// different test's temp directory: a lookup by archive digest found a
    /// record that belonged to somebody else.
    ///
    /// Each state owns its store now, and this pins that: two states, two
    /// stores, one record each, and each state sees only its own record.
    #[test]
    fn spec_f076_two_states_in_one_process_keep_a_journal_store_of_their_own() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first =
            AppState::from_db(mm_core::db::open(&dir.path().join("first.db")).expect("first db"));
        let second =
            AppState::from_db(mm_core::db::open(&dir.path().join("second.db")).expect("second db"));

        // Two stores, not one - and the journal writes to the private one.
        let first_root = first.journal_root().to_path_buf();
        let second_root = second.journal_root().to_path_buf();
        assert_ne!(
            first_root, second_root,
            "the two states share one journal store"
        );
        assert_eq!(
            store_of(&first),
            first_root,
            "the first journal does not write to its private store"
        );
        assert_eq!(
            store_of(&second),
            second_root,
            "the second journal does not write to its private store"
        );

        // Each state writes a record of its own.
        let first_id = write_record(&first, "first");
        let second_id = write_record(&second, "second");

        // And each state sees its own record and nothing else.
        assert_eq!(
            record_ids(&first),
            vec![first_id],
            "the first state sees a record that is not its own"
        );
        assert_eq!(
            record_ids(&second),
            vec![second_id],
            "the second state sees a record that is not its own"
        );

        // The store does not outlive the state that owns it: a test leaves
        // nothing behind for the next one to inherit.
        drop(first);
        assert!(
            !first_root.exists(),
            "the journal store outlived the state that owned it"
        );
    }

    /// A record of its own, for the isolation test.
    fn write_record(state: &AppState, target: &str) -> String {
        let handle = state.journal_handle();
        let guard = handle.lock().expect("journal");
        guard
            .start(
                mm_core::journal::OperationKind::DownloadBatch,
                "i1",
                target,
                vec![],
            )
            .expect("write a record")
            .operation_id
    }

    /// Every record the state can see.
    fn record_ids(state: &AppState) -> Vec<String> {
        let handle = state.journal_handle();
        let guard = handle.lock().expect("journal");
        guard
            .discover()
            .records()
            .map(|r| r.operation_id.clone())
            .collect()
    }

    /// The store the journal actually reads and writes.
    fn store_of(state: &AppState) -> std::path::PathBuf {
        state
            .journal_handle()
            .lock()
            .expect("journal")
            .root()
            .to_path_buf()
    }
}
