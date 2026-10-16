//! Atomic file writes (`F-021`, `F-034` mechanics).
//!
//! Every write goes to a sibling `.part` file first: `sync_all`, drop the
//! handle, then rename. On any failure the `.part` is removed, so a graceful
//! exit never leaves one behind. Hash verification stays with the caller
//! (the downloader): this module only makes the commit atomic.

use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Bounded rename retries for antivirus/indexer locks (`ERROR_SHARING_VIOLATION`).
const RENAME_RETRIES: u32 = 5;
const RENAME_WAIT_MS: u64 = 50;

#[derive(Debug, Error)]
pub enum AtomicError {
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("destination locked by another process: {}", .0.display())]
    ShareViolation(PathBuf),
}

/// Sibling part path: same directory, so rename stays on one volume.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Write `src` to `dest` atomically via a sibling `.part` file.
///
/// A stale `.part` from a killed run is never trusted: it is removed first.
/// On any failure the `.part` is removed again, so only success leaves `dest`.
/// Returns the bytes written.
pub fn write_atomic(dest: &Path, src: &mut impl Read) -> Result<u64, AtomicError> {
    let part = part_path(dest);
    let _ = std::fs::remove_file(&part);
    let result = write_inner(dest, &part, src);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn io_err(op: &'static str, path: &Path, source: std::io::Error) -> AtomicError {
    AtomicError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

fn write_inner(dest: &Path, part: &Path, src: &mut impl Read) -> Result<u64, AtomicError> {
    let mut f = std::fs::File::create(part).map_err(|e| io_err("create", part, e))?;
    let bytes = std::io::copy(src, &mut f).map_err(|e| io_err("write", part, e))?;
    f.sync_all().map_err(|e| io_err("sync", part, e))?;
    drop(f);
    rename_retry(part, dest)?;
    Ok(bytes)
}

/// Rename with bounded retries: antivirus/indexers briefly lock fresh files.
fn rename_retry(part: &Path, dest: &Path) -> Result<(), AtomicError> {
    let mut wait = RENAME_WAIT_MS;
    for attempt in 0..=RENAME_RETRIES {
        match std::fs::rename(part, dest) {
            Ok(()) => return Ok(()),
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied && attempt < RENAME_RETRIES =>
            {
                std::thread::sleep(std::time::Duration::from_millis(wait));
                wait *= 2;
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(AtomicError::ShareViolation(dest.to_path_buf()));
            }
            Err(e) => {
                return Err(AtomicError::Io {
                    op: "rename",
                    path: dest.to_path_buf(),
                    source: e,
                });
            }
        }
    }
    Err(AtomicError::ShareViolation(dest.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn spec_f021_write_atomic_replaces_target_and_leaves_no_part() {
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("a.jar");
        std::fs::write(&dest, b"old").expect("fixture");
        write_atomic(&dest, &mut Cursor::new(b"new bytes")).expect("writes");
        assert_eq!(std::fs::read(&dest).expect("read"), b"new bytes");
        assert!(!part_path(&dest).exists(), "no .part left behind");
    }

    #[test]
    fn spec_f021_write_atomic_failure_leaves_no_part_and_keeps_target() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("a.jar");
        std::fs::write(&dest, b"old").expect("fixture");
        assert!(write_atomic(&dest, &mut Failing).is_err());
        assert_eq!(std::fs::read(&dest).expect("read"), b"old");
        assert!(!part_path(&dest).exists(), "no .part left behind");
    }

    #[test]
    fn spec_f021_write_atomic_cleans_stale_part_first() {
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("a.jar");
        std::fs::write(part_path(&dest), b"stale").expect("fixture");
        write_atomic(&dest, &mut Cursor::new(b"fresh")).expect("writes");
        assert_eq!(std::fs::read(&dest).expect("read"), b"fresh");
    }
}
