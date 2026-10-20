//! Reading a `.mrpack` archive: inspect it under limits, read the index out
//! of it, and turn that into an install plan.
//!
//! Nothing here writes to an instance. The plan is the deliverable, for the
//! same reason every other mutation in this project is a plan first: a pack
//! is untrusted input, and the user should see what it wants before any of it
//! happens. `AGENTS.md` §5.4.
//!
//! The bomb-policy half of the safety rules is not this module's to invent —
//! `inspect_zip` already refuses symlinks, duplicates, encrypted members,
//! excessive counts, oversized members and absurd compression ratios, and a
//! pack that survives it is bounded by construction. What is this module's
//! job is the *format*-specific part: which member is the index, and what the
//! index asks to be done.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek};

use zip::ZipArchive;

use super::mrpack::{parse_index, reserved_destination, IndexError, Pack, Reserved};
use crate::safety::inspect::{inspect_zip, read_limited, ArchiveLimits, MemberInfo};
use crate::safety::path::safe_member_name;

/// An index large enough to be suspicious, small enough to read.
pub const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("archive could not be opened: {0}")]
    Zip(String),
    #[error(transparent)]
    Inspect(#[from] crate::safety::inspect::InspectError),
    #[error("the archive has no {0} at its root")]
    MissingIndex(&'static str),
    #[error("the index is {size} bytes, over the {limit} byte ceiling")]
    IndexTooLarge { size: u64, limit: u64 },
    #[error(transparent)]
    Index(#[from] IndexError),
    // NameError is Debug-only, same as in mrpack.rs, so it cannot be a
    // transparent source.
    #[error("override member {path:?} has an unsafe name: {reason:?}")]
    Name {
        path: String,
        reason: crate::safety::path::NameError,
    },
    #[error("pack file {path:?} would land outside the instance")]
    OutsideInstance { path: String },
    // The three checks below also run in `parse_index`. They run again here
    // because `plan_install` accepts a `Pack` that a caller could have built
    // without going through the parser.
    #[error("pack destination {path:?} ends with '/'; a pack file is never a directory")]
    DirectoryPath { path: String },
    #[error("pack destination {path:?} would overwrite the application-managed file {name:?}")]
    ManagedPath { path: String, name: String },
    #[error("pack destination {path:?} targets the installer's staging directory")]
    StagingPath { path: String },
}

/// One thing the pack wants done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOp {
    /// Fetch this file by hash, verify it, and place it at `safe_path`.
    Download {
        safe_path: String,
        sha512: String,
        size: Option<u64>,
    },
    /// Copy an override that is already inside the archive.
    ///
    /// `source_layer` is the layer folder the winning copy lives under. The
    /// plan names it instead of leaving the executor to re-derive it: with
    /// several layers holding the same path, only this field says which bytes
    /// the user was shown.
    CopyOverride {
        safe_path: String,
        source_layer: String,
    },
}

/// The whole of what a pack asks for, before any of it is done.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstallPlan {
    pub ops: Vec<InstallOp>,
}

impl InstallPlan {
    pub fn downloads(&self) -> usize {
        self.ops
            .iter()
            .filter(|o| matches!(o, InstallOp::Download { .. }))
            .count()
    }
    pub fn overrides(&self) -> usize {
        self.ops
            .iter()
            .filter(|o| matches!(o, InstallOp::CopyOverride { .. }))
            .count()
    }
}

/// Opens a `.mrpack`, checks it, and reads its index.
pub fn read_pack<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    limits: &ArchiveLimits,
) -> Result<(Pack, Vec<MemberInfo>), PackError> {
    read_pack_bounded(archive, limits, MAX_INDEX_BYTES)
}

/// The index ceiling is a parameter so it can be tested without building an
/// eight-megabyte fixture. `read_pack` is what production calls.
pub fn read_pack_bounded<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    limits: &ArchiveLimits,
    max_index_bytes: u64,
) -> Result<(Pack, Vec<MemberInfo>), PackError> {
    let inventory = inspect_zip(archive, limits)?;

    let at = inventory
        .iter()
        .find(|m| !m.is_dir && m.name.eq_ignore_ascii_case(super::mrpack::INDEX_ENTRY))
        .ok_or(PackError::MissingIndex(super::mrpack::INDEX_ENTRY))?;
    if at.size > max_index_bytes {
        return Err(PackError::IndexTooLarge {
            size: at.size,
            limit: max_index_bytes,
        });
    }

    let bytes = read_limited(archive, &inventory, at.index, max_index_bytes)
        .map_err(|e| PackError::Zip(format!("reading the index failed: {e}")))?;
    let pack = parse_index(&bytes)?;
    Ok((pack, inventory))
}

/// Convenience for callers holding the whole archive in memory.
pub fn read_pack_bytes(bytes: &[u8], limits: &ArchiveLimits) -> Result<Pack, PackError> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| PackError::Zip(e.to_string()))?;
    read_pack(&mut zip, limits).map(|(pack, _)| pack)
}

/// Turns a pack plus the archive inventory into the work list.
///
/// Override files are the archive members that sit under a declared layer.
/// The layer order matters and is the format's, not ours: client, then
/// overrides, then server, each overwriting the last.
///
/// The inventory is in central-directory order, which says nothing about
/// precedence. Iterating the layers first and the members second is what makes
/// the collapse follow the format: a pack that lists `server-overrides` before
/// `overrides` in the zip still gets the server file, because the layer loop
/// visits `overrides` first and inserts the server value last.
pub fn plan_install(pack: &Pack, inventory: &[MemberInfo]) -> Result<InstallPlan, PackError> {
    let mut overrides: BTreeMap<String, InstallOp> = BTreeMap::new();
    for layer in &pack.override_layers {
        // Defence in depth: `parse_index` already strips this, but a `Pack`
        // can be built by hand and handed to this function.
        let layer = layer.strip_suffix('/').unwrap_or(layer);
        if layer.is_empty() {
            continue;
        }
        let layer = safe_member_name(layer).map_err(|reason| PackError::Name {
            path: layer.to_string(),
            reason,
        })?;
        let prefix = format!("{layer}/");
        for member in inventory {
            if member.is_dir {
                continue;
            }
            let Some(rest) = member.name.strip_prefix(&prefix) else {
                continue;
            };
            let safe = safe_member_name(rest).map_err(|reason| PackError::Name {
                path: member.name.clone(),
                reason,
            })?;
            // A file member whose name ends in '/' is a directory entry that
            // the inspector did not classify as one; it has no bytes to copy.
            if safe.ends_with('/') {
                continue;
            }
            check_reserved(&safe, &member.name)?;
            overrides.insert(
                safe.clone(),
                InstallOp::CopyOverride {
                    safe_path: safe,
                    source_layer: layer.clone(),
                },
            );
        }
    }

    let mut ops = Vec::with_capacity(pack.files.len() + overrides.len());
    for file in &pack.files {
        if file.safe_path.ends_with('/') {
            return Err(PackError::DirectoryPath {
                path: file.safe_path.clone(),
            });
        }
        check_reserved(&file.safe_path, &file.safe_path)?;
        // An override for the same destination wins. The format applies the
        // override layers after the index files, and the bundled bytes are the
        // author's final word: the file is in the archive the user chose, so a
        // later download of the same path could only undo that choice. The
        // winner is a `CopyOverride`, so the executor never fetches a file it
        // would immediately replace.
        if overrides.contains_key(&file.safe_path) {
            continue;
        }
        ops.push(InstallOp::Download {
            safe_path: file.safe_path.clone(),
            sha512: file.sha512.clone(),
            size: file.file_size,
        });
    }
    ops.extend(overrides.into_values());

    Ok(InstallPlan { ops })
}

fn check_reserved(safe_path: &str, original: &str) -> Result<(), PackError> {
    match reserved_destination(safe_path) {
        Some(Reserved::ManagedFile(name)) => Err(PackError::ManagedPath {
            path: original.to_string(),
            name: name.to_string(),
        }),
        Some(Reserved::Staging) => Err(PackError::StagingPath {
            path: original.to_string(),
        }),
        None => Ok(()),
    }
}
