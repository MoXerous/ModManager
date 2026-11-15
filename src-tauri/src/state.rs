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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use mm_core::db::Db;
use mm_core::journal::Journal;
use mm_core::paths::Paths;

use crate::dto::IpcError;

/// F-036: a second install tried to register while one is already running.
///
/// Refused, never queued and never silently merged into the running slot: two
/// concurrent batches sharing one flag would let the first operation's
/// `end_install` clear the second's cancel flag, turning a cancel into a
/// silent no-op. The UI answers this with `ErrorCode::Busy`.
///
/// `Display` by hand: `thiserror` is a `core` dependency, and this crate
/// gains nothing by carrying it for one variant.
#[derive(Debug)]
pub struct InstallAlreadyRunning;

impl std::fmt::Display for InstallAlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "another install is already running")
    }
}

impl std::error::Error for InstallAlreadyRunning {}

/// `Mutex`, not `RwLock`: the point is to serialise access to a connection that
/// is not `Sync`, not to allow concurrent reads.
#[derive(Clone)]
pub struct AppState {
    db: Arc<Mutex<Db>>,
    journal: Arc<Mutex<Journal>>,
    /// Resolved platform roots. Every command that needs a directory asks here
    /// instead of trusting a path from the webview (§5.2).
    paths: Paths,
    /// F-036: the cancel flag of the install currently running, if any.
    ///
    /// One slot, not a map: the journal operation id only exists once the
    /// batch has started writing, so there is nothing the webview could
    /// address a mid-flight cancel by. The install registers its own flag on
    /// `begin_install` and the slot is dropped when the batch settles. The
    /// flag carries its identity (`Arc::ptr_eq`), so an operation ending can
    /// only ever clear the flag IT registered — a second install is refused
    /// rather than left to race the first one for the slot.
    install_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    /// Test seam: the private journal store this state owns.
    ///
    /// `None` in production, where the journal is the app's own directory.
    /// That one has to outlive the state: the app must find its own
    /// unfinished operations after a restart, so it is never a random
    /// per-state directory.
    #[cfg(test)]
    owned_journal_root: Option<Arc<tempfile::TempDir>>,
    /// Test seam: the private config directory this state owns, for the same
    /// reason as the journal store — two tests must never see each other's
    /// key.
    #[cfg(test)]
    owned_config_root: Option<Arc<tempfile::TempDir>>,
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
            paths,
            install_cancel: Arc::new(Mutex::new(None)),
            // Production owns no private store: the journal is the app's own
            // directory, and it outlives the state.
            #[cfg(test)]
            owned_journal_root: None,
            #[cfg(test)]
            owned_config_root: None,
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
        // A config directory of its own: a shared one let one test's key
        // answer another's lookup.
        let config_root = tempfile::TempDir::new().expect("a private config directory");
        // Same layout production uses: `<root>/config` is the config dir, so
        // `config_root()` and the reported paths stay in agreement.
        let paths = Paths::from_root(config_root.path());
        // `save_key` writes a sibling temp and renames it over the target, so
        // the parent must exist before the first write.
        std::fs::create_dir_all(paths.config_dir()).expect("create the private config directory");
        Self {
            db: Arc::new(Mutex::new(db)),
            journal: Arc::new(Mutex::new(Journal::new(root.path()))),
            paths,
            install_cancel: Arc::new(Mutex::new(None)),
            owned_journal_root: Some(Arc::new(root)),
            owned_config_root: Some(Arc::new(config_root)),
        }
    }

    /// The resolved platform roots (storage layout §1-8).
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The config directory this state reads and writes (ADR 0004 §2-1-1).
    pub fn config_dir(&self) -> &std::path::Path {
        self.paths.config_dir()
    }

    /// The root of the private config directory a test state owns.
    ///
    /// Test seam: the isolation test reads it to prove the status path is
    /// under a private temp root, not the machine's real profile.
    #[cfg(test)]
    pub(crate) fn config_root(&self) -> &std::path::Path {
        self.owned_config_root
            .as_ref()
            .expect("a test state owns a private config directory")
            .path()
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

    /// Test seam: a clone of the flag currently registered, if any. The
    /// resolve-phase cancel test needs to flip the flag from outside the
    /// install task while the resolve phase is in flight.
    #[cfg(test)]
    pub(crate) fn install_flag_for_test(&self) -> Option<Arc<AtomicBool>> {
        self.install_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// F-036: registers the flag of the install about to run and hands it back.
    ///
    /// `execute_install` calls this at the very top — BEFORE the resolve
    /// phase — so a cancel that lands while the network calls are in flight
    /// is observed the moment they return, instead of a flag that registers
    /// after the fact and stops nothing.
    ///
    /// A second install while one is registered is REFUSED
    /// ([`InstallAlreadyRunning`]), never queued and never merged: two
    /// operations sharing one slot is exactly the race this slot exists to
    /// prevent. The caller reports it as `ErrorCode::Busy`.
    ///
    /// A poisoned lock is recovered from, not panicked on: the value inside is
    /// always a plain `Option`, and a cancel must never take the app down.
    pub fn begin_install(&self) -> Result<Arc<AtomicBool>, InstallAlreadyRunning> {
        let flag = Arc::new(AtomicBool::new(false));
        let mut slot = self
            .install_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            return Err(InstallAlreadyRunning);
        }
        *slot = Some(flag.clone());
        Ok(flag)
    }

    /// F-036: flips the running install's flag. `true` when one was running.
    ///
    /// The answer is data, not an error: asking to cancel with nothing
    /// running is a normal race between the click and the batch's own end.
    pub fn cancel_active_install(&self) -> bool {
        let slot = self
            .install_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match slot.as_ref() {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// F-036: drops the registration once the batch has settled, whatever the
    /// outcome — but ONLY the flag this operation registered.
    ///
    /// The flag's identity (`Arc::ptr_eq`) is what makes the slot safe to
    /// share a process with: an operation finishing late cannot clear the
    /// flag of an operation that took the slot after it, which used to turn
    /// that operation's cancel into a silent no-op. And a stale flag left
    /// behind would "cancel" an install that has not started yet.
    pub fn end_install(&self, flag: &Arc<AtomicBool>) {
        let mut slot = self
            .install_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, flag))
        {
            *slot = None;
        }
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

/// RAII guard for a registered install flag (F-036).
///
/// `execute_install` registers the flag before the resolve phase, and the
/// resolve phase can fail through plain `?` returns long before the batch
/// starts. The guard makes "every exit drops the registration" hold by
/// construction instead of by remembering it at each return — and its
/// `end_install` only ever clears the flag it was born with, so a second
/// install that took the slot in between keeps its own.
pub struct InstallGuard<'a> {
    state: &'a AppState,
    flag: Arc<AtomicBool>,
}

impl<'a> InstallGuard<'a> {
    pub fn new(state: &'a AppState, flag: Arc<AtomicBool>) -> Self {
        Self { state, flag }
    }
}

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.state.end_install(&self.flag);
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

    /// F-036: the whole begin → cancel → end cycle.
    ///
    /// `begin_install` registers the flag so `cancel_active_install` can flip
    /// it, and `end_install` takes the slot back down: a stale flag left
    /// registered would "cancel" an install that has not started yet.
    #[test]
    fn spec_f036_begin_cancel_end_cycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = AppState::from_db(mm_core::db::open(&dir.path().join("t.db")).expect("db"));

        let cancel = state.begin_install().expect("nothing else is running");
        assert!(
            state.cancel_active_install(),
            "a registered install is the one a cancel reaches"
        );
        assert!(
            cancel.load(Ordering::Relaxed),
            "the flag the batch polls must be the one the cancel flipped"
        );

        state.end_install(&cancel);
        assert!(
            !state.cancel_active_install(),
            "after end_install there is nothing left to cancel"
        );
        assert!(state.install_flag_for_test().is_none());
    }

    /// F-036: two concurrent operations must not share the slot.
    ///
    /// The old code let a second `begin_install` silently steal the slot, so
    /// the first operation's `end_install` cleared the SECOND operation's
    /// flag — a cancel became a silent no-op. Now the second begin is
    /// refused, and an `end_install` clears only the flag it owns.
    #[test]
    fn spec_f036_a_second_install_is_refused_and_end_install_cannot_steal_the_slot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = AppState::from_db(mm_core::db::open(&dir.path().join("t.db")).expect("db"));

        let first = state.begin_install().expect("the first registers");
        // The second is refused — the slot is not stolen out from under the
        // first operation.
        let refused = state
            .begin_install()
            .expect_err("a second install must be refused, not merged into the slot");
        assert_eq!(refused.to_string(), "another install is already running");
        // Nothing was registered for it, so a cancel answers for the first.
        assert!(state.cancel_active_install());
        assert!(first.load(Ordering::Relaxed));

        // An end with a flag that is NOT the registered one changes nothing:
        // this is the old bug, pinned as a refusal.
        let foreign = Arc::new(AtomicBool::new(false));
        state.end_install(&foreign);
        assert!(
            state
                .install_flag_for_test()
                .is_some_and(|current| Arc::ptr_eq(&current, &first)),
            "a foreign end_install must not clear the registered flag"
        );

        // The first ends; the slot is free and the next install registers.
        state.end_install(&first);
        assert!(state.install_flag_for_test().is_none());
        let second = state.begin_install().expect("the slot is free again");
        assert!(!second.load(Ordering::Relaxed));
        state.end_install(&second);
    }

    /// F-036: the guard ends the install on every drop, and dropping it
    /// cannot clear a registration that replaced its own.
    #[test]
    fn spec_f036_the_guard_ends_only_the_install_it_was_born_with() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = AppState::from_db(mm_core::db::open(&dir.path().join("t.db")).expect("db"));

        let registered = {
            let cancel = state.begin_install().expect("registers");
            let guard = InstallGuard::new(&state, cancel.clone());
            assert!(
                state
                    .install_flag_for_test()
                    .is_some_and(|current| Arc::ptr_eq(&current, &cancel)),
                "the guard must not end the install while it lives"
            );
            drop(guard);
            assert!(
                state.install_flag_for_test().is_none(),
                "dropping the guard ends the install it registered"
            );
            cancel
        };
        // A late drop (or a double end) with the old flag: nothing left to
        // clear, and nothing to steal.
        state.end_install(&registered);
        assert!(state.install_flag_for_test().is_none());
    }
}
