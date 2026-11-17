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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use mm_core::db::Db;
use mm_core::journal::Journal;
use mm_core::paths::Paths;

use crate::dto::{ErrorCode, IpcError};

/// F-139: the process exit code a refused startup returns.
///
/// 2 is what the old application left with, so a shortcut or a script that
/// checks it keeps telling "the database was refused" from a crash.
pub const STARTUP_FAILURE_EXIT_CODE: i32 = 2;

/// F-150: what the startup dialog says when the detail was withheld.
///
/// The startup detail is sanitized before it is shown or logged (F-138/F-150),
/// and a failure to sanitize means the text is not shareable at all. The user
/// gets this sentence instead of `<REDACTED:unavailable>`, which explains
/// nothing; `src/i18n/en.json` carries the same sentence under
/// `error.startupDetailUnavailable`, and a test pins the two together.
pub const DETAIL_UNAVAILABLE: &str =
    "The details of this failure could not be shown: they could not be redacted safely, so they were withheld rather than displayed. The log records that this happened.";

/// F-139: why the database gate refused to start the app.
///
/// Typed, not a `String`: the code is the part the UI translates, so the
/// startup dialog and the in-app error surface say the same thing. `message`
/// is the diagnostic, safe to show (see [`StartupFailure::from_db`]).
#[derive(Debug)]
pub struct StartupFailure {
    pub code: ErrorCode,
    pub message: String,
    /// F-138: the cause behind the sentence, sanitized.
    ///
    /// The dialog never carries it — a rusqlite or io Display may name a
    /// directory, and the sentence is what the user reads. This is what the
    /// log gets, because a failure with no cause on record cannot be
    /// diagnosed by anyone.
    pub detail: Option<String>,
}

/// F-138: text that is safe to show, log and quote in a report.
///
/// The migration message is already path-free by construction; this closes
/// the remaining cases (io, WAL), whose `Display` can name a directory.
///
/// F-150: when the text cannot be sanitized it is not shown at all — the
/// fixed token stands in for it, because the raw `Display` is exactly what
/// must not escape.
///
/// This runs for the *local* log too, which F-149 otherwise keeps raw. That is
/// a deliberate, small deviation inherited from F-138: the startup detail is
/// the one line that may be quoted for a bug report, and a marker is cheaper
/// than a second, sanitised copy of the same failure.
fn safe_text(message: impl std::fmt::Display) -> String {
    mm_core::logging::sanitize(&message.to_string())
        .unwrap_or_else(|_| mm_core::logging::UNAVAILABLE.to_string())
}

/// F-138: the source chain of a failure, sanitized, for the log only.
///
/// The whole chain, because a tagged failure's `detail` usually wraps the
/// rusqlite error that actually says why. `None` when the failure has no
/// cause of its own (`Busy` and `NewerSchema` explain themselves).
fn cause_of(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut parts = Vec::new();
    let mut next = error.source();
    while let Some(source) = next {
        parts.push(source.to_string());
        next = source.source();
    }
    if parts.is_empty() {
        None
    } else {
        Some(safe_text(parts.join(": ")))
    }
}

impl StartupFailure {
    /// F-138: maps the core failure to a code and a sanitized sentence.
    pub fn from_db(error: &mm_core::db::DbError) -> Self {
        use mm_core::db::DbError as D;
        let code = match error {
            D::NewerSchema { .. } => ErrorCode::NewerSchema,
            D::Busy => ErrorCode::DatabaseBusy,
            D::MigrationFailed { .. } => ErrorCode::MigrationFailed,
            D::Io(_) => ErrorCode::Io,
            D::Wal(_) | D::Migration(_) | D::Sql(_) => ErrorCode::Internal,
        };
        Self {
            code,
            message: safe_text(error),
            detail: cause_of(error),
        }
    }

    /// F-139: Tauri could not build the main window.
    ///
    /// A refused startup like the gate's, not a panic: the window layer's own
    /// `Display` can name a directory, so it goes to the log and the sentence
    /// stays fixed and path-free.
    pub fn window_failed(error: &(dyn std::error::Error + 'static)) -> Self {
        Self {
            code: ErrorCode::Internal,
            message: "the main window could not be created".to_string(),
            detail: Some(safe_text(error)),
        }
    }

    /// F-139: the sentence the startup dialog shows.
    ///
    /// English, like the log: the dialog appears before there is a locale, so
    /// it does not translate `error.<code>` itself — `error.databaseBusy` in
    /// front of the user says nothing at all. The catalog carries the same
    /// sentences for the in-app surface, and one of them,
    /// `error.startupDetailUnavailable`, is pinned to [`DETAIL_UNAVAILABLE`].
    ///
    /// F-150: a detail that could not be sanitized is *not* printed as the
    /// `<REDACTED:unavailable>` token. That token is an instruction to a
    /// developer, not a sentence for a user: the dialog explains the
    /// withholding, and the log keeps the token.
    pub fn dialog_text(&self) -> String {
        let message = if self.message.contains(mm_core::logging::UNAVAILABLE) {
            DETAIL_UNAVAILABLE
        } else {
            self.message.as_str()
        };
        format!("ModManager cannot start.\n\n{message}")
    }

    /// F-138: the line the log gets before the app leaves.
    ///
    /// The sentence, the cause it deliberately omits, and the typed code. The
    /// code belongs here and not in the dialog: it is the token a bug report
    /// and the in-app surface share, and a developer is the reader of a log.
    /// Sanitized, because a raw source may carry a directory.
    pub fn diagnostic(&self) -> String {
        let code = self.code.i18n_key();
        match &self.detail {
            Some(detail) => format!("[{code}] {}; cause: {detail}", self.message),
            None => format!("[{code}] {}", self.message),
        }
    }
}

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
    /// F-113/F-114: the cancel flag of the backup or restore running on each
    /// root, keyed by the resolved content root.
    ///
    /// A map where the install slot is one `Option`, because the two operations
    /// differ in what the webview can name: a backup or restore is addressed by
    /// its instance root, and two instances may be copied at once (the apply
    /// lock that serialises a run is per root), so one shared slot would let the
    /// second run's cancel reach the first. A run registers its own flag and
    /// drops it when it ends; the flag carries its identity (`Arc::ptr_eq`), so
    /// an operation finishing late can only clear the flag IT registered.
    transfer_cancel: Arc<Mutex<HashMap<PathBuf, Arc<AtomicBool>>>>,
    /// F-168: the content watcher of the instance the UI is currently looking
    /// at. One slot: the webview asks per instance and a new watch replaces
    /// the old one, so a stale watcher must never survive its instance.
    content_watch: Arc<Mutex<Option<ContentWatchSlot>>>,
    /// F-168: where the app itself recently wrote, for the watcher's
    /// suppression rule. Write commands note their root; the watcher drops
    /// events under a noted root inside its window.
    own_writes: mm_core::watcher::OwnWrites,
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
    pub fn open() -> Result<Self, StartupFailure> {
        let paths = Paths::resolve().map_err(|e| StartupFailure {
            code: ErrorCode::Io,
            message: safe_text(format_args!("cannot resolve the data directories: {e}")),
            detail: Some(safe_text(&e)),
        })?;
        Self::open_at(paths)
    }

    /// F-139: the gate, with the roots injected.
    ///
    /// `open()` is this plus `Paths::resolve()`. The split exists so the
    /// pre-window gate can be driven against a temp root: it must be tested
    /// on the real refusal path (newer schema, held writer, broken upgrade),
    /// and those need a database the test owns.
    pub fn open_at(paths: Paths) -> Result<Self, StartupFailure> {
        paths.ensure().map_err(|e| StartupFailure {
            code: ErrorCode::Io,
            message: safe_text(format_args!("cannot create the data directories: {e}")),
            detail: Some(safe_text(&e)),
        })?;
        let db = mm_core::db::open(&paths.db_file()).map_err(|e| StartupFailure::from_db(&e))?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            journal: Arc::new(Mutex::new(Journal::new(paths.operations_dir()))),
            paths,
            install_cancel: Arc::new(Mutex::new(None)),
            transfer_cancel: Arc::new(Mutex::new(HashMap::new())),
            content_watch: Arc::new(Mutex::new(None)),
            own_writes: mm_core::watcher::new_own_writes(),
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
            transfer_cancel: Arc::new(Mutex::new(HashMap::new())),
            content_watch: Arc::new(Mutex::new(None)),
            own_writes: mm_core::watcher::new_own_writes(),
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

    /// F-113/F-114: registers the flag of the backup or restore about to run on
    /// `root`, and hands it back.
    ///
    /// `execute_instance_backup`/`execute_instance_restore` call this BEFORE the
    /// plan is built, so a cancel that lands while the plan walks the tree is
    /// observed instead of being answered for a run that has not registered yet.
    ///
    /// A flag already registered for this root is ADOPTED, never replaced: a
    /// cancel arrives on another thread, and the one that lands between the click
    /// and the blocking task's start must be honoured rather than dropped. The
    /// per-root apply lock is what keeps adoption safe — at most one copy runs
    /// per root, so the flag has exactly one owner.
    ///
    /// A poisoned lock is recovered from, not panicked on: the value inside is a
    /// plain map, and a cancel must never take the app down.
    pub fn begin_transfer(&self, root: &Path) -> Arc<AtomicBool> {
        let mut slot = self
            .transfer_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    /// F-113/F-114: flips the flag of the copy running on `root`, if any.
    ///
    /// The answer is data, not an error: asking to cancel with nothing running is
    /// a normal race between the click and the run's own end.
    pub fn cancel_transfer(&self, root: &Path) -> bool {
        let slot = self
            .transfer_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match slot.get(root) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// F-113/F-114: drops this run's registration — but ONLY the flag this call
    /// was handed, so a run ending late cannot clear the flag of a run that
    /// registered after it for the same root.
    pub fn end_transfer(&self, root: &Path, flag: &Arc<AtomicBool>) {
        let mut slot = self
            .transfer_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot
            .get(root)
            .is_some_and(|current| Arc::ptr_eq(current, flag))
        {
            slot.remove(root);
        }
    }

    /// F-168: where the watcher reads the app's own recent writes.
    ///
    /// Write commands call [`Self::note_own_write`] with the root their batch
    /// touches; the watcher drops events under a noted root inside its
    /// suppression window, so an install never tells the user the content is
    /// stale.
    pub fn own_writes(&self) -> &mm_core::watcher::OwnWrites {
        &self.own_writes
    }

    pub fn note_own_write(&self, root: &Path) {
        mm_core::watcher::note_own_write(&self.own_writes, root);
    }

    /// F-168: the watcher slot the watch commands own. One slot, replaced on
    /// every new watch, so the previous instance's watcher never survives.
    pub fn replace_content_watch(&self, slot: Option<ContentWatchSlot>) {
        let mut guard = self
            .content_watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Teardown before replacement: the old watcher stops, whatever the
        // new one does next.
        if let Some(previous) = guard.take() {
            previous.handle.stop();
        }
        *guard = slot;
    }

    /// F-168: tears the current watcher down (instance change, app exit).
    pub fn stop_content_watch(&self) {
        self.replace_content_watch(None);
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

/// RAII guard for a registered backup/restore cancel flag (F-113/F-114).
///
/// The run registers its flag before the plan is built, and the plan can fail
/// through plain `?` returns long before the copy starts. The guard makes "every
/// exit drops the registration" hold by construction, and its `end_transfer`
/// only ever clears the flag it was born with — so a cancel after the run ends
/// finds nothing, and never reaches the next run.
pub struct TransferGuard<'a> {
    state: &'a AppState,
    root: PathBuf,
    flag: Arc<AtomicBool>,
}

impl<'a> TransferGuard<'a> {
    pub fn new(state: &'a AppState, root: PathBuf, flag: Arc<AtomicBool>) -> Self {
        Self { state, root, flag }
    }
}

impl Drop for TransferGuard<'_> {
    fn drop(&mut self) {
        self.state.end_transfer(&self.root, &self.flag);
    }
}

/// F-168: one live content watcher.
///
/// The handle itself is the identity: a replacement stops exactly the
/// watcher it replaces and never a newer one registered behind it.
#[derive(Debug)]
pub struct ContentWatchSlot {
    pub handle: Arc<mm_core::watcher::WatcherHandle>,
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
    /// F-139: the window must not exist before the database gate has run.
    ///
    /// Tauri creates every config-declared window before `.setup` runs, so a
    /// window declared there is a window on top of a database that may be
    /// refused. `"create": false` moves creation into `.setup`, after the
    /// gate, and `lib.rs` builds the window there.
    #[test]
    fn spec_f139_the_window_is_created_after_the_gate_not_by_the_config() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json is json");
        let windows = config["app"]["windows"]
            .as_array()
            .expect("the app declares a windows array");
        assert!(!windows.is_empty(), "no window is declared at all");
        // `create_main_window` builds `windows.first()`: a second declaration
        // would be a window that silently never exists.
        assert_eq!(
            windows.len(),
            1,
            "only the first declared window is ever built"
        );
        for window in windows {
            assert_eq!(
                window["create"].as_bool(),
                Some(false),
                "a config window exists before .setup: the gate would run behind it"
            );
        }
    }

    /// F-139: a file written by a newer version refuses startup, before any
    /// window exists, with a typed answer and the process exit code 2.
    ///
    /// The gate has to run against the real database path (that is what it
    /// protects), so it takes the resolved paths as an argument instead of
    /// resolving the machine's own directories: a test must never touch the
    /// real profile.
    #[test]
    fn spec_f139_open_at_refuses_a_newer_schema_with_a_typed_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::from_root(dir.path());
        let db_path = paths.db_file();
        std::fs::create_dir_all(db_path.parent().expect("the db sits in a directory"))
            .expect("create the data directory");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("raw open");
            conn.execute_batch("PRAGMA user_version = 99")
                .expect("stamp a newer schema");
        }

        let failure = match AppState::open_at(paths) {
            Ok(_) => panic!("a newer schema must refuse startup"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.code,
            ErrorCode::NewerSchema,
            "the UI can only translate a code: {}",
            failure.message
        );
        assert!(failure.message.contains("99"), "{}", failure.message);
        assert!(
            failure
                .message
                .contains(&mm_core::db::CURRENT_VERSION.to_string()),
            "the supported version is missing: {}",
            failure.message
        );
        let root = dir.path().to_string_lossy().into_owned();
        assert!(
            !failure.message.contains(&root),
            "the failure leaked a directory: {}",
            failure.message
        );
    }

    /// F-139: a refused startup leaves an exit code a script can read.
    ///
    /// The refusal path ends in `handle.exit(STARTUP_FAILURE_EXIT_CODE)`, and
    /// nothing here can run an event loop to observe it — so what is pinned
    /// is the contract a shortcut or an installer depends on: 2, the code the
    /// old application left with, distinct from every other way this process
    /// can end (0 a normal close, 1 a failed build, 101 a Rust panic).
    #[test]
    fn spec_f139_a_refused_startup_leaves_an_exit_code_of_its_own() {
        assert_eq!(
            STARTUP_FAILURE_EXIT_CODE, 2,
            "the old application left with 2; scripts and shortcuts rely on it"
        );
        for other in [0, 1, 101] {
            assert_ne!(
                STARTUP_FAILURE_EXIT_CODE, other,
                "a refused startup must be distinguishable from exit {other}"
            );
        }
    }

    /// F-138: a failed upgrade reaches startup as the safe sentence.
    ///
    /// The same failure the core test drives, one layer up: what the startup
    /// dialog and the log get is the stage, the two versions and the file
    /// name — and never the resolved path with the user name in it.
    #[test]
    fn spec_f138_a_migration_failure_reaches_startup_without_a_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::from_root(dir.path());
        let db_path = paths.db_file();
        std::fs::create_dir_all(db_path.parent().expect("the db sits in a directory"))
            .expect("create the data directory");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("raw open");
            conn.execute_batch(
                "CREATE TABLE instances (id TEXT PRIMARY KEY);
                 PRAGMA user_version = 0;",
            )
            .expect("a conflicting table at v0");
        }

        let failure = match AppState::open_at(paths) {
            Ok(_) => panic!("the conflicting table must refuse startup"),
            Err(failure) => failure,
        };
        assert_eq!(failure.code, ErrorCode::MigrationFailed);
        assert!(
            failure.message.contains("failed during migration"),
            "the stage is missing: {}",
            failure.message
        );
        assert!(
            failure.message.contains("from schema version 0"),
            "the versions are missing: {}",
            failure.message
        );
        assert!(
            failure.message.contains("app.sqlite3"),
            "the file name is missing: {}",
            failure.message
        );
        let root = dir.path().to_string_lossy().into_owned();
        assert!(
            !failure.message.contains(&root),
            "the failure leaked a directory: {}",
            failure.message
        );
    }

    /// F-138: the cause of a failed upgrade is kept for the log.
    ///
    /// The message is safe for the user but says nothing about why. The raw
    /// rusqlite/io failure is `MigrationFailed`'s `source`, and nothing read
    /// it: the diagnosis reached neither the dialog nor a log. The line the
    /// app records before it leaves carries it, sanitized.
    #[test]
    fn spec_f138_a_failed_migration_keeps_its_cause_for_the_log() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::from_root(dir.path());
        let db_path = paths.db_file();
        std::fs::create_dir_all(db_path.parent().expect("the db sits in a directory"))
            .expect("create the data directory");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("raw open");
            conn.execute_batch(
                "CREATE TABLE instances (id TEXT PRIMARY KEY);
                 PRAGMA user_version = 0;",
            )
            .expect("a conflicting table at v0");
        }

        let failure = match AppState::open_at(paths) {
            Ok(_) => panic!("the conflicting table must refuse startup"),
            Err(failure) => failure,
        };

        // The diagnostic carries what the safe sentence cannot: the cause.
        let diagnostic = failure.diagnostic();
        assert!(
            diagnostic.contains("failed during migration"),
            "the stage is missing from the log line: {diagnostic}"
        );
        assert!(
            diagnostic.contains("already exists"),
            "the cause is missing from the log line: {diagnostic}"
        );
        // ...and the user's sentence stays the safe one.
        assert!(
            !failure.dialog_text().contains("already exists"),
            "the cause belongs in the log, not in the dialog: {}",
            failure.dialog_text()
        );
        let root = dir.path().to_string_lossy().into_owned();
        assert!(
            !diagnostic.contains(&root),
            "the log line leaked a directory: {diagnostic}"
        );
    }

    /// F-139: the startup dialog speaks; it does not print catalog keys.
    ///
    /// It used to append `error.databaseBusy` in parentheses — the key, not
    /// the sentence, and in no language at all. The dialog is English, like
    /// the log; the code stays in the typed answer for the in-app surface.
    #[test]
    fn spec_f139_the_startup_dialog_does_not_print_a_translation_key() {
        let cases = [
            (ErrorCode::DatabaseBusy, "the database is busy"),
            (
                ErrorCode::MigrationFailed,
                "the database could not be upgraded",
            ),
            (ErrorCode::NewerSchema, "the database schema is newer"),
            (ErrorCode::Internal, "something failed"),
        ];
        for (code, message) in cases {
            let failure = StartupFailure {
                code,
                message: message.to_string(),
                detail: None,
            };
            let text = failure.dialog_text();
            assert!(
                !text.contains(&code.i18n_key()),
                "the dialog shows the key, not the text: {text}"
            );
            assert!(
                !text.contains("error."),
                "a catalog key reached the user: {text}"
            );
            assert!(text.contains(message), "the sentence is missing: {text}");
        }
    }

    /// F-150: a withheld startup detail is explained, never printed as a token.
    ///
    /// `safe_text` answers `<REDACTED:unavailable>` when a failure's `Display`
    /// cannot be sanitised, and the dialog used to print that token at a user
    /// with no explanation. The token stays in the log, where a developer
    /// reads it; the dialog says what happened, in a sentence both catalogs
    /// carry.
    #[test]
    fn spec_f150_the_startup_dialog_explains_a_withheld_detail() {
        let failure = StartupFailure {
            code: ErrorCode::Internal,
            message: mm_core::logging::UNAVAILABLE.to_string(),
            detail: None,
        };

        let text = failure.dialog_text();
        assert!(
            !text.contains(mm_core::logging::UNAVAILABLE),
            "the dialog shows a raw token: {text}"
        );
        assert!(
            text.contains(DETAIL_UNAVAILABLE),
            "the dialog does not explain itself: {text}"
        );
        // The log keeps the token: a maintainer needs to know it was withheld.
        assert!(
            failure.diagnostic().contains(mm_core::logging::UNAVAILABLE),
            "the log lost the withheld marker: {}",
            failure.diagnostic()
        );

        let en: serde_json::Value =
            serde_json::from_str(include_str!("../../src/i18n/en.json")).expect("en.json is json");
        let ar: serde_json::Value =
            serde_json::from_str(include_str!("../../src/i18n/ar.json")).expect("ar.json is json");
        assert_eq!(
            en["error.startupDetailUnavailable"].as_str(),
            Some(DETAIL_UNAVAILABLE),
            "the dialog sentence and en.json drifted apart"
        );
        assert!(
            ar["error.startupDetailUnavailable"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "ar.json has no error.startupDetailUnavailable"
        );
    }

    /// F-139: a window Tauri cannot build is a refused startup, not a panic.
    ///
    /// `create_main_window` returned `?` into `.setup`, so Tauri's own setup
    /// panic answered it: no sentence and exit 101. It is a `StartupFailure`
    /// like the gate's, and its message names no directory.
    #[test]
    fn spec_f139_a_window_that_cannot_be_built_is_a_startup_failure() {
        let error = std::io::Error::other(
            r"failed to create window at C:\Users\Alice\AppData\Local\ModManager",
        );
        let failure = StartupFailure::window_failed(&error);
        assert_eq!(failure.code, ErrorCode::Internal);
        let text = failure.dialog_text();
        assert!(
            text.contains("window"),
            "the sentence is unreadable: {text}"
        );
        assert!(
            !text.contains("Alice"),
            "the dialog leaked a directory: {text}"
        );
        let logged = failure.diagnostic();
        assert!(
            logged.contains("failed to create window"),
            "the cause is missing from the log: {logged}"
        );
        assert!(
            !logged.contains("Alice"),
            "the log line was not sanitized: {logged}"
        );
    }

    /// F-137/F-138: every new code has a sentence in both catalogs.
    ///
    /// The UI translates `error.<code>`; a code with no key renders as the raw
    /// dotted identifier, which is worse than no message at all.
    #[test]
    fn spec_f137_and_f138_the_new_codes_are_translated_in_both_catalogs() {
        let en: serde_json::Value =
            serde_json::from_str(include_str!("../../src/i18n/en.json")).expect("en.json is json");
        let ar: serde_json::Value =
            serde_json::from_str(include_str!("../../src/i18n/ar.json")).expect("ar.json is json");
        for code in [ErrorCode::DatabaseBusy, ErrorCode::MigrationFailed] {
            let key = code.i18n_key();
            for (locale, catalog) in [("en", &en), ("ar", &ar)] {
                let text = catalog
                    .get(&key)
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                assert!(!text.is_empty(), "{locale} has no {key}");
            }
        }
    }

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

    /// F-113/F-114: a backup or restore is cancelled from the screen, so the
    /// flag the copy polls must be reachable by root — and only by the flag the
    /// run registered.
    #[test]
    fn spec_f113_a_cancel_reaches_the_flag_the_running_copy_polls() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = AppState::from_db(mm_core::db::open(&dir.path().join("t.db")).expect("db"));
        let root = dir.path().join("instance");
        let other = dir.path().join("other-instance");

        let flag = state.begin_transfer(&root);
        assert!(
            !state.cancel_transfer(&other),
            "a cancel is addressed to one root, not to whatever is running"
        );
        assert!(
            state.cancel_transfer(&root),
            "the registered run is reached"
        );
        assert!(
            flag.load(Ordering::Relaxed),
            "the flag the blocking copy polls must be the one the cancel flipped"
        );

        // The same root adopts its own flag instead of replacing it: a cancel
        // that lands between the click and the run's registration is honoured,
        // not dropped.
        assert!(
            Arc::ptr_eq(&flag, &state.begin_transfer(&root)),
            "the registration was replaced under the running cancel"
        );

        state.end_transfer(&root, &flag);
        assert!(
            !state.cancel_transfer(&root),
            "after the run ends there is nothing left to cancel"
        );
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
