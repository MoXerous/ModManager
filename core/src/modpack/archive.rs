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

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Seek};

use zip::ZipArchive;

use super::mrpack::{parse_index, IndexError, Pack};
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
    CopyOverride { safe_path: String },
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
pub fn plan_install(pack: &Pack, inventory: &[MemberInfo]) -> Result<InstallPlan, PackError> {
    let layers: BTreeSet<&str> = pack.override_layers.iter().map(String::as_str).collect();
    let mut ops: Vec<InstallOp> = pack
        .files
        .iter()
        .map(|f| InstallOp::Download {
            safe_path: f.safe_path.clone(),
            sha512: f.sha512.clone(),
            size: f.file_size,
        })
        .collect();

    // Later layers overwrite earlier ones, so a path seen twice is one file
    // in the plan, not two. `BTreeMap::insert` keeps the *first* position and
    // takes the *last* value, which is exactly what a layer stack should do
    // to a listing: the order the user reads stays stable, the content is the
    // one that would actually win.
    let mut overrides: BTreeMap<String, InstallOp> = BTreeMap::new();
    for member in inventory {
        if member.is_dir {
            continue;
        }
        let Some((layer, rest)) = member.name.split_once('/') else {
            continue;
        };
        if !layers.contains(layer) {
            continue;
        }
        let safe = safe_member_name(rest).map_err(|reason| PackError::Name {
            path: member.name.clone(),
            reason,
        })?;
        overrides.insert(safe.clone(), InstallOp::CopyOverride { safe_path: safe });
    }
    ops.extend(overrides.into_values());

    Ok(InstallPlan { ops })
}
