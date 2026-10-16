//! Scan cache keyed `(path, size, mtime)`: an unchanged jar is never
//! re-opened. No hashing — mtime+size misses same-second rewrites,
//! acceptable for a scan cache (re-check on flake: hash the file).

use super::LocalModMetadata;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Default)]
pub struct ScanCache {
    entries: HashMap<(PathBuf, u64, SystemTime), Option<LocalModMetadata>>,
    /// Test-only open counter (proves skip vs rescan).
    #[cfg(test)]
    pub(crate) reads: usize,
}

impl ScanCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&mut self, path: &Path) -> Option<LocalModMetadata> {
        let meta = std::fs::metadata(path).ok()?;
        let key = (
            path.to_path_buf(),
            meta.len(),
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        );
        if let Some(hit) = self.entries.get(&key) {
            return hit.clone();
        }
        #[cfg(test)]
        {
            self.reads += 1;
        }
        let data = super::read_local_mod_metadata(path);
        self.entries.insert(key, data.clone());
        data
    }
}
