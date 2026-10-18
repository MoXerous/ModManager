//! Process-wide application state.
//!
//! The database handle lives here rather than being opened per call: opening
//! runs the version gate, the pre-migration backup and the migrations, so it
//! must happen once. Every command that touches it goes through
//! `spawn_blocking`, because a `rusqlite::Connection` blocks and `Db` is
//! `Send` but not `Sync`.

use std::sync::{Arc, Mutex};

use mm_core::db::Db;
use mm_core::paths::Paths;

/// `Mutex`, not `RwLock`: the point is to serialise access to a connection that
/// is not `Sync`, not to allow concurrent reads.
#[derive(Clone)]
pub struct AppState {
    db: Arc<Mutex<Db>>,
}

impl AppState {
    /// Resolves the platform directories, creates them, and opens the database
    /// at its standard location. Fails loudly at startup rather than per call.
    pub fn open() -> Result<Self, String> {
        let paths = Paths::resolve().map_err(|e| e.to_string())?;
        paths
            .ensure()
            .map_err(|e| format!("cannot create data directories: {e}"))?;
        let db = mm_core::db::open(&paths.db_file()).map_err(|e| e.to_string())?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
        })
    }

    /// Test seam: injects an already-open database so IPC tests can drive the
    /// real handle without touching the machine's data directory.
    #[cfg(test)]
    pub fn from_db(db: Db) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
        }
    }

    /// Runs `f` against the database on a blocking thread.
    pub async fn with_db<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&Db) -> Result<T, String> + Send + 'static,
    {
        let db = self.db.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let guard = db
                .lock()
                .map_err(|_| "database lock poisoned".to_string())?;
            f(&guard)
        })
        .await
        .map_err(|e| format!("blocking task failed: {e}"))?
    }
}
