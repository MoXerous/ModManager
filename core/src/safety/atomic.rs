//! Atomic file writes (`F-021`, `F-034` mechanics).
//!
//! Every write goes to a sibling part file first: `sync_all`, drop the
//! handle, then rename. The protocol has two halves and one implementation
//! of each: [`write_part`] puts the bytes there, [`commit_part`] makes them
//! durable and renames them into place, and [`write_atomic`] is the two
//! composed. A caller that wants to *look at the bytes before they land* -
//! the pack executor, which hashes the part before it renames it - uses the
//! halves directly.
//!
//! Two part names exist because the files fall into two groups. A transfer
//! that resumes (a mod download, a pack member) owns the fixed `.part` and
//! must find it again; [`write_atomic`] therefore never deletes one it cannot
//! attribute and refuses instead ([`AtomicError::StalePart`]). A file we
//! rewrite in place (the `profile.json` mirror) has nothing to resume:
//! [`write_atomic_unique`] writes through a fresh `.<name>.<uuid>.tmp` on
//! every attempt, so a temp left by a crashed run can never block the next
//! write and never has to be attributed. On a failure of our own write the
//! fresh part is removed, so a graceful exit never leaves one behind. Hash
//! verification stays with the caller (the downloader): this module only
//! makes the commit atomic.

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
    #[error("a `.part` file already exists at {}, and it is not provably ours; refusing to replace it", .0.display())]
    StalePart(PathBuf),
}

/// Sibling part path: same directory, so rename stays on one volume.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Sync + rename an already-written sibling `.part` (the second half of the
/// `.part` protocol).
///
/// Unlike [`write_atomic`], this never creates or truncates: the caller owns
/// the part file's whole lifecycle (streaming, resume). Fails if no `.part`.
///
/// The part is **not** removed when the rename fails: the caller may want to
/// resume from it, which is exactly what the downloader does. A caller that
/// owns the part and wants it gone on failure says so itself — that is what
/// [`write_atomic`] does.
pub fn commit_part(part: &Path, dest: &Path) -> Result<(), AtomicError> {
    // write:true: FlushFileBuffers fails on read-only handles (Windows).
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(part)
        .map_err(|e| io_err("open", part, e))?;
    f.sync_all().map_err(|e| io_err("sync", part, e))?;
    drop(f);
    rename_retry(part, dest)
}

/// Write `src` into the sibling `.part` file of `dest` and stop there.
///
/// This is the first half of the `.part` protocol, and the only place the
/// protocol is written: bytes go to `dest.part`, `sync_all`, drop the
/// handle. The file is **not** renamed — that is [`commit_part`]'s job, and
/// keeping the two apart is what lets a caller verify the bytes that are
/// about to land before they land.
///
/// A `.part` already sitting at that path is never deleted: after the suffix
/// is reserved on the writing side (`commit_part`'s resume protocol,
/// `safety::extract` refusing `.part`-named members), a file there is either
/// our own interrupted write or something we did not put there, and the two
/// cannot be told apart — so this refuses instead of destroying it
/// ([`AtomicError::StalePart`]). The check goes through `symlink_metadata`
/// so a planted symlink named `dest.part` is refused rather than followed or
/// deleted. On a failure of our own fresh write the `.part` is removed
/// again, so only success leaves one behind. Hash verification stays with
/// the caller: this module only makes the commit atomic.
///
/// Returns the bytes written.
pub fn write_part(dest: &Path, src: &mut impl Read) -> Result<u64, AtomicError> {
    let part = part_path(dest);
    if part.symlink_metadata().is_ok() {
        return Err(AtomicError::StalePart(part));
    }
    let result = write_inner(&part, src);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Write `src` to `dest` atomically via a sibling `.part` file.
///
/// Atomicity: bytes go to `dest.part`, `sync_all`, then rename, so the
/// rename stays on one volume. A `.part` already at `dest.part` is never
/// removed: it may be an interrupted write of this same destination — or a
/// foreign file the caller knows nothing about — and they are
/// indistinguishable from here ([`AtomicError::StalePart`]). The check goes
/// through `symlink_metadata` so a planted symlink named `dest.part` is
/// refused rather than followed or deleted. On a failure of our own fresh
/// write the `.part` is removed again, so only success leaves `dest`.
/// Returns the bytes written.
pub fn write_atomic(dest: &Path, src: &mut impl Read) -> Result<u64, AtomicError> {
    let part = part_path(dest);
    let bytes = write_part(dest, src)?;
    if let Err(e) = commit_part(&part, dest) {
        // The part is ours - we wrote it a moment ago - so a failed rename
        // must not leave it behind for the next run to trip over.
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    Ok(bytes)
}

/// Write `src` to `dest` atomically through a fresh, unique sibling temp.
///
/// Same commit as [`write_atomic`] - sync, then rename - but the intermediate
/// name carries a UUID instead of the fixed `.part`. Use this for files that
/// are rewritten in place and have no resume story (the `profile.json`
/// mirror): a temp left behind by a crashed run has a different name, so it
/// is never in the way and never has to be attributed. `.part` stays reserved
/// for transfers that genuinely resume.
///
/// The temp is created with `create_new` so a planted link at that name is
/// refused rather than followed, and it is removed on any failure. Returns
/// the bytes written.
pub fn write_atomic_unique(dest: &Path, src: &mut impl Read) -> Result<u64, AtomicError> {
    let temp = unique_temp_path(dest);
    let result = write_new(&temp, src).and_then(|bytes| {
        commit_part(&temp, dest)?;
        Ok(bytes)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
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

fn write_inner(part: &Path, src: &mut impl Read) -> Result<u64, AtomicError> {
    let f = std::fs::File::create(part).map_err(|e| io_err("create", part, e))?;
    write_stream(f, part, src)
}

/// Create a brand-new temp: `create_new` refuses a pre-existing file (or a
/// symlink planted at that name) instead of truncating through it.
fn write_new(part: &Path, src: &mut impl Read) -> Result<u64, AtomicError> {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(part)
        .map_err(|e| io_err("create", part, e))?;
    write_stream(f, part, src)
}

fn write_stream(
    mut f: std::fs::File,
    part: &Path,
    src: &mut impl Read,
) -> Result<u64, AtomicError> {
    let bytes = std::io::copy(src, &mut f).map_err(|e| io_err("write", part, e))?;
    f.sync_all().map_err(|e| io_err("sync", part, e))?;
    drop(f);
    Ok(bytes)
}

/// `.<name>.<uuid>.tmp` beside `dest`, so the rename stays on one volume.
fn unique_temp_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let temp = format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple());
    match dest.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(temp),
        _ => PathBuf::from(temp),
    }
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
    fn spec_f021_write_part_stops_at_the_part_and_commit_part_renames_it() {
        // The two halves of the protocol, used apart: the bytes land in the
        // part and the destination is still empty, so a caller can look at
        // them before they become the file.
        let dir = crate::test_tempdir();
        let dest = dir.path().join("a.jar");
        let bytes = write_part(&dest, &mut Cursor::new(b"verified bytes")).expect("writes");
        assert_eq!(bytes, 14);
        assert!(!dest.exists(), "the destination must not exist yet");
        let part = part_path(&dest);
        assert!(part.is_file(), "the part holds the bytes");
        assert_eq!(std::fs::read(&part).expect("read"), b"verified bytes");

        commit_part(&part, &dest).expect("commits");
        assert_eq!(std::fs::read(&dest).expect("read"), b"verified bytes");
        assert!(!part.exists(), "the part is renamed, not copied");
    }

    #[test]
    fn spec_f021_write_part_refuses_a_part_it_cannot_attribute() {
        // The same refusal the composed write has: a part that is not ours is
        // never deleted, and the write never happens.
        let dir = crate::test_tempdir();
        let dest = dir.path().join("a.jar");
        std::fs::write(part_path(&dest), b"foreign").expect("fixture");
        let err = write_part(&dest, &mut Cursor::new(b"fresh")).expect_err("refused");
        assert!(matches!(err, AtomicError::StalePart(_)), "got {err:?}");
        assert_eq!(
            std::fs::read(part_path(&dest)).expect("read"),
            b"foreign",
            "the foreign part survives untouched"
        );
        assert!(!dest.exists());
    }

    #[test]
    fn spec_f021_write_part_failure_leaves_no_part() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let dir = crate::test_tempdir();
        let dest = dir.path().join("a.jar");
        assert!(write_part(&dest, &mut Failing).is_err());
        assert!(
            !part_path(&dest).exists(),
            "our own failed write must not leave a part behind"
        );
    }

    #[test]
    fn spec_f021_write_atomic_never_deletes_a_part_it_did_not_write() {
        // `<dest>.part` sitting next to the real cannot be told apart from a
        // foreign file: the old code removed it and took its place, silently
        // destroying whatever the user (or an interrupted peer) had there.
        // Refusing is the honest failure.
        let dir = crate::test_tempdir();
        let dest = dir.path().join("a.jar");
        std::fs::write(part_path(&dest), b"foreign bytes").expect("fixture");
        let err = write_atomic(&dest, &mut Cursor::new(b"fresh"))
            .expect_err("a foreign .part must be refused, not deleted");
        assert!(matches!(err, AtomicError::StalePart(_)), "got {err:?}");
        assert!(
            !dest.exists(),
            "no write may have happened through a refused part path"
        );
        assert_eq!(
            std::fs::read(part_path(&dest)).expect("read"),
            b"foreign bytes",
            "the foreign .part must survive untouched"
        );
    }

    #[test]
    fn spec_f021_write_atomic_replaces_target_and_leaves_no_part() {
        let dir = crate::test_tempdir();
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
        let dir = crate::test_tempdir();
        let dest = dir.path().join("a.jar");
        std::fs::write(&dest, b"old").expect("fixture");
        assert!(write_atomic(&dest, &mut Failing).is_err());
        assert_eq!(std::fs::read(&dest).expect("read"), b"old");
        assert!(!part_path(&dest).exists(), "no .part left behind");
    }

    #[test]
    fn spec_f021_write_atomic_refuses_a_part_it_cannot_attribute() {
        // The old contract ("a stale .part from a killed run is removed
        // first") also removed files we did not write: an archive member
        // named `x.jar.part` extracted earlier, or anything the user put
        // there. Neither can be told apart from our own interrupted write,
        // so refusal is the honest failure.
        let dir = crate::test_tempdir();
        let dest = dir.path().join("a.jar");
        std::fs::write(part_path(&dest), b"stale").expect("fixture");
        let err = write_atomic(&dest, &mut Cursor::new(b"fresh"))
            .expect_err("a pre-existing .part must be refused");
        assert!(matches!(err, AtomicError::StalePart(_)), "got {err:?}");
        assert_eq!(
            std::fs::read(part_path(&dest)).expect("read"),
            b"stale",
            "the part content survives untouched"
        );
        assert!(
            !dest.exists(),
            "dest must not be created through the refusal"
        );
    }

    #[test]
    fn spec_f021_write_atomic_unique_ignores_a_stale_part() {
        // The mirror is rewritten on every registry change. With the fixed
        // `.part` a single crashed run made every later write refuse, so the
        // file could never be updated again. The unique temp does not look at
        // the part: it neither uses nor deletes it.
        let dir = crate::test_tempdir();
        let dest = dir.path().join("profile.json");
        std::fs::write(part_path(&dest), b"foreign part").expect("fixture");
        let bytes =
            write_atomic_unique(&dest, &mut Cursor::new(b"new mirror")).expect("must not block");
        assert_eq!(bytes, 10);
        assert_eq!(std::fs::read(&dest).expect("read"), b"new mirror");
        assert_eq!(
            std::fs::read(part_path(&dest)).expect("read"),
            b"foreign part",
            "a part this call did not write survives untouched"
        );
        let temps: Vec<String> = std::fs::read_dir(dir.path())
            .expect("ls")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(temps.is_empty(), "temporaries left: {temps:?}");
    }

    #[test]
    fn spec_f021_write_atomic_unique_failure_leaves_no_temp_and_keeps_target() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let dir = crate::test_tempdir();
        let dest = dir.path().join("profile.json");
        std::fs::write(&dest, b"old").expect("fixture");
        assert!(write_atomic_unique(&dest, &mut Failing).is_err());
        assert_eq!(std::fs::read(&dest).expect("read"), b"old");
        let temps: Vec<String> = std::fs::read_dir(dir.path())
            .expect("ls")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(temps.is_empty(), "temporaries left: {temps:?}");
    }
}
