//! Safe archive extraction (`F-041`–`F-044`, `F-047`, `F-049` at the extract layer).
//!
//! Plan-first: [`inspect::inspect_zip`] validates everything before the first
//! byte is written, so a rejected archive leaves the destination untouched.
//! Members come only from the validated inventory, by index. No symlink is
//! ever created; a pre-existing symlink inside the destination is refused.

use std::io::{Read, Seek};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use thiserror::Error;
use zip::ZipArchive;

use crate::safety::atomic::{self, AtomicError};
use crate::safety::inspect::{ArchiveLimits, InspectError, MemberInfo};

/// Windows MAX_PATH: joined paths beyond it fail mid-tree.
const MAX_JOINED_PATH_LEN: usize = 260;

/// What extraction wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractReport {
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error(transparent)]
    Inspect(#[from] InspectError),
    #[error(transparent)]
    Atomic(#[from] AtomicError),
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cancelled")]
    Cancelled,
    #[error("path escapes destination: {}", .0.display())]
    Escape(PathBuf),
    #[error("joined path too long: {}", .0.display())]
    TooLong(PathBuf),
    #[error("empty archive (F-049)")]
    Empty,
}

/// A pre-existing symlink or reparse point (e.g. NTFS junction) inside the
/// destination would redirect writes outside it. `is_symlink` alone misses
/// junctions, so on Windows the reparse-point attribute is checked too.
fn is_link_or_reparse(p: &Path) -> bool {
    match p.symlink_metadata() {
        Ok(m) => {
            if m.file_type().is_symlink() {
                return true;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
                (m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
        Err(_) => false,
    }
}

fn io_err(op: &'static str, path: &Path, source: std::io::Error) -> ExtractError {
    ExtractError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// Fails reads past `limit` bytes: actual decompressed bytes must never
/// exceed the declared member size (lying central-directory headers).
struct Capped<R> {
    inner: R,
    remaining: u64,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Err(std::io::Error::other("member exceeds declared size"));
        }
        let cap = self.remaining.min(buf.len() as u64) as usize;
        let n = self.inner.read(&mut buf[..cap])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// Extract a validated inventory into `dest` (`F-041`–`F-044`, `F-047`, `F-049`).
///
/// Plan-first: [`inspect::inspect_zip`] runs before the first write, so a
/// rejected archive leaves the destination untouched. Members come only from
/// the inventory, by index; no symlink is ever created, and a pre-existing
/// symlink inside the destination is refused.
pub fn extract_safely<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    dest: &Path,
    limits: &ArchiveLimits,
    on_member: &mut dyn FnMut(&MemberInfo) -> ControlFlow<()>,
) -> Result<ExtractReport, ExtractError> {
    let inventory = crate::safety::inspect::inspect_zip(archive, limits)?;
    if inventory.is_empty() {
        return Err(ExtractError::Empty);
    }
    std::fs::create_dir_all(dest).map_err(|e| io_err("create_dir", dest, e))?;
    let mut report = ExtractReport::default();
    for member in &inventory {
        if on_member(member).is_break() {
            return Err(ExtractError::Cancelled);
        }
        let target = dest.join(&member.name);
        // Defense in depth: names are validated, but re-check containment here.
        if !target.starts_with(dest) {
            return Err(ExtractError::Escape(target));
        }
        if target.as_os_str().len() > MAX_JOINED_PATH_LEN {
            return Err(ExtractError::TooLong(target));
        }
        // Refuse to write through a pre-existing symlink or junction (TOCTOU).
        let mut anc = target.parent();
        while let Some(a) = anc {
            if a == dest {
                break;
            }
            if is_link_or_reparse(a) {
                return Err(ExtractError::Escape(a.to_path_buf()));
            }
            anc = a.parent();
        }
        if member.is_dir {
            std::fs::create_dir_all(&target).map_err(|e| io_err("create_dir", &target, e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("create_dir", parent, e))?;
        }
        let reader = archive
            .by_index(member.index)
            .map_err(|e| io_err("open", &target, e.into()))?;
        // Cap actual bytes at declared size + 1: a lying header must fail
        // here, not decompress unbounded into the destination. Note zip's own
        // reader also rejects over-declared streams; either way no file lands.
        let mut capped = Capped {
            inner: reader,
            remaining: member.size.saturating_add(1),
        };
        match atomic::write_atomic(&target, &mut capped) {
            Ok(bytes) => {
                report.files += 1;
                report.bytes += bytes;
            }
            // The stream met or exceeded the declared size when trouble hit:
            // the central-directory sizes lied.
            Err(_) if capped.remaining <= 1 => {
                return Err(ExtractError::Inspect(InspectError::BadRatio(
                    member.name.clone(),
                )));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a directory symlink with whichever API the platform has.
    ///
    /// Calling `std::os::windows` directly meant this file did not compile on
    /// Linux at all — the whole `check` job on ubuntu-22.04 had been failing with
    /// `cannot find 'windows' in 'os'` since 2026-10-17. The rule under test is not
    /// Windows-specific, so the test now runs on both systems rather than being
    /// gated away on one.
    #[cfg(windows)]
    fn make_symlink(src: &Path, dest: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(src, dest)
    }

    #[cfg(unix)]
    fn make_symlink(src: &Path, dest: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dest)
    }

    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn archive_of(entries: &[(&str, &[u8])]) -> ZipArchive<Cursor<Vec<u8>>> {
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            w.start_file(*name, SimpleFileOptions::default())
                .expect("fixture");
            w.write_all(data).expect("fixture");
        }
        let buf = w.finish().expect("fixture").into_inner();
        ZipArchive::new(Cursor::new(buf)).expect("fixture opens")
    }

    fn no_parts_left(dest: &Path) {
        let mut leftovers = Vec::new();
        for e in walkdir(dest) {
            if e.extension().map(|x| x == "part").unwrap_or(false) {
                leftovers.push(e);
            }
        }
        assert!(leftovers.is_empty(), "stale .part files: {leftovers:?}");
    }

    fn walkdir(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let rd = std::fs::read_dir(&d).expect("fixture dir");
            for e in rd {
                let p = e.expect("fixture entry").path();
                if p.is_dir() {
                    stack.push(p.clone());
                }
                out.push(p);
            }
        }
        out
    }

    fn go(_m: &MemberInfo) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    #[test]
    fn spec_f042_extract_writes_members_and_reports() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut a = archive_of(&[("mods/a.jar", b"aaa"), ("config/c.cfg", b"cc")]);
        let rep =
            extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut go).expect("extracts");
        assert_eq!(rep.files, 2);
        assert_eq!(rep.bytes, 5);
        assert_eq!(
            std::fs::read(dest.join("mods/a.jar")).expect("read"),
            b"aaa"
        );
        no_parts_left(&dest);
    }

    #[test]
    fn spec_f041_extract_rejects_traversal_and_writes_nothing() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut a = archive_of(&[("../evil.txt", b"x")]);
        assert!(extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut go).is_err());
        assert!(!dest.exists() || walkdir(&dest).is_empty());
    }

    #[test]
    fn spec_f044_extract_never_creates_symlink() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        w.start_file(
            "link",
            SimpleFileOptions::default().external_attributes(0o120777 << 16),
        )
        .expect("fixture");
        w.write_all(b"target").expect("fixture");
        let buf = w.finish().expect("fixture").into_inner();
        let mut a = ZipArchive::new(Cursor::new(buf)).expect("fixture opens");
        let r = extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut go);
        assert!(matches!(r, Err(ExtractError::Inspect(_))));
        let link = dest.join("link");
        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "no symlink may exist"
        );
    }

    #[test]
    fn spec_f043_extract_enforces_limits_before_any_write() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut a = archive_of(&[("big.bin", &[0u8; 64])]);
        let lim = ArchiveLimits {
            max_member_size: 8,
            ..ArchiveLimits::default()
        };
        assert!(extract_safely(&mut a, &dest, &lim, &mut go).is_err());
        assert!(!dest.exists() || walkdir(&dest).is_empty());
    }

    #[test]
    fn spec_f049_extract_rejects_empty_archive() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut a = archive_of(&[]);
        assert_eq!(
            extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut go)
                .unwrap_err()
                .to_string(),
            ExtractError::Empty.to_string()
        );
    }

    #[test]
    fn spec_f042_extract_cancel_leaves_no_partial() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut a = archive_of(&[("one.txt", b"1"), ("two.txt", b"2")]);
        let mut calls = 0;
        let mut cb = |_m: &MemberInfo| {
            calls += 1;
            if calls >= 1 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        assert_eq!(
            extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut cb)
                .unwrap_err()
                .to_string(),
            ExtractError::Cancelled.to_string()
        );
        no_parts_left(&dest);
        // No truncated second file may exist.
        assert!(!dest.join("two.txt").exists());
    }

    #[test]
    fn spec_f044_extract_refuses_preexisting_dest_symlink() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        std::fs::create_dir_all(dest.join("sub")).expect("fixture");
        // Symlink creation needs privileges; skip honestly when unavailable.
        if make_symlink(dir.path(), &dest.join("sub/link")).is_err() {
            return;
        }
        let mut a = archive_of(&[("sub/link/evil.txt", b"x")]);
        let r = extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut go);
        assert!(matches!(r, Err(ExtractError::Escape(_))));
    }

    #[test]
    fn spec_f042_extract_rejects_lying_header_actual_over_declared() {
        // Central-directory size says 10 bytes, the deflated body is 4 KiB:
        // inspect passes on metadata, extract must cap actual bytes and fail.
        use zip::CompressionMethod;
        let data = vec![b'B'; 4096];
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        w.start_file(
            "lying.bin",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .expect("fixture");
        w.write_all(&data).expect("fixture");
        let mut buf = w.finish().expect("fixture").into_inner();
        let bytes = &mut buf;
        let mut i = 0;
        while i + 32 < bytes.len() {
            if bytes[i..].starts_with(b"PK\x01\x02") {
                bytes[i + 24..i + 28].copy_from_slice(&10u32.to_le_bytes());
                break;
            }
            i += 1;
        }
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut a = ZipArchive::new(Cursor::new(bytes.clone())).expect("fixture opens");
        let r = extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut go);
        // Either the crate's own size check (Io) or the Capped backstop
        // (BadRatio) must fire; what matters is no file and no .part land.
        assert!(r.is_err(), "lying header must fail, got {r:?}");
        assert!(!dest.join("lying.bin").exists());
        no_parts_left(&dest);
    }

    #[test]
    fn spec_f047_extract_rejects_reserved_names() {
        let dir = crate::test_tempdir();
        let dest = dir.path().join("out");
        let mut a = archive_of(&[("CON.txt", b"x")]);
        assert!(extract_safely(&mut a, &dest, &ArchiveLimits::default(), &mut go).is_err());
        assert!(!dest.exists() || walkdir(&dest).is_empty());
    }
}
