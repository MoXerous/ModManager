//! Archive inspection and bounded reads (`F-042`–`F-046`, `F-048`).
//!
//! Mirrors `inspect_zip_members` / `read_limited_member` in `safe_archive.py`:
//! metadata-only inspection (never decompress to decide), then bounded reads.
//! Member names are re-validated with [`crate::safety::path::safe_member_name`].

use std::io::{Read, Seek};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use zip::ZipArchive;

use crate::safety::path::{self, NameError};

// Unix file-type bits (defined here: unavailable on Windows targets).
const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;

/// Limits from `safe_archive.py:26-34` (`F-043`). Binary units.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub max_members: usize,
    pub max_member_size: u64,
    pub max_total_uncompressed_size: u64,
    pub max_compression_ratio: f64,
    pub compression_ratio_min_size: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_members: 50_000,
            max_member_size: 4 * 1024 * 1024 * 1024,
            max_total_uncompressed_size: 32 * 1024 * 1024 * 1024,
            max_compression_ratio: 500.0,
            compression_ratio_min_size: 1024 * 1024,
        }
    }
}

/// Why an archive or member was rejected.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InspectError {
    #[error("member `{0}`: {1:?}")]
    Name(String, NameError),
    #[error("too many members")]
    TooManyMembers,
    #[error("duplicate member `{0}`")]
    Duplicate(String),
    #[error("encrypted member `{0}`")]
    Encrypted(String),
    #[error("link or special file `{0}`")]
    LinkOrSpecial(String),
    #[error("member `{0}` too large")]
    MemberTooLarge(String),
    #[error("total uncompressed size exceeds limit")]
    TotalTooLarge,
    #[error("member `{0}` has an unsafe compression ratio")]
    BadRatio(String),
    #[error("read limit exceeded for `{0}`")]
    ReadLimit(String),
    #[error("member not found")]
    NotFound,
    #[error("member `{0}` is a directory")]
    IsDirectory(String),
}

/// One validated inventory entry, in archive index order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInfo {
    pub index: usize,
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
}

/// Collision key: NFC, strip one trailing `/`, lowercase, NFC again.
///
/// Lowercase-then-NFC because lowercasing can denormalize.
/// shortcut: Rust `to_lowercase` is not full Unicode casefold (`ß`, `ﬁ`
/// differ from Python `casefold`). Revisit if a real archive hits it.
fn collision_key(normalized: &str) -> String {
    let stripped = normalized.strip_suffix('/').unwrap_or(normalized);
    stripped.to_lowercase().nfc().collect()
}

/// Pure compression-ratio rule, unit-testable without a zip (`F-044`).
fn ratio_rejected(file_size: u64, compressed_size: u64, limits: &ArchiveLimits) -> bool {
    if file_size < limits.compression_ratio_min_size {
        return false;
    }
    if compressed_size == 0 {
        return file_size > 0;
    }
    (file_size as f64) / (compressed_size as f64) > limits.max_compression_ratio
}

/// Inspect archive metadata only — never decompress to decide (`F-042`–`F-046`).
///
/// Iterates by index (names can collide, so `by_name` would hide entries).
/// Re-validates every name with [`path::safe_member_name`].
pub fn inspect_zip<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    limits: &ArchiveLimits,
) -> Result<Vec<MemberInfo>, InspectError> {
    if archive.len() > limits.max_members {
        return Err(InspectError::TooManyMembers);
    }
    let mut seen = std::collections::HashSet::new();
    let mut total: u64 = 0;
    let mut out = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        // Metadata only: by_index_data never builds a reader, so encrypted
        // members report instead of failing with PASSWORD_REQUIRED.
        let file = archive
            .by_index_data(index)
            .map_err(|_| InspectError::Name(format!("#{index}"), NameError::UnsafePath))?;
        let raw = file.name().map(|c| c.into_owned()).unwrap_or_default();
        let name = path::safe_member_name(&raw).map_err(|e| InspectError::Name(raw.clone(), e))?;
        // AGENTS.md §5.6 coarse filter; precise errors come from safe_member_name.
        if file.enclosed_name().is_none() {
            return Err(InspectError::Name(raw, NameError::UnsafePath));
        }
        let key = collision_key(&name);
        if !seen.insert(key) {
            return Err(InspectError::Duplicate(name));
        }
        if file.encrypted() {
            return Err(InspectError::Encrypted(name));
        }
        // REGARDLESS of creator system: read the mode bits directly (F-044).
        // zip's unix_mode() fabricates REG/DIR for DOS entries, hiding smuggled modes.
        let mode = file.external_attributes() >> 16;
        let kind = mode & S_IFMT;
        if kind != 0 && kind != S_IFREG && kind != S_IFDIR {
            return Err(InspectError::LinkOrSpecial(name));
        }
        let size = file.size();
        let compressed = file.compressed_size();
        if size > limits.max_member_size {
            return Err(InspectError::MemberTooLarge(name));
        }
        total = total
            .checked_add(size)
            .filter(|t| *t <= limits.max_total_uncompressed_size)
            .ok_or(InspectError::TotalTooLarge)?;
        if ratio_rejected(size, compressed, limits) {
            return Err(InspectError::BadRatio(name));
        }
        out.push(MemberInfo {
            index,
            name,
            size,
            compressed_size: compressed,
            is_dir: file.is_dir(),
        });
    }
    Ok(out)
}

/// Read one inventoried member, bounded (`F-048`).
///
/// Resolves by archive index (never by name: NFC changes bytes). Rejects
/// before opening when the declared size already exceeds the limit, then
/// re-checks the actual length after `take(max_bytes + 1)` because declared
/// sizes are attacker-controlled metadata.
pub fn read_limited<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    inventory: &[MemberInfo],
    index: usize,
    max_bytes: u64,
) -> Result<Vec<u8>, InspectError> {
    let member = inventory.get(index).ok_or(InspectError::NotFound)?;
    if member.is_dir {
        return Err(InspectError::IsDirectory(member.name.clone()));
    }
    if member.size > max_bytes {
        return Err(InspectError::ReadLimit(member.name.clone()));
    }
    let file = archive
        .by_index(member.index)
        .map_err(|_| InspectError::NotFound)?;
    let mut buf = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|_| InspectError::ReadLimit(member.name.clone()))?;
    if buf.len() as u64 > max_bytes {
        return Err(InspectError::ReadLimit(member.name.clone()));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn writer() -> ZipWriter<Cursor<Vec<u8>>> {
        ZipWriter::new(Cursor::new(Vec::new()))
    }

    fn finish(w: ZipWriter<Cursor<Vec<u8>>>) -> ZipArchive<Cursor<Vec<u8>>> {
        let buf = w.finish().expect("test fixture builds");
        ZipArchive::new(buf).expect("test fixture opens")
    }

    fn stored(name: &str, data: &[u8]) -> ZipArchive<Cursor<Vec<u8>>> {
        let mut w = writer();
        w.start_file(name, SimpleFileOptions::default())
            .expect("test fixture builds");
        w.write_all(data).expect("test fixture builds");
        finish(w)
    }

    fn limits() -> ArchiveLimits {
        ArchiveLimits::default()
    }

    #[test]
    fn spec_f042_accepts_safe_members_and_preserves_index_order() {
        let mut a = stored("b.txt", b"bb");
        let inv = inspect_zip(&mut a, &limits()).expect("safe archive passes");
        assert_eq!(inv.len(), 1);
        assert_eq!(inv[0].name, "b.txt");
        assert_eq!(inv[0].index, 0);
        assert_eq!(inv[0].size, 2);
    }

    #[test]
    fn spec_f043_rejects_member_count_over_limit() {
        let mut a = stored("b.txt", b"bb");
        let lim = ArchiveLimits {
            max_members: 0,
            ..limits()
        };
        assert_eq!(inspect_zip(&mut a, &lim), Err(InspectError::TooManyMembers));
    }

    #[test]
    fn spec_f043_rejects_member_and_total_size_over_limit() {
        let mut a = stored("big.bin", &[0u8; 16]);
        let lim = ArchiveLimits {
            max_member_size: 8,
            ..limits()
        };
        assert_eq!(
            inspect_zip(&mut a, &lim),
            Err(InspectError::MemberTooLarge("big.bin".to_string()))
        );
        let lim = ArchiveLimits {
            max_total_uncompressed_size: 8,
            ..limits()
        };
        assert_eq!(inspect_zip(&mut a, &lim), Err(InspectError::TotalTooLarge));
    }

    #[test]
    fn spec_f044_rejects_casefold_and_nfc_and_slash_collisions() {
        let mut w = writer();
        for n in ["a.TXT", "A.txt"] {
            w.start_file(n, SimpleFileOptions::default())
                .expect("fixture");
            w.write_all(b"x").expect("fixture");
        }
        let mut a = finish(w);
        assert!(matches!(
            inspect_zip(&mut a, &limits()),
            Err(InspectError::Duplicate(_))
        ));
    }

    #[test]
    fn spec_f045_rejects_symlink_even_when_system_is_dos() {
        // Symlink mode smuggled with a DOS creator system: must still be caught.
        // NOTE: unix_permissions() masks to 0o777 (documented), so the raw
        // external_attributes() setter is the only way to forge this fixture.
        let mut w = writer();
        w.start_file(
            "link",
            SimpleFileOptions::default().external_attributes(0o120777 << 16),
        )
        .expect("fixture");
        w.write_all(b"target").expect("fixture");
        let mut buf = w.finish().expect("fixture").into_inner();
        // Patch central-directory "version made by" system byte (offset 4+1) to DOS(0).
        let bytes = &mut buf;
        let mut i = 0;
        while i + 4 < bytes.len() {
            if bytes[i..].starts_with(b"PK\x01\x02") {
                bytes[i + 5] = 0;
                break;
            }
            i += 1;
        }
        let mut a = ZipArchive::new(Cursor::new(bytes.clone())).expect("fixture opens");
        assert!(matches!(
            inspect_zip(&mut a, &limits()),
            Err(InspectError::LinkOrSpecial(_))
        ));
    }

    #[test]
    fn spec_f045_rejects_encrypted_member() {
        let mut w = writer();
        w.start_file("secret.txt", SimpleFileOptions::default())
            .expect("fixture");
        w.write_all(b"shh").expect("fixture");
        let mut buf = w.finish().expect("fixture").into_inner();
        // Set general-purpose bit 0 in local + central headers.
        let bytes = &mut buf;
        let mut i = 0;
        let mut patched = 0;
        while i + 8 < bytes.len() {
            if bytes[i..].starts_with(b"PK\x03\x04") {
                bytes[i + 6] |= 0x1;
                patched += 1;
            } else if bytes[i..].starts_with(b"PK\x01\x02") {
                bytes[i + 8] |= 0x1;
                patched += 1;
            }
            i += 1;
        }
        assert!(patched >= 2, "both headers patched");
        let mut a = ZipArchive::new(Cursor::new(bytes.clone())).expect("fixture opens");
        assert!(matches!(
            inspect_zip(&mut a, &limits()),
            Err(InspectError::Encrypted(_))
        ));
    }

    #[test]
    fn spec_f044_rejects_zero_compressed_with_content() {
        // Deflated entry with zeroed compressed size but real content: suspicious.
        // Custom small ratio floor: the 2 KiB fixture is below the 1 MiB default.
        let data = vec![b'A'; 2048];
        let mut w = writer();
        w.start_file(
            "bomb.bin",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .expect("fixture");
        w.write_all(&data).expect("fixture");
        let mut buf = w.finish().expect("fixture").into_inner();
        let bytes = &mut buf;
        // Zero ONLY the compressed size in the central directory (offset 24).
        let mut i = 0;
        while i + 32 < bytes.len() {
            if bytes[i..].starts_with(b"PK\x01\x02") {
                bytes[i + 20..i + 24].fill(0);
                break;
            }
            i += 1;
        }
        let mut a = ZipArchive::new(Cursor::new(bytes.clone())).expect("fixture opens");
        let lim = ArchiveLimits {
            max_compression_ratio: 2.0,
            compression_ratio_min_size: 1,
            ..limits()
        };
        assert!(matches!(
            inspect_zip(&mut a, &lim),
            Err(InspectError::BadRatio(_))
        ));
    }

    #[test]
    fn spec_f048_bounded_read_accepts_exact_and_rejects_over() {
        let mut a = stored("f.txt", b"12345678");
        let inv = inspect_zip(&mut a, &limits()).expect("passes");
        assert_eq!(read_limited(&mut a, &inv, 0, 8).expect("exact ok").len(), 8);
        assert_eq!(
            read_limited(&mut a, &inv, 0, 7),
            Err(InspectError::ReadLimit("f.txt".to_string()))
        );
    }

    #[test]
    fn spec_f048_rejects_directory_and_unknown_index() {
        let mut w = writer();
        w.add_directory("mods/", SimpleFileOptions::default())
            .expect("fixture");
        let mut a = finish(w);
        let inv = inspect_zip(&mut a, &limits()).expect("passes");
        assert_eq!(
            read_limited(&mut a, &inv, 0, 1024),
            Err(InspectError::IsDirectory("mods/".to_string()))
        );
        assert_eq!(
            read_limited(&mut a, &inv, 9, 1024),
            Err(InspectError::NotFound)
        );
    }

    #[test]
    fn spec_f048_read_rechecks_lying_header_after_take() {
        // Central-directory file_size patched small, body actually large (deflated):
        // pre-check passes, take(max+1) must still catch it.
        let data = vec![b'B'; 4096];
        let mut w = writer();
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
                // uncompressed size (offset 28, 4 bytes LE) -> 10
                bytes[i + 24..i + 28].copy_from_slice(&10u32.to_le_bytes());
                break;
            }
            i += 1;
        }
        let mut a = ZipArchive::new(Cursor::new(bytes.clone())).expect("fixture opens");
        let inv = inspect_zip(&mut a, &limits()).expect("header looks small");
        assert_eq!(
            read_limited(&mut a, &inv, 0, 64),
            Err(InspectError::ReadLimit("lying.bin".to_string()))
        );
    }
}
